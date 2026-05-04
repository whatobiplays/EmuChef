"""Execution handler for the ``wait`` built-in step."""

from __future__ import annotations

from collections.abc import Mapping

from emuchef.domain import ExecutionStep, RuntimeValue
from emuchef.executor.step_runtime import ExecutionContext


def handle(context: ExecutionContext, step: ExecutionStep, resolved_params: Mapping[str, object]) -> dict[str, RuntimeValue]:
    raw_duration = resolved_params["duration_ms"]
    if isinstance(raw_duration, bool) or not isinstance(raw_duration, int) or raw_duration <= 0:
        raise ValueError(f"wait step requires a positive integer duration_ms: {raw_duration!r}")
    context.sleep_fn(raw_duration / 1000.0)
    return {}
