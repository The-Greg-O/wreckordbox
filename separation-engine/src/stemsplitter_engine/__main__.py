"""Engine entry point: ``python -m stemsplitter_engine``.

stdout is protocol-only, so logging is routed to stderr before anything else runs
(audio-separator and its dependencies log verbosely and MUST NOT leak into stdout).
"""

from __future__ import annotations

import argparse
import logging
import os
import sys
from pathlib import Path

_DEFAULT_MODEL_DIR = "~/Library/Application Support/Dubplate/models"


def _ensure_ffmpeg_on_path(support_dir: Path) -> None:
    """audio-separator shells out to a bare ``ffmpeg``, which a Finder-launched
    app never finds: the GUI PATH has no homebrew. Prefer the static binary the
    venv ships via imageio-ffmpeg (works on machines with no ffmpeg at all),
    exposed under the stable name through a shim symlink; append the homebrew
    locations as fallback for anything else that expects them."""
    entries: list[str] = []
    try:
        from imageio_ffmpeg import get_ffmpeg_exe

        exe = Path(get_ffmpeg_exe())
        shim_dir = support_dir / "bin"
        shim_dir.mkdir(parents=True, exist_ok=True)
        link = shim_dir / "ffmpeg"
        # Recreate each start: idempotent, and survives venv rebuilds moving
        # the target binary.
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(exe)
        entries.append(str(shim_dir))
    except Exception:  # noqa: BLE001 - shim is best-effort; brew paths below still help
        pass
    for candidate in ("/opt/homebrew/bin", "/usr/local/bin"):
        if Path(candidate).is_dir():
            entries.append(candidate)
    os.environ["PATH"] = ":".join(entries + [os.environ.get("PATH", "")])


def main(argv: list[str] | None = None) -> None:
    logging.basicConfig(stream=sys.stderr, level=logging.WARNING)

    parser = argparse.ArgumentParser(prog="stemsplitter_engine")
    parser.add_argument("--model-dir", default=_DEFAULT_MODEL_DIR)
    args = parser.parse_args(argv)

    from .audio_separator_backend import AudioSeparatorBackend
    from .worker import Worker

    model_dir = Path(args.model_dir).expanduser()
    model_dir.mkdir(parents=True, exist_ok=True)
    _ensure_ffmpeg_on_path(model_dir.parent)
    Worker(AudioSeparatorBackend(model_dir)).run()


if __name__ == "__main__":
    main()
