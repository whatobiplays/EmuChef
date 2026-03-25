"""Dumb execution runner for emitted plans."""

from __future__ import annotations

import logging
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path

from emuchef.domain import ExecutionPlan

from .adb import AdbInterface, AdbResolutionError
from .conditions import evaluate_condition
from .result import (
    ExecutionProgressEvent,
    ExecutionRunResult,
    ProgressPhase,
    ProgressStatus,
    StepRunRecord,
    StepRunStatus,
)
from .step_handlers import execute_step

logger = logging.getLogger(__name__)

ProgressCallback = Callable[[ExecutionProgressEvent], None]


class ExecutorRunner:
    def __init__(self, adb: AdbInterface, workdir: str | Path | None = None) -> None:
        self._adb = adb
        self._workdir = Path(workdir or ".").resolve()

    def run(self, plan: ExecutionPlan, progress_callback: ProgressCallback | None = None) -> ExecutionRunResult:
        _validate_execution_plan(plan)

        total_steps = len(plan.steps)
        records: list[StepRunRecord] = []
        for step_index, step in enumerate(plan.steps, start=1):
            try:
                _emit_progress(
                    progress_callback,
                    step_index=step_index,
                    total_steps=total_steps,
                    step_id=step.id,
                    step_name=step.name,
                    phase=ProgressPhase.CHECKING_SKIP_CONDITIONS,
                )
                logger.debug("Evaluating skip_if conditions for %s", step.id)
                if any(evaluate_condition(self._adb, condition) for condition in step.skip_if):
                    logger.info("Skipped step %s because skip_if matched", step.id)
                    records.append(StepRunRecord(step_id=step.id, status=StepRunStatus.SKIPPED, message="skip_if matched"))
                    _emit_progress(
                        progress_callback,
                        step_index=step_index,
                        total_steps=total_steps,
                        step_id=step.id,
                        step_name=step.name,
                        phase=ProgressPhase.FINISHED,
                        status=ProgressStatus.SKIPPED,
                        message="skip_if matched",
                    )
                    continue
                _emit_progress(
                    progress_callback,
                    step_index=step_index,
                    total_steps=total_steps,
                    step_id=step.id,
                    step_name=step.name,
                    phase=ProgressPhase.EXECUTING,
                )
                logger.info("Executing step %s (%s)", step.id, step.type.value)
                execute_step(self._adb, step, self._workdir)
                _emit_progress(
                    progress_callback,
                    step_index=step_index,
                    total_steps=total_steps,
                    step_id=step.id,
                    step_name=step.name,
                    phase=ProgressPhase.VERIFYING,
                )
                logger.debug("Verifying step %s", step.id)
                failed_verify = [condition.type for condition in step.verify if not evaluate_condition(self._adb, condition)]
                if failed_verify:
                    message = f"verify failed: {', '.join(failed_verify)}"
                    logger.error("Verification failed for step %s: %s", step.id, failed_verify)
                    records.append(
                        StepRunRecord(
                            step_id=step.id,
                            status=StepRunStatus.FAILED,
                            message=message,
                        )
                    )
                    _emit_progress(
                        progress_callback,
                        step_index=step_index,
                        total_steps=total_steps,
                        step_id=step.id,
                        step_name=step.name,
                        phase=ProgressPhase.FINISHED,
                        status=ProgressStatus.FAILED,
                        message=message,
                    )
                    return ExecutionRunResult(success=False, total_steps=total_steps, steps=tuple(records))
                logger.info("Step succeeded: %s", step.id)
                records.append(StepRunRecord(step_id=step.id, status=StepRunStatus.EXECUTED))
                _emit_progress(
                    progress_callback,
                    step_index=step_index,
                    total_steps=total_steps,
                    step_id=step.id,
                    step_name=step.name,
                    phase=ProgressPhase.FINISHED,
                    status=ProgressStatus.SUCCEEDED,
                )
            except Exception as exc:
                if isinstance(exc, AdbResolutionError):
                    raise
                logger.exception("Step failed: %s", step.id)
                message = str(exc)
                records.append(StepRunRecord(step_id=step.id, status=StepRunStatus.FAILED, message=message))
                _emit_progress(
                    progress_callback,
                    step_index=step_index,
                    total_steps=total_steps,
                    step_id=step.id,
                    step_name=step.name,
                    phase=ProgressPhase.FINISHED,
                    status=ProgressStatus.FAILED,
                    message=message,
                )
                return ExecutionRunResult(success=False, total_steps=total_steps, steps=tuple(records))

        return ExecutionRunResult(success=True, total_steps=total_steps, steps=tuple(records))


def _validate_execution_plan(plan: ExecutionPlan) -> None:
    for input_value in plan.inputs_resolved:
        if _contains_ref(input_value.value):
            raise ValueError(f"Execution plan input {input_value.id!r} still contains a ref.")
    for step in plan.steps:
        if _contains_ref(step.params):
            raise ValueError(f"Execution step {step.id!r} still contains a ref.")


def _contains_ref(value) -> bool:
    if isinstance(value, Mapping):
        if "ref" in value:
            return True
        return any(_contains_ref(item) for item in value.values())
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        return any(_contains_ref(item) for item in value)
    return False


def _emit_progress(
    callback: ProgressCallback | None,
    *,
    step_index: int,
    total_steps: int,
    step_id: str,
    step_name: str,
    phase: ProgressPhase,
    status: ProgressStatus | None = None,
    message: str | None = None,
) -> None:
    if callback is None:
        return
    callback(
        ExecutionProgressEvent(
            step_index=step_index,
            total_steps=total_steps,
            step_id=step_id,
            step_name=step_name,
            phase=phase,
            status=status,
            message=message,
        )
    )
