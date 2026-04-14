"""Draft-plan construction and normalization helpers."""

from __future__ import annotations

import logging
from collections import OrderedDict
from collections.abc import Mapping, Sequence

from emuchef.domain import (
    Availability,
    AvailabilityCode,
    AvailabilityReason,
    DeviceContext,
    DraftInputState,
    DraftPlan,
    DraftPlanSource,
    DraftRecipeState,
    DraftStepState,
    ErrorCode,
    ErrorMessage,
    RefKind,
    RuntimeCapabilities,
    parse_reference,
)

from .bindings import build_binding_table
from .catalog import AuthoredCatalog
from .conflicts import StepConflictContext, resolve_step_conflicts
from .contracts import referenced_bindings
from .dependencies import expand_recipe_dependencies
from .ids import make_execution_input_id, make_execution_step_id

logger = logging.getLogger(__name__)


def build_draft_plan(
    catalog: AuthoredCatalog,
    draft_id: str,
    device_plan_ref: str,
    device_profile_ref: str,
    device_context: DeviceContext,
    runtime_capabilities: RuntimeCapabilities,
    user_selected_recipe_refs: tuple[str, ...],
    step_selection_overrides: Mapping[str, bool],
    user_bindings: Mapping[str, object],
    planner_overrides: Mapping[str, object],
) -> tuple[DraftPlan | None, tuple[ErrorMessage, ...]]:
    expanded_recipe_refs, dependency_errors = expand_recipe_dependencies(catalog, user_selected_recipe_refs)
    if dependency_errors:
        return None, dependency_errors
    logger.debug("Building draft plan %s with expanded recipes: %s", draft_id, list(expanded_recipe_refs))

    selected_namespaced_step_ids: set[str] = set()
    step_contexts: dict[str, StepConflictContext] = {}
    step_availability: dict[str, bool] = {}
    step_reasons: dict[str, AvailabilityReason | None] = {}
    auto_included_by_recipe: dict[str, bool] = {}

    for recipe_ref in expanded_recipe_refs:
        recipe = catalog.recipes[recipe_ref]
        available_by_step_id: dict[str, bool] = {}
        reason_by_step_id: dict[str, AvailabilityReason | None] = {}
        requested_local_step_ids: list[str] = []

        for step in recipe.steps:
            namespaced_id = make_execution_step_id(recipe_ref, step.id)
            missing_capabilities = [
                capability
                for capability in step.constraints.capabilities
                if not bool(getattr(runtime_capabilities, capability, False))
            ]
            if missing_capabilities:
                logger.debug(
                    "Step unavailable during capability shaping: %s/%s missing %s",
                    recipe_ref,
                    step.id,
                    missing_capabilities,
                )
                available_by_step_id[step.id] = False
                reason_by_step_id[step.id] = AvailabilityReason(
                    code=AvailabilityCode.REQUIRED_CAPABILITY_MISSING,
                    message=(
                        "This step requires capabilities that are unavailable on this device."
                        if len(missing_capabilities) > 1
                        else f"This step requires {missing_capabilities[0]}, which is unavailable on this device."
                    ),
                    details={
                        "capability": missing_capabilities[0] if len(missing_capabilities) == 1 else missing_capabilities,
                    },
                )
            else:
                available_by_step_id[step.id] = True
                reason_by_step_id[step.id] = None

            if step_selection_overrides.get(namespaced_id, True) and available_by_step_id[step.id]:
                requested_local_step_ids.append(step.id)

        selected_local_step_ids, selection_errors = _select_step_ids(
            recipe,
            tuple(requested_local_step_ids),
            available_by_step_id,
        )
        if selection_errors:
            return None, selection_errors

        auto_included = recipe_ref not in user_selected_recipe_refs
        auto_included_by_recipe[recipe_ref] = auto_included

        for step in recipe.steps:
            namespaced_id = make_execution_step_id(recipe_ref, step.id)
            step_availability[namespaced_id] = available_by_step_id[step.id]
            step_reasons[namespaced_id] = reason_by_step_id[step.id]
            step_contexts[namespaced_id] = StepConflictContext(
                step_id=namespaced_id,
                recipe_ref=recipe_ref,
                step=step,
                auto_included_recipe=auto_included,
                explicitly_selected_step=step_selection_overrides.get(namespaced_id) is True,
            )
            if step.id in selected_local_step_ids:
                selected_namespaced_step_ids.add(namespaced_id)

    resolved_selected_step_ids, conflict_errors = resolve_step_conflicts(step_contexts, selected_namespaced_step_ids)
    if conflict_errors or resolved_selected_step_ids is None:
        return None, conflict_errors
    logger.debug("Draft selected steps after normalization: %s", sorted(resolved_selected_step_ids))
    resolved_selected_step_ids = _prune_optional_input_steps(
        catalog,
        expanded_recipe_refs,
        resolved_selected_step_ids,
        step_availability,
        step_reasons,
        user_bindings,
        planner_overrides,
    )
    logger.debug("Draft selected steps after optional-input pruning: %s", sorted(resolved_selected_step_ids))

    draft_recipes: list[DraftRecipeState] = []
    draft_steps: list[DraftStepState] = []
    selected_input_ids: OrderedDict[str, list[str]] = OrderedDict()

    for recipe_ref in expanded_recipe_refs:
        recipe = catalog.recipes[recipe_ref]
        recipe_step_ids = tuple(make_execution_step_id(recipe_ref, step.id) for step in recipe.steps)
        recipe_available = any(step_availability[step_id] for step_id in recipe_step_ids) or not recipe.steps
        recipe_reason = None if recipe_available else _merge_recipe_reason(
            tuple(reason for step_id in recipe_step_ids if (reason := step_reasons[step_id]) is not None)
        )
        draft_recipes.append(
            DraftRecipeState(
                id=recipe_ref,
                selected=True,
                auto_included=auto_included_by_recipe[recipe_ref],
                user_toggleable=not auto_included_by_recipe[recipe_ref],
                availability=Availability.AVAILABLE if recipe_available else Availability.UNAVAILABLE,
                reason=recipe_reason,
            )
        )

        for step in recipe.steps:
            namespaced_id = make_execution_step_id(recipe_ref, step.id)
            selected = namespaced_id in resolved_selected_step_ids
            draft_steps.append(
                DraftStepState(
                    id=namespaced_id,
                    recipe_ref=recipe_ref,
                    type=step.type,
                    name=step.name,
                    selected=selected,
                    user_toggleable=step.user_toggleable,
                    availability=Availability.AVAILABLE if step_availability[namespaced_id] else Availability.UNAVAILABLE,
                    reason=step_reasons[namespaced_id],
                )
            )
            if not selected:
                continue
            for _, ref in referenced_bindings(step):
                try:
                    parsed = parse_reference(ref)
                except ValueError:
                    continue
                if parsed.kind is not RefKind.INPUT:
                    continue
                input_id = make_execution_input_id(recipe_ref, parsed.target_id)
                selected_input_ids.setdefault(input_id, []).append(namespaced_id)

    binding_table = build_binding_table(
        tuple(selected_input_ids),
        catalog.binding_inputs,
        user_bindings,  # type: ignore[arg-type]
        planner_overrides,  # type: ignore[arg-type]
    )

    draft_inputs = [
        DraftInputState(
            id=input_id,
            label=catalog.binding_inputs[input_id].label,
            description=catalog.binding_inputs[input_id].description,
            required=catalog.binding_inputs[input_id].required,
            multiple=catalog.binding_inputs[input_id].multiple,
            resolved=input_id in binding_table,
            value=binding_table[input_id].value if input_id in binding_table else None,
            required_by=tuple(selected_input_ids[input_id]),
        )
        for input_id in selected_input_ids
    ]
    logger.debug("Draft required inputs: %s", [item.id for item in draft_inputs])

    return (
        DraftPlan(
            id=draft_id,
            source=DraftPlanSource(
                device_profile_ref=device_profile_ref,
                device_plan_ref=device_plan_ref,
                selected_recipe_refs=user_selected_recipe_refs,
            ),
            device_context=device_context,
            runtime_capabilities=runtime_capabilities,
            recipes=tuple(draft_recipes),
            steps=tuple(draft_steps),
            inputs=tuple(draft_inputs),
            warnings=(),
        ),
        (),
    )


