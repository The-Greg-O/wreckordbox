from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Protocol

ProgressFn = Callable[[str, float], None]  # (stage, fraction 0..1)
CancelledFn = Callable[[], bool]


@dataclass
class SeparationResult:
    stems: list[tuple[str, Path]]  # [("vocals", Path), ("instrumental", Path)] etc.
    source_samples: int
    sample_rate: int
    model: str


class SeparatorBackend(Protocol):
    def default_models(self) -> dict[str, str]: ...  # {"two_stem": "<ckpt>", "four_stem": "<ckpt>"}

    def separate(
        self,
        input_path: Path,
        mode: str,
        workspace: Path,
        progress: ProgressFn,
        cancelled: CancelledFn,
        quality: str = "standard",
    ) -> SeparationResult: ...

    def prefetch(self, progress: ProgressFn, cancelled: CancelledFn) -> list[str]: ...


class JobCancelled(Exception):
    """Raised by backends when cancelled() returns True at a stage boundary."""
