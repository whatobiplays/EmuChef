"""Planner operation types."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Literal, TypeAlias

from emuchef.domain import JSONValue


class OperationType(str, Enum):
    SELECT_RECIPE = "select_recipe"
    DESELECT_RECIPE = "deselect_recipe"
    SELECT_STEP = "select_step"
    DESELECT_STEP = "deselect_step"
    BIND_INPUT = "bind_input"
    UNBIND_INPUT = "unbind_input"


@dataclass(frozen=True, slots=True)
class SelectRecipe:
    recipe_ref: str
    type: Literal[OperationType.SELECT_RECIPE] = OperationType.SELECT_RECIPE


@dataclass(frozen=True, slots=True)
class DeselectRecipe:
    recipe_ref: str
    type: Literal[OperationType.DESELECT_RECIPE] = OperationType.DESELECT_RECIPE


@dataclass(frozen=True, slots=True)
class SelectStep:
    step_id: str
    type: Literal[OperationType.SELECT_STEP] = OperationType.SELECT_STEP


@dataclass(frozen=True, slots=True)
class DeselectStep:
    step_id: str
    type: Literal[OperationType.DESELECT_STEP] = OperationType.DESELECT_STEP


@dataclass(frozen=True, slots=True)
class BindInput:
    input_id: str
    value: JSONValue
    type: Literal[OperationType.BIND_INPUT] = OperationType.BIND_INPUT


@dataclass(frozen=True, slots=True)
class UnbindInput:
    input_id: str
    type: Literal[OperationType.UNBIND_INPUT] = OperationType.UNBIND_INPUT


DraftOperation: TypeAlias = SelectRecipe | DeselectRecipe | SelectStep | DeselectStep | BindInput | UnbindInput


def operation_name(operation: DraftOperation) -> str:
    return operation.type.value
