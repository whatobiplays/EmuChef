"""Typed draft change payloads."""

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class DraftPlanChanges:
    added_recipe_ids: tuple[str, ...] = ()
    removed_recipe_ids: tuple[str, ...] = ()
    selected_recipe_ids: tuple[str, ...] = ()
    deselected_recipe_ids: tuple[str, ...] = ()
    availability_changed_recipe_ids: tuple[str, ...] = ()
    added_step_ids: tuple[str, ...] = ()
    removed_step_ids: tuple[str, ...] = ()
    selected_step_ids: tuple[str, ...] = ()
    deselected_step_ids: tuple[str, ...] = ()
    availability_changed_step_ids: tuple[str, ...] = ()
    added_input_ids: tuple[str, ...] = ()
    removed_input_ids: tuple[str, ...] = ()
    bound_input_ids: tuple[str, ...] = ()
    unbound_input_ids: tuple[str, ...] = ()
