"""Tests for the export module.

These assert what the Swift app / a DJ would observe: output files, exact
sample counts, 24-bit encoding, per-stem gain, shared anti-clip gain, and copied
tags. They never assert internal call sequences.

Each stem is ``(name, path, window, gain)``: an optional per-stem ``window`` on
the source timeline and a linear ``gain`` (1.0 == unchanged). The whole export
spans a caller-provided ``span``. A stem is silent outside its window, its gain
scales its buffer before the shared anti-clip pass, and everything stays aligned
on the original timeline.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
import pytest
import soundfile as sf
from mutagen.aiff import AIFF
from mutagen.id3 import TIT2, TPE1
from mutagen.wave import WAVE

from stemsplitter_engine.exporter import ExportError, export_stems

SR = 44100


# --- fixtures / helpers ------------------------------------------------------


def _write_stem(path: Path, data: np.ndarray, sr: int = SR) -> None:
    """Write a float32 stem WAV, subtype FLOAT so values round-trip exactly."""
    sf.write(str(path), data.astype("float32"), sr, subtype="FLOAT", format="WAV")


def _sine(n: int, freq: float = 220.0, amp: float = 0.5, sr: int = SR) -> np.ndarray:
    t = np.arange(n, dtype="float32") / sr
    return (amp * np.sin(2 * np.pi * freq * t)).astype("float32").reshape(-1, 1)


def _collect_progress():
    calls: list[tuple[str, float]] = []
    return calls, lambda stage, frac: calls.append((stage, frac))


def _peak(path: Path) -> float:
    data, _ = sf.read(str(path), always_2d=True, dtype="float32")
    return float(np.max(np.abs(data)))


# --- full-track (window=None) length + encoding ------------------------------


@pytest.mark.parametrize("fmt,ext,sf_format", [("aiff", "aiff", "AIFF"), ("wav", "wav", "WAV")])
def test_full_track_export_encoding(tmp_path, fmt, ext, sf_format):
    n = SR  # 1s
    stems = []
    for name in ("vocals", "instrumental"):
        p = tmp_path / f"{name}.wav"
        _write_stem(p, _sine(n, amp=0.4))
        stems.append((name, p, None, 1.0))  # full track
    source = tmp_path / "My Track.mp3"  # dummy; only .stem + tags used
    _, progress = _collect_progress()

    out = export_stems(
        stems, source, source_samples=n, span=(0, n), fmt=fmt,
        out_dir=tmp_path / "out", suffixes={"vocals": "Acapella", "instrumental": "Instrumental"},
        progress=progress,
    )

    assert len(out) == 2
    for p in out:
        assert p.exists()
        info = sf.info(str(p))
        assert info.frames == n  # span length
        assert info.subtype == "PCM_24"
        assert info.format == sf_format
        assert p.suffix == f".{ext}"


def test_sample_rate_preserved(tmp_path):
    n = 22050
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.3, sr=48000), sr=48000)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    out = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "out", suffixes={"vocals": "Acapella"}, progress=progress,
    )
    assert sf.info(str(out[0])).samplerate == 48000


def test_window_equals_span_equals_full_track_is_byte_equivalent(tmp_path):
    """window == span == full track reproduces the old untrimmed behaviour exactly:
    the file is byte-for-byte identical to a window=None full-track export."""
    n = 4000
    data = _sine(n, freq=330.0, amp=0.45)
    p = tmp_path / "vocals.wav"
    _write_stem(p, data)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    windowed = export_stems(
        [("vocals", p, (0, n), 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "windowed", suffixes={"vocals": "Acapella"}, progress=progress,
    )
    full = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "full", suffixes={"vocals": "Acapella"}, progress=progress,
    )
    assert windowed[0].read_bytes() == full[0].read_bytes()
    read_back, _ = sf.read(str(windowed[0]), always_2d=True, dtype="float32")
    assert read_back.shape[0] == n
    np.testing.assert_allclose(read_back, data, atol=1e-6)


# --- length invariant --------------------------------------------------------


def test_length_invariant_violation(tmp_path):
    n = SR
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n - 1, amp=0.4))  # one sample short
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    with pytest.raises(ExportError) as ei:
        export_stems(
            [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="aiff",
            out_dir=tmp_path / "out", suffixes={}, progress=progress,
        )
    assert ei.value.category == "internal"


def test_stem_shorter_than_source_is_internal(tmp_path):
    n = 10000
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(4000, amp=0.4))  # shorter than source_samples
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    with pytest.raises(ExportError) as ei:
        export_stems(
            [("vocals", p, (1000, 5000), 1.0)], source, source_samples=n, span=(0, n), fmt="aiff",
            out_dir=tmp_path / "out", suffixes={}, progress=progress,
        )
    assert ei.value.category == "internal"


# --- per-stem window on a shared timeline ------------------------------------


def test_window_inside_larger_span(tmp_path):
    """A stem windowed to [w_start, w_end) inside a larger span: output has span
    length; silent before the in-point, the source slice inside, silent after."""
    n = 10000
    data = _sine(n, freq=440.0, amp=0.5)
    p = tmp_path / "vocals.wav"
    _write_stem(p, data)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    window = (2000, 6000)
    span = (1000, 9000)  # larger than the window, with a non-zero start
    out = export_stems(
        [("vocals", p, window, 1.0)], source, source_samples=n, span=span, fmt="wav",
        out_dir=tmp_path / "out", suffixes={"vocals": "Acapella"}, progress=progress,
    )
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")

    assert read_back.shape[0] == span[1] - span[0]  # exactly span length
    # buffer index = source sample - span_start (1000)
    np.testing.assert_allclose(read_back[: 2000 - 1000], 0.0, atol=1e-6)  # before in-point
    np.testing.assert_allclose(
        read_back[2000 - 1000 : 6000 - 1000], data[2000:6000], atol=1e-6
    )  # inside window == source slice
    np.testing.assert_allclose(read_back[6000 - 1000 :], 0.0, atol=1e-6)  # after out-point


@pytest.mark.parametrize("window,span", [((0, 3000), (5000, 9000)), ((9500, 10000), (5000, 9000))])
def test_window_entirely_outside_span_is_silent(tmp_path, window, span):
    """A window that does not intersect the span yields pure silence, not an error."""
    n = 10000
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.6))
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    out = export_stems(
        [("vocals", p, window, 1.0)], source, source_samples=n, span=span, fmt="wav",
        out_dir=tmp_path / "out", suffixes={"vocals": "Acapella"}, progress=progress,
    )
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    assert read_back.shape[0] == span[1] - span[0]
    np.testing.assert_allclose(read_back, 0.0, atol=1e-6)


# --- bad bounds --------------------------------------------------------------


@pytest.mark.parametrize("span", [(-1, 5000), (0, 10001), (5000, 5000), (6000, 5000)])
def test_bad_span_bounds(tmp_path, span):
    n = 10000
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.4))
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    with pytest.raises(ExportError) as ei:
        export_stems(
            [("vocals", p, None, 1.0)], source, source_samples=n, span=span, fmt="aiff",
            out_dir=tmp_path / "out", suffixes={}, progress=progress,
        )
    assert ei.value.category == "internal"


@pytest.mark.parametrize("window", [(-1, 5000), (0, 10001), (5000, 5000), (6000, 5000)])
def test_bad_window_bounds(tmp_path, window):
    n = 10000
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.4))
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    with pytest.raises(ExportError) as ei:
        export_stems(
            [("vocals", p, window, 1.0)], source, source_samples=n, span=(0, n), fmt="aiff",
            out_dir=tmp_path / "out", suffixes={}, progress=progress,
        )
    assert ei.value.category == "internal"


# --- anti-clip shared gain ---------------------------------------------------


def test_anticlip_shared_gain_preserves_ratio_across_windowed_buffers(tmp_path):
    n = 2000
    loud = _sine(n, amp=1.2)   # peak 1.2 (> 1.0, preserved in FLOAT WAV)
    quiet = _sine(n, amp=0.6)  # peak 0.6, exactly half of loud
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, loud)
    _write_stem(pb, quiet)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    window = (200, 1800)  # both windowed, same span -> ratio must survive the shared gain
    out = export_stems(
        [("vocals", pa, window, 1.0), ("drums", pb, window, 1.0)], source, source_samples=n,
        span=(0, n), fmt="wav", out_dir=tmp_path / "out",
        suffixes={"vocals": "Acapella", "drums": "Drums"}, progress=progress,
    )
    peak_a = _peak(out[0])
    peak_b = _peak(out[1])
    assert peak_a == pytest.approx(0.999, abs=1e-3)
    assert (peak_b / peak_a) == pytest.approx(0.5, abs=1e-3)


def test_no_gain_when_below_threshold(tmp_path):
    n = 2000
    a = _sine(n, amp=0.5)
    b = _sine(n, freq=330.0, amp=0.3)
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, a)
    _write_stem(pb, b)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    out = export_stems(
        [("vocals", pa, None, 1.0), ("drums", pb, None, 1.0)], source, source_samples=n,
        span=(0, n), fmt="wav", out_dir=tmp_path / "out",
        suffixes={"vocals": "V", "drums": "D"}, progress=progress,
    )
    ra, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    rb, _ = sf.read(str(out[1]), always_2d=True, dtype="float32")
    # No gain applied: exported == input within 24-bit round-trip resolution.
    np.testing.assert_allclose(ra, a, atol=1e-6)
    np.testing.assert_allclose(rb, b, atol=1e-6)


# --- per-stem gain -----------------------------------------------------------


def test_gain_scales_output_samples(tmp_path):
    """A per-stem gain below the anti-clip ceiling scales the output by exactly
    that factor (ratio check against an unity-gain export of the same stem)."""
    n = 2000
    data = _sine(n, freq=220.0, amp=0.3)
    p = tmp_path / "vocals.wav"
    _write_stem(p, data)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    unity = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "unity", suffixes={"vocals": "V"}, progress=progress,
    )
    gained = export_stems(
        [("vocals", p, None, 0.5)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "gained", suffixes={"vocals": "V"}, progress=progress,
    )
    ru, _ = sf.read(str(unity[0]), always_2d=True, dtype="float32")
    rg, _ = sf.read(str(gained[0]), always_2d=True, dtype="float32")
    np.testing.assert_allclose(rg, ru * 0.5, atol=1e-6)  # gain scales samples exactly
    np.testing.assert_allclose(rg, data * 0.5, atol=1e-6)


def test_gain_absent_defaults_to_unity_via_shape(tmp_path):
    """gain 1.0 leaves the buffer untouched: output == input (round-trip)."""
    n = 1500
    data = _sine(n, amp=0.4)
    p = tmp_path / "vocals.wav"
    _write_stem(p, data)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    out = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "out", suffixes={"vocals": "V"}, progress=progress,
    )
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    np.testing.assert_allclose(read_back, data, atol=1e-6)


def test_gain_interacts_with_anticlip(tmp_path):
    """A gain that pushes a stem's peak past 0.999 triggers the shared anti-clip
    pass: the boosted peak is pulled back to the ceiling, and the waveform is a
    pure scaling of the gained buffer."""
    n = 2000
    data = _sine(n, amp=0.6)  # peak 0.6; a 1.8x gain -> peak 1.08 (> 0.999)
    p = tmp_path / "vocals.wav"
    _write_stem(p, data)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    gain = 1.8
    out = export_stems(
        [("vocals", p, None, gain)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "out", suffixes={"vocals": "V"}, progress=progress,
    )
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    gained = data * gain
    peak = float(np.max(np.abs(gained)))
    assert peak > 0.999  # precondition: gained stem clips
    assert float(np.max(np.abs(read_back))) == pytest.approx(0.999, abs=1e-3)
    np.testing.assert_allclose(read_back, gained * (0.999 / peak), atol=1e-6)


def test_gain_preserves_inter_stem_ratio_under_anticlip(tmp_path):
    """Two stems with different gains: the shared anti-clip gain preserves the
    gained ratio between them (never per-stem normalization)."""
    n = 2000
    base = _sine(n, amp=0.5)
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, base)
    _write_stem(pb, base)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    # vocals gained 2.0 (peak 1.0), drums gained 0.5 (peak 0.25) -> ratio 4:1.
    out = export_stems(
        [("vocals", pa, None, 2.0), ("drums", pb, None, 0.5)], source, source_samples=n,
        span=(0, n), fmt="wav", out_dir=tmp_path / "out",
        suffixes={"vocals": "V", "drums": "D"}, progress=progress,
    )
    peak_v = _peak(out[0])
    peak_d = _peak(out[1])
    assert peak_v == pytest.approx(0.999, abs=1e-3)  # loudest pulled to ceiling
    assert (peak_v / peak_d) == pytest.approx(4.0, abs=1e-2)  # 2.0 / 0.5


def test_combined_respects_per_stem_gains(tmp_path):
    """In combined mode each stem is scaled by its gain before it is summed."""
    n = 4000
    a = _sine(n, freq=220.0, amp=0.3)
    b = _sine(n, freq=330.0, amp=0.3)
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, a)
    _write_stem(pb, b)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    out = export_stems(
        [("vocals", pa, None, 0.5), ("drums", pb, None, 2.0)], source, source_samples=n,
        span=(0, n), fmt="wav", out_dir=tmp_path / "out",
        suffixes={}, progress=progress, combine_suffix="Mix",
    )
    assert len(out) == 1
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    np.testing.assert_allclose(read_back, a * 0.5 + b * 2.0, atol=1e-6)  # gained sum


# --- naming / sanitize -------------------------------------------------------


def test_naming_basic(tmp_path):
    n = 1000
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.4))
    source = tmp_path / "My Track.mp3"
    _, progress = _collect_progress()
    out = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="aiff",
        out_dir=tmp_path / "out", suffixes={"vocals": "Acapella"}, progress=progress,
    )
    assert out[0].name == "My Track (Acapella).aiff"


def test_naming_sanitizes_slash_and_colon(tmp_path):
    n = 1000
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.4))
    source = tmp_path / "AC:DC.mp3"  # colon in basename
    _, progress = _collect_progress()
    out = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "out", suffixes={"vocals": "Lead/Backing"}, progress=progress,
    )
    assert out[0].name == "AC-DC (Lead-Backing).wav"
    assert "/" not in out[0].name and ":" not in out[0].name


def test_suffix_fallback_to_title(tmp_path):
    n = 1000
    p = tmp_path / "bass.wav"
    _write_stem(p, _sine(n, amp=0.4))
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    out = export_stems(
        [("bass", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "out", suffixes={}, progress=progress,  # no mapping -> .title()
    )
    assert out[0].name == "Track (Bass).wav"


# --- tags --------------------------------------------------------------------


def test_tags_copied_with_suffix(tmp_path):
    n = 1500
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.4))

    source = tmp_path / "source.wav"
    sf.write(str(source), np.zeros((100, 1), dtype="float32"), SR, subtype="FLOAT", format="WAV")
    tagged = WAVE(str(source))
    tagged.add_tags()
    tagged.tags.add(TIT2(encoding=3, text=["Toxic Pony"]))
    tagged.tags.add(TPE1(encoding=3, text=["X"]))
    tagged.save()

    _, progress = _collect_progress()
    out = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="aiff",
        out_dir=tmp_path / "out", suffixes={"vocals": "Acapella"}, progress=progress,
    )
    exported = AIFF(str(out[0]))
    assert str(exported.tags["TIT2"].text[0]) == "Toxic Pony (Acapella)"
    assert str(exported.tags["TPE1"].text[0]) == "X"


def test_untagged_source_title_fallback(tmp_path):
    n = 1500
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.4))
    source = tmp_path / "Cool Song.mp3"  # nonexistent / unreadable -> no tags
    _, progress = _collect_progress()
    out = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "out", suffixes={"vocals": "Acapella"}, progress=progress,
    )
    exported = WAVE(str(out[0]))
    assert str(exported.tags["TIT2"].text[0]) == "Cool Song (Acapella)"
    assert "TPE1" not in exported.tags  # missing artist omitted


# --- overwrite ---------------------------------------------------------------


def test_existing_target_overwritten(tmp_path):
    n = 1000
    p = tmp_path / "vocals.wav"
    _write_stem(p, _sine(n, amp=0.4))
    source = tmp_path / "Track.mp3"
    out_dir = tmp_path / "out"
    _, progress = _collect_progress()

    first = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=out_dir, suffixes={"vocals": "Acapella"}, progress=progress,
    )
    target = first[0]
    assert target.exists()

    # Change the stem content, export again to the same target: no error, content updated.
    new_data = _sine(n, freq=660.0, amp=0.7)
    _write_stem(p, new_data)
    second = export_stems(
        [("vocals", p, None, 1.0)], source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=out_dir, suffixes={"vocals": "Acapella"}, progress=progress,
    )
    assert second[0] == target
    read_back, _ = sf.read(str(target), always_2d=True, dtype="float32")
    np.testing.assert_allclose(read_back, new_data, atol=1e-6)


# --- progress ----------------------------------------------------------------


def test_progress_final_fraction(tmp_path):
    n = 1000
    stems = []
    for name in ("vocals", "drums", "bass"):
        pp = tmp_path / f"{name}.wav"
        _write_stem(pp, _sine(n, amp=0.4))
        stems.append((name, pp, None, 1.0))
    source = tmp_path / "Track.mp3"
    calls, progress = _collect_progress()

    export_stems(
        stems, source, source_samples=n, span=(0, n), fmt="wav",
        out_dir=tmp_path / "out",
        suffixes={"vocals": "V", "drums": "D", "bass": "B"}, progress=progress,
    )
    assert calls, "progress was never called"
    assert calls[-1] == ("exporting", 1.0)
    assert all(stage == "exporting" for stage, _ in calls)


def test_export_error_carries_category_and_message():
    err = ExportError("internal", "boom")
    assert err.category == "internal"
    assert "boom" in str(err)


# --- combined (summed) export ------------------------------------------------


def test_combined_sums_stems_into_one_file(tmp_path):
    n = 4000
    a = _sine(n, freq=220.0, amp=0.3)
    b = _sine(n, freq=330.0, amp=0.3)  # sum peaks well below the ceiling
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, a)
    _write_stem(pb, b)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    out = export_stems(
        [("vocals", pa, None, 1.0), ("drums", pb, None, 1.0)], source, source_samples=n,
        span=(0, n), fmt="wav", out_dir=tmp_path / "out",
        suffixes={"vocals": "V", "drums": "D"}, progress=progress, combine_suffix="No Other",
    )
    assert len(out) == 1  # ONE file regardless of stem count
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    assert read_back.shape[0] == n  # frames == span length
    np.testing.assert_allclose(read_back, a + b, atol=1e-6)  # sample-wise sum


def test_combined_span_frames_and_content(tmp_path):
    n = 10000
    a = _sine(n, freq=220.0, amp=0.3)
    b = _sine(n, freq=440.0, amp=0.3)
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, a)
    _write_stem(pb, b)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    # Full-track stems clipped to a span: output is the summed slice on that span.
    span = (1000, 5000)
    out = export_stems(
        [("vocals", pa, None, 1.0), ("drums", pb, None, 1.0)], source, source_samples=n,
        span=span, fmt="wav", out_dir=tmp_path / "out", suffixes={}, progress=progress,
        combine_suffix="No Other",
    )
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    assert read_back.shape[0] == span[1] - span[0]  # frames == span length
    np.testing.assert_allclose(read_back, (a + b)[1000:5000], atol=1e-6)


def test_combined_different_windows_enter_and_exit(tmp_path):
    """Two stems with different windows, summed: the mix enters/exits per window.
    Where only A's window covers, the sum equals A alone; in the overlap it equals
    A + B; where only B's window covers, it equals B alone."""
    n = 12000
    a = _sine(n, freq=220.0, amp=0.3)
    b = _sine(n, freq=330.0, amp=0.3)  # sum stays below the ceiling -> no gain
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, a)
    _write_stem(pb, b)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    window_a = (0, 8000)
    window_b = (4000, 12000)
    span = (0, n)
    out = export_stems(
        [("vocals", pa, window_a, 1.0), ("drums", pb, window_b, 1.0)], source, source_samples=n,
        span=span, fmt="wav", out_dir=tmp_path / "out", suffixes={}, progress=progress,
        combine_suffix="Mix",
    )
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    assert read_back.shape[0] == n
    np.testing.assert_allclose(read_back[0:4000], a[0:4000], atol=1e-6)  # only A
    np.testing.assert_allclose(read_back[4000:8000], (a + b)[4000:8000], atol=1e-6)  # overlap
    np.testing.assert_allclose(read_back[8000:12000], b[8000:12000], atol=1e-6)  # only B


