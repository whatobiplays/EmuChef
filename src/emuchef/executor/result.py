"""Executor result models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from enum import Enum

from emuchef.domain import RuntimeValue


class StepRunStatus(str, Enum):
    EXECUTED = "executed"
    SKIPPED = "skipped"
    BLOCKED = "blocked"
    FAILED = "failed"


class ProgressPhase(str, Enum):
    CHECKING_SKIP_CONDITIONS = "checking_skip_conditions"
    EXECUTING = "executing"
    VERIFYING = "verifying"
    FINISHED = "finished"


class ProgressStatus(str, Enum):
    SKIPPED = "skipped"
    BLOCKED = "blocked"
    SUCCEEDED = "succeeded"
    FAILED = "failed"


@dataclass(frozen=True, slots=True)
class StepRunRecord:
    step_id: str
    status: StepRunStatus
    message: str | None = None
    outputs: Mapping[str, RuntimeValue] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class ExecutionProgressEvent:
    step_index: int
    total_steps: int
    step_id: str
    step_name: str
    phase: ProgressPhase
    status: ProgressStatus | None = None
    message: str | None = None


@dataclass(frozen=True, slots=True)
class ExecutionRunResult:
    success: bool
    total_steps: int
    steps: tuple[StepRunRecord, ...]
