"""Reference index helpers for authored recipe documents."""

from __future__ import annotations

from dataclasses import dataclass

from emuchef.domain import PRIMARY_OUTPUT_STEP_TYPES, Recipe
from emuchef.planner.contracts import RUNTIME_ARTIFACT_FIELDS


@dataclass(frozen=True, slots=True)
class RefIndex:
    input_refs: tuple[str, ...]
    artifact_refs: tuple[str, ...]
    step_refs: tuple[str, ...]
    step_output_refs: tuple[str, ...]

    @property
    def all_refs(self) -> tuple[str, ...]:
        return self.input_refs + self.artifact_refs + self.step_refs + self.step_output_refs


def build_ref_index(recipe: Recipe) -> RefIndex:
    input_refs = tuple(f"inputs.{input_id}" for input_id in sorted(recipe.inputs))
    artifact_refs = tuple(
        f"artifacts.{artifact_id}.{field}"
        for artifact_id in sorted(recipe.artifacts)
        for field in sorted(RUNTIME_ARTIFACT_FIELDS)
    )
    step_refs = tuple(f"steps.{step.id}" for step in recipe.steps)
    step_output_refs = tuple(
        f"steps.{step.id}.outputs.{output_name}"
        for step in recipe.steps
        for output_name in _step_output_names(step.type)
    )
    return RefIndex(
        input_refs=input_refs,
        artifact_refs=artifact_refs,
        step_refs=step_refs,
        step_output_refs=step_output_refs,
    )


def _step_output_names(step_type) -> tuple[str, ...]:
    output_name = PRIMARY_OUTPUT_STEP_TYPES.get(step_type)
    if output_name is None:
        return ()
    return (output_name,)
