"""Execution plan emission."""

from __future__ import annotations

import logging
from collections.abc import Mapping
from pathlib import Path

from emuchef.domain import (
    AppOpGrant,
    BoundParamValue,
    CopyPolicy,
    ErrorCode,
    ErrorMessage,
    ExecutionPermissionPlan,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    LiteralParamValue,
    ManualPermissionRequirement,
    PermissionPlanAction,
    PermissionPlanReason,
    PermissionPlanSource,
    PlanningResult,
    PlanningStatus,
    ResolvedInputValue,
    RuntimePermissionGrant,
    StepType,
    WarningCode,
    WarningMessage,
)

from .bindings import BindingEntry, build_binding_table, validate_required_bindings
from .catalog import AuthoredCatalog
from .conflicts import StepConflictContext, resolve_step_conflicts
from .contracts import STEP_CONTRACTS
from .dependencies import topologically_sort_steps
from .draft_builder import make_execution_step_id

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
        logger.debug("Binding validation failed for plan %s: %s", plan_id, [error.code.value for error in binding_errors])
        return PlanningResult(
            status=PlanningStatus.ERROR,
            warnings=(),
            errors=tuple(binding_errors),
            execution_plan=None,
        )

    warnings: list[WarningMessage] = []
    late_pruned_step_ids: list[str] = []
    missing_capabilities: set[str] = set()
    candidate_step_ids = {step.id for step in draft_plan.steps if step.selected}
    logger.debug("Initial candidate execution steps: %s", sorted(candidate_step_ids))

    auto_included_by_recipe = {recipe.id: recipe.auto_included for recipe in draft_plan.recipes}
    conflict_contexts: dict[str, StepConflictContext] = {}
    for draft_step in draft_plan.steps:
        recipe = catalog.recipes[draft_step.recipe_ref]
        authored_step = next(step for step in recipe.steps if step.id == draft_step.id.split("/", 1)[1])
        conflict_contexts[draft_step.id] = StepConflictContext(
            step_id=draft_step.id,
            recipe_ref=draft_step.recipe_ref,
            step=authored_step,
            auto_included_recipe=auto_included_by_recipe[draft_step.recipe_ref],
            explicitly_selected_step=step_selection_overrides.get(draft_step.id) is True,
        )

    candidate_step_ids, conflict_errors = resolve_step_conflicts(conflict_contexts, candidate_step_ids)
    if conflict_errors or candidate_step_ids is None:
        logger.debug("Conflict resolution failed for plan %s", plan_id)
        return PlanningResult(
            status=PlanningStatus.ERROR,
            warnings=(),
            errors=conflict_errors,
            execution_plan=None,
        )

    changed = True
    while changed:
        changed = False
        for draft_step in draft_plan.steps:
            if draft_step.id not in candidate_step_ids:
                continue
            recipe = catalog.recipes[draft_step.recipe_ref]
            authored_step = next(step for step in recipe.steps if step.id == draft_step.id.split("/", 1)[1])
            missing = [
                capability
                for capability in authored_step.constraints.capabilities
                if not bool(getattr(draft_plan.runtime_capabilities, capability, False))
            ]
            dependency_ids = {
                make_execution_step_id(draft_step.recipe_ref, dependency) for dependency in authored_step.dependencies
            }
            dependencies_missing = any(dependency_id not in candidate_step_ids for dependency_id in dependency_ids)
            if not missing and not dependencies_missing:
                continue
            if not authored_step.user_toggleable:
                if missing:
                    detail = {"step_id": draft_step.id, "capabilities": missing}
                    message = f"Required step {draft_step.id!r} is incompatible with current runtime capabilities."
                else:
                    detail = {"step_id": draft_step.id, "dependencies": sorted(dependency_ids - candidate_step_ids)}
                    message = f"Required step {draft_step.id!r} no longer has its selected dependencies."
                return PlanningResult(
                    status=PlanningStatus.ERROR,
                    warnings=(),
                    errors=(
                        ErrorMessage(
                            code=ErrorCode.CAPABILITY_REDUCTION_FAILED,
                            message=message,
                            details=detail,
                        ),
                    ),
                    execution_plan=None,
                )
            candidate_step_ids.remove(draft_step.id)
            late_pruned_step_ids.append(draft_step.id)
            missing_capabilities.update(missing)
            logger.debug("Late-pruned step %s (missing=%s, dependency_gap=%s)", draft_step.id, missing, dependencies_missing)
            changed = True

    authored_steps: list[tuple[str, object]] = []
    for draft_step in draft_plan.steps:
        if draft_step.id not in candidate_step_ids:
            continue
        recipe = catalog.recipes[draft_step.recipe_ref]
        authored_step = next(step for step in recipe.steps if step.id == draft_step.id.split("/", 1)[1])
        authored_steps.append((draft_step.id, authored_step))

    ordered_steps, step_errors = topologically_sort_steps(tuple(authored_steps))
    if step_errors:
        logger.debug("Topological sort failed for plan %s", plan_id)
        return PlanningResult(
            status=PlanningStatus.ERROR,
            warnings=(),
            errors=step_errors,
            execution_plan=None,
        )

    execution_steps: list[ExecutionStep] = []
    resolution_errors: list[ErrorMessage] = []
    for execution_step_id, authored_step in ordered_steps:
        resolved_params = _resolve_step_params(
            execution_step_id=execution_step_id,
            step=authored_step,
            binding_table=binding_table,
            catalog=catalog,
        )
        if isinstance(resolved_params, ErrorMessage):
            resolution_errors.append(resolved_params)
            continue
        logger.debug("Resolved params for %s: %s", execution_step_id, resolved_params)
        execution_steps.append(
            ExecutionStep(
                id=execution_step_id,
                recipe_ref=execution_step_id.split("/", 1)[0],
                type=authored_step.type,
                name=authored_step.name,
                params=resolved_params,
                skip_if=authored_step.skip_if,
                verify=authored_step.verify,
            )
        )

    if resolution_errors:
        logger.debug("Param resolution failed for plan %s", plan_id)
        return PlanningResult(
            status=PlanningStatus.ERROR,
            warnings=(),
            errors=tuple(resolution_errors),
            execution_plan=None,
        )

    if not execution_steps:
        return PlanningResult(
            status=PlanningStatus.ERROR,
            warnings=tuple(warnings),
            errors=(
                ErrorMessage(
                    code=ErrorCode.EMPTY_EXECUTION_PLAN,
                    message="Execution plan emission produced no runnable steps.",
                    details={"plan_id": plan_id},
                ),
            ),
            execution_plan=None,
        )

    if late_pruned_step_ids:
        warnings.append(
            WarningMessage(
                code=WarningCode.OPTIONAL_STEPS_OMITTED_FOR_CAPABILITIES,
                message="Some optional steps were omitted because this device does not support them.",
                details={
                    "step_ids": late_pruned_step_ids,
                    "missing_capabilities": sorted(missing_capabilities),
                },
            )
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
        inputs_resolved=tuple(
            ResolvedInputValue(id=input_id, value=binding_table[input_id].value) for input_id in input_ids
        ),
        steps=tuple(execution_steps),
        permission_plan=_emit_permission_plan(catalog, draft_plan),
    )
    warnings.extend(_emit_orphaned_permission_warnings(execution_plan))
    status = PlanningStatus.WARNING if warnings else PlanningStatus.SUCCESS
    logger.debug("Execution plan %s emitted with %d steps", plan_id, len(execution_steps))
    return PlanningResult(
        status=status,
        warnings=tuple(warnings),
        errors=(),
        execution_plan=execution_plan,
    )