def test_combined_anticlip_scales_sum(tmp_path):
    n = 2000
    a = _sine(n, freq=220.0, amp=0.7)
    b = _sine(n, freq=220.0, amp=0.7)  # in phase -> sum peaks ~1.4 (> 0.999)
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, a)
    _write_stem(pb, b)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()

    out = export_stems(
        [("vocals", pa, None, 1.0), ("drums", pb, None, 1.0)], source, source_samples=n,
        span=(0, n), fmt="wav", out_dir=tmp_path / "out", suffixes={}, progress=progress,
        combine_suffix="No Other",
    )
    read_back, _ = sf.read(str(out[0]), always_2d=True, dtype="float32")
    summed = a + b
    peak = float(np.max(np.abs(summed)))
    assert peak > 0.999  # precondition: the sum really does clip
    # peak pulled to the ceiling, and the waveform is a pure scaling of the sum.
    assert float(np.max(np.abs(read_back))) == pytest.approx(0.999, abs=1e-3)
    np.testing.assert_allclose(read_back, summed * (0.999 / peak), atol=1e-6)


def test_combined_naming_and_tags(tmp_path):
    n = 1500
    a = _sine(n, amp=0.3)
    b = _sine(n, freq=330.0, amp=0.3)
    pa = tmp_path / "vocals.wav"
    pb = tmp_path / "drums.wav"
    _write_stem(pa, a)
    _write_stem(pb, b)

    source = tmp_path / "Toxic Pony.wav"
    sf.write(str(source), np.zeros((100, 1), dtype="float32"), SR, subtype="FLOAT", format="WAV")
    tagged = WAVE(str(source))
    tagged.add_tags()
    tagged.tags.add(TIT2(encoding=3, text=["Toxic Pony"]))
    tagged.save()

    _, progress = _collect_progress()
    out = export_stems(
        [("vocals", pa, None, 1.0), ("drums", pb, None, 1.0)], source, source_samples=n,
        span=(0, n), fmt="aiff", out_dir=tmp_path / "out", suffixes={}, progress=progress,
        combine_suffix="No Other",
    )
    assert len(out) == 1
    assert out[0].name == "Toxic Pony (No Other).aiff"
    exported = AIFF(str(out[0]))
    assert str(exported.tags["TIT2"].text[0]) == "Toxic Pony (No Other)"


