"""Stem export: per-stem trim windows on a shared timeline, per-stem gain,
shared anti-clip gain, 24-bit AIFF/WAV encode, tag copy.

Engine-side invariants (see docs/PROTOCOL.md, "Export behavior"):

- Float32 pipeline throughout; output encoded PCM 24-bit (AIFF or WAV).
- Every loaded stem MUST have exactly ``source_samples`` frames (length
  invariant); a violation is an ``internal`` error.
- The output spans ``span = (start, end)`` on the source timeline, requiring
  ``0 <= start < end <= source_samples``; every output file (per-stem and the
  combined mix) has exactly ``end - start`` frames.
- Each stem has an optional ``window = (start, end)`` (``None`` == full track),
  validated the same way as ``span``. The stem is silent outside its window;
  inside the window it carries the source samples, aligned on the original
  timeline. A window that does not intersect the span yields pure silence
  (allowed, not an error).
- Each stem carries a linear ``gain`` (default 1.0) applied to its built,
  windowed buffer BEFORE the shared anti-clip pass — so a boosted stem can be
  what pushes the mix into the anti-clip guard. In combined mode each stem is
  gained before it is summed into the mix.
- A single shared anti-clip gain (``0.999 / peak`` over ALL stems together,
  measured after per-stem gain) is applied to every stem equally when the
  combined peak exceeds 0.999. Never per-stem normalization: relative levels
  between stems (as set by their gains) are preserved exactly.
- Filenames are ``<source basename> (<suffix>).<ext>`` with ``/`` and ``:``
  sanitized to ``-``; title/artist/album are copied from the source tags and
  the suffix is appended to the title.
"""

from __future__ import annotations

from pathlib import Path
from typing import Callable

import numpy as np
import soundfile as sf

import mutagen
import mutagen.aiff
import mutagen.id3
import mutagen.wave

ProgressFn = Callable[[str, float], None]  # (stage, fraction 0..1)
Window = tuple[int, int]  # (start_sample, end_sample) on the source timeline

_CLIP_CEILING = 0.999

# fmt -> (file extension, soundfile format string)
_FORMATS: dict[str, tuple[str, str]] = {
    "aiff": ("aiff", "AIFF"),
    "wav": ("wav", "WAV"),
}


class ExportError(Exception):
    """Export failure. ``category`` is read by the worker to build the protocol
    error event ("internal" for invariant violations, "decode" for unreadable
    inputs)."""

    def __init__(self, category: str, message: str):
        super().__init__(message)
        self.category = category
        self.message = message


def export_stems(
    stems: list[tuple[str, Path, Window | None, float]],
    source: Path,
    source_samples: int,
    span: Window,
    fmt: str,
    out_dir: Path,
    suffixes: dict[str, str],
    progress: ProgressFn,
    combine_suffix: str | None = None,
) -> list[Path]:
    """Export each separated stem WAV to a 24-bit AIFF/WAV, returning the written
    paths in ``stems`` order.

    Each stem is ``(name, path, window, gain)``: an optional ``window``
    (``None`` == full track) and a linear ``gain`` (1.0 == unchanged). The output
    covers ``span`` on the source timeline; a stem is silent outside its window
    and everything stays aligned on the original timeline, so every output has
    exactly ``span`` length. Each stem's built buffer is multiplied by its gain
    before the shared anti-clip pass.

    When ``combine_suffix`` is given, the (gained) stems are instead summed into a
    single mix written as ONE file named ``<source basename> (<combine_suffix>).<ext>``;
    ``suffixes`` is ignored and the returned list holds just that one path."""
    ext, sf_format = _resolve_format(fmt)
    _validate_window(span, source_samples, "span")
    for _, _, window, _ in stems:
        if window is not None:
            _validate_window(window, source_samples, "window")

    source_tags = _read_source_tags(source)
    out_dir.mkdir(parents=True, exist_ok=True)

    if combine_suffix is not None:
        return _export_combined(
            stems, source, source_samples, span, ext, sf_format, fmt,
            out_dir, source_tags, combine_suffix, progress,
        )

    # Pass 1: load + window every stem into a span-length buffer and apply its
    # per-stem gain, so the anti-clip gain is shared across all gained buffers.
    buffers: list[tuple[str, np.ndarray, int]] = []
    for name, path, window, gain in stems:
        data, samplerate = _load_stem(path)
        _check_length(data, source_samples)
        buffer = _build_buffer(data, window, span, source_samples)
        if gain != 1.0:
            buffer = buffer * gain
        buffers.append((name, buffer, samplerate))

    clip_gain = _shared_anticlip_gain(buf for _, buf, _ in buffers)

    # Pass 2: apply the shared anti-clip gain, encode, tag.
    written: list[Path] = []
    total = len(stems)
    for i, (name, buffer, samplerate) in enumerate(buffers, start=1):
        suffix = suffixes.get(name) or name.title()
        target = out_dir / _sanitize(f"{source.stem} ({suffix})")
        target = target.with_name(f"{target.name}.{ext}")

        out = buffer if clip_gain == 1.0 else buffer * clip_gain
        try:
            sf.write(str(target), out, samplerate, subtype="PCM_24", format=sf_format)
        except Exception as exc:  # noqa: BLE001 - surfaced as an invariant failure
            raise ExportError("internal", f"failed writing {target}: {exc}") from exc

        _write_tags(target, fmt, source_tags, suffix, source.stem)
        written.append(target)
        progress("exporting", i / total)

    progress("exporting", 1.0)
    return written


