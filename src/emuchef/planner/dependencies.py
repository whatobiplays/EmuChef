"""Dependency expansion and cycle detection helpers."""

from __future__ import annotations

import logging
from collections import deque

from emuchef.domain import ErrorCode, ErrorMessage, Recipe, Step

from .catalog import AuthoredCatalog

logger = logging.getLogger(__name__)


def expand_recipe_dependencies(
    catalog: AuthoredCatalog, selected_recipe_refs: tuple[str, ...]
) -> tuple[tuple[str, ...], tuple[ErrorMessage, ...]]:
    logger.debug("Expanding recipe dependencies for explicit recipes: %s", list(selected_recipe_refs))
    ordered: list[str] = []
    permanent: set[str] = set()
    temporary: set[str] = set()
    errors: list[ErrorMessage] = []

    def visit(recipe_ref: str, stack: tuple[str, ...]) -> None:
        if recipe_ref in permanent:
            return
        if recipe_ref in temporary:
            cycle = stack + (recipe_ref,)
            errors.append(
                ErrorMessage(
                    code=ErrorCode.DEPENDENCY_CYCLE,
                    message=f"Recipe dependency cycle detected at {recipe_ref!r}.",
                    details={"cycle": list(cycle)},
                )
            )
            return
        recipe = catalog.recipes.get(recipe_ref)
        if recipe is None:
            errors.append(
                ErrorMessage(
                    code=ErrorCode.RECIPE_NOT_FOUND,
                    message=f"Recipe {recipe_ref!r} was not found.",
                    details={"recipe_ref": recipe_ref},
                )
            )
            return
        temporary.add(recipe_ref)
        for dependency_ref in recipe.recipe_dependencies:
            visit(dependency_ref, stack + (recipe_ref,))
        temporary.remove(recipe_ref)
        permanent.add(recipe_ref)
        if recipe_ref not in ordered:
            ordered.append(recipe_ref)

    for recipe_ref in selected_recipe_refs:
        visit(recipe_ref, ())

    logger.debug("Expanded recipe order: %s", ordered)
    return tuple(ordered), tuple(errors)


def validate_recipe_step_cycles(recipe: Recipe) -> tuple[ErrorMessage, ...]:
    by_id = {step.id: step for step in recipe.steps}
    permanent: set[str] = set()
    temporary: set[str] = set()
    missing_dependencies: set[tuple[str, str]] = set()
    errors: list[ErrorMessage] = []

    def visit(step_id: str, stack: tuple[str, ...]) -> None:
        if step_id in permanent:
            return
        if step_id in temporary:
            cycle = stack + (step_id,)
            errors.append(
                ErrorMessage(
                    code=ErrorCode.DEPENDENCY_CYCLE,
                    message=f"Step dependency cycle detected in recipe {recipe.id!r}.",
                    details={"recipe_ref": recipe.id, "cycle": list(cycle)},
                )
            )
            return
        step = by_id.get(step_id)
        if step is None:
            if not stack:
                return
            dependent_step_id = stack[-1]
            missing_key = (dependent_step_id, step_id)
            if missing_key in missing_dependencies:
                return
            missing_dependencies.add(missing_key)
            errors.append(
                ErrorMessage(
                    code=ErrorCode.STEP_NOT_FOUND,
                    message=f"Step {dependent_step_id!r} depends on unknown step {step_id!r}.",
                    details={
                        "recipe_ref": recipe.id,
                        "step_id": dependent_step_id,
                        "dependency": step_id,
                    },
                )
            )
            return
        temporary.add(step_id)
        for dependency in step.dependencies:
            visit(dependency, stack + (step_id,))
        temporary.remove(step_id)
        permanent.add(step_id)

    for step_id in by_id:
        visit(step_id, ())

    return tuple(errors)


def topologically_sort_steps(
    steps: tuple[tuple[str, Step], ...],
) -> tuple[tuple[tuple[str, Step], ...], tuple[ErrorMessage, ...]]:
    ordered_steps = list(steps)
    index_by_id = {step_id: index for index, (step_id, _) in enumerate(ordered_steps)}
    dependency_graph: dict[str, set[str]] = {}
    reverse_graph: dict[str, set[str]] = {}
    indegree: dict[str, int] = {}

    for step_id, step in ordered_steps:
        step_dependencies = {f"{step_id.split('/')[0]}/{dependency}" for dependency in step.dependencies}
        dependency_graph[step_id] = step_dependencies
        indegree[step_id] = len(step_dependencies)
        reverse_graph.setdefault(step_id, set())
        for dependency in step_dependencies:
            reverse_graph.setdefault(dependency, set()).add(step_id)

    queue = deque(
        sorted(
            (step_id for step_id, degree in indegree.items() if degree == 0),
            key=index_by_id.__getitem__,
        )
    )
    result: list[tuple[str, Step]] = []

    while queue:
        step_id = queue.popleft()
        result.append((step_id, dict(ordered_steps)[step_id]))
        for dependent in sorted(reverse_graph.get(step_id, ()), key=index_by_id.__getitem__):
            indegree[dependent] -= 1
            if indegree[dependent] == 0:
                queue.append(dependent)

    if len(result) != len(ordered_steps):
        return (), (
            ErrorMessage(
                code=ErrorCode.DEPENDENCY_CYCLE,
                message="Execution step dependency cycle detected.",
                details={"step_ids": list(index_by_id)},
            ),
        )

    return tuple(result), ()
