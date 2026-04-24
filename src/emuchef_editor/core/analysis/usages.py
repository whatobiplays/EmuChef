"""In-file usage analysis for authored recipe ids and references.

The editor intentionally analyzes only structured authored fields it can update
through the typed model. Preserved unsupported step content is reported as a
warning condition and left unchanged by rename and delete tooling.
"""

from __future__ import annotations

from collections import OrderedDict
from collections.abc import Iterable, Mapping
from dataclasses import dataclass, fields
from typing import Literal

from emuchef.domain import (
    RefKind,
    RefParamValue,
    Recipe,
    RuntimeCapabilities,
    STEP_SPECS,
    Step,
    parse_reference,
)

UsageTargetKind = Literal["recipe", "input", "artifact", "artifact_group", "step"]

SUPPORTED_CONDITION_TYPES: tuple[str, ...] = (
    "path_exists",
    "file_exists",
    "package_installed",
)
KNOWN_CAPABILITIES: tuple[str, ...] = tuple(field.name for field in fields(RuntimeCapabilities))


@dataclass(frozen=True, slots=True)
class UsageTarget:
    kind: UsageTargetKind
    id: str


@dataclass(frozen=True, slots=True)
class Usage:
    group: str
    summary: str
    location: str
    step_id: str | None = None
    field: str | None = None
    value: str | None = None


@dataclass(frozen=True, slots=True)
class UsageGroup:
    title: str
    usages: tuple[Usage, ...]


@dataclass(frozen=True, slots=True)
class UsageAnalysis:
    target: UsageTarget
    groups: tuple[UsageGroup, ...]
    has_preserved_unsupported_content_warning: bool = False

    @property
    def usages(self) -> tuple[Usage, ...]:
        return tuple(usage for group in self.groups for usage in group.usages)


def analyze_recipe_usages(recipe: Recipe, target: UsageTarget) -> UsageAnalysis:
    """Return supported in-file usages of an authored id target."""

    grouped: "OrderedDict[str, list[Usage]]" = OrderedDict()

    def add(group: str, summary: str, location: str, *, step_id: str | None = None, field: str | None = None, value: str | None = None) -> None:
        grouped.setdefault(group, []).append(
            Usage(
                group=group,
                summary=summary,
                location=location,
                step_id=step_id,
                field=field,
                value=value,
            )
        )

    if target.kind == "recipe":
        for index, dependency_ref in enumerate(recipe.recipe_dependencies):
            if dependency_ref == target.id:
                add(
                    "Recipe Dependencies",
                    f"recipe_dependencies[{index}] references {target.id}",
                    f"recipe_dependencies[{index}]",
                    field="recipe_dependencies",
                    value=dependency_ref,
                )

    for group_id, members in recipe.artifact_groups.items():
        if target.kind == "artifact":
            for index, artifact_id in enumerate(members):
                if artifact_id == target.id:
                    add(
                        "Artifact-Group Membership",
                        f"artifact group {group_id} member {index} references {target.id}",
                        f"artifact_groups.{group_id}[{index}]",
                        field="artifact_groups",
                        value=artifact_id,
                    )

    for step_index, step in enumerate(recipe.steps):
        _collect_param_ref_usages(recipe, target, step, step_index, add)
        _collect_step_relationship_usages(target, step, step_index, add)
        _collect_artifact_selection_usages(target, step, step_index, add)

    return UsageAnalysis(
        target=target,
        groups=tuple(UsageGroup(title, tuple(usages)) for title, usages in grouped.items() if usages),
        has_preserved_unsupported_content_warning=_has_preserved_unsupported_content(recipe),
    )