def _export_combined(
    stems: list[tuple[str, Path, Window | None, float]],
    source: Path,
    source_samples: int,
    span: Window,
    ext: str,
    sf_format: str,
    fmt: str,
    out_dir: Path,
    source_tags: dict[str, str],
    combine_suffix: str,
    progress: ProgressFn,
) -> list[Path]:
    """Sum every stem into a single mix and write ONE file. Each stem obeys the
    same length invariant and is windowed into the same span-length buffer as
    per-stem export, then scaled by its per-stem gain before it is summed; the
    sum shares one anti-clip gain."""
    total: np.ndarray | None = None
    channels: int | None = None
    samplerate: int | None = None
    for name, path, window, gain in stems:
        data, sr = _load_stem(path)
        _check_length(data, source_samples)
        buffer = _build_buffer(data, window, span, source_samples)
        if gain != 1.0:
            buffer = buffer * gain  # a fresh array we own; safe to accumulate into
        if total is None:
            total = buffer  # a fresh zeros() buffer we own; safe to accumulate into
            channels = buffer.shape[1]
            samplerate = sr
            continue
        if buffer.shape[1] != channels:
            raise ExportError(
                "internal",
                f"channel-count mismatch: stem {name!r} has {buffer.shape[1]} "
                f"channels, expected {channels}",
            )
        total = total + buffer

    if total is None:
        return []

    clip_gain = _shared_anticlip_gain([total])
    out = total if clip_gain == 1.0 else total * clip_gain

    target = out_dir / _sanitize(f"{source.stem} ({combine_suffix})")
    target = target.with_name(f"{target.name}.{ext}")
    try:
        sf.write(str(target), out, samplerate, subtype="PCM_24", format=sf_format)
    except Exception as exc:  # noqa: BLE001 - surfaced as an invariant failure
        raise ExportError("internal", f"failed writing {target}: {exc}") from exc

    _write_tags(target, fmt, source_tags, combine_suffix, source.stem)
    progress("exporting", 1.0)
    return [target]


# --- helpers -----------------------------------------------------------------


def _resolve_format(fmt: str) -> tuple[str, str]:
    try:
        return _FORMATS[fmt.lower()]
    except KeyError:
        raise ExportError("internal", f"unsupported format: {fmt!r}") from None


def _validate_window(window: Window, source_samples: int, label: str) -> None:
    start, end = window
    if not (0 <= start < end <= source_samples):
        raise ExportError(
            "internal",
            f"invalid {label} bounds: (start={start}, end={end}) for "
            f"source_samples={source_samples}",
        )


