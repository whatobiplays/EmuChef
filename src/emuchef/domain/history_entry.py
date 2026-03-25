"""History entry models."""

from __future__ import annotations

from dataclasses import dataclass

from .draft_plan import DraftPlan


@dataclass(frozen=True, slots=True)
class HistoryEntry:
    operation: str
    before: DraftPlan
    after: DraftPlan
