"""Dumb execution runner for emitted plans."""

from __future__ import annotations

import logging
import time
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path

from emuchef.domain import ExecutionPlan, StepType

from .adb import AdbInterface, AdbResolutionError
from .conditions import evaluate_condition
from .result import (
    ExecutionProgressEvent,
    ExecutionRunResult,
    PermissionRunRecord,
    PermissionRunStatus,
    ProgressPhase,
    ProgressStatus,
    StepRunRecord,
    StepRunStatus,
)
from .step_handlers import execute_step

logger = logging.getLogger(__name__)

ProgressCallback = Callable[[ExecutionProgressEvent], None]


class ExecutorRunner:
    def __init__(
        self,
        adb: AdbInterface,
        workdir: str | Path | None = None,
        sleep_fn: Callable[[float], None] | None = None,
    ) -> None:
        self._adb = adb
        self._workdir = Path(workdir or ".").resolve()
        self._sleep_fn = sleep_fn or time.sleep

    def run(self, plan: ExecutionPlan, progress_callback: ProgressCallback | None = None) -> ExecutionRunResult:
        _validate_execution_plan(plan)

        total_steps = len(plan.steps)
        records: list[StepRunRecord] = []
        permission_results: list[PermissionRunRecord] = []
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
                if step.type is StepType.GRANT_PERMISSIONS:
                    step_permission_results = _execute_permission_actions_for_step(self._adb, plan, step)
                    permission_results.extend(step_permission_results)
                    permission_failures = [result for result in step_permission_results if result.status is PermissionRunStatus.FAILED]
                    if permission_failures:
                        message = permission_failures[0].message or "grant_permissions step failed."
                        logger.error("Permission grant step failed for %s: %s", step.id, message)
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
                        return ExecutionRunResult(
                            success=False,
                            total_steps=total_steps,
                            steps=tuple(records),
                            permission_results=tuple(permission_results),
                        )
                else:
                    execute_step(self._adb, step, self._workdir, sleep_fn=self._sleep_fn)
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
                    return ExecutionRunResult(
                        success=False,
                        total_steps=total_steps,
                        steps=tuple(records),
                        permission_results=tuple(permission_results),
                    )
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
                return ExecutionRunResult(
                    success=False,
                    total_steps=total_steps,
                    steps=tuple(records),
                    permission_results=tuple(permission_results),
                )

        return ExecutionRunResult(
            success=True,
            total_steps=total_steps,
            steps=tuple(records),
            permission_results=tuple(permission_results),
        )


def _validate_execution_plan(plan: ExecutionPlan) -> None:
    for input_value in plan.inputs_resolved:
        if _contains_ref(input_value.value):
            raise ValueError(f"Execution plan input {input_value.id!r} still contains a ref.")
    for step in plan.steps:
        if _contains_ref(step.params):
            raise ValueError(f"Execution step {step.id!r} still contains a ref.")


def _execute_permission_actions_for_step(adb: AdbInterface, plan: ExecutionPlan, step) -> list[PermissionRunRecord]:
    if plan.permission_plan is None:
        raise ValueError(f"grant_permissions step {step.id!r} has no permission_plan available.")

    recipe_actions = [action for action in plan.permission_plan.actions if action.source.recipe_id == step.recipe_ref]
    if not recipe_actions:
        raise ValueError(f"grant_permissions step {step.id!r} has no permission actions for recipe {step.recipe_ref!r}.")

    records: list[PermissionRunRecord] = []
    for action in recipe_actions:
        message = action.reason.message if action.reason is not None else None
        if action.status == "skipped":
            logger.info("Permission action skipped: %s %s", action.kind, action.source.section)
            records.append(_permission_result(step.id, action, status=PermissionRunStatus.SKIPPED, message=message))
            continue
        if action.status == "manual_required":
            logger.info("Permission action requires manual follow-up: %s %s", action.kind, action.source.section)
            records.append(_permission_result(step.id, action, status=PermissionRunStatus.MANUAL_REQUIRED, message=message))
            continue
        if not action.command:
            failure_message = "Applicable permission action has no executable command."
            logger.error("Permission action failed: %s %s", action.kind, failure_message)
            records.append(_permission_result(step.id, action, status=PermissionRunStatus.FAILED, message=failure_message))
            break
        try:
            logger.info("Executing permission action %s for %s", action.kind, action.package_name)
            adb.run_plan_command(action.command)
            records.append(_permission_result(step.id, action, status=PermissionRunStatus.EXECUTED))
        except Exception as exc:
            if isinstance(exc, AdbResolutionError):
                raise
            message = str(exc)
            logger.exception("Permission action failed: %s %s", action.kind, action.source.section)
            records.append(_permission_result(step.id, action, status=PermissionRunStatus.FAILED, message=message))
            break
    return records


def _permission_result(step_id: str, action, *, status: PermissionRunStatus, message: str | None = None) -> PermissionRunRecord:
    return PermissionRunRecord(
        step_id=step_id,
        status=status,
        kind=action.kind,
        package_name=action.package_name,
        permission=action.permission,
        op=action.op,
        desired_mode=action.desired_mode,
        manual_type=action.manual_type,
        command=action.command,
        source_recipe_id=action.source.recipe_id,
        source_section=action.source.section,
        message=message,
    )


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
