"""The real worker protocol over a fake backend, as a process of its own.

    python3 separation-engine/tests/fake_worker.py [options]

What wreckordbox's own tests launch in place of the engine: `Worker` is the
real one (hello, progress, done, error, cancel, shutdown), and only the
separation is faked. The fake writes one silent 16-bit WAV per stem, as long
as the source, so no model, no download and no third-party package is
needed: this file and `stemsplitter_engine.worker` use the standard library
only. The source must be a WAV file.

Options, all for tests:

--log PATH        append one JSON line per `separate` command, so a test can
                  count how many times the engine was really asked to work
--short ROLE      write that stem one frame shorter than the source
--delay SECONDS   take this long over the separation, reporting progress and
                  honouring `cancel`
--fail CATEGORY   fail every separation with this error category
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import wave
from pathlib import Path

# Runs from a checkout with no install: the package is beside this folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))

from stemsplitter_engine.backend import JobCancelled, SeparationResult
from stemsplitter_engine.worker import Worker

FOUR_STEMS = ("vocals", "drums", "bass", "other")
TWO_STEMS = ("vocals", "instrumental")
MODEL = "fake-separator"


class FakeFailure(Exception):
    def __init__(self, category: str) -> None:
        super().__init__(f"the fake backend was told to fail ({category})")
        self.category = category


class FileWritingFakeBackend:
    def __init__(self, log: Path | None, short: str | None, delay: float, fail: str | None) -> None:
        self._log = log
        self._short = short
        self._delay = delay
        self._fail = fail

    def default_models(self) -> dict[str, str]:
        return {"two_stem": MODEL, "four_stem": MODEL}

    def separate(self, input_path, mode, workspace, progress, cancelled, quality="standard"):
        if self._log is not None:
            with self._log.open("a") as log:
                log.write(json.dumps({"input": str(input_path), "mode": mode, "quality": quality}) + "\n")
        progress("loading_model", 0.05)
        if self._fail is not None:
            raise FakeFailure(self._fail)
        with wave.open(str(input_path), "rb") as source:
            frames = source.getnframes()
            rate = source.getframerate()
            channels = source.getnchannels()

        steps = max(1, int(self._delay / 0.05))
        for step in range(steps):
            if cancelled():
                raise JobCancelled("cancelled")
            progress("separating", 0.15 + 0.7 * step / steps)
            if self._delay:
                time.sleep(self._delay / steps)

        progress("writing", 0.9)
        workspace.mkdir(parents=True, exist_ok=True)
        stems = []
        for name in FOUR_STEMS if mode == "four_stem" else TWO_STEMS:
            path = workspace / f"{input_path.stem}_({name.capitalize()})_{MODEL}.wav"
            length = frames - 1 if name == self._short else frames
            with wave.open(str(path), "wb") as out:
                out.setnchannels(channels)
                out.setsampwidth(2)
                out.setframerate(rate)
                out.writeframes(bytes(2 * channels * length))
            stems.append((name, path))
        return SeparationResult(stems=stems, source_samples=frames, sample_rate=rate, model=MODEL)

    def prefetch(self, progress, cancelled):
        progress("downloading", 1.0)
        return [MODEL]


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(prog="fake_worker")
    parser.add_argument("--log", type=Path, default=None)
    parser.add_argument("--short", default=None)
    parser.add_argument("--delay", type=float, default=0.0)
    parser.add_argument("--fail", default=None)
    args = parser.parse_args(argv)
    Worker(FileWritingFakeBackend(args.log, args.short, args.delay, args.fail)).run()


if __name__ == "__main__":
    main(sys.argv[1:])
