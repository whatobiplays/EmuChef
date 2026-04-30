"""Reference index helpers for authored recipe documents."""

from __future__ import annotations

from dataclasses import dataclass

from emuchef.domain import InputType, Recipe, RuntimeValueType
from emuchef.planner.contracts import RUNTIME_ARTIFACT_FIELDS
from emuchef.steps import builtin_step_registry


@dataclass(frozen=True, slots=True)
class RefCandidate:
    """A typed explicit authored ref that can be offered in structured pickers."""

    ref: str
    label: str
    value_type: RuntimeValueType
    source_kind: str
    source_id: str


@dataclass(frozen=True, slots=True)
class RefIndex:
    input_refs: tuple[str, ...]
    artifact_refs: tuple[str, ...]
    step_refs: tuple[str, ...]
    step_output_refs: tuple[str, ...]
    candidates: tuple[RefCandidate, ...]

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
    candidates = (
        _input_candidates(recipe)
        + _artifact_candidates(recipe)
        + _step_output_candidates(recipe)
    )
    return RefIndex(
        input_refs=input_refs,
        artifact_refs=artifact_refs,
        step_refs=step_refs,
        step_output_refs=step_output_refs,
        candidates=candidates,
    )


def _step_output_names(step_type) -> tuple[str, ...]:
    plugin = builtin_step_registry().get(step_type)
    if plugin is None:
        return ()
    return tuple(output.name for output in plugin.outputs)


def _input_candidates(recipe: Recipe) -> tuple[RefCandidate, ...]:
    candidates: list[RefCandidate] = []
    for input_id, declaration in recipe.inputs.items():
        candidates.append(
            RefCandidate(
                ref=f"inputs.{input_id}",
                label=f"Input · {input_id}",
                value_type=_input_value_type(declaration.type, declaration.multiple),
                source_kind="input",
                source_id=input_id,
            )
        )
    return tuple(candidates)


def _artifact_candidates(recipe: Recipe) -> tuple[RefCandidate, ...]:
    candidates: list[RefCandidate] = []
    for artifact_id in recipe.artifacts:
        for field in sorted(RUNTIME_ARTIFACT_FIELDS):
            value_type = _ARTIFACT_FIELD_TYPES.get(field)
            if value_type is None:
                continue
            candidates.append(
                RefCandidate(
                    ref=f"artifacts.{artifact_id}.{field}",
                    label=f"Artifact · {artifact_id}.{field}",
                    value_type=value_type,
                    source_kind="artifact",
                    source_id=artifact_id,
                )
            )
    return tuple(candidates)


def _step_output_candidates(recipe: Recipe) -> tuple[RefCandidate, ...]:
    candidates: list[RefCandidate] = []
    for step in recipe.steps:
        for output_name in _step_output_names(step.type):
            value_type = _PRIMARY_OUTPUT_TYPES.get((step.type, output_name))
            if value_type is None:
                continue
            candidates.append(
                RefCandidate(
                    ref=f"steps.{step.id}.outputs.{output_name}",
                    label=f"Step Output · {step.id}.{output_name}",
                    value_type=value_type,
                    source_kind="step_output",
                    source_id=step.id,
                )
            )
    return tuple(candidates)


def _input_value_type(input_type: InputType, multiple: bool) -> RuntimeValueType:
    if multiple:
        return RuntimeValueType.PATH_LIST
    if input_type is InputType.DIRECTORY:
        return RuntimeValueType.DIRECTORY_PATH
    return RuntimeValueType.FILE_PATH


_ARTIFACT_FIELD_TYPES: dict[str, RuntimeValueType] = {
    "status": RuntimeValueType.STRING,
    "local_path": RuntimeValueType.FILE_PATH,
    "resolved_url": RuntimeValueType.STRING,
    "filename": RuntimeValueType.STRING,
    "cache_hit": RuntimeValueType.BOOLEAN,
    "error": RuntimeValueType.STRING,
}

_PRIMARY_OUTPUT_TYPES: dict[tuple[object, str], RuntimeValueType] = {
    (plugin.type, output.name): output.value_type
    for plugin in builtin_step_registry().plugins
    for output in plugin.outputs
}
