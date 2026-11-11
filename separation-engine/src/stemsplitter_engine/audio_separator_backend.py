"""Real separation backend: wraps the MIT ``audio-separator`` package.

Implements the ``SeparatorBackend`` protocol (see ``backend.py``). Mode maps to a
pinned checkpoint; ``audio-separator`` decodes any ffmpeg-readable input, runs the
model (MPS for PyTorch checkpoints, CoreML for ONNX), and writes one WAV per stem
into the job workspace. ``audio_separator`` is imported lazily inside methods so
this module imports fine without the ``full`` extra installed.

Pinned checkpoints (July 2026 research, MVSep Multisong leaderboard + audio-separator
SDR listing):

- two_stem: ``model_bs_roformer_ep_317_sdr_12.9755.ckpt`` — community BS-RoFormer,
  vocals SDR ~12.9, the best practical 2-stem (vocals/instrumental) default.
- four_stem: ``htdemucs_ft.yaml`` — Demucs v4 fine-tuned. The audio-separator model
  registry offers no single-pass vocals/drums/bass/other model in the RoFormer/SCNet
  (MDXC) family — those checkpoints are 2-stem or specialised (drum-bass, drum
  components) — so the fine-tuned Demucs v4 is the highest-quality 4-stem available.

Quality "max" (see ``separate(..., quality=)``): trades wall time for SDR by
running an ensemble/cascade of the installed checkpoints. The extra checkpoints
below were chosen by interrogating the installed registry
(``Separator().list_supported_model_files()``, July 2026):

- two_stem + max: average the BS-RoFormer ep317 default with a SECOND, differently
  architected vocal model. ``vocals_mel_band_roformer.ckpt`` (Kim Jensen Mel-Band
  RoFormer) is the highest-SDR general Mel-Band RoFormer vocals checkpoint in the
  registry (vocals SDR 12.60; the next general Mel-Band RoFormer vocals models —
  unwa Big Beta4 12.52, Kim FT 12.44 — score lower). BS-RoFormer and Mel-Band
  RoFormer factor the spectrogram differently, so their errors decorrelate and the
  pairwise mean is cleaner than either alone.
- four_stem + max: cascade. Pass 1 removes vocals with ep317 (much stronger vocal
  SDR than htdemucs), then pass 2 splits the vocal-free INSTRUMENTAL into
  drums/bass with ``htdemucs_ft`` (drums SDR 10.02, bass 12.02 — the registry's
  best drum/bass scores: plain htdemucs is 9.45/11.64 and the kuielab MDX23 A
  drum/bass pair is 6.99/10.45, and no drum-bass RoFormer exists). ``other`` is
  the exact residual instrumental - drums - bass, so the four stems sum back to
  pass 1's vocals + instrumental. Reusing the two already-pinned default
  checkpoints means max adds NO new download over the standard modes.
"""

from __future__ import annotations

import json
import logging
import shutil
import subprocess
import threading
import time
from pathlib import Path

import numpy as np
import soundfile as sf

from .backend import CancelledFn, JobCancelled, ProgressFn, SeparationResult

TWO_STEM_MODEL = "model_bs_roformer_ep_317_sdr_12.9755.ckpt"
FOUR_STEM_MODEL = "htdemucs_ft.yaml"

# Second vocal checkpoint for the two_stem "max" ensemble (see module docstring):
# highest-SDR general Mel-Band RoFormer vocals model in the installed registry.
# Emits ('vocals', 'other') where 'other' is the instrumental complement.
ENSEMBLE_VOCALS_MODEL = "vocals_mel_band_roformer.ckpt"

# Pass-2 checkpoint for the four_stem "max" cascade: htdemucs_ft (== the standard
# four_stem model) run on the vocal-free instrumental, using only its drums/bass
# stems. Best installed drum/bass SDR, and already downloaded for standard mode.
CASCADE_DRUMBASS_MODEL = FOUR_STEM_MODEL

_MODE_MODELS = {"two_stem": TWO_STEM_MODEL, "four_stem": FOUR_STEM_MODEL}

# Protocol stem name -> case-insensitive marker in audio-separator output filenames
# (e.g. "<track>_(Vocals)_<model>.wav"). Insertion order is the protocol stem order.
_STEM_MARKERS: dict[str, dict[str, str]] = {
    "two_stem": {"vocals": "(vocals)", "instrumental": "(instrumental)"},
    "four_stem": {
        "vocals": "(vocals)",
        "drums": "(drums)",
        "bass": "(bass)",
        "other": "(other)",
    },
}

