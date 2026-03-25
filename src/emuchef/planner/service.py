"""Planner service and mutable planner sessions."""

from __future__ import annotations

from dataclasses import dataclass, replace

from emuchef.domain import (
    DeviceContext,
    DraftPlan,
    DraftPlanChanges,
    DraftUpdateResult,
    ErrorCode,
    ErrorMessage,
    PlanningResult,
    RuntimeCapabilities,
)

from .bindings import extract_planner_overrides, validate_binding_value
from .catalog import AuthoredCatalog
from .draft_builder import build_draft_plan
from .emitter import emit_execution_plan
from .history import HistoryManager
from .operations import (
    BindInput,
    DeselectRecipe,
    DeselectStep,
    DraftOperation,
    SelectRecipe,
    SelectStep,
    UnbindInput,
    operation_name,
)


@dataclass(frozen=True, slots=True)
class _SessionState:
    draft_id: str
    plan_id: str
    device_plan_ref: str
    device_profile_ref: str
    device_context: DeviceContext
    runtime_capabilities: RuntimeCapabilities
    user_selected_recipe_refs: tuple[str, ...]
    step_selection_overrides: dict[str, bool]
    user_bindings: dict[str, object]
    planner_overrides: dict[str, object]


class Planner:
    def __init__(self, catalog: AuthoredCatalog) -> None:
        self.catalog = catalog

    def start_session(
        self,
        device_plan_ref: str,
        device_context: DeviceContext,
        runtime_capabilities: RuntimeCapabilities | None = None,
        planner_overrides: dict[str, object] | None = None,
        draft_id: str | None = None,
        plan_id: str | None = None,
    ) -> "PlannerSession":
        device_plan = self.catalog.device_plans.get(device_plan_ref)
        if device_plan is None:
            raise ValueError(f"Unknown device plan: {device_plan_ref}")
        device_profile = self.catalog.device_profiles.get(device_plan.device_profile_ref)
        if device_profile is None:
            raise ValueError(f"Unknown device profile: {device_plan.device_profile_ref}")

        merged_context = device_context
        if not merged_context.device_tags:
            merged_context = replace(merged_context, device_tags=device_profile.device_tags)

        state = _SessionState(
            draft_id=draft_id or f"draft.{device_plan_ref}.001",
            plan_id=plan_id or f"plan.{device_plan_ref}.001",
            device_plan_ref=device_plan_ref,
            device_profile_ref=device_plan.device_profile_ref,
            device_context=merged_context,
            runtime_capabilities=runtime_capabilities or device_profile.capability_defaults,
            user_selected_recipe_refs=tuple(
                selection.recipe_ref for selection in device_plan.recipes if selection.selected_by_default
            ),
            step_selection_overrides={},
            user_bindings={},
            planner_overrides={
                # Only top-level full-ref keys participate in binding resolution.
                # Nested metadata such as `overrides.config_variants` is preserved but ignored here.
                **extract_planner_overrides(device_plan.overrides),
                **(planner_overrides or {}),
            },
        )
        draft_plan, errors = build_draft_plan(
            catalog=self.catalog,
            draft_id=state.draft_id,
            device_plan_ref=state.device_plan_ref,
            device_profile_ref=state.device_profile_ref,
            device_context=state.device_context,
            runtime_capabilities=state.runtime_capabilities,
            user_selected_recipe_refs=state.user_selected_recipe_refs,
            step_selection_overrides=state.step_selection_overrides,
            user_bindings=state.user_bindings,
            planner_overrides=state.planner_overrides,
        )
        if errors or draft_plan is None:
            raise ValueError(errors[0].message if errors else "Failed to build initial draft")
        return PlannerSession(catalog=self.catalog, state=state, draft_plan=draft_plan)


