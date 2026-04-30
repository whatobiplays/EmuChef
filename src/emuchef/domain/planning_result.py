"""Planning result models."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Literal

from .constants import SCHEMA_VERSION
from .execution_plan import ExecutionPlan
from .issues import ErrorMessage, WarningMessage


class PlanningStatus(str, Enum):
    SUCCESS = "success"
    WARNING = "warning"
    ERROR = "error"


@dataclass(frozen=True, slots=True)
class PlanningResult:
    status: PlanningStatus
    warnings: tuple[WarningMessage, ...]
    errors: tuple[ErrorMessage, ...]
    execution_plan: ExecutionPlan | None
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["planning_result"] = "planning_result"

    def __post_init__(self) -> None:
        if self.status in {PlanningStatus.SUCCESS, PlanningStatus.WARNING} and self.execution_plan is None:
            raise ValueError("Successful or warning planning results require an execution plan")
        if self.status is PlanningStatus.ERROR and self.execution_plan is not None:
            raise ValueError("Error planning results must not include an execution plan")
