"""Draft update result models."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from .constants import SCHEMA_VERSION
from .draft_changes import DraftPlanChanges
from .draft_plan import DraftPlan
from .history_entry import HistoryEntry
from .issues import ErrorMessage, WarningMessage


@dataclass(frozen=True, slots=True)
class DraftUpdateResult:
    draft_plan: DraftPlan
    changes: DraftPlanChanges
    history_entry: HistoryEntry | None = None
    warnings: tuple[WarningMessage, ...] = ()
    errors: tuple[ErrorMessage, ...] = ()
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["draft_update_result"] = "draft_update_result"
