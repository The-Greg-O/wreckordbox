from __future__ import annotations

import json
import os
import queue
import threading
from pathlib import Path

import pytest

from stemsplitter_engine import PROTOCOL_VERSION, __version__
from stemsplitter_engine.backend import JobCancelled, SeparationResult
from stemsplitter_engine.worker import Worker


# --------------------------------------------------------------------------
# Test doubles
# --------------------------------------------------------------------------

DEFAULT_MODELS = {"two_stem": "two.ckpt", "four_stem": "four.ckpt"}


class FakeBackend:
    def __init__(self, models: dict[str, str] | None = None) -> None:
        self._models = models if models is not None else dict(DEFAULT_MODELS)

    def default_models(self) -> dict[str, str]:
        return self._models

    def separate(self, input_path, mode, workspace, progress, cancelled, quality="standard"):
        self.last_quality = quality
        progress("separating", 0.5)
        return SeparationResult(
            stems=[
                ("vocals", workspace / "vocals.wav"),
                ("instrumental", workspace / "instrumental.wav"),
            ],
            source_samples=44100,
            sample_rate=44100,
            model="two.ckpt",
        )

    def prefetch(self, progress, cancelled):
        progress("downloading", 1.0)
        return ["two.ckpt", "four.ckpt"]


class BlockingBackend(FakeBackend):
    """separate() blocks until release is set, so the test controls timing."""

    def __init__(self) -> None:
        super().__init__()
        self.entered = threading.Event()
        self.release = threading.Event()

    def separate(self, input_path, mode, workspace, progress, cancelled, quality="standard"):
        self.entered.set()
        assert self.release.wait(5), "release never set"
        return SeparationResult(
            stems=[("vocals", workspace / "vocals.wav")],
            source_samples=1,
            sample_rate=44100,
            model="two.ckpt",
        )


class CancelBackend(FakeBackend):
    """separate() reaches a stage boundary, checks cancelled(), raises if set."""

    def __init__(self) -> None:
        super().__init__()
        self.entered = threading.Event()
        self.may_check = threading.Event()

    def separate(self, input_path, mode, workspace, progress, cancelled, quality="standard"):
        self.entered.set()
        assert self.may_check.wait(5), "may_check never set"
        if cancelled():
            raise JobCancelled("cancelled at stage boundary")
        return SeparationResult(
            stems=[("vocals", workspace / "vocals.wav")],
            source_samples=1,
            sample_rate=44100,
            model="two.ckpt",
        )


class CategorizedError(Exception):
    def __init__(self, message: str, category: str) -> None:
        super().__init__(message)
        self.category = category


class ErrorBackend(FakeBackend):
    def __init__(self, exc: Exception) -> None:
        super().__init__()
        self._exc = exc

    def separate(self, input_path, mode, workspace, progress, cancelled, quality="standard"):
        raise self._exc


class FakeExporter:
    def __init__(self, files: list[Path]) -> None:
        self.files = files
        self.calls: list[dict] = []

    def __call__(
        self, stems, source, source_samples, span, fmt, out_dir, suffixes, progress,
        combine_suffix=None,
    ):
        self.calls.append(
            {
                "stems": stems,
                "source": source,
                "source_samples": source_samples,
                "span": span,
                "fmt": fmt,
                "out_dir": out_dir,
                "suffixes": suffixes,
                "combine_suffix": combine_suffix,
            }
        )
        progress("exporting", 1.0)
        return self.files


# --------------------------------------------------------------------------
# Harness: drives Worker.run() on a thread over a real pipe (stdin) and a
# queue-backed writer (stdout). Fully deterministic — no sleeps.
# --------------------------------------------------------------------------


class QueueWriter:
    def __init__(self) -> None:
        self.q: queue.Queue[str] = queue.Queue()
        self._buf = ""
        self._lock = threading.Lock()

    def write(self, s: str) -> int:
        with self._lock:
            self._buf += s
            while "\n" in self._buf:
                line, self._buf = self._buf.split("\n", 1)
                self.q.put(line)
        return len(s)

    def flush(self) -> None:
        pass