def _collect_param_ref_usages(recipe: Recipe, target: UsageTarget, step: Step, step_index: int, add) -> None:
    for param_name, value in step.params.items():
        if not _is_supported_param(step, param_name) or not isinstance(value, RefParamValue):
            continue
        rewritten = _rewrite_ref_for_target(value.ref, target, target.id)
        if rewritten is None:
            continue
        add(
            "Param Refs",
            f"step {step.id} param {param_name} references {value.ref}",
            f"steps[{step_index}].params.{param_name}.ref",
            step_id=step.id,
            field=f"params.{param_name}",
            value=value.ref,
        )


def _collect_step_relationship_usages(target: UsageTarget, step: Step, step_index: int, add) -> None:
    if target.kind != "step":
        return
    for index, dependency in enumerate(step.dependencies):
        if dependency == target.id:
            add(
                "Dependencies",
                f"step {step.id} depends on {target.id}",
                f"steps[{step_index}].dependencies[{index}]",
                step_id=step.id,
                field="dependencies",
                value=dependency,
            )
    for index, conflict in enumerate(step.constraints.conflicts_with):
        if conflict == target.id:
            add(
                "Constraints / Conflicts",
                f"step {step.id} conflicts with {target.id}",
                f"steps[{step_index}].constraints.conflicts_with[{index}]",
                step_id=step.id,
                field="constraints.conflicts_with",
                value=conflict,
            )


def _collect_artifact_selection_usages(target: UsageTarget, step: Step, step_index: int, add) -> None:
    if target.kind not in {"artifact", "artifact_group"}:
        return
    field = "artifacts" if target.kind == "artifact" else "artifact_groups"
    group = "Step Artifact Selections" if target.kind == "artifact" else "Step Artifact-Group Selections"
    if not _is_supported_param(step, field):
        return
    for index, value in enumerate(_coerce_string_values(step.params.get(field))):
        if value == target.id:
            add(
                group,
                f"step {step.id} {field}[{index}] references {target.id}",
                f"steps[{step_index}].params.{field}[{index}]",
                step_id=step.id,
                field=f"params.{field}",
                value=value,
            )


def _has_preserved_unsupported_content(recipe: Recipe) -> bool:
    step_ids = {step.id for step in recipe.steps}
    for step in recipe.steps:
        spec = STEP_SPECS.get(step.type)
        expected_params = set(spec.params) if spec is not None else set()
        if any(param_name not in expected_params for param_name in step.params):
            return True
        if any(capability not in KNOWN_CAPABILITIES for capability in step.constraints.capabilities):
            return True
        if any(conflict not in step_ids for conflict in step.constraints.conflicts_with):
            return True
        if any(condition.type not in SUPPORTED_CONDITION_TYPES for condition in step.skip_if):
            return True
        if any(condition.type not in SUPPORTED_CONDITION_TYPES for condition in step.verify):
            return True
    return False


def _is_supported_param(step: Step, param_name: str) -> bool:
    spec = STEP_SPECS.get(step.type)
    return spec is not None and param_name in spec.params


def _coerce_string_values(value: object | None) -> tuple[str, ...]:
    if value is None:
        return ()
    if hasattr(value, "value"):
        value = getattr(value, "value")
    if isinstance(value, str):
        return (value,)
    if isinstance(value, Iterable) and not isinstance(value, Mapping):
        return tuple(str(item) for item in value)
    return ()


def _rewrite_ref_for_target(ref: str, target: UsageTarget, new_id: str) -> str | None:
    try:
        parsed = parse_reference(ref)
    except ValueError:
        return None
    if target.kind == "input" and parsed.kind is RefKind.INPUT and parsed.target_id == target.id:
        return f"inputs.{new_id}"
    if target.kind == "artifact" and parsed.kind is RefKind.ARTIFACT_FIELD and parsed.target_id == target.id:
        return f"artifacts.{new_id}.{parsed.field}"
    if target.kind == "step" and parsed.target_id == target.id:
        if parsed.kind is RefKind.STEP_SHORTHAND:
            return f"steps.{new_id}"
        if parsed.kind is RefKind.STEP_OUTPUT:
            return f"steps.{new_id}.outputs.{parsed.field}"
    return None