def _load_stem(path: Path) -> tuple[np.ndarray, int]:
    try:
        data, samplerate = sf.read(str(path), always_2d=True, dtype="float32")
    except Exception as exc:  # noqa: BLE001 - any unreadable input is a decode error
        raise ExportError("decode", f"could not read stem {path}: {exc}") from exc
    return data, samplerate


def _check_length(data: np.ndarray, source_samples: int) -> None:
    n = data.shape[0]
    if n != source_samples:
        raise ExportError(
            "internal",
            f"length invariant: stem has {n} samples, expected {source_samples}",
        )


def _build_buffer(
    data: np.ndarray, window: Window | None, span: Window, source_samples: int
) -> np.ndarray:
    """Place the stem's windowed samples onto a span-length, source-aligned buffer.

    Everything outside the window (or outside the span) stays zero, so a window
    that does not intersect the span yields pure silence."""
    span_start, span_end = span
    buffer = np.zeros((span_end - span_start, data.shape[1]), dtype="float32")
    w_start, w_end = window if window is not None else (0, source_samples)
    overlap_start = max(w_start, span_start)
    overlap_end = min(w_end, span_end)
    if overlap_start < overlap_end:
        buffer[overlap_start - span_start : overlap_end - span_start] = (
            data[overlap_start:overlap_end]
        )
    return buffer


def _shared_anticlip_gain(segments) -> float:
    peak = 0.0
    for segment in segments:
        if segment.size:
            peak = max(peak, float(np.max(np.abs(segment))))
    if peak > _CLIP_CEILING:
        return _CLIP_CEILING / peak
    return 1.0


def _sanitize(name: str) -> str:
    return name.replace("/", "-").replace(":", "-")


# --- tags --------------------------------------------------------------------

_ID3_FIELDS = (("TIT2", "title"), ("TPE1", "artist"), ("TALB", "album"))


def _read_source_tags(source: Path) -> dict[str, str]:
    """Read title/artist/album from the source. Returns a dict; never raises."""
    tags: dict[str, str] = {}
    ext = source.suffix.lower()
    try:
        if ext in (".wav", ".wave"):
            _extract_id3(mutagen.wave.WAVE(str(source)).tags, tags)
        elif ext in (".aiff", ".aif", ".aifc"):
            _extract_id3(mutagen.aiff.AIFF(str(source)).tags, tags)
        else:
            audio = mutagen.File(str(source), easy=True)
            if audio is not None and audio.tags is not None:
                for _, key in _ID3_FIELDS:
                    value = audio.tags.get(key)
                    if value:
                        tags[key] = str(value[0] if isinstance(value, list) else value)
    except Exception:  # noqa: BLE001 - unreadable/absent source is fine, fall back
        return tags
    return tags


def _extract_id3(id3, tags: dict[str, str]) -> None:
    if id3 is None:
        return
    for frame_id, key in _ID3_FIELDS:
        frame = id3.get(frame_id)
        if frame is not None and getattr(frame, "text", None):
            tags[key] = str(frame.text[0])


def _write_tags(
    target: Path, fmt: str, source_tags: dict[str, str], suffix: str, source_stem: str
) -> None:
    """Write ID3 title/artist/album onto the exported file. Silent tag loss is a
    bug, so any failure is an ``internal`` ExportError."""
    title = f"{source_tags.get('title') or source_stem} ({suffix})"
    try:
        audio = (
            mutagen.aiff.AIFF(str(target))
            if fmt.lower() == "aiff"
            else mutagen.wave.WAVE(str(target))
        )
        if audio.tags is None:
            audio.add_tags()
        id3 = audio.tags
        id3.setall("TIT2", [mutagen.id3.TIT2(encoding=3, text=[title])])
        artist = source_tags.get("artist")
        if artist:
            id3.setall("TPE1", [mutagen.id3.TPE1(encoding=3, text=[artist])])
        album = source_tags.get("album")
        if album:
            id3.setall("TALB", [mutagen.id3.TALB(encoding=3, text=[album])])
        audio.save()
    except Exception as exc:  # noqa: BLE001 - silent tag loss would be a product bug
        raise ExportError("internal", f"failed tagging {target}: {exc}") from exc