def _resolve_step_params(
    execution_step_id: str,
    step,
    binding_table: Mapping[str, BindingEntry],
    catalog: AuthoredCatalog,
):
    contract = STEP_CONTRACTS[step.type]
    resolved: dict[str, object] = {}

    for param_name, param_contract in contract.params.items():
        if param_name not in step.params:
            continue
        value = step.params[param_name]
        if param_contract.mode.value == "literal_only":
            if isinstance(value, (LiteralParamValue, BoundParamValue)):
                return ErrorMessage(
                    code=ErrorCode.PARAM_CONTRACT_VIOLATION,
                    message=f"Literal-only param {param_name!r} was provided using a wrapper object.",
                    details={"step_id": execution_step_id, "param": param_name},
                )
            resolved[param_name] = _normalize_execution_param_value(
                step_type=step.type,
                param_name=param_name,
                value=value,
                catalog=catalog,
            )
            continue
        if isinstance(value, LiteralParamValue):
            resolved[param_name] = _normalize_execution_param_value(
                step_type=step.type,
                param_name=param_name,
                value=value.value,
                catalog=catalog,
            )
            continue
        if isinstance(value, BoundParamValue):
            binding = binding_table.get(value.ref.full)
            if binding is None:
                return ErrorMessage(
                    code=ErrorCode.BINDING_MISSING,
                    message=f"Binding {value.ref.full!r} is required by step {execution_step_id!r}.",
                    details={"step_id": execution_step_id, "binding_ref": value.ref.full},
                )
            resolved[param_name] = _normalize_execution_param_value(
                step_type=step.type,
                param_name=param_name,
                value=binding.value,
                catalog=catalog,
            )
            continue
        return ErrorMessage(
            code=ErrorCode.PARAM_CONTRACT_VIOLATION,
            message=f"Param {param_name!r} does not satisfy its contract.",
            details={"step_id": execution_step_id, "param": param_name},
        )

    return resolved