def _prune_optional_input_steps(
    catalog: AuthoredCatalog,
    recipe_refs: Sequence[str],
    selected_step_ids: set[str],
    step_availability: dict[str, bool],
    step_reasons: dict[str, AvailabilityReason | None],
    user_bindings: Mapping[str, object],
    planner_overrides: Mapping[str, object],
) -> set[str]:
    selected = set(selected_step_ids)

    while True:
        binding_table = build_binding_table(
            _selected_input_ids(catalog, recipe_refs, selected),
            catalog.binding_inputs,
            user_bindings,  # type: ignore[arg-type]
            planner_overrides,  # type: ignore[arg-type]
        )
        prune_targets = _optional_input_prune_targets(catalog, recipe_refs, selected, binding_table)
        if not prune_targets:
            return selected

        removed_step_ids: set[str] = set()
        for step_id, input_ids in prune_targets.items():
            step_availability[step_id] = False
            step_reasons[step_id] = _optional_input_unbound_reason(input_ids)
            removed_step_ids.add(step_id)

            recipe_ref, local_step_id = step_id.split("/", 1)
            removed_step_ids.update(
                make_execution_step_id(recipe_ref, dependent_step_id)
                for dependent_step_id in _dependent_step_ids(catalog.recipes[recipe_ref], local_step_id)
            )

        selected.difference_update(removed_step_ids)


