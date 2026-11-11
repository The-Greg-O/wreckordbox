"""The fake worker wreckordbox's tests launch: the real protocol, as a child
process, with stems written by a fake backend."""

from __future__ import annotations

import json
import subprocess
import sys
import wave
from pathlib import Path

SCRIPT = Path(__file__).parent / "fake_worker.py"


def write_source(path: Path, frames: int) -> None:
    with wave.open(str(path), "wb") as out:
        out.setnchannels(2)
        out.setsampwidth(2)
        out.setframerate(44100)
        out.writeframes(bytes(4 * frames))


def frames_of(path: str) -> int:
    with wave.open(path, "rb") as stem:
        return stem.getnframes()


def separate(tmp_path: Path, *options: str) -> dict:
    """Runs one four-stem separation of a 1000-frame source and answers the
    worker's last event."""
    source = tmp_path / "Track.wav"
    write_source(source, 1000)
    worker = subprocess.Popen(
        [sys.executable, str(SCRIPT), *options],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        hello = json.loads(worker.stdout.readline())
        assert hello["type"] == "hello" and hello["protocol"] == 1
        request = {
            "cmd": "separate",
            "id": "1",
            "input": str(source),
            "mode": "four_stem",
            "workspace": str(tmp_path / "out"),
        }
        worker.stdin.write(json.dumps(request) + "\n")
        worker.stdin.flush()
        while True:
            event = json.loads(worker.stdout.readline())
            if event["type"] != "progress":
                break
        worker.stdin.write('{"cmd":"shutdown"}\n')
        worker.stdin.flush()
        assert worker.wait(timeout=10) == 0
        return event
    finally:
        worker.kill()


def test_four_stems_are_written_as_long_as_the_source(tmp_path):
    done = separate(tmp_path)

    assert done["type"] == "done"
    result = done["result"]
    assert [stem["name"] for stem in result["stems"]] == ["vocals", "drums", "bass", "other"]
    assert result["source_samples"] == 1000
    assert result["sample_rate"] == 44100
    assert [frames_of(stem["path"]) for stem in result["stems"]] == [1000] * 4


def test_a_stem_can_be_made_short_and_each_run_is_logged(tmp_path):
    log = tmp_path / "runs.log"

    done = separate(tmp_path, "--short", "drums", "--log", str(log))

    lengths = {stem["name"]: frames_of(stem["path"]) for stem in done["result"]["stems"]}
    assert lengths == {"vocals": 1000, "drums": 999, "bass": 1000, "other": 1000}
    assert len(log.read_text().splitlines()) == 1


def test_it_can_be_told_to_fail(tmp_path):
    error = separate(tmp_path, "--fail", "model")

    assert error["type"] == "error"
    assert error["category"] == "model"
