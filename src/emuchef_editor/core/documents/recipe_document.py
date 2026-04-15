"""Recipe document abstraction for the editor core."""

from __future__ import annotations

from pathlib import Path

from emuchef.domain import Recipe

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
        self._baseline_yaml = baseline_yaml or emit_recipe_yaml(working_recipe)
        self.ref_index: RefIndex = build_ref_index(self.working_recipe)
        self.validation_result: DiagnosticResult = self.validate()

    @property
    def is_dirty(self) -> bool:
        return self.to_yaml() != self._baseline_yaml

    def set_working_recipe(self, recipe: Recipe) -> None:
        self.working_recipe = recipe
        self.ref_index = build_ref_index(self.working_recipe)
        self.validation_result = self.validate()

    def validate(self) -> DiagnosticResult:
        self.validation_result = self._validator_service.validate_recipe(
            self.working_recipe,
            path=self.path,
            authored_root=self.authored_root,
        )
        return self.validation_result

    def to_yaml(self) -> str:
        return emit_recipe_yaml(self.working_recipe)

    def save(self) -> str:
        payload = write_recipe_yaml(self.working_recipe, self.path)
        self._baseline_yaml = payload
        self.ref_index = build_ref_index(self.working_recipe)
        self.validation_result = self.validate()
        return payload