def _normalize_execution_param_value(step_type, param_name: str, value, catalog: AuthoredCatalog):
    if step_type.value == "install_apk" and param_name == "app" and isinstance(value, str):
        path = Path(value).expanduser()
        if not path.is_absolute():
            resolved = str(catalog.asset_root / path)
            logger.debug("Resolved authored asset path %s -> %s", value, resolved)
            return resolved
        return str(path)
    if step_type.value in {"copy_byo_input", "push_file", "push_dir"} and param_name in {"input", "source"} and isinstance(value, str):
        path = Path(value).expanduser()
        if path.is_absolute():
            return str(path)
        return str(Path.cwd() / path)
    if param_name == "copy_policy":
        return CopyPolicy(str(value)).value
    return value


def _emit_permission_plan(catalog: AuthoredCatalog, draft_plan) -> ExecutionPermissionPlan | None:
    actions: list[PermissionPlanAction] = []
    rooted = draft_plan.runtime_capabilities.root_shell
    android_api_level = draft_plan.device_context.android_api_level

    for draft_recipe in draft_plan.recipes:
        recipe = catalog.recipes[draft_recipe.id]
        for index, grant in enumerate(recipe.permissions.runtime):
            actions.append(
                _emit_runtime_permission_action(
                    recipe.id,
                    index,
                    grant,
                    rooted=rooted,
                    android_api_level=android_api_level,
                )
            )
        for index, grant in enumerate(recipe.permissions.appops):
            actions.append(
                _emit_appop_action(
                    recipe.id,
                    index,
                    grant,
                    rooted=rooted,
                    android_api_level=android_api_level,
                )
            )
        for index, grant in enumerate(recipe.permissions.manual):
            actions.append(
                _emit_manual_permission_action(
                    recipe.id,
                    index,
                    grant,
                    rooted=rooted,
                    android_api_level=android_api_level,
                )
            )

    if not actions:
        return None
    return ExecutionPermissionPlan(actions=tuple(actions))


def _emit_orphaned_permission_warnings(execution_plan: ExecutionPlan) -> tuple[WarningMessage, ...]:
    if execution_plan.permission_plan is None:
        return ()

    grant_recipe_refs = {
        step.recipe_ref for step in execution_plan.steps if step.type is StepType.GRANT_PERMISSIONS
    }
    action_counts_by_recipe: dict[str, int] = {}
    for action in execution_plan.permission_plan.actions:
        recipe_ref = action.source.recipe_id
        action_counts_by_recipe[recipe_ref] = action_counts_by_recipe.get(recipe_ref, 0) + 1

    warnings: list[WarningMessage] = []
    for recipe_ref, action_count in action_counts_by_recipe.items():
        if recipe_ref in grant_recipe_refs:
            continue
        action_label = "permission action" if action_count == 1 else "permission actions"
        warnings.append(
            WarningMessage(
                code=WarningCode.ORPHANED_PERMISSION_ACTIONS,
                message=(
                    f"Recipe {recipe_ref!r} produced {action_count} {action_label} but has no "
                    "grant_permissions step. These permissions will not be applied during execution."
                ),
                details={
                    "recipe_ref": recipe_ref,
                    "permission_action_count": action_count,
                },
            )
        )
    return tuple(warnings)