_DURATION_TOLERANCE_S = 0.1

# Wall-seconds of processing per audio-second, used to estimate separation
# progress (the models expose no mid-inference callbacks). Seeds are replaced
# by a learned per-model factor after the first completed job on this machine.
_PERF_SEEDS = {"two_stem": 0.65, "four_stem": 0.35}
# Per-model seeds for the multi-pass "max" pipelines, whose progress estimate
# sums the perf factors of every pass. The Mel-Band RoFormer pass is seeded like
# the BS-RoFormer default; htdemucs_ft like the four_stem default.
_MODEL_PERF_SEEDS = {
    TWO_STEM_MODEL: 0.65,
    ENSEMBLE_VOCALS_MODEL: 0.65,
    FOUR_STEM_MODEL: 0.35,
}
_DEFAULT_PERF_SEED = 0.6
_PERF_EMA_WEIGHT = 0.4
# Estimated progress maps into [0.15, 0.90]; 0.90+ is the real "writing" stage.
_SEPARATING_FLOOR = 0.15
_SEPARATING_SPAN = 0.75


class EngineError(Exception):
    """Separation failure. ``category`` is read by the worker to build the protocol
    error event ("decode", "model", or "internal")."""

    def __init__(self, category: str, message: str) -> None:
        super().__init__(message)
        self.category = category
        self.message = message