def _selected_input_ids(
    catalog: AuthoredCatalog,
    recipe_refs: Sequence[str],
    selected_step_ids: set[str],
) -> tuple[str, ...]:
    input_ids: OrderedDict[str, None] = OrderedDict()
    for recipe_ref in recipe_refs:
        recipe = catalog.recipes[recipe_ref]
        for step in recipe.steps:
            namespaced_id = make_execution_step_id(recipe_ref, step.id)
            if namespaced_id not in selected_step_ids:
                continue
            for _, ref in referenced_bindings(step):
                try:
                    parsed = parse_reference(ref)
                except ValueError:
                    continue
                if parsed.kind is not RefKind.INPUT:
                    continue
                input_ids.setdefault(make_execution_input_id(recipe_ref, parsed.target_id), None)
    return tuple(input_ids)


def _optional_input_prune_targets(
    catalog: AuthoredCatalog,
    recipe_refs: Sequence[str],
    selected_step_ids: set[str],
    binding_table,
) -> dict[str, tuple[str, ...]]:
    targets: dict[str, tuple[str, ...]] = {}
    for recipe_ref in recipe_refs:
        recipe = catalog.recipes[recipe_ref]
        for step in recipe.steps:
            namespaced_id = make_execution_step_id(recipe_ref, step.id)
            if namespaced_id not in selected_step_ids:
                continue

            unbound_optional_inputs: OrderedDict[str, None] = OrderedDict()
            for _, ref in referenced_bindings(step):
                try:
                    parsed = parse_reference(ref)
                except ValueError:
                    continue
                if parsed.kind is not RefKind.INPUT:
                    continue
                input_id = make_execution_input_id(recipe_ref, parsed.target_id)
                declaration = catalog.binding_inputs[input_id]
                if declaration.required or input_id in binding_table:
                    continue
                unbound_optional_inputs.setdefault(input_id, None)

            if unbound_optional_inputs:
                targets[namespaced_id] = tuple(unbound_optional_inputs)
    return targets


