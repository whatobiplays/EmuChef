"""Recipe definition models."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from typing import Literal

from ._validation import ensure_known, ensure_unique
from .constants import SCHEMA_VERSION
from .input_declaration import InputDeclaration
from .step import Step


@dataclass(frozen=True, slots=True)
class RecipeProvides:
    features: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Recipe:
    id: str
    name: str
    recipe_dependencies: tuple[str, ...]
    provides: RecipeProvides
    inputs: tuple[InputDeclaration, ...]
    steps: tuple[Step, ...]
    description: str | None = None
    schema_version: Literal[SCHEMA_VERSION] = SCHEMA_VERSION
    kind: Literal["recipe"] = "recipe"

    def __post_init__(self) -> None:
        ensure_unique(self.recipe_dependencies, "recipe dependency refs")
        ensure_unique((item.id for item in self.inputs), "recipe input ids")
        ensure_unique((step.id for step in self.steps), "recipe step ids")
        step_ids = {step.id for step in self.steps}
        for step in self.steps:
            ensure_known(step.dependencies, step_ids, "step dependencies")
