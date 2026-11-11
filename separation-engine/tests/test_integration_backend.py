"""End-to-end backend tests. The network/model test drives the real audio-separator
stack through the JSON-lines protocol as a subprocess."""

from __future__ import annotations

import json
import queue
import subprocess
import threading
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

pytest.importorskip("audio_separator")

# The app's default model dir (Splittr): downloading here caches checkpoints for
# the shipped app. It already holds ep317 (two_stem) and htdemucs_ft (four_stem),
# so four_stem max adds no new download and two_stem max only fetches the second
# ensemble vocal checkpoint.
DEFAULT_MODEL_DIR = Path(
    "~/Library/Application Support/Splittr/models"
).expanduser()

ENGINE_DIR = Path(__file__).resolve().parents[1]

SAMPLE_RATE = 44100
DURATION_S = 5


def _write_fixture_wav(path: Path) -> None:
    """5 s of 44.1 kHz stereo: a 440 Hz sine plus white noise, so the model's
    vocal/instrumental masks have real signal to separate."""
    t = np.linspace(0, DURATION_S, DURATION_S * SAMPLE_RATE, endpoint=False)
    tone = 0.3 * np.sin(2 * np.pi * 440 * t)
    noise = 0.05 * np.random.default_rng(0).standard_normal(t.shape)
    mono = (tone + noise).astype(np.float32)
    stereo = np.stack([mono, mono], axis=1)
    sf.write(str(path), stereo, SAMPLE_RATE, subtype="PCM_16")


class _Engine:
    """Spawns the worker and reads its protocol events off a background thread."""

    def __init__(self, model_dir: Path, stderr_log: Path) -> None:
        self._errf = open(stderr_log, "w")
        self._proc = subprocess.Popen(
            [
                "uv", "run", "--no-sync", "python", "-m", "stemsplitter_engine",
                "--model-dir", str(model_dir),
            ],
            cwd=str(ENGINE_DIR),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self._errf,
            text=True,
        )
        self._events: queue.Queue[dict | None] = queue.Queue()
        self._reader = threading.Thread(target=self._pump, daemon=True)
        self._reader.start()

    def _pump(self) -> None:
        for line in self._proc.stdout:
            line = line.strip()
            if line:
                self._events.put(json.loads(line))
        self._events.put(None)

    def send(self, obj: dict) -> None:
        self._proc.stdin.write(json.dumps(obj) + "\n")
        self._proc.stdin.flush()

    def recv(self, timeout: float) -> dict:
        evt = self._events.get(timeout=timeout)
        if evt is None:
            raise AssertionError("engine exited before emitting an event")
        return evt

    def recv_terminal(self, timeout: float) -> dict:
        while True:
            evt = self.recv(timeout)
            if evt["type"] in ("done", "error"):
                return evt

    def close(self) -> None:
        try:
            self.send({"cmd": "shutdown"})
            self._proc.wait(timeout=30)
        except Exception:
            self._proc.kill()
        finally:
            self._errf.close()


def test_default_models_without_audio_separator_touch():
    """Fast, no network: the backend module imports and default_models() returns
    both keys. audio_separator is imported lazily inside methods, so it is never
    bound at module scope (the import must succeed without the full extra)."""
    import stemsplitter_engine.audio_separator_backend as backend

    assert not hasattr(backend, "Separator")
    b = backend.AudioSeparatorBackend(Path("/nonexistent"))
    models = b.default_models()
    assert set(models) == {"two_stem", "four_stem"}
    assert models["two_stem"] == backend.TWO_STEM_MODEL
    assert models["four_stem"] == backend.FOUR_STEM_MODEL


