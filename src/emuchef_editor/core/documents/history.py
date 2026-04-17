"""Snapshot-based command history for editor documents."""

from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass

from emuchef.domain import Recipe


@dataclass(frozen=True, slots=True)
class HistoryEntry:
    """A single undoable editor mutation."""

    operation: str
    before_recipe: Recipe
    after_recipe: Recipe


class HistoryManager:
    """Tracks document-local undo and redo stacks."""

    def __init__(self) -> None:
        self._undo_stack: list[HistoryEntry] = []
        self._redo_stack: list[HistoryEntry] = []

    def record(self, operation: str, before_recipe: Recipe, after_recipe: Recipe) -> None:
        self._undo_stack.append(
            HistoryEntry(
                operation=operation,
                before_recipe=deepcopy(before_recipe),
                after_recipe=deepcopy(after_recipe),
            )
        )
        self._redo_stack.clear()

    def undo(self) -> Recipe | None:
        if not self._undo_stack:
            return None
        entry = self._undo_stack.pop()
        self._redo_stack.append(entry)
        return deepcopy(entry.before_recipe)

    def redo(self) -> Recipe | None:
        if not self._redo_stack:
            return None
        entry = self._redo_stack.pop()
        self._undo_stack.append(entry)
        return deepcopy(entry.after_recipe)

    @property
    def can_undo(self) -> bool:
        return bool(self._undo_stack)

    @property
    def can_redo(self) -> bool:
        return bool(self._redo_stack)
