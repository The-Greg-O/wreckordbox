from stemsplitter_engine.audio_separator_backend import (
    _blend_perf,
    _load_perf_factor,
    _store_perf_factor,
)


def test_perf_round_trip(tmp_path):
    _store_perf_factor(tmp_path, "model-a.ckpt", 0.61)
    assert _load_perf_factor(tmp_path, "model-a.ckpt", 9.9) == 0.61


def test_perf_falls_back_to_seed(tmp_path):
    assert _load_perf_factor(tmp_path, "missing.ckpt", 0.65) == 0.65
    (tmp_path / "perf.json").write_text("not json")
    assert _load_perf_factor(tmp_path, "missing.ckpt", 0.65) == 0.65


def test_perf_store_preserves_other_models(tmp_path):
    _store_perf_factor(tmp_path, "a", 0.5)
    _store_perf_factor(tmp_path, "b", 0.3)
    assert _load_perf_factor(tmp_path, "a", 0) == 0.5
    assert _load_perf_factor(tmp_path, "b", 0) == 0.3


def test_blend_moves_toward_observed():
    blended = _blend_perf(0.65, 0.45)
    assert 0.45 < blended < 0.65
    assert abs(blended - 0.57) < 0.001
