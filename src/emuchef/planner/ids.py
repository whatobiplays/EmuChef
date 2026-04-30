"""Shared planner/execution-plan identifier helpers."""

from __future__ import annotations


def make_execution_step_id(recipe_ref: str, step_id: str) -> str:
    return f"{recipe_ref}/{step_id}"


def make_execution_input_id(recipe_ref: str, input_id: str) -> str:
    return f"{recipe_ref}/{input_id}"


def make_execution_artifact_id(recipe_ref: str, artifact_id: str) -> str:
    return f"{recipe_ref}/{artifact_id}"
