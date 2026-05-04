"""Execution handler for the ``force_stop_app`` built-in step."""

from __future__ import annotations

from collections.abc import Mapping

from emuchef.domain import ExecutionStep, RuntimeValue
from emuchef.executor.step_runtime import ExecutionContext


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    package_name = str(resolved_params["package_name"])
    if not package_name.strip():
        raise ValueError("force_stop_app step requires a non-empty package_name.")
    context.adb.force_stop_app(package_name)
    return {}