def test_combined_length_invariant_per_stem(tmp_path):
    n = SR
    good = tmp_path / "vocals.wav"
    bad = tmp_path / "drums.wav"
    _write_stem(good, _sine(n, amp=0.3))
    _write_stem(bad, _sine(n - 1, amp=0.3))  # one sample short
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    with pytest.raises(ExportError) as ei:
        export_stems(
            [("vocals", good, None, 1.0), ("drums", bad, None, 1.0)], source, source_samples=n,
            span=(0, n), fmt="aiff", out_dir=tmp_path / "out", suffixes={}, progress=progress,
            combine_suffix="No Other",
        )
    assert ei.value.category == "internal"


def test_combined_channel_mismatch_is_internal(tmp_path):
    n = 2000
    mono = tmp_path / "vocals.wav"
    stereo = tmp_path / "drums.wav"
    _write_stem(mono, _sine(n, amp=0.3))  # (n, 1)
    _write_stem(stereo, np.hstack([_sine(n, amp=0.3), _sine(n, freq=330.0, amp=0.3)]))  # (n, 2)
    source = tmp_path / "Track.mp3"
    _, progress = _collect_progress()
    with pytest.raises(ExportError) as ei:
        export_stems(
            [("vocals", mono, None, 1.0), ("drums", stereo, None, 1.0)], source, source_samples=n,
            span=(0, n), fmt="wav", out_dir=tmp_path / "out", suffixes={}, progress=progress,
            combine_suffix="No Other",
        )
    assert ei.value.category == "internal"
