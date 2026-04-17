"""Recipe document abstraction for the editor core."""

from __future__ import annotations

from pathlib import Path

from emuchef.domain import Recipe

from .commands import RecipeCommand, apply_recipe_command
from .history import HistoryManager
from ..refs.ref_index import RefIndex, build_ref_index
from ..validation.validator_service import DiagnosticResult, ValidatorService
from ..yaml.writer import emit_recipe_yaml, write_recipe_yaml


class RecipeDocument:
    """Tracks a typed authored recipe plus its semantic editor state."""

    def __init__(
        self,
        *,
        path: str | Path,
        authored_root: str | Path | None,
        working_recipe: Recipe,
        validator_service: ValidatorService | None = None,
        baseline_yaml: str | None = None,
    ) -> None:
        self.path = Path(path).resolve()
        self.authored_root = Path(authored_root).resolve() if authored_root is not None else None
        self.working_recipe = working_recipe
        self._validator_service = validator_service or ValidatorService()
        self._history = HistoryManager()
        self._canonical_yaml = emit_recipe_yaml(self.working_recipe)
        self._baseline_yaml = baseline_yaml or self._canonical_yaml
        self.ref_index: RefIndex = build_ref_index(self.working_recipe)
        self.validation_result: DiagnosticResult = self.validate()

    @property
    def is_dirty(self) -> bool:
        return self._canonical_yaml != self._baseline_yaml

    @property
    def can_undo(self) -> bool:
        return self._history.can_undo

    @property
    def can_redo(self) -> bool:
        return self._history.can_redo

    def set_working_recipe(self, recipe: Recipe) -> None:
        self.working_recipe = recipe
        self._refresh_derived_state()

    def apply_command(self, command: RecipeCommand) -> bool:
        updated_recipe, operation = apply_recipe_command(self.working_recipe, command)
        updated_yaml = emit_recipe_yaml(updated_recipe)
        if updated_yaml == self._canonical_yaml:
            return False
        before_recipe = self.working_recipe
        self.working_recipe = updated_recipe
        self._history.record(operation, before_recipe, updated_recipe)
        self._refresh_derived_state(canonical_yaml=updated_yaml)
        return True

    def undo(self) -> bool:
        snapshot = self._history.undo()
        if snapshot is None:
            return False
        self.working_recipe = snapshot
        self._refresh_derived_state()
        return True

    def redo(self) -> bool:
        snapshot = self._history.redo()
        if snapshot is None:
            return False
        self.working_recipe = snapshot
        self._refresh_derived_state()
        return True

    def validate(self) -> DiagnosticResult:
        self.validation_result = self._validator_service.validate_recipe(
            self.working_recipe,
            path=self.path,
            authored_root=self.authored_root,
        )
        return self.validation_result

    def to_yaml(self) -> str:
        return self._canonical_yaml

    def save(self) -> str:
        payload = write_recipe_yaml(self.working_recipe, self.path)
        self._baseline_yaml = payload
        self._refresh_derived_state()
        return payload

    def _refresh_derived_state(self, *, canonical_yaml: str | None = None) -> None:
        self._canonical_yaml = canonical_yaml or emit_recipe_yaml(self.working_recipe)
        self.ref_index = build_ref_index(self.working_recipe)
        self.validation_result = self.validate()
