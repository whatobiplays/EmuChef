"""Execution handler for the ``launch_app`` built-in step."""

from __future__ import annotations

from collections.abc import Mapping

from emuchef.domain import ExecutionStep, RuntimeValue
from emuchef.executor.step_runtime import ExecutionContext


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    package_name = str(resolved_params["package_name"])
    activity = resolved_params.get("activity")
    context.adb.launch_app(package_name, str(activity) if activity is not None else None)
    return {}