class AudioSeparatorBackend:
    def __init__(self, model_dir: Path, separator_factory=None) -> None:
        self._model_dir = model_dir
        # One long-lived Separator writing into a stable staging dir keeps the
        # loaded checkpoint warm across queue items (output_dir is fixed at
        # construction, so per-job dirs would force a reload per track).
        # Outputs are moved from staging into the job workspace after each run.
        self._staging = model_dir.parent / "staging"
        self._separator = None
        self._loaded_model: str | None = None
        # Injection seam for tests: a callable(model_file_dir, output_dir) that
        # returns a Separator-shaped object. None => the real audio_separator.
        self._separator_factory = separator_factory

    def default_models(self) -> dict[str, str]:
        return {"two_stem": TWO_STEM_MODEL, "four_stem": FOUR_STEM_MODEL}

    def separate(
        self,
        input_path: Path,
        mode: str,
        workspace: Path,
        progress: ProgressFn,
        cancelled: CancelledFn,
        quality: str = "standard",
    ) -> SeparationResult:
        if mode not in _MODE_MODELS:
            raise EngineError("internal", f"unknown mode: {mode!r}")
        if not input_path.exists():
            raise EngineError("decode", f"input not found: {input_path}")
        workspace.mkdir(parents=True, exist_ok=True)

        if quality == "max" and mode == "two_stem":
            return self._separate_two_stem_ensemble(input_path, workspace, progress, cancelled)
        if quality == "max" and mode == "four_stem":
            return self._separate_four_stem_cascade(input_path, workspace, progress, cancelled)
        return self._separate_single(input_path, mode, workspace, progress, cancelled)

    def _separate_single(
        self,
        input_path: Path,
        mode: str,
        workspace: Path,
        progress: ProgressFn,
        cancelled: CancelledFn,
    ) -> SeparationResult:
        """Standard quality: one checkpoint, one pass (unchanged behaviour)."""
        model = _MODE_MODELS[mode]
        duration = _probe_duration(input_path)

        _raise_if_cancelled(cancelled)
        progress("loading_model", 0.05)
        separator = self._separator_for(model)

        _raise_if_cancelled(cancelled)
        progress("separating", _SEPARATING_FLOOR)
        self._clear_staging()
        factor = _load_perf_factor(self._model_dir, model, _PERF_SEEDS.get(mode, _DEFAULT_PERF_SEED))
        started = time.monotonic()
        stop_ticker = threading.Event()
        ticker = threading.Thread(
            target=_progress_ticker,
            args=(progress, started, max(duration * factor, 1.0), stop_ticker),
            daemon=True,
        )
        ticker.start()
        try:
            outputs = separator.separate(str(input_path))
        except JobCancelled:
            raise
        except Exception as exc:  # noqa: BLE001 - undecodable/unseparable input
            raise EngineError("decode", f"could not separate {input_path}: {exc}") from exc
        finally:
            stop_ticker.set()
            ticker.join(timeout=3)

        observed = (time.monotonic() - started) / max(duration, 0.1)
        _store_perf_factor(self._model_dir, model, _blend_perf(factor, observed))

        _raise_if_cancelled(cancelled)
        progress("writing", 0.9)

        moved = self._move_to_workspace(outputs, workspace)
        stem_paths = _map_outputs(moved, mode)
        source_samples, sample_rate = _measure_stems(stem_paths)
        _check_duration(duration, source_samples, sample_rate)

        stems = [(name, stem_paths[name]) for name in _STEM_MARKERS[mode]]
        return SeparationResult(
            stems=stems,
            source_samples=source_samples,
            sample_rate=sample_rate,
            model=model,
        )

    def _separate_two_stem_ensemble(
        self,
        input_path: Path,
        workspace: Path,
        progress: ProgressFn,
        cancelled: CancelledFn,
    ) -> SeparationResult:
        """two_stem + max: average two vocal/instrumental checkpoints pairwise.

        Both models run through the cached-Separator staging flow; each pass's
        outputs are relocated into the workspace (staging is cleared per pass)
        before the next pass runs. The final vocals/instrumental WAVs are the
        pairwise mean written as float32; both passes must share a frame count."""
        models = [TWO_STEM_MODEL, ENSEMBLE_VOCALS_MODEL]
        duration = _probe_duration(input_path)

        _raise_if_cancelled(cancelled)
        progress("loading_model", 0.05)
        progress("separating", _SEPARATING_FLOOR)
        _, stop_ticker, ticker = self._start_ticker(
            progress, self._estimate_seconds(duration, models)
        )
        try:
            out_a = self._separate_once(input_path, models[0], workspace, cancelled, duration, "ens_a")
            voc_a, inst_a = _split_vocals_complement(out_a, models[0])
            out_b = self._separate_once(input_path, models[1], workspace, cancelled, duration, "ens_b")
            voc_b, inst_b = _split_vocals_complement(out_b, models[1])
        finally:
            self._stop_ticker(stop_ticker, ticker)

        _raise_if_cancelled(cancelled)
        progress("writing", 0.9)

        vocals_path = workspace / "vocals.wav"
        instrumental_path = workspace / "instrumental.wav"
        sr = _average_wavs([voc_a, voc_b], vocals_path)
        _average_wavs([inst_a, inst_b], instrumental_path)

        stem_paths = {"vocals": vocals_path, "instrumental": instrumental_path}
        source_samples, sample_rate = _measure_stems(stem_paths)
        _check_duration(duration, source_samples, sample_rate)

        stems = [(name, stem_paths[name]) for name in _STEM_MARKERS["two_stem"]]
        return SeparationResult(
            stems=stems,
            source_samples=source_samples,
            sample_rate=sample_rate,
            model=f"ensemble({models[0]} + {models[1]})",
        )

    def _separate_four_stem_cascade(
        self,
        input_path: Path,
        workspace: Path,
        progress: ProgressFn,
        cancelled: CancelledFn,
    ) -> SeparationResult:
        """four_stem + max: cascade. Pass 1 (ep317) -> vocals, instrumental; pass 2
        (htdemucs_ft on the instrumental) -> drums, bass; other is the exact
        residual instrumental - drums - bass, so the four stems sum back to
        vocals + instrumental. All four outputs share one frame count."""
        models = [TWO_STEM_MODEL, CASCADE_DRUMBASS_MODEL]
        duration = _probe_duration(input_path)

        _raise_if_cancelled(cancelled)
        progress("loading_model", 0.05)
        progress("separating", _SEPARATING_FLOOR)
        _, stop_ticker, ticker = self._start_ticker(
            progress, self._estimate_seconds(duration, models)
        )
        try:
            out1 = self._separate_once(input_path, models[0], workspace, cancelled, duration, "casc_p1")
            vocals_src, instrumental_src = _split_vocals_complement(out1, models[0])
            out2 = self._separate_once(
                instrumental_src, models[1], workspace, cancelled, duration, "casc_p2"
            )
            drums_src = _find_marker(out2, "(drums)", models[1])
            bass_src = _find_marker(out2, "(bass)", models[1])
        finally:
            self._stop_ticker(stop_ticker, ticker)

        _raise_if_cancelled(cancelled)
        progress("writing", 0.9)

        vocals, sr_v = _load_float32(vocals_src)
        instrumental, sr_i = _load_float32(instrumental_src)
        drums, sr_d = _load_float32(drums_src)
        bass, sr_b = _load_float32(bass_src)
        _require_same_shape(
            {"vocals": vocals, "instrumental": instrumental, "drums": drums, "bass": bass}
        )
        other = (instrumental - drums - bass).astype("float32")

        stem_paths: dict[str, Path] = {}
        for name, arr, sr in (
            ("vocals", vocals, sr_v),
            ("drums", drums, sr_d),
            ("bass", bass, sr_b),
            ("other", other, sr_i),
        ):
            target = workspace / f"{name}.wav"
            _write_float32_wav(arr, sr, target)
            stem_paths[name] = target

        source_samples, sample_rate = _measure_stems(stem_paths)
        _check_duration(duration, source_samples, sample_rate)

        stems = [(name, stem_paths[name]) for name in _STEM_MARKERS["four_stem"]]
        return SeparationResult(
            stems=stems,
            source_samples=source_samples,
            sample_rate=sample_rate,
            model=(
                f"cascade({models[0]} -> {models[1]} [drums, bass]; "
                "other = instrumental - drums - bass)"
            ),
        )

    def _separate_once(
        self,
        input_path: Path,
        model: str,
        workspace: Path,
        cancelled: CancelledFn,
        duration: float,
        prefix: str,
    ) -> list[Path]:
        """Run one checkpoint over ``input_path`` and relocate its staged outputs
        into ``workspace`` under ``prefix``-tagged names. Checks cancellation
        before the pass and updates the model's learned perf factor after it."""
        _raise_if_cancelled(cancelled)
        separator = self._separator_for(model)
        self._clear_staging()
        seed = _MODEL_PERF_SEEDS.get(model, _DEFAULT_PERF_SEED)
        factor = _load_perf_factor(self._model_dir, model, seed)
        started = time.monotonic()
        try:
            outputs = separator.separate(str(input_path))
        except JobCancelled:
            raise
        except Exception as exc:  # noqa: BLE001 - undecodable/unseparable input
            raise EngineError("decode", f"could not separate {input_path}: {exc}") from exc
        observed = (time.monotonic() - started) / max(duration, 0.1)
        _store_perf_factor(self._model_dir, model, _blend_perf(factor, observed))
        return self._move_pass_outputs(outputs, workspace, prefix)

    def _estimate_seconds(self, duration: float, models: list[str]) -> float:
        """Progress-ticker estimate for a multi-pass job: the sum of each pass's
        (learned or seeded) perf factor times the source duration."""
        total = sum(
            _load_perf_factor(self._model_dir, m, _MODEL_PERF_SEEDS.get(m, _DEFAULT_PERF_SEED))
            for m in models
        )
        return max(duration * total, 1.0)

    def _start_ticker(self, progress: ProgressFn, estimate_s: float):
        started = time.monotonic()
        stop = threading.Event()
        ticker = threading.Thread(
            target=_progress_ticker, args=(progress, started, estimate_s, stop), daemon=True
        )
        ticker.start()
        return started, stop, ticker

    def _stop_ticker(self, stop: threading.Event, ticker: threading.Thread) -> None:
        stop.set()
        ticker.join(timeout=3)

    def prefetch(self, progress: ProgressFn, cancelled: CancelledFn) -> list[str]:
        models = [TWO_STEM_MODEL, FOUR_STEM_MODEL]
        for i, model in enumerate(models):
            _raise_if_cancelled(cancelled)
            progress("downloading", i / len(models))
            self._separator_for(model)
        return models

    def _separator_for(self, model: str):
        if self._separator is None:
            self._staging.mkdir(parents=True, exist_ok=True)
            self._separator = self._make_separator()
        if self._loaded_model != model:
            self._separator.load_model(model_filename=model)
            self._loaded_model = model
        return self._separator

    def _make_separator(self):
        if self._separator_factory is not None:
            return self._separator_factory(str(self._model_dir), str(self._staging))
        from audio_separator.separator import Separator

        return Separator(
            model_file_dir=str(self._model_dir),
            output_dir=str(self._staging),
            output_format="WAV",
            log_level=logging.WARNING,
        )

    def _clear_staging(self) -> None:
        for stale in self._staging.glob("*"):
            try:
                stale.unlink()
            except OSError:
                pass

    def _move_to_workspace(self, outputs: list[str], workspace: Path) -> list[Path]:
        moved: list[Path] = []
        for out in outputs:
            path = Path(out)
            if not path.is_absolute():
                path = self._staging / path.name
            target = workspace / path.name
            try:
                shutil.move(str(path), str(target))
            except OSError as exc:
                raise EngineError("internal", f"could not move {path} to {target}: {exc}") from exc
            moved.append(target)
        return moved

    def _move_pass_outputs(self, outputs: list[str], workspace: Path, prefix: str) -> list[Path]:
        """Like ``_move_to_workspace`` but tags each moved file with a per-pass
        ``prefix`` so a later pass (which clears staging and may reuse stem names)
        cannot clobber an earlier pass's intermediates in the workspace."""
        moved: list[Path] = []
        for out in outputs:
            path = Path(out)
            if not path.is_absolute():
                path = self._staging / path.name
            target = workspace / f"{prefix}__{path.name}"
            try:
                shutil.move(str(path), str(target))
            except OSError as exc:
                raise EngineError("internal", f"could not move {path} to {target}: {exc}") from exc
            moved.append(target)
        return moved