def _optional_input_unbound_reason(input_ids: Sequence[str]) -> AvailabilityReason:
    unique_ids = tuple(OrderedDict.fromkeys(str(input_id) for input_id in input_ids))
    if len(unique_ids) == 1:
        return AvailabilityReason(
            code=AvailabilityCode.OPTIONAL_INPUT_UNBOUND,
            message=f"This step requires optional input {unique_ids[0]!r}, which is currently unbound.",
            details={"input_id": unique_ids[0]},
        )
    return AvailabilityReason(
        code=AvailabilityCode.OPTIONAL_INPUT_UNBOUND,
        message="This step requires optional inputs that are currently unbound.",
        details={"input_id": list(unique_ids)},
    )


def _select_step_ids(
    recipe,
    requested_ids: Sequence[str],
    available_by_step_id: Mapping[str, bool],
) -> tuple[set[str], tuple[ErrorMessage, ...]]:
    by_id = {step.id: step for step in recipe.steps}
    selected: set[str] = set()
    can_select_cache: dict[str, bool] = {}
    temporary: set[str] = set()

    def can_select(step_id: str) -> bool:
        if step_id in can_select_cache:
            return can_select_cache[step_id]
        if step_id in temporary:
            raise ValueError(step_id)
        temporary.add(step_id)
        step = by_id[step_id]
        allowed = available_by_step_id[step_id] and all(can_select(dep) for dep in step.dependencies)
        temporary.remove(step_id)
        can_select_cache[step_id] = allowed
        return allowed

    def add_with_dependencies(step_id: str) -> None:
        if step_id in selected:
            return
        for dependency in by_id[step_id].dependencies:
            add_with_dependencies(dependency)
        selected.add(step_id)

    try:
        for requested_id in requested_ids:
            if can_select(requested_id):
                add_with_dependencies(requested_id)
    except ValueError as exc:
        return set(), (
            ErrorMessage(
                code=ErrorCode.DEPENDENCY_CYCLE,
                message=f"Step dependency cycle detected in recipe {recipe.id!r}.",
                details={"recipe_ref": recipe.id, "step_id": str(exc)},
            ),
        )

    return selected, ()


def _merge_recipe_reason(reasons: tuple[AvailabilityReason, ...]) -> AvailabilityReason | None:
    if not reasons:
        return None
    if len(reasons) == 1:
        return reasons[0]
    if all(reason.code is AvailabilityCode.OPTIONAL_INPUT_UNBOUND for reason in reasons):
        input_ids: list[str] = []
        for reason in reasons:
            detail = reason.details.get("input_id")
            if isinstance(detail, list):
                input_ids.extend(str(item) for item in detail)
            elif detail is not None:
                input_ids.append(str(detail))
        return AvailabilityReason(
            code=AvailabilityCode.OPTIONAL_INPUT_UNBOUND,
            message="This recipe has no currently runnable steps because optional inputs are unbound.",
            details={"input_id": list(sorted(set(input_ids)))},
        )
    missing: list[str] = []
    for reason in reasons:
        detail = reason.details.get("capability")
        if isinstance(detail, list):
            missing.extend(str(item) for item in detail)
        elif detail is not None:
            missing.append(str(detail))
    unique_missing = tuple(sorted(set(missing)))
    return AvailabilityReason(
        code=AvailabilityCode.REQUIRED_CAPABILITY_MISSING,
        message="This recipe has no currently runnable steps for the available device capabilities.",
        details={"capability": list(unique_missing)},
    )


def _dependent_step_ids(recipe, dependency_id: str) -> set[str]:
    dependents: set[str] = set()
    changed = True
    while changed:
        changed = False
        for step in recipe.steps:
            if step.id == dependency_id or step.id in dependents:
                continue
            if dependency_id in step.dependencies or dependents.intersection(step.dependencies):
                dependents.add(step.id)
                changed = True
    return dependents