@pytest.mark.integration
def test_real_two_stem_separation(tmp_path):
    input_wav = tmp_path / "fixture.wav"
    _write_fixture_wav(input_wav)
    workspace = tmp_path / "workspace"
    DEFAULT_MODEL_DIR.mkdir(parents=True, exist_ok=True)

    engine = _Engine(DEFAULT_MODEL_DIR, tmp_path / "engine.stderr.log")
    try:
        hello = engine.recv(timeout=60)
        assert hello["type"] == "hello"
        assert hello["protocol"] == 1

        engine.send(
            {
                "cmd": "separate",
                "id": "job-1",
                "input": str(input_wav),
                "mode": "two_stem",
                "workspace": str(workspace),
            }
        )
        # First run downloads the BS-RoFormer checkpoint, then separates on CPU/MPS.
        evt = engine.recv_terminal(timeout=600)
        assert evt["type"] == "done", evt
        result = evt["result"]

        names = [s["name"] for s in result["stems"]]
        assert sorted(names) == ["instrumental", "vocals"]
        assert result["sample_rate"] == SAMPLE_RATE

        for stem in result["stems"]:
            path = Path(stem["path"])
            assert path.exists(), path
            assert sf.info(str(path)).frames == result["source_samples"]
    finally:
        engine.close()


@pytest.mark.integration
def test_real_two_stem_max_ensemble(tmp_path):
    """quality=max two_stem: downloads the second (Mel-Band RoFormer) vocal
    checkpoint into the app model dir, runs BOTH models, and returns the pairwise
    average. Asserts two stems, equal frames, and the ensemble model string."""
    input_wav = tmp_path / "fixture.wav"
    _write_fixture_wav(input_wav)
    workspace = tmp_path / "workspace"
    DEFAULT_MODEL_DIR.mkdir(parents=True, exist_ok=True)

    engine = _Engine(DEFAULT_MODEL_DIR, tmp_path / "engine.stderr.log")
    try:
        hello = engine.recv(timeout=60)
        assert hello["type"] == "hello"

        engine.send(
            {
                "cmd": "separate",
                "id": "job-max2",
                "input": str(input_wav),
                "mode": "two_stem",
                "workspace": str(workspace),
                "quality": "max",
            }
        )
        # First run downloads the second checkpoint, then runs two RoFormer passes.
        evt = engine.recv_terminal(timeout=1800)
        assert evt["type"] == "done", evt
        result = evt["result"]

        names = sorted(s["name"] for s in result["stems"])
        assert names == ["instrumental", "vocals"]  # exactly two stems
        assert result["model"].startswith("ensemble("), result["model"]
        assert result["sample_rate"] == SAMPLE_RATE

        frame_counts = set()
        for stem in result["stems"]:
            path = Path(stem["path"])
            assert path.exists(), path
            frame_counts.add(sf.info(str(path)).frames)
        assert frame_counts == {result["source_samples"]}  # both stems equal frames
    finally:
        engine.close()


@pytest.mark.integration
def test_real_four_stem_max_cascade(tmp_path):
    """quality=max four_stem: cascade ep317 -> htdemucs_ft(instrumental). Both
    checkpoints are already cached in the app model dir (ep317 + htdemucs_ft), so
    this adds no new download. Asserts four stems, equal frames, and the cascade
    model string."""
    input_wav = tmp_path / "fixture.wav"
    _write_fixture_wav(input_wav)
    workspace = tmp_path / "workspace"
    DEFAULT_MODEL_DIR.mkdir(parents=True, exist_ok=True)

    engine = _Engine(DEFAULT_MODEL_DIR, tmp_path / "engine.stderr.log")
    try:
        hello = engine.recv(timeout=60)
        assert hello["type"] == "hello"

        engine.send(
            {
                "cmd": "separate",
                "id": "job-max4",
                "input": str(input_wav),
                "mode": "four_stem",
                "workspace": str(workspace),
                "quality": "max",
            }
        )
        # ep317 pass + htdemucs_ft (bag-of-4) pass on the instrumental.
        evt = engine.recv_terminal(timeout=1800)
        assert evt["type"] == "done", evt
        result = evt["result"]

        names = sorted(s["name"] for s in result["stems"])
        assert names == ["bass", "drums", "other", "vocals"]  # exactly four stems
        assert result["model"].startswith("cascade("), result["model"]
        assert result["sample_rate"] == SAMPLE_RATE

        frame_counts = set()
        for stem in result["stems"]:
            path = Path(stem["path"])
            assert path.exists(), path
            frame_counts.add(sf.info(str(path)).frames)
        assert frame_counts == {result["source_samples"]}  # all four equal frames
    finally:
        engine.close()
