import os
import sys
import types
from pathlib import Path

from stemsplitter_engine.__main__ import _ensure_ffmpeg_on_path


def test_shim_symlink_and_path_order(tmp_path, monkeypatch):
    fake_exe = tmp_path / "ffmpeg-real-binary"
    fake_exe.write_bytes(b"#!/bin/sh\n")
    fake_module = types.ModuleType("imageio_ffmpeg")
    fake_module.get_ffmpeg_exe = lambda: str(fake_exe)
    monkeypatch.setitem(sys.modules, "imageio_ffmpeg", fake_module)
    monkeypatch.setenv("PATH", "/usr/bin:/bin")

    support = tmp_path / "support"
    _ensure_ffmpeg_on_path(support)

    link = support / "bin" / "ffmpeg"
    assert link.is_symlink()
    assert link.resolve() == fake_exe
    assert os.environ["PATH"].startswith(str(support / "bin"))
    assert os.environ["PATH"].endswith("/usr/bin:/bin")


def test_shim_recreated_when_target_moves(tmp_path, monkeypatch):
    old = tmp_path / "old-binary"
    old.write_bytes(b"")
    new = tmp_path / "new-binary"
    new.write_bytes(b"")
    support = tmp_path / "support"
    (support / "bin").mkdir(parents=True)
    (support / "bin" / "ffmpeg").symlink_to(old)

    fake_module = types.ModuleType("imageio_ffmpeg")
    fake_module.get_ffmpeg_exe = lambda: str(new)
    monkeypatch.setitem(sys.modules, "imageio_ffmpeg", fake_module)
    monkeypatch.setenv("PATH", "/usr/bin:/bin")

    _ensure_ffmpeg_on_path(support)
    assert (support / "bin" / "ffmpeg").resolve() == new


def test_missing_imageio_still_appends_brew_paths(tmp_path, monkeypatch):
    monkeypatch.setitem(sys.modules, "imageio_ffmpeg", None)  # import raises
    monkeypatch.setenv("PATH", "/usr/bin:/bin")

    _ensure_ffmpeg_on_path(tmp_path / "support")

    path = os.environ["PATH"]
    assert path.endswith("/usr/bin:/bin")
    if Path("/opt/homebrew/bin").is_dir():
        assert "/opt/homebrew/bin" in path
    assert not (tmp_path / "support" / "bin" / "ffmpeg").exists()