class Harness:
    def __init__(self, backend, exporter=None) -> None:
        r_fd, w_fd = os.pipe()
        self._stdin_r = os.fdopen(r_fd, "r")
        self._stdin_w = os.fdopen(w_fd, "w")
        self.out = QueueWriter()
        self.worker = Worker(backend, stdin=self._stdin_r, stdout=self.out, exporter=exporter)
        self.thread = threading.Thread(target=self.worker.run, daemon=True)

    def __enter__(self) -> "Harness":
        self.thread.start()
        return self

    def __exit__(self, *exc) -> None:
        self.close_stdin()
        self.thread.join(timeout=5.0)
        try:
            self._stdin_r.close()
        except Exception:
            pass

    def send(self, obj: dict) -> None:
        self.send_raw(json.dumps(obj))

    def send_raw(self, text: str) -> None:
        self._stdin_w.write(text + "\n")
        self._stdin_w.flush()

    def close_stdin(self) -> None:
        try:
            self._stdin_w.close()
        except Exception:
            pass

    def recv(self, timeout: float = 5.0) -> dict:
        return json.loads(self.out.q.get(timeout=timeout))

    def recv_until(self, type_: str, timeout: float = 5.0) -> dict:
        while True:
            evt = self.recv(timeout=timeout)
            if evt.get("type") == type_:
                return evt

    def expect_hello(self) -> dict:
        evt = self.recv()
        assert evt["type"] == "hello"
        return evt


# --------------------------------------------------------------------------
# Tests
# --------------------------------------------------------------------------


def test_hello_emitted_first():
    with Harness(FakeBackend()) as h:
        evt = h.recv()
        assert evt["type"] == "hello"
        assert evt["protocol"] == PROTOCOL_VERSION == 1
        assert evt["engine_version"] == __version__
        assert evt["default_models"] == DEFAULT_MODELS


def test_hello_reflects_backend_models():
    custom = {"two_stem": "custom2.ckpt", "four_stem": "custom4.ckpt"}
    with Harness(FakeBackend(custom)) as h:
        evt = h.recv()
        assert evt["default_models"] == custom


def test_ping_pong():
    with Harness(FakeBackend()) as h:
        h.expect_hello()
        h.send({"cmd": "ping", "id": "ping-1"})
        evt = h.recv()
        assert evt == {"type": "pong", "id": "ping-1"}


def test_separate_happy_path():
    with Harness(FakeBackend()) as h:
        h.expect_hello()
        h.send(
            {
                "cmd": "separate",
                "id": "j1",
                "input": "/songs/track.mp3",
                "mode": "two_stem",
                "workspace": "/ws",
            }
        )
        prog = h.recv()
        assert prog == {"type": "progress", "id": "j1", "stage": "separating", "fraction": 0.5}
        done = h.recv()
        assert done["type"] == "done"
        assert done["id"] == "j1"
        assert done["result"] == {
            "stems": [
                {"name": "vocals", "path": "/ws/vocals.wav"},
                {"name": "instrumental", "path": "/ws/instrumental.wav"},
            ],
            "source_samples": 44100,
            "sample_rate": 44100,
            "model": "two.ckpt",
        }
        # paths are strings, not objects
        assert all(isinstance(s["path"], str) for s in done["result"]["stems"])


def test_prefetch_done_result():
    with Harness(FakeBackend()) as h:
        h.expect_hello()
        h.send({"cmd": "prefetch", "id": "p1"})
        prog = h.recv()
        assert prog["type"] == "progress"
        assert prog["stage"] == "downloading"
        done = h.recv()
        assert done["type"] == "done"
        assert done["id"] == "p1"
        assert done["result"] == {"models": ["two.ckpt", "four.ckpt"]}


def test_busy_second_job_rejected():
    backend = BlockingBackend()
    with Harness(backend) as h:
        h.expect_hello()
        h.send(
            {"cmd": "separate", "id": "j1", "input": "/a.mp3", "mode": "two_stem", "workspace": "/ws"}
        )
        assert backend.entered.wait(5), "first job never started"
        # second job while first is in flight
        h.send(
            {"cmd": "separate", "id": "j2", "input": "/b.mp3", "mode": "two_stem", "workspace": "/ws"}
        )
        err = h.recv_until("error")
        assert err["id"] == "j2"
        assert err["category"] == "busy"
        # release the first job, it completes normally
        backend.release.set()
        done = h.recv_until("done")
        assert done["id"] == "j1"


def test_cancel_in_flight_job():
    backend = CancelBackend()
    with Harness(backend) as h:
        h.expect_hello()
        h.send(
            {"cmd": "separate", "id": "j1", "input": "/a.mp3", "mode": "two_stem", "workspace": "/ws"}
        )
        assert backend.entered.wait(5), "job never started"
        # request cancel, then a ping. Commands are processed in order on the read
        # thread, so receiving the pong proves the cancel was already registered
        # (and proves ping works while a job runs).
        h.send({"cmd": "cancel", "target_id": "j1"})
        h.send({"cmd": "ping", "id": "probe"})
        pong = h.recv_until("pong")
        assert pong["id"] == "probe"
        # now release the backend to hit its cancelled() check
        backend.may_check.set()
        err = h.recv_until("error")
        assert err["id"] == "j1"
        assert err["category"] == "cancelled"


