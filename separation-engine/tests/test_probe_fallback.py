import numpy as np
import pytest
import soundfile as sf

from stemsplitter_engine import audio_separator_backend as backend


def _write_wav(path, seconds=2.0, sr=44100):
    data = np.zeros((int(seconds * sr), 2), dtype="float32")
    sf.write(str(path), data, sr, subtype="PCM_16", format="WAV")


def test_duration_falls_back_to_soundfile_without_ffprobe(tmp_path, monkeypatch):
    wav = tmp_path / "t.wav"
    _write_wav(wav, seconds=2.0)
    monkeypatch.setattr(backend, "_find_ffprobe", lambda: None)
    assert backend._probe_duration(wav) == pytest.approx(2.0, abs=0.001)


def test_unreadable_without_ffprobe_names_the_fix(tmp_path, monkeypatch):
    fake_m4a = tmp_path / "t.m4a"
    fake_m4a.write_bytes(b"not audio")
    monkeypatch.setattr(backend, "_find_ffprobe", lambda: None)
    with pytest.raises(backend.EngineError) as err:
        backend._probe_duration(fake_m4a)
    assert err.value.category == "decode"
    assert "brew install ffmpeg" in str(err.value)


def test_ffprobe_used_when_available(tmp_path):
    # On this machine ffprobe exists; the normal path must still work.
    if backend._find_ffprobe() is None:
        pytest.skip("no ffprobe on this machine")
    wav = tmp_path / "t.wav"
    _write_wav(wav, seconds=1.0)
    assert backend._probe_duration(wav) == pytest.approx(1.0, abs=0.01)
