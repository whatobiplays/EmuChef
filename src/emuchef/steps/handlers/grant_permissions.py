"""Execution handler for the ``grant_permissions`` built-in step."""

from __future__ import annotations

from collections.abc import Mapping

from emuchef.domain import ExecutionStep, RuntimeValue, RuntimeValueType
from emuchef.executor.adb import AdbResolutionError
from emuchef.executor.permission_helpers import (
    permission_actions,
    permission_command,
    permission_not_applicable_reason,
    permission_policy,
    permission_result_base,
)
from emuchef.executor.step_runtime import ExecutionContext, StepExecutionFailure


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    policy = permission_policy(resolved_params.get("policy"))
    action_results: list[dict[str, object]] = []
    failure_message: str | None = None

    for action in permission_actions(resolved_params):
        reason = permission_not_applicable_reason(
            action.get("when"),
            rooted=context.runtime_capabilities.root_shell,
            android_api_level=context.device_context.android_api_level,
        )
        if reason is not None:
            action_results.append({**permission_result_base(step, action), "status": "not_applicable", **reason})
            continue

        try:
            context.adb.run_plan_command(tuple(permission_command(action)))
            action_results.append({**permission_result_base(step, action), "status": "executed"})
        except Exception as exc:
            if isinstance(exc, AdbResolutionError):
                raise
            message = str(exc)
            action_results.append({**permission_result_base(step, action), "status": "failed", "message": message})
            if bool(action.get("required", True)) or policy["require_all"] or policy["on_failure"] == "fail":
                failure_message = message
                break

    outputs = {
        "permission_results": RuntimeValue(
            type=RuntimeValueType.OBJECT,
            value={"actions": action_results},
        )
    }
    if failure_message is not None:
        raise StepExecutionFailure(failure_message, outputs=outputs)
    return outputs
