"""Conflict resolution for selected steps."""

from __future__ import annotations

from dataclasses import dataclass

from emuchef.domain import ErrorCode, ErrorMessage, Step


@dataclass(frozen=True, slots=True)
class StepConflictContext:
    step_id: str
    recipe_ref: str
    step: Step
    auto_included_recipe: bool
    explicitly_selected_step: bool

    @property
    def priority(self) -> tuple[int, int, int]:
        return (
            1 if self.explicitly_selected_step else 0,
            1 if not self.auto_included_recipe else 0,
            1 if not self.step.user_toggleable else 0,
        )


def resolve_step_conflicts(
    contexts: dict[str, StepConflictContext],
    selected_step_ids: set[str],
) -> tuple[set[str] | None, tuple[ErrorMessage, ...]]:
    selected = set(selected_step_ids)

    while True:
        changed = False
        _prune_orphaned_steps(contexts, selected)

        seen_pairs: set[tuple[str, str]] = set()
        for step_id in sorted(selected):
            context = contexts[step_id]
            for target_id in _normalize_conflict_targets(context):
                if target_id not in selected or target_id not in contexts:
                    continue
                pair = tuple(sorted((step_id, target_id)))
                if pair in seen_pairs:
                    continue
                seen_pairs.add(pair)

                other = contexts[target_id]
                if context.priority == other.priority:
                    return None, (
                        ErrorMessage(
                            code=ErrorCode.CONFLICT_UNRESOLVED,
                            message=f"Conflict between {step_id!r} and {target_id!r} could not be resolved deterministically.",
                            details={"step_ids": [step_id, target_id]},
                        ),
                    )

                loser = step_id if context.priority < other.priority else target_id
                if loser in selected:
                    selected.remove(loser)
                    changed = True

        if not changed:
            break

    _prune_orphaned_steps(contexts, selected)
    return selected, ()


def _normalize_conflict_targets(context: StepConflictContext) -> tuple[str, ...]:
    targets: list[str] = []
    for raw_target in context.step.constraints.conflicts_with:
        if "/" in raw_target:
            targets.append(raw_target)
        else:
            targets.append(f"{context.recipe_ref}/{raw_target}")
    return tuple(targets)


def _prune_orphaned_steps(contexts: dict[str, StepConflictContext], selected: set[str]) -> None:
    changed = True
    while changed:
        changed = False
        for step_id in list(selected):
            context = contexts[step_id]
            dependency_ids = {f"{context.recipe_ref}/{dependency}" for dependency in context.step.dependencies}
            if dependency_ids.issubset(selected):
                continue
            selected.remove(step_id)
            changed = True