def _emit_runtime_permission_action(
    recipe_id: str,
    index: int,
    grant: RuntimePermissionGrant,
    *,
    rooted: bool,
    android_api_level: int | None,
) -> PermissionPlanAction:
    reason = _evaluate_permission_when(grant.when, rooted=rooted, android_api_level=android_api_level)
    return PermissionPlanAction(
        status="applicable" if reason is None else "skipped",
        kind="runtime_permission",
        package_name=grant.package_name,
        permission=grant.name,
        required=grant.required,
        command=("adb", "shell", "pm", "grant", grant.package_name, grant.name),
        source=PermissionPlanSource(recipe_id=recipe_id, section=f"permissions.runtime[{index}]"),
        reason=reason,
    )


def _emit_appop_action(
    recipe_id: str,
    index: int,
    grant: AppOpGrant,
    *,
    rooted: bool,
    android_api_level: int | None,
) -> PermissionPlanAction:
    reason = _evaluate_permission_when(grant.when, rooted=rooted, android_api_level=android_api_level)
    return PermissionPlanAction(
        status="applicable" if reason is None else "skipped",
        kind="appop",
        package_name=grant.package_name,
        op=grant.op,
        desired_mode=grant.mode,
        required=grant.required,
        command=("adb", "shell", "appops", "set", grant.package_name, grant.op, grant.mode),
        source=PermissionPlanSource(recipe_id=recipe_id, section=f"permissions.appops[{index}]"),
        reason=reason,
    )


def _emit_manual_permission_action(
    recipe_id: str,
    index: int,
    grant: ManualPermissionRequirement,
    *,
    rooted: bool,
    android_api_level: int | None,
) -> PermissionPlanAction:
    applicability_reason = _evaluate_permission_when(grant.when, rooted=rooted, android_api_level=android_api_level)
    reason = applicability_reason or PermissionPlanReason(code="manual_required", message=grant.reason)
    status = "skipped" if applicability_reason is not None else "manual_required"
    return PermissionPlanAction(
        status=status,
        kind="manual_requirement",
        package_name=grant.package_name,
        manual_type=grant.manual_type,
        required=grant.required,
        command=(),
        source=PermissionPlanSource(recipe_id=recipe_id, section=f"permissions.manual[{index}]"),
        reason=reason,
    )


def _evaluate_permission_when(when, *, rooted: bool, android_api_level: int | None) -> PermissionPlanReason | None:
    if when is None:
        return None
    if when.rooted is True and not rooted:
        return PermissionPlanReason(code="requires_root", message="Device is not rooted.")
    if when.rooted is False and rooted:
        return PermissionPlanReason(code="requires_unrooted", message="Device is rooted.")
    if (when.android_api_min is not None or when.android_api_max is not None) and android_api_level is None:
        return PermissionPlanReason(code="missing_android_api_level", message="Device Android API level is unknown.")
    if not _android_api_in_range(android_api_level, minimum=when.android_api_min, maximum=when.android_api_max):
        return PermissionPlanReason(
            code="android_api_out_of_range",
            message=(
                f"Device Android API {android_api_level} is outside supported range "
                f"{_format_android_api_range(when.android_api_min, when.android_api_max)}."
            ),
        )
    return None


def _android_api_in_range(android_api_level: int | None, *, minimum: int | None, maximum: int | None) -> bool:
    if android_api_level is None:
        return False
    if minimum is not None and android_api_level < minimum:
        return False
    if maximum is not None and android_api_level > maximum:
        return False
    return True


def _format_android_api_range(minimum: int | None, maximum: int | None) -> str:
    if minimum is not None and maximum is not None:
        return f"min={minimum} max={maximum}"
    if minimum is not None:
        return f">= {minimum}"
    if maximum is not None:
        return f"<= {maximum}"
    return "any"
