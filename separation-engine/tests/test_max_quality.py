"""Hermetic tests for quality="max" separation (ensemble + cascade).

These drive AudioSeparatorBackend end-to-end with a FAKE Separator injected via
``separator_factory`` — no models, no network, no audio-separator. The fake
writes constant-valued stem WAVs into the staging dir per loaded checkpoint, so
the tests can assert exactly what a caller observes: the number of stems, equal
frame counts, the ensemble/cascade model string, the pairwise mean (two_stem)
and the exact residual + sum invariant (four_stem), and cancellation between
passes.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

from stemsplitter_engine.backend import JobCancelled
from stemsplitter_engine import audio_separator_backend as backend
from stemsplitter_engine.audio_separator_backend import (
    AudioSeparatorBackend,
    CASCADE_DRUMBASS_MODEL,
    ENSEMBLE_VOCALS_MODEL,
    EngineError,
    TWO_STEM_MODEL,
    _average_wavs,
    _split_vocals_complement,
)

SR = 44100

# Constant sample value written for each (model, stem-marker). The complements
# are chosen so the ensemble mean and the cascade residual are exact, checkable
# numbers (see the asserts below).
_STEM_SPECS: dict[str, list[tuple[str, float]]] = {
    TWO_STEM_MODEL: [("(Vocals)", 0.2), ("(Instrumental)", 0.7)],
    ENSEMBLE_VOCALS_MODEL: [("(Vocals)", 0.4), ("(Other)", 0.5)],
    CASCADE_DRUMBASS_MODEL: [
        ("(Vocals)", 0.0),
        ("(Drums)", 0.1),
        ("(Bass)", 0.2),
        ("(Other)", 0.3),  # htdemucs's own 'other' — discarded for the residual
    ],
}


class FakeSeparator:
    """Mimics the slice of audio_separator.Separator the backend uses."""

    def __init__(self, output_dir: str, frame_deltas: dict[str, int]) -> None:
        self._output_dir = Path(output_dir)
        self._frame_deltas = frame_deltas
        self._model: str | None = None
        self.separate_calls = 0

    def load_model(self, model_filename: str) -> None:
        self._model = model_filename

    def separate(self, input_path: str) -> list[str]:
        self.separate_calls += 1
        info = sf.info(input_path)
        frames = info.frames + self._frame_deltas.get(self._model, 0)
        names: list[str] = []
        for i, (marker, value) in enumerate(_STEM_SPECS[self._model]):
            name = f"stem_{i}_{marker}.wav"
            arr = np.full((frames, 2), value, dtype="float32")
            sf.write(str(self._output_dir / name), arr, info.samplerate,
                     subtype="FLOAT", format="WAV")
            names.append(name)
        return names


def _factory(frame_deltas: dict[str, int] | None = None):
    deltas = frame_deltas or {}
    holder: dict[str, FakeSeparator] = {}

    def make(model_dir: str, output_dir: str) -> FakeSeparator:
        sep = FakeSeparator(output_dir, deltas)
        holder["sep"] = sep
        return sep

    make.holder = holder  # type: ignore[attr-defined]
    return make


def _fixture(tmp_path: Path, frames: int = 8820) -> Path:
    wav = tmp_path / "fixture.wav"
    sf.write(str(wav), np.zeros((frames, 2), dtype="float32"), SR,
             subtype="FLOAT", format="WAV")
    return wav


def _make_backend(tmp_path: Path, factory) -> AudioSeparatorBackend:
    return AudioSeparatorBackend(tmp_path / "models", separator_factory=factory)


def _noop_progress(stage, fraction):  # pragma: no cover - trivial
    pass


def _read(path: Path) -> np.ndarray:
    data, _ = sf.read(str(path), always_2d=True, dtype="float32")
    return data


# --- two_stem ensemble -------------------------------------------------------


def test_two_stem_max_averages_two_models(tmp_path):
    frames = 8820
    wav = _fixture(tmp_path, frames)
    b = _make_backend(tmp_path, _factory())
    result = b.separate(wav, "two_stem", tmp_path / "ws", _noop_progress, lambda: False,
                        quality="max")

    names = [n for n, _ in result.stems]
    assert names == ["vocals", "instrumental"]  # two stems, protocol order
    assert result.source_samples == frames
    assert result.sample_rate == SR
    # model string names the ensemble members
    assert result.model == f"ensemble({TWO_STEM_MODEL} + {ENSEMBLE_VOCALS_MODEL})"

    paths = dict(result.stems)
    vocals = _read(paths["vocals"])
    instrumental = _read(paths["instrumental"])
    assert vocals.shape[0] == instrumental.shape[0] == frames  # equal frames
    np.testing.assert_allclose(vocals, (0.2 + 0.4) / 2, atol=1e-6)  # mean of vocals
    np.testing.assert_allclose(instrumental, (0.7 + 0.5) / 2, atol=1e-6)  # mean of complements


def test_two_stem_max_frame_mismatch_is_internal(tmp_path):
    """Both passes must produce identical frame counts, else EngineError internal."""
    wav = _fixture(tmp_path)
    # Make the second model's outputs 5 frames longer than the first's.
    b = _make_backend(tmp_path, _factory({ENSEMBLE_VOCALS_MODEL: 5}))
    with pytest.raises(EngineError) as ei:
        b.separate(wav, "two_stem", tmp_path / "ws", _noop_progress, lambda: False,
                   quality="max")
    assert ei.value.category == "internal"


def test_two_stem_max_cancels_between_passes(tmp_path):
    wav = _fixture(tmp_path)
    factory = _factory()
    b = _make_backend(tmp_path, factory)

    def cancelled() -> bool:
        # False for the first pass's boundary check, True once a pass has run.
        return factory.holder.get("sep") is not None and factory.holder["sep"].separate_calls >= 1

    with pytest.raises(JobCancelled):
        b.separate(wav, "two_stem", tmp_path / "ws", _noop_progress, cancelled, quality="max")
    # exactly one pass ran before the between-pass cancel fired
    assert factory.holder["sep"].separate_calls == 1


# --- four_stem cascade -------------------------------------------------------


def test_four_stem_max_cascade_residual_and_sum(tmp_path):
    frames = 8820
    wav = _fixture(tmp_path, frames)
    b = _make_backend(tmp_path, _factory())
    result = b.separate(wav, "four_stem", tmp_path / "ws", _noop_progress, lambda: False,
                        quality="max")

    names = [n for n, _ in result.stems]
    assert names == ["vocals", "drums", "bass", "other"]  # four stems, protocol order
    assert result.source_samples == frames
    assert "cascade" in result.model
    assert TWO_STEM_MODEL in result.model and CASCADE_DRUMBASS_MODEL in result.model

    paths = dict(result.stems)
    v = _read(paths["vocals"])
    d = _read(paths["drums"])
    ba = _read(paths["bass"])
    o = _read(paths["other"])
    assert v.shape[0] == d.shape[0] == ba.shape[0] == o.shape[0] == frames  # equal frames

    np.testing.assert_allclose(v, 0.2, atol=1e-6)   # vocals from pass 1
    np.testing.assert_allclose(d, 0.1, atol=1e-6)   # drums from pass 2
    np.testing.assert_allclose(ba, 0.2, atol=1e-6)  # bass from pass 2
    # other = instrumental (0.7) - drums (0.1) - bass (0.2) = 0.4 (exact residual)
    np.testing.assert_allclose(o, 0.7 - 0.1 - 0.2, atol=1e-6)
    # the four stems sum back to pass-1's vocals + instrumental
    np.testing.assert_allclose(v + d + ba + o, 0.2 + 0.7, atol=1e-6)


def test_four_stem_max_cancels_between_passes(tmp_path):
    wav = _fixture(tmp_path)
    factory = _factory()
    b = _make_backend(tmp_path, factory)

    def cancelled() -> bool:
        return factory.holder.get("sep") is not None and factory.holder["sep"].separate_calls >= 1

    with pytest.raises(JobCancelled):
        b.separate(wav, "four_stem", tmp_path / "ws", _noop_progress, cancelled, quality="max")
    assert factory.holder["sep"].separate_calls == 1


# --- standard quality unaffected ---------------------------------------------


def test_standard_two_stem_uses_single_model(tmp_path):
    wav = _fixture(tmp_path)
    b = _make_backend(tmp_path, _factory())
    result = b.separate(wav, "two_stem", tmp_path / "ws", _noop_progress, lambda: False)
    assert result.model == TWO_STEM_MODEL  # single-model model string, not an ensemble
    assert [n for n, _ in result.stems] == ["vocals", "instrumental"]


# --- pure helpers ------------------------------------------------------------


def test_split_vocals_complement_ok():
    voc, comp = _split_vocals_complement(
        [Path("a_(Instrumental).wav"), Path("a_(Vocals).wav")], "m"
    )
    assert voc.name == "a_(Vocals).wav"
    assert comp.name == "a_(Instrumental).wav"


def test_split_vocals_complement_missing_vocals_is_model_error():
    with pytest.raises(EngineError) as ei:
        _split_vocals_complement([Path("a_(Drums).wav"), Path("a_(Bass).wav")], "m")
    assert ei.value.category == "model"


def test_average_wavs_shape_mismatch_is_internal(tmp_path):
    a = tmp_path / "a.wav"
    b = tmp_path / "b.wav"
    sf.write(str(a), np.zeros((100, 2), dtype="float32"), SR, subtype="FLOAT", format="WAV")
    sf.write(str(b), np.zeros((101, 2), dtype="float32"), SR, subtype="FLOAT", format="WAV")
    with pytest.raises(EngineError) as ei:
        _average_wavs([a, b], tmp_path / "out.wav")
    assert ei.value.category == "internal"
