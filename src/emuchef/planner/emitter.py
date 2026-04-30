"""Execution plan emission."""

from __future__ import annotations

import logging

from emuchef.domain import (
    ErrorCode,
    ErrorMessage,
    ExecutionArtifact,
    ExecutionInputValue,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    PlanningResult,
    PlanningStatus,
    RuntimeValue,
    RuntimeValueType,
)

from .bindings import build_binding_table, validate_required_bindings
from .catalog import AuthoredCatalog
from .contracts import normalize_step_params_for_execution
from .dependencies import topologically_sort_steps
from .ids import make_execution_artifact_id, make_execution_step_id

logger = logging.getLogger(__name__)


def emit_execution_plan(
    catalog: AuthoredCatalog,
    draft_plan,
    user_bindings,
    planner_overrides,
    step_selection_overrides,
    plan_id: str,
) -> PlanningResult:
    logger.debug("Emitting execution plan %s from draft %s", plan_id, draft_plan.id)

    input_ids = tuple(item.id for item in draft_plan.inputs)
    binding_table = build_binding_table(input_ids, catalog.binding_inputs, user_bindings, planner_overrides)
    binding_errors = list(validate_required_bindings(input_ids, catalog.binding_inputs, binding_table))
    if binding_errors:
        return PlanningResult(
            status=PlanningStatus.ERROR,
            warnings=(),
            errors=tuple(binding_errors),
            execution_plan=None,
        )

    authored_steps: list[tuple[str, object]] = []
    selected_recipe_ids: list[str] = []
    for draft_recipe in draft_plan.recipes:
        if not draft_recipe.selected:
            continue
        selected_recipe_ids.append(draft_recipe.id)
        recipe = catalog.recipes[draft_recipe.id]
        for draft_step in draft_plan.steps:
            if draft_step.recipe_ref != draft_recipe.id or not draft_step.selected:
                continue
            authored_step = next(step for step in recipe.steps if step.id == draft_step.id.split("/", 1)[1])
            authored_steps.append((draft_step.id, authored_step))

    ordered_steps, step_errors = topologically_sort_steps(tuple(authored_steps))
    if step_errors:
        return PlanningResult(
            status=PlanningStatus.ERROR,
            warnings=(),
            errors=step_errors,
            execution_plan=None,
        )

    execution_steps: list[ExecutionStep] = []
    for execution_step_id, authored_step in ordered_steps:
        recipe_ref = execution_step_id.split("/", 1)[0]
        recipe = catalog.recipes[recipe_ref]
        normalized_params = normalize_step_params_for_execution(recipe, authored_step)
        execution_steps.append(
            ExecutionStep(
                id=execution_step_id,
                recipe_ref=recipe_ref,
                type=authored_step.type,
                name=authored_step.name,
                dependencies=tuple(make_execution_step_id(recipe_ref, dependency) for dependency in authored_step.dependencies),
                constraints=type(authored_step.constraints)(
                    capabilities=authored_step.constraints.capabilities,
                    conflicts_with=tuple(
                        make_execution_step_id(recipe_ref, conflict_id)
                        for conflict_id in authored_step.constraints.conflicts_with
                    ),
                ),
                params=normalized_params,
                skip_if=authored_step.skip_if,
                verify=authored_step.verify,
            )
        )

    if not execution_steps:
        return PlanningResult(
            status=PlanningStatus.ERROR,
            warnings=(),
            errors=(
                ErrorMessage(
                    code=ErrorCode.EMPTY_EXECUTION_PLAN,
                    message="Execution plan emission produced no runnable steps.",
                    details={"plan_id": plan_id},
                ),
            ),
            execution_plan=None,
        )

    execution_plan = ExecutionPlan(
        id=plan_id,
        source=ExecutionPlanSource(
            device_profile_ref=draft_plan.source.device_profile_ref,
            device_plan_ref=draft_plan.source.device_plan_ref,
            selected_recipe_refs=draft_plan.source.selected_recipe_refs,
            expanded_recipe_refs=tuple(recipe.id for recipe in draft_plan.recipes),
        ),
        device_context=draft_plan.device_context,
        runtime_capabilities=draft_plan.runtime_capabilities,
        inputs=tuple(
            ExecutionInputValue(
                id=input_id,
                value=_binding_to_runtime_value(catalog.binding_inputs[input_id], binding_table[input_id].value),
            )
            for input_id in input_ids
        ),
        artifacts=tuple(_emit_execution_artifacts(catalog, selected_recipe_ids)),
        steps=tuple(execution_steps),
    )
    return PlanningResult(
        status=PlanningStatus.SUCCESS,
        warnings=(),
        errors=(),
        execution_plan=execution_plan,
    )


def _emit_execution_artifacts(catalog: AuthoredCatalog, selected_recipe_ids: list[str]) -> tuple[ExecutionArtifact, ...]:
    artifacts: list[ExecutionArtifact] = []
    for recipe_id in selected_recipe_ids:
        recipe = catalog.recipes[recipe_id]
        for artifact_id, artifact in recipe.artifacts.items():
            artifacts.append(
                ExecutionArtifact(
                    id=make_execution_artifact_id(recipe_id, artifact_id),
                    type=artifact.type,
                    url=artifact.url,
                    cache=artifact.cache,
                )
            )
    return tuple(artifacts)


def _binding_to_runtime_value(declaration, value) -> RuntimeValue:
    if declaration.multiple:
        return RuntimeValue(type=RuntimeValueType.PATH_LIST, value=value, location="host")
    if declaration.type.value == "file":
        return RuntimeValue(type=RuntimeValueType.FILE_PATH, value=value, location="host")
    if declaration.type.value == "directory":
        return RuntimeValue(type=RuntimeValueType.DIRECTORY_PATH, value=value, location="host")
    return _coerce_runtime_value(value)


def _coerce_runtime_value(value) -> RuntimeValue:
    if value is None:
        return RuntimeValue(type=RuntimeValueType.NULL, value=None)
    if isinstance(value, bool):
        return RuntimeValue(type=RuntimeValueType.BOOLEAN, value=value)
    if isinstance(value, int):
        return RuntimeValue(type=RuntimeValueType.INTEGER, value=value)
    if isinstance(value, str):
        return RuntimeValue(type=RuntimeValueType.STRING, value=value)
    return RuntimeValue(type=RuntimeValueType.OBJECT, value=value)
