"""Snapshot-based draft history manager."""

from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass
from typing import Generic, TypeVar

from emuchef.domain import DraftPlan, HistoryEntry

StateT = TypeVar("StateT")


@dataclass(frozen=True, slots=True)
class HistorySnapshot(Generic[StateT]):
    state: StateT
    draft_plan: DraftPlan


class HistoryManager(Generic[StateT]):
    def __init__(self) -> None:
        self._undo_stack: list[tuple[HistoryEntry, HistorySnapshot[StateT], HistorySnapshot[StateT]]] = []
        self._redo_stack: list[tuple[HistoryEntry, HistorySnapshot[StateT], HistorySnapshot[StateT]]] = []

    def record(
        self,
        operation: str,
        before_state: StateT,
        before_draft: DraftPlan,
        after_state: StateT,
        after_draft: DraftPlan,
    ) -> HistoryEntry:
        entry = HistoryEntry(operation=operation, before=before_draft, after=after_draft)
        before_snapshot = HistorySnapshot(state=deepcopy(before_state), draft_plan=deepcopy(before_draft))
        after_snapshot = HistorySnapshot(state=deepcopy(after_state), draft_plan=deepcopy(after_draft))
        self._undo_stack.append((entry, before_snapshot, after_snapshot))
        self._redo_stack.clear()
        return entry

    def undo(self) -> tuple[HistoryEntry, HistorySnapshot[StateT]] | None:
        if not self._undo_stack:
            return None
        item = self._undo_stack.pop()
        self._redo_stack.append(item)
        entry, before_snapshot, _ = item
        return entry, deepcopy(before_snapshot)

    def redo(self) -> tuple[HistoryEntry, HistorySnapshot[StateT]] | None:
        if not self._redo_stack:
            return None
        item = self._redo_stack.pop()
        self._undo_stack.append(item)
        entry, _, after_snapshot = item
        return entry, deepcopy(after_snapshot)

    @property
    def can_undo(self) -> bool:
        return bool(self._undo_stack)

    @property
    def can_redo(self) -> bool:
        return bool(self._redo_stack)