# --- helpers -----------------------------------------------------------------


def _raise_if_cancelled(cancelled: CancelledFn) -> None:
    if cancelled():
        raise JobCancelled()


def _map_outputs(resolved: list[Path], mode: str) -> dict[str, Path]:
    """Match each protocol stem name to the output file whose name contains the
    stem marker (case-insensitive)."""
    stem_paths: dict[str, Path] = {}
    for name, marker in _STEM_MARKERS[mode].items():
        match = next((p for p in resolved if marker in p.name.lower()), None)
        if match is None:
            raise EngineError(
                "model",
                f"missing expected stem {name!r} ({marker}) in outputs: "
                f"{[p.name for p in resolved]}",
            )
        stem_paths[name] = match
    return stem_paths


def _split_vocals_complement(outputs: list[Path], model: str) -> tuple[Path, Path]:
    """Split a 2-stem model's outputs into (vocals, complement). The complement is
    whatever the model calls the non-vocal stem — ``instrumental`` for ep317,
    ``other`` for the Mel-Band RoFormer — so both are handled by matching only
    the vocals marker and taking the remaining file."""
    if len(outputs) != 2:
        raise EngineError(
            "model",
            f"{model}: expected 2 stem outputs, got {[p.name for p in outputs]}",
        )
    vocals = next((p for p in outputs if "(vocals)" in p.name.lower()), None)
    if vocals is None:
        raise EngineError(
            "model", f"{model}: no vocals stem in {[p.name for p in outputs]}"
        )
    complement = next(p for p in outputs if p != vocals)
    return vocals, complement