class PlannerSession:
    def __init__(self, catalog: AuthoredCatalog, state: _SessionState, draft_plan: DraftPlan) -> None:
        self._catalog = catalog
        self._state = state
        self._draft_plan = draft_plan
        self._history = HistoryManager[_SessionState]()

    @property
    def draft_plan(self) -> DraftPlan:
        return self._draft_plan

    @property
    def history(self) -> HistoryManager[_SessionState]:
        return self._history

    def select_recipe(self, recipe_ref: str) -> DraftUpdateResult:
        return self.apply(SelectRecipe(recipe_ref=recipe_ref))

    def deselect_recipe(self, recipe_ref: str) -> DraftUpdateResult:
        return self.apply(DeselectRecipe(recipe_ref=recipe_ref))

    def select_step(self, step_id: str) -> DraftUpdateResult:
        return self.apply(SelectStep(step_id=step_id))

    def deselect_step(self, step_id: str) -> DraftUpdateResult:
        return self.apply(DeselectStep(step_id=step_id))

    def bind_input(self, input_id: str, value) -> DraftUpdateResult:
        return self.apply(BindInput(input_id=input_id, value=value))

    def unbind_input(self, input_id: str) -> DraftUpdateResult:
        return self.apply(UnbindInput(input_id=input_id))

    def apply(self, operation: DraftOperation) -> DraftUpdateResult:
        before_state = self._state
        before_draft = self._draft_plan

        maybe_error = self._validate_operation(operation)
        if maybe_error is not None:
            return DraftUpdateResult(
                draft_plan=self._draft_plan,
                changes=DraftPlanChanges(),
                history_entry=None,
                warnings=(),
                errors=(maybe_error,),
            )

        after_state = self._mutate_state(operation)
        after_draft, errors = build_draft_plan(
            catalog=self._catalog,
            draft_id=after_state.draft_id,
            device_plan_ref=after_state.device_plan_ref,
            device_profile_ref=after_state.device_profile_ref,
            device_context=after_state.device_context,
            runtime_capabilities=after_state.runtime_capabilities,
            user_selected_recipe_refs=after_state.user_selected_recipe_refs,
            step_selection_overrides=after_state.step_selection_overrides,
            user_bindings=after_state.user_bindings,
            planner_overrides=after_state.planner_overrides,
        )
        if errors or after_draft is None:
            return DraftUpdateResult(
                draft_plan=self._draft_plan,
                changes=DraftPlanChanges(),
                history_entry=None,
                warnings=(),
                errors=errors,
            )

        self._state = after_state
        self._draft_plan = after_draft
        history_entry = self._history.record(
            operation=operation_name(operation),
            before_state=before_state,
            before_draft=before_draft,
            after_state=after_state,
            after_draft=after_draft,
        )
        return DraftUpdateResult(
            draft_plan=after_draft,
            changes=_compute_changes(before_draft, after_draft),
            history_entry=history_entry,
            warnings=after_draft.warnings,
            errors=(),
        )

    def undo(self) -> DraftUpdateResult:
        undone = self._history.undo()
        if undone is None:
            return DraftUpdateResult(
                draft_plan=self._draft_plan,
                changes=DraftPlanChanges(),
                history_entry=None,
                warnings=(),
                errors=(
                    ErrorMessage(
                        code=ErrorCode.INVALID_OPERATION,
                        message="There is no draft history entry to undo.",
                        details={},
                    ),
                ),
            )
        _, snapshot = undone
        before_draft = self._draft_plan
        self._state = snapshot.state
        self._draft_plan = snapshot.draft_plan
        return DraftUpdateResult(
            draft_plan=self._draft_plan,
            changes=_compute_changes(before_draft, self._draft_plan),
            history_entry=None,
            warnings=self._draft_plan.warnings,
            errors=(),
        )

    def redo(self) -> DraftUpdateResult:
        redone = self._history.redo()
        if redone is None:
            return DraftUpdateResult(
                draft_plan=self._draft_plan,
                changes=DraftPlanChanges(),
                history_entry=None,
                warnings=(),
                errors=(
                    ErrorMessage(
                        code=ErrorCode.INVALID_OPERATION,
                        message="There is no draft history entry to redo.",
                        details={},
                    ),
                ),
            )
        _, snapshot = redone
        before_draft = self._draft_plan
        self._state = snapshot.state
        self._draft_plan = snapshot.draft_plan
        return DraftUpdateResult(
            draft_plan=self._draft_plan,
            changes=_compute_changes(before_draft, self._draft_plan),
            history_entry=None,
            warnings=self._draft_plan.warnings,
            errors=(),
        )

    def emit_execution_plan(self) -> PlanningResult:
        return emit_execution_plan(
            catalog=self._catalog,
            draft_plan=self._draft_plan,
            user_bindings=self._state.user_bindings,
            planner_overrides=self._state.planner_overrides,
            step_selection_overrides=self._state.step_selection_overrides,
            plan_id=self._state.plan_id,
        )

    def _validate_operation(self, operation: DraftOperation) -> ErrorMessage | None:
        if isinstance(operation, (SelectRecipe, DeselectRecipe)):
            if operation.recipe_ref not in self._catalog.recipes:
                return ErrorMessage(
                    code=ErrorCode.RECIPE_NOT_FOUND,
                    message=f"Recipe {operation.recipe_ref!r} was not found.",
                    details={"recipe_ref": operation.recipe_ref},
                )
        if isinstance(operation, (SelectStep, DeselectStep)):
            if operation.step_id not in {step.id for step in self._draft_plan.steps}:
                return ErrorMessage(
                    code=ErrorCode.STEP_NOT_FOUND,
                    message=f"Step {operation.step_id!r} was not found in the current draft plan.",
                    details={"step_id": operation.step_id},
                )
        if isinstance(operation, DeselectStep):
            step_state = next(step for step in self._draft_plan.steps if step.id == operation.step_id)
            if not step_state.user_toggleable:
                return ErrorMessage(
                    code=ErrorCode.STEP_NOT_TOGGLEABLE,
                    message=f"Step {operation.step_id!r} cannot be deselected because it is not user-toggleable.",
                    details={"step_id": operation.step_id},
                )
        if isinstance(operation, (BindInput, UnbindInput)):
            if operation.input_id not in {item.id for item in self._draft_plan.inputs}:
                return ErrorMessage(
                    code=ErrorCode.INPUT_NOT_FOUND,
                    message=f"Input {operation.input_id!r} was not found in the current draft plan.",
                    details={"input_id": operation.input_id},
                )
        if isinstance(operation, BindInput):
            declaration = self._catalog.binding_inputs[operation.input_id]
            validation_errors = validate_binding_value(operation.input_id, declaration, operation.value)
            if validation_errors:
                return validation_errors[0]
        if isinstance(operation, DeselectRecipe):
            if operation.recipe_ref not in self._draft_plan.source.selected_recipe_refs:
                return ErrorMessage(
                    code=ErrorCode.INVALID_OPERATION,
                    message=f"Recipe {operation.recipe_ref!r} is not explicitly selected.",
                    details={"recipe_ref": operation.recipe_ref},
                )
        return None

    def _mutate_state(self, operation: DraftOperation) -> _SessionState:
        selected_recipes = list(self._state.user_selected_recipe_refs)
        step_overrides = dict(self._state.step_selection_overrides)
        user_bindings = dict(self._state.user_bindings)

        if isinstance(operation, SelectRecipe) and operation.recipe_ref not in selected_recipes:
            selected_recipes.append(operation.recipe_ref)

        if isinstance(operation, DeselectRecipe):
            selected_recipes = [recipe_ref for recipe_ref in selected_recipes if recipe_ref != operation.recipe_ref]
            step_overrides = {
                step_id: selected
                for step_id, selected in step_overrides.items()
                if not step_id.startswith(f"{operation.recipe_ref}/")
            }

        if isinstance(operation, SelectStep):
            step_overrides[operation.step_id] = True

        if isinstance(operation, DeselectStep):
            step_overrides[operation.step_id] = False
            recipe_ref, local_step_id = operation.step_id.split("/", 1)
            recipe = self._catalog.recipes[recipe_ref]
            dependents = _dependent_step_ids(recipe, local_step_id)
            for dependent in dependents:
                step_overrides[f"{recipe_ref}/{dependent}"] = False

        if isinstance(operation, BindInput):
            user_bindings[operation.input_id] = operation.value

        if isinstance(operation, UnbindInput):
            user_bindings.pop(operation.input_id, None)

        return replace(
            self._state,
            user_selected_recipe_refs=tuple(selected_recipes),
            step_selection_overrides=step_overrides,
            user_bindings=user_bindings,
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


def _compute_changes(before: DraftPlan, after: DraftPlan) -> DraftPlanChanges:
    before_recipes = {item.id: item for item in before.recipes}
    after_recipes = {item.id: item for item in after.recipes}
    before_steps = {item.id: item for item in before.steps}
    after_steps = {item.id: item for item in after.steps}
    before_inputs = {item.id: item for item in before.inputs}
    after_inputs = {item.id: item for item in after.inputs}

    return DraftPlanChanges(
        added_recipe_ids=tuple(sorted(after_recipes.keys() - before_recipes.keys())),
        removed_recipe_ids=tuple(sorted(before_recipes.keys() - after_recipes.keys())),
        selected_recipe_ids=tuple(
            sorted(
                recipe_id
                for recipe_id in before_recipes.keys() & after_recipes.keys()
                if not before_recipes[recipe_id].selected and after_recipes[recipe_id].selected
            )
        ),
        deselected_recipe_ids=tuple(
            sorted(
                recipe_id
                for recipe_id in before_recipes.keys() & after_recipes.keys()
                if before_recipes[recipe_id].selected and not after_recipes[recipe_id].selected
            )
        ),
        availability_changed_recipe_ids=tuple(
            sorted(
                recipe_id
                for recipe_id in before_recipes.keys() & after_recipes.keys()
                if before_recipes[recipe_id].availability != after_recipes[recipe_id].availability
            )
        ),
        added_step_ids=tuple(sorted(after_steps.keys() - before_steps.keys())),
        removed_step_ids=tuple(sorted(before_steps.keys() - after_steps.keys())),
        selected_step_ids=tuple(
            sorted(
                step_id
                for step_id in before_steps.keys() & after_steps.keys()
                if not before_steps[step_id].selected and after_steps[step_id].selected
            )
        ),
        deselected_step_ids=tuple(
            sorted(
                step_id
                for step_id in before_steps.keys() & after_steps.keys()
                if before_steps[step_id].selected and not after_steps[step_id].selected
            )
        ),
        availability_changed_step_ids=tuple(
            sorted(
                step_id
                for step_id in before_steps.keys() & after_steps.keys()
                if before_steps[step_id].availability != after_steps[step_id].availability
            )
        ),
        added_input_ids=tuple(sorted(after_inputs.keys() - before_inputs.keys())),
        removed_input_ids=tuple(sorted(before_inputs.keys() - after_inputs.keys())),
        bound_input_ids=tuple(
            sorted(
                input_id
                for input_id in before_inputs.keys() & after_inputs.keys()
                if after_inputs[input_id].resolved
                and (
                    not before_inputs[input_id].resolved
                    or before_inputs[input_id].value != after_inputs[input_id].value
                )
            )
        ),
        unbound_input_ids=tuple(
            sorted(
                input_id
                for input_id in before_inputs.keys() & after_inputs.keys()
                if before_inputs[input_id].resolved and not after_inputs[input_id].resolved
            )
        ),
    )