def test_export_maps_args_and_returns_files():
    exporter = FakeExporter([Path("/out/track (Acapella).aiff")])
    with Harness(FakeBackend(), exporter=exporter) as h:
        h.expect_hello()
        h.send(
            {
                "cmd": "export",
                "id": "e1",
                "stems": [
                    {
                        "name": "vocals",
                        "path": "/ws/vocals.wav",
                        "window": {"start_sample": 100, "end_sample": 500},
                    }
                ],
                "source": "/songs/track.mp3",
                "source_samples": 1000,
                "span": {"start_sample": 0, "end_sample": 1000},
                "format": "aiff",
                "out_dir": "/out",
                "suffixes": {"vocals": "Acapella"},
            }
        )
        done = h.recv_until("done")
        assert done["id"] == "e1"
        assert done["result"] == {"files": ["/out/track (Acapella).aiff"]}

        assert len(exporter.calls) == 1
        call = exporter.calls[0]
        assert call["stems"] == [("vocals", Path("/ws/vocals.wav"), (100, 500), 1.0)]
        assert isinstance(call["stems"][0][1], Path)
        assert isinstance(call["stems"][0][2], tuple)
        assert call["stems"][0][3] == 1.0  # gain absent -> unity
        assert call["source"] == Path("/songs/track.mp3")
        assert isinstance(call["source"], Path)
        assert call["source_samples"] == 1000
        assert call["span"] == (0, 1000)
        assert isinstance(call["span"], tuple)
        assert call["fmt"] == "aiff"
        assert call["out_dir"] == Path("/out")
        assert isinstance(call["out_dir"], Path)
        assert call["suffixes"] == {"vocals": "Acapella"}


def test_export_window_absent_or_null_maps_to_none():
    exporter = FakeExporter([Path("/out/track (Vocals).wav")])
    with Harness(FakeBackend(), exporter=exporter) as h:
        h.expect_hello()
        h.send(
            {
                "cmd": "export",
                "id": "e2",
                "stems": [
                    {"name": "vocals", "path": "/ws/vocals.wav"},  # window absent
                    {"name": "drums", "path": "/ws/drums.wav", "window": None},  # window null
                ],
                "source": "/songs/track.mp3",
                "source_samples": 1000,
                "span": {"start_sample": 0, "end_sample": 1000},
                "format": "wav",
                "out_dir": "/out",
                "suffixes": {"vocals": "Vocals"},
            }
        )
        done = h.recv_until("done")
        assert done["result"] == {"files": ["/out/track (Vocals).wav"]}
        stems = exporter.calls[0]["stems"]
        assert stems == [
            ("vocals", Path("/ws/vocals.wav"), None, 1.0),
            ("drums", Path("/ws/drums.wav"), None, 1.0),
        ]
        assert exporter.calls[0]["span"] == (0, 1000)
        assert exporter.calls[0]["fmt"] == "wav"
        # absent combine_suffix field maps to None
        assert exporter.calls[0]["combine_suffix"] is None


def test_export_missing_span_is_protocol_error():
    exporter = FakeExporter([Path("/out/track (Vocals).wav")])
    with Harness(FakeBackend(), exporter=exporter) as h:
        h.expect_hello()
        h.send(
            {
                "cmd": "export",
                "id": "e_nospan",
                "stems": [{"name": "vocals", "path": "/ws/vocals.wav"}],
                "source": "/songs/track.mp3",
                "source_samples": 1000,
                # span omitted
                "format": "wav",
                "out_dir": "/out",
                "suffixes": {"vocals": "Vocals"},
            }
        )
        err = h.recv_until("error")
        assert err["id"] == "e_nospan"
        assert err["category"] == "protocol"
        assert exporter.calls == []  # never reached the exporter


def test_export_combine_suffix_maps_to_exporter():
    exporter = FakeExporter([Path("/out/track (No Other).aiff")])
    with Harness(FakeBackend(), exporter=exporter) as h:
        h.expect_hello()
        h.send(
            {
                "cmd": "export",
                "id": "e3",
                "stems": [
                    {"name": "vocals", "path": "/ws/vocals.wav"},
                    {"name": "drums", "path": "/ws/drums.wav"},
                ],
                "source": "/songs/track.mp3",
                "source_samples": 1000,
                "span": {"start_sample": 0, "end_sample": 1000},
                "format": "aiff",
                "out_dir": "/out",
                "suffixes": {},
                "combine_suffix": "No Other",
            }
        )
        done = h.recv_until("done")
        assert done["result"] == {"files": ["/out/track (No Other).aiff"]}
        assert exporter.calls[0]["combine_suffix"] == "No Other"