def _find_marker(outputs: list[Path], marker: str, model: str) -> Path:
    """Return the output whose filename contains ``marker`` (case-insensitive)."""
    match = next((p for p in outputs if marker in p.name.lower()), None)
    if match is None:
        raise EngineError(
            "model", f"{model}: missing {marker} stem in {[p.name for p in outputs]}"
        )
    return match


def _load_float32(path: Path) -> tuple[np.ndarray, int]:
    try:
        data, sr = sf.read(str(path), always_2d=True, dtype="float32")
    except Exception as exc:  # noqa: BLE001 - a written stem must be readable
        raise EngineError("internal", f"could not read stem {path}: {exc}") from exc
    return data, sr


def _write_float32_wav(arr: np.ndarray, sr: int, target: Path) -> None:
    try:
        sf.write(str(target), arr, sr, subtype="FLOAT", format="WAV")
    except Exception as exc:  # noqa: BLE001 - surfaced as an invariant failure
        raise EngineError("internal", f"could not write stem {target}: {exc}") from exc


def _require_same_shape(named: dict[str, np.ndarray]) -> None:
    """All arrays must share an identical (frames, channels) shape, else internal."""
    shapes = {name: arr.shape for name, arr in named.items()}
    if len(set(shapes.values())) != 1:
        raise EngineError("internal", f"stem frame/channel counts differ: {shapes}")


def _average_wavs(paths: list[Path], out_path: Path) -> int:
    """Load every WAV in ``paths``, require identical shape, and write their
    sample-wise mean as a float32 WAV to ``out_path``. Returns the sample rate."""
    arrays: list[np.ndarray] = []
    rates: set[int] = set()
    for p in paths:
        data, sr = _load_float32(p)
        arrays.append(data)
        rates.add(sr)
    if len(rates) != 1:
        raise EngineError("internal", f"ensemble stem sample rates differ: {sorted(rates)}")
    _require_same_shape({str(i): a for i, a in enumerate(arrays)})
    mean = np.mean(np.stack(arrays, axis=0), axis=0).astype("float32")
    sr = rates.pop()
    _write_float32_wav(mean, sr, out_path)
    return sr


