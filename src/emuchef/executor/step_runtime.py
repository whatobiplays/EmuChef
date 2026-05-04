"""Shared runtime contracts and registry dispatch for step execution."""

from __future__ import annotations

from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path

from emuchef.domain import (
    DeviceContext,
    ErrorCode,
    ExecutionArtifact,
    ExecutionState,
    ExecutionStep,
    RuntimeCapabilities,
    RuntimeValue,
)
from emuchef.steps import builtin_step_registry

from .adb import AdbInterface


class StepExecutionError(RuntimeError):
    def __init__(self, code: ErrorCode, message: str, outputs: Mapping[str, RuntimeValue] | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.outputs = dict(outputs or {})


class StepExecutionFailure(RuntimeError):
    def __init__(self, message: str, outputs: Mapping[str, RuntimeValue] | None = None) -> None:
        super().__init__(message)
        self.outputs = dict(outputs or {})


@dataclass(slots=True)
class ExecutionContext:
    adb: AdbInterface
    workdir: Path
    artifacts_by_id: Mapping[str, ExecutionArtifact]
    state: ExecutionState
    device_context: DeviceContext
    runtime_capabilities: RuntimeCapabilities
    sleep_fn: Callable[[float], None]


def execute_step(
    context: ExecutionContext,
    step: ExecutionStep,
    resolved_params: Mapping[str, object],
) -> dict[str, RuntimeValue]:
    plugin = builtin_step_registry().require(step.type)
    result = plugin.handler(context, step, resolved_params)
    return {} if result is None else result
