from __future__ import annotations

import json
import sys
import threading
from pathlib import Path
from typing import Any, Callable, Optional, TextIO

from . import PROTOCOL_VERSION, __version__
from .backend import CancelledFn, JobCancelled, ProgressFn, SeparationResult, SeparatorBackend

_JOB_COMMANDS = frozenset({"separate", "export", "prefetch"})


class _CommandError(Exception):
    """A job command that fails field-level validation on the job thread. Carries
    a ``protocol`` category so ``_run_job`` surfaces it as an ``error`` event."""

    category = "protocol"


def _window(field: Optional[dict[str, Any]]) -> Optional[tuple[int, int]]:
    """Map an optional wire ``window`` object to a ``(start, end)`` tuple; an
    absent or null window (full track) maps to ``None``."""
    if field is None:
        return None
    return (field["start_sample"], field["end_sample"])


def _gain(field: Any) -> float:
    """Map an optional wire per-stem ``gain`` to a linear float; an absent or
    null gain maps to 1.0 (unity)."""
    if field is None:
        return 1.0
    return float(field)


class Worker:
    def __init__(
        self,
        backend: SeparatorBackend,
        stdin: Optional[TextIO] = None,
        stdout: Optional[TextIO] = None,
        exporter: Optional[Callable[..., list[Path]]] = None,
    ) -> None:
        self._backend = backend
        self._stdin: TextIO = stdin if stdin is not None else sys.stdin
        self._stdout: TextIO = stdout if stdout is not None else sys.stdout
        self._exporter = exporter
        self._write_lock = threading.Lock()
        self._state_lock = threading.Lock()
        self._job_active = False
        self._job_thread: Optional[threading.Thread] = None
        self._cancelled: set[str] = set()

    # -- lifecycle ---------------------------------------------------------

    def run(self) -> None:
        self._emit(
            {
                "type": "hello",
                "protocol": PROTOCOL_VERSION,
                "engine_version": __version__,
                "default_models": self._backend.default_models(),
            }
        )
        while True:
            line = self._stdin.readline()
            if not line:  # "" == EOF ("\n" is a blank line and is truthy)
                break
            stripped = line.strip()
            if not stripped:
                continue
            if self._handle_line(stripped):  # True => shutdown
                break
        self._join_job()

    # -- command dispatch (read thread) ------------------------------------

    def _handle_line(self, line: str) -> bool:
        try:
            msg = json.loads(line)
        except ValueError:
            self._emit_protocol_error(None, "malformed JSON")
            return False
        if not isinstance(msg, dict):
            self._emit_protocol_error(None, "command must be a JSON object")
            return False

        cmd = msg.get("cmd")
        msg_id = msg.get("id")

        if cmd == "ping":
            self._emit({"type": "pong", "id": msg_id})
            return False
        if cmd == "cancel":
            target = msg.get("target_id")
            if target is not None:
                with self._state_lock:
                    self._cancelled.add(target)
            return False
        if cmd == "shutdown":
            return True
        if cmd in _JOB_COMMANDS:
            self._submit_job(cmd, msg, msg_id)
            return False

        self._emit_protocol_error(msg_id, f"unknown cmd: {cmd!r}")
        return False

    def _submit_job(self, cmd: str, msg: dict[str, Any], msg_id: Any) -> None:
        with self._state_lock:
            busy = self._job_active
            if not busy:
                self._job_active = True
        if busy:
            self._emit(
                {
                    "type": "error",
                    "id": msg_id,
                    "category": "busy",
                    "message": "another job is already running",
                }
            )
            return
        prev = self._job_thread
        if prev is not None:
            prev.join()
        self._job_thread = threading.Thread(
            target=self._run_job, args=(cmd, msg, msg_id), daemon=True
        )
        self._job_thread.start()

    def _join_job(self) -> None:
        thread = self._job_thread
        if thread is not None:
            thread.join()

    # -- job execution (job thread) ----------------------------------------

    def _run_job(self, cmd: str, msg: dict[str, Any], job_id: Any) -> None:
        progress = self._make_progress(job_id)
        cancelled = self._make_cancelled(job_id)
        try:
            if cmd == "separate":
                result = self._do_separate(msg, progress, cancelled)
            elif cmd == "export":
                result = self._do_export(msg, progress)
            else:  # prefetch
                result = self._do_prefetch(progress, cancelled)
            event = {"type": "done", "id": job_id, "result": result}
        except JobCancelled as exc:
            event = {
                "type": "error",
                "id": job_id,
                "category": "cancelled",
                "message": str(exc) or "cancelled",
            }
        except Exception as exc:  # noqa: BLE001 — mapped to an error event by contract
            event = {
                "type": "error",
                "id": job_id,
                "category": getattr(exc, "category", "internal"),
                "message": str(exc),
            }
        with self._state_lock:
            self._job_active = False
        self._emit(event)

    def _do_separate(
        self, msg: dict[str, Any], progress: ProgressFn, cancelled: CancelledFn
    ) -> dict[str, Any]:
        result: SeparationResult = self._backend.separate(
            Path(msg["input"]),
            msg["mode"],
            Path(msg["workspace"]),
            progress,
            cancelled,
            msg.get("quality") or "standard",
        )
        return {
            "stems": [{"name": name, "path": str(path)} for name, path in result.stems],
            "source_samples": result.source_samples,
            "sample_rate": result.sample_rate,
            "model": result.model,
        }

    def _do_export(self, msg: dict[str, Any], progress: ProgressFn) -> dict[str, Any]:
        exporter = self._exporter
        if exporter is None:
            from .exporter import export_stems

            exporter = export_stems

        stems = [
            (s["name"], Path(s["path"]), _window(s.get("window")), _gain(s.get("gain")))
            for s in msg["stems"]
        ]
        span_field = msg.get("span")
        if span_field is None:
            raise _CommandError("export command missing required 'span'")
        span = (span_field["start_sample"], span_field["end_sample"])

        files = exporter(
            stems,
            Path(msg["source"]),
            msg["source_samples"],
            span,
            msg["format"],
            Path(msg["out_dir"]),
            msg["suffixes"],
            progress,
            combine_suffix=msg.get("combine_suffix"),
        )
        return {"files": [str(p) for p in files]}

    def _do_prefetch(self, progress: ProgressFn, cancelled: CancelledFn) -> dict[str, Any]:
        models = self._backend.prefetch(progress, cancelled)
        return {"models": list(models)}

    # -- callbacks handed to the backend/exporter --------------------------

    def _make_progress(self, job_id: Any) -> ProgressFn:
        def progress(stage: str, fraction: float) -> None:
            self._emit(
                {"type": "progress", "id": job_id, "stage": stage, "fraction": fraction}
            )

        return progress

    def _make_cancelled(self, job_id: Any) -> CancelledFn:
        def cancelled() -> bool:
            with self._state_lock:
                return job_id in self._cancelled

        return cancelled

    # -- output ------------------------------------------------------------

    def _emit_protocol_error(self, msg_id: Any, message: str) -> None:
        self._emit(
            {"type": "error", "id": msg_id, "category": "protocol", "message": message}
        )

    def _emit(self, obj: dict[str, Any]) -> None:
        data = json.dumps(obj)
        with self._write_lock:
            self._stdout.write(data + "\n")
            self._stdout.flush()