def _measure_stems(stem_paths: dict[str, Path]) -> tuple[int, int]:
    """Return the shared (frame count, sample rate) across all stems.

    source_samples is defined off the separated stems rather than the input file:
    audio-separator decodes and may resample internally, so the stems are the
    canonical post-decode timeline. The exporter's length invariant compares
    exported output against this count, so it must be the stems' own frame count.
    """
    frames: dict[str, int] = {}
    rates: set[int] = set()
    for name, path in stem_paths.items():
        try:
            info = sf.info(str(path))
        except Exception as exc:  # noqa: BLE001 - a written stem must be readable
            raise EngineError("internal", f"could not read stem {path}: {exc}") from exc
        frames[name] = info.frames
        rates.add(info.samplerate)

    if len(set(frames.values())) != 1:
        raise EngineError("internal", f"stem frame counts differ: {frames}")
    if len(rates) != 1:
        raise EngineError("internal", f"stem sample rates differ: {sorted(rates)}")
    return next(iter(frames.values())), rates.pop()


def _progress_ticker(progress, started: float, estimate_s: float,
                     stop: threading.Event) -> None:
    """Emit estimated separating progress every 2s until the model returns.
    Caps below the writing stage so the bar never runs backwards."""
    while not stop.wait(2.0):
        fraction = min((time.monotonic() - started) / estimate_s, 1.0)
        progress("separating", _SEPARATING_FLOOR + _SEPARATING_SPAN * fraction)


def _perf_path(model_dir: Path) -> Path:
    return model_dir / "perf.json"


def _load_perf_factor(model_dir: Path, model: str, seed: float) -> float:
    try:
        return float(json.loads(_perf_path(model_dir).read_text())[model])
    except Exception:  # noqa: BLE001 - missing/corrupt perf data falls back to seed
        return seed


def _store_perf_factor(model_dir: Path, model: str, factor: float) -> None:
    data: dict[str, float] = {}
    try:
        data = json.loads(_perf_path(model_dir).read_text())
    except Exception:  # noqa: BLE001 - start fresh on missing/corrupt file
        pass
    data[model] = round(factor, 4)
    try:
        model_dir.mkdir(parents=True, exist_ok=True)
        _perf_path(model_dir).write_text(json.dumps(data))
    except OSError:
        pass  # perf memory is an optimization; never fail the job over it


def _blend_perf(previous: float, observed: float) -> float:
    return previous * (1 - _PERF_EMA_WEIGHT) + observed * _PERF_EMA_WEIGHT


def _check_duration(duration: float, source_samples: int, sample_rate: int) -> None:
    """Sanity-check the stem timeline against the source duration to catch gross
    truncation, tolerating resample/codec-delay rounding at the edges."""
    stem_seconds = source_samples / sample_rate
    if abs(stem_seconds - duration) >= _DURATION_TOLERANCE_S:
        raise EngineError(
            "internal",
            f"stem timeline {stem_seconds:.3f}s deviates from source "
            f"{duration:.3f}s (source_samples={source_samples}, sr={sample_rate})",
        )


def _find_ffprobe() -> str | None:
    found = shutil.which("ffprobe")
    if found:
        return found
    fallback = "/opt/homebrew/bin/ffprobe"
    return fallback if Path(fallback).exists() else None


def _probe_duration(input_path: Path) -> float:
    """Source duration in seconds: ffprobe when available, else libsndfile.

    ffmpeg isn't guaranteed on other machines. soundfile covers every format
    we support except M4A/AAC — and those can't be decoded by the separator
    without ffmpeg either, so the error message points at the real fix."""
    ffprobe = _find_ffprobe()
    if ffprobe is None:
        try:
            info = sf.info(str(input_path))
            return info.frames / info.samplerate
        except Exception as exc:  # noqa: BLE001 - m4a/aac (or corrupt) without ffmpeg
            raise EngineError(
                "decode",
                f"cannot read {input_path.name} without ffmpeg — "
                "install it with: brew install ffmpeg",
            ) from exc
    try:
        result = subprocess.run(
            [ffprobe, "-v", "error", "-show_entries", "format=duration",
             "-of", "csv=p=0", str(input_path)],
            capture_output=True,
            text=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        raise EngineError("decode", f"ffprobe failed on {input_path}: {exc}") from exc
    text = result.stdout.strip()
    try:
        return float(text)
    except ValueError:
        raise EngineError(
            "decode", f"ffprobe returned no duration for {input_path}: {text!r}"
        ) from None