def test_export_gain_maps_and_defaults():
    """Per-stem gain: a present number passes through; absent or null -> 1.0."""
    exporter = FakeExporter([Path("/out/x.wav")])
    with Harness(FakeBackend(), exporter=exporter) as h:
        h.expect_hello()
        h.send(
            {
                "cmd": "export",
                "id": "e_gain",
                "stems": [
                    {"name": "vocals", "path": "/ws/vocals.wav", "gain": 0.5},
                    {"name": "drums", "path": "/ws/drums.wav"},  # gain absent
                    {"name": "bass", "path": "/ws/bass.wav", "gain": None},  # gain null
                    {"name": "other", "path": "/ws/other.wav", "gain": 2},  # int -> float
                ],
                "source": "/songs/track.mp3",
                "source_samples": 1000,
                "span": {"start_sample": 0, "end_sample": 1000},
                "format": "wav",
                "out_dir": "/out",
                "suffixes": {},
            }
        )
        h.recv_until("done")
        stems = exporter.calls[0]["stems"]
        gains = [s[3] for s in stems]
        assert gains == [0.5, 1.0, 1.0, 2.0]
        assert all(isinstance(g, float) for g in gains)


def test_separate_passes_quality_max():
    backend = FakeBackend()
    with Harness(backend) as h:
        h.expect_hello()
        h.send(
            {
                "cmd": "separate",
                "id": "jq",
                "input": "/a.mp3",
                "mode": "two_stem",
                "workspace": "/ws",
                "quality": "max",
            }
        )
        h.recv_until("done")
        assert backend.last_quality == "max"


def test_separate_quality_defaults_to_standard():
    backend = FakeBackend()
    with Harness(backend) as h:
        h.expect_hello()
        h.send(
            {"cmd": "separate", "id": "jd", "input": "/a.mp3", "mode": "two_stem", "workspace": "/ws"}
        )
        h.recv_until("done")
        assert backend.last_quality == "standard"


def test_backend_error_with_category():
    backend = ErrorBackend(CategorizedError("could not decode input", "decode"))
    with Harness(backend) as h:
        h.expect_hello()
        h.send(
            {"cmd": "separate", "id": "j1", "input": "/a.mp3", "mode": "two_stem", "workspace": "/ws"}
        )
        err = h.recv_until("error")
        assert err["id"] == "j1"
        assert err["category"] == "decode"
        assert err["message"] == "could not decode input"


def test_backend_error_without_category_is_internal():
    backend = ErrorBackend(RuntimeError("kaboom"))
    with Harness(backend) as h:
        h.expect_hello()
        h.send(
            {"cmd": "separate", "id": "j1", "input": "/a.mp3", "mode": "two_stem", "workspace": "/ws"}
        )
        err = h.recv_until("error")
        assert err["id"] == "j1"
        assert err["category"] == "internal"
        assert err["message"] == "kaboom"


def test_malformed_json_is_protocol_error_id_null():
    with Harness(FakeBackend()) as h:
        h.expect_hello()
        h.send_raw("this is not json {")
        err = h.recv_until("error")
        assert err["id"] is None
        assert err["category"] == "protocol"


def test_non_object_json_is_protocol_error_id_null():
    with Harness(FakeBackend()) as h:
        h.expect_hello()
        h.send_raw("[1, 2, 3]")
        err = h.recv_until("error")
        assert err["id"] is None
        assert err["category"] == "protocol"


def test_unknown_cmd_is_protocol_error_with_id():
    with Harness(FakeBackend()) as h:
        h.expect_hello()
        h.send({"cmd": "frobnicate", "id": "x9"})
        err = h.recv_until("error")
        assert err["id"] == "x9"
        assert err["category"] == "protocol"


def test_shutdown_returns_from_run():
    with Harness(FakeBackend()) as h:
        h.expect_hello()
        h.send({"cmd": "shutdown"})
        h.thread.join(timeout=5.0)
        assert not h.thread.is_alive()


def test_eof_terminates_run():
    with Harness(FakeBackend()) as h:
        h.expect_hello()
        h.close_stdin()
        h.thread.join(timeout=5.0)
        assert not h.thread.is_alive()


def test_shutdown_waits_for_running_job():
    backend = BlockingBackend()
    with Harness(backend) as h:
        h.expect_hello()
        h.send(
            {"cmd": "separate", "id": "j1", "input": "/a.mp3", "mode": "two_stem", "workspace": "/ws"}
        )
        assert backend.entered.wait(5)
        h.send({"cmd": "shutdown"})
        # run() must not return while the job thread is still working
        h.thread.join(timeout=0.2)
        assert h.thread.is_alive()
        backend.release.set()
        h.thread.join(timeout=5.0)
        assert not h.thread.is_alive()
        # the finished job still emitted its done event
        done = h.recv_until("done")
        assert done["id"] == "j1"
