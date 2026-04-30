"""Single-threaded execution runner for normalized execution plans."""

from __future__ import annotations

import logging
import time
from collections.abc import Callable, Mapping
from pathlib import Path

from emuchef.domain import (
    ArtifactRuntimeState,
    ErrorCode,
    ExecutionPlan,
    ExecutionState,
    LiteralParamValue,
    RefParamValue,
    StepRuntimeState,
    StepRuntimeStatus,
)

from .adb import AdbInterface, AdbResolutionError
from .conditions import evaluate_condition
from .resolver import RefResolutionError, resolve_runtime_ref
from .result import (
    ExecutionProgressEvent,
    ExecutionRunResult,
    ProgressPhase,
    ProgressStatus,
    StepRunRecord,
    StepRunStatus,
)
from .step_handlers import ExecutionContext, execute_step

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
        total_steps = len(plan.steps)
        records: list[StepRunRecord] = []
        state = ExecutionState(
            inputs={item.id: item.value for item in plan.inputs},
            artifacts={artifact.id: ArtifactRuntimeState(artifact_id=artifact.id) for artifact in plan.artifacts},
            steps={},
        )
        context = ExecutionContext(
            adb=self._adb,
            workdir=self._workdir,
            artifacts_by_id={artifact.id: artifact for artifact in plan.artifacts},
            state=state,
            device_context=plan.device_context,
            runtime_capabilities=plan.runtime_capabilities,
            sleep_fn=self._sleep_fn,
        )
        step_ids_in_plan = {step.id for step in plan.steps}

        for step_index, step in enumerate(plan.steps, start=1):
            _emit_progress(
                progress_callback,
                step_index=step_index,
                total_steps=total_steps,
                step_id=step.id,
                step_name=step.name,
                phase=ProgressPhase.CHECKING_SKIP_CONDITIONS,
            )

            blocking_dependencies = _blocking_dependencies(state, step.dependencies)
            if blocking_dependencies:
                message = f"dependency blocked: {', '.join(blocking_dependencies)}"
                logger.info("Blocking step %s because dependencies did not succeed: %s", step.id, blocking_dependencies)
                state.steps[step.id] = StepRuntimeState(
                    step_id=step.id,
                    status=StepRuntimeStatus.BLOCKED,
                    error=message,
                )
                records.append(StepRunRecord(step_id=step.id, status=StepRunStatus.BLOCKED, message=message))
                _emit_progress(
                    progress_callback,
                    step_index=step_index,
                    total_steps=total_steps,
                    step_id=step.id,
                    step_name=step.name,
                    phase=ProgressPhase.FINISHED,
                    status=ProgressStatus.BLOCKED,
                    message=message,
                )
                continue

            missing_capabilities = [
                capability
                for capability in step.constraints.capabilities
                if not bool(getattr(plan.runtime_capabilities, capability, False))
            ]
            if missing_capabilities:
                message = f"missing required capabilities: {', '.join(missing_capabilities)}"
                logger.error("Step failed: %s", message)
                state.steps[step.id] = StepRuntimeState(step_id=step.id, status=StepRuntimeStatus.FAILED, error=message)
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
                continue

            active_conflicts = [conflict for conflict in step.constraints.conflicts_with if conflict in step_ids_in_plan]
            if active_conflicts:
                message = f"conflicting steps present: {', '.join(active_conflicts)}"
                logger.error("Step failed: %s", message)
                state.steps[step.id] = StepRuntimeState(step_id=step.id, status=StepRuntimeStatus.FAILED, error=message)
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
                continue

            try:
                if any(evaluate_condition(self._adb, condition) for condition in step.skip_if):
                    logger.info("Skipped step %s because skip_if matched", step.id)
                    state.steps[step.id] = StepRuntimeState(step_id=step.id, status=StepRuntimeStatus.SKIPPED)
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
                logger.info("Executing step %s (%s)", step.id, step.type)
                resolved_params = _resolve_step_params(state, step.params)
                outputs = execute_step(context, step, resolved_params)

                _emit_progress(
                    progress_callback,
                    step_index=step_index,
                    total_steps=total_steps,
                    step_id=step.id,
                    step_name=step.name,
                    phase=ProgressPhase.VERIFYING,
                )
                failed_verify = [condition.type for condition in step.verify if not evaluate_condition(self._adb, condition)]
                if failed_verify:
                    message = f"verify failed: {', '.join(failed_verify)}"
                    logger.error("Verification failed for step %s: %s", step.id, failed_verify)
                    state.steps[step.id] = StepRuntimeState(step_id=step.id, status=StepRuntimeStatus.FAILED, error=message)
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
                    continue

                state.steps[step.id] = StepRuntimeState(
                    step_id=step.id,
                    status=StepRuntimeStatus.SUCCEEDED,
                    outputs=dict(outputs),
                )
                records.append(StepRunRecord(step_id=step.id, status=StepRunStatus.EXECUTED, outputs=dict(outputs)))
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
                outputs = dict(getattr(exc, "outputs", {}))
                if isinstance(exc, RefResolutionError):
                    message = f"{exc.code.value}: {message}"
                elif isinstance(getattr(exc, "code", None), ErrorCode):
                    message = f"{exc.code.value}: {message}"
                state.steps[step.id] = StepRuntimeState(step_id=step.id, status=StepRuntimeStatus.FAILED, error=message)
                records.append(StepRunRecord(step_id=step.id, status=StepRunStatus.FAILED, message=message, outputs=outputs))
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

        success = not any(record.status in {StepRunStatus.FAILED, StepRunStatus.BLOCKED} for record in records)
        return ExecutionRunResult(
            success=success,
            total_steps=total_steps,
            steps=tuple(records),
        )


def _resolve_step_params(state: ExecutionState, params: Mapping[str, object]) -> dict[str, object]:
    resolved: dict[str, object] = {}
    for param_name, value in params.items():
        if isinstance(value, LiteralParamValue):
            resolved[param_name] = value.value
            continue
        if isinstance(value, RefParamValue):
            resolved[param_name] = resolve_runtime_ref(state, value.ref)
            continue
        resolved[param_name] = value
    return resolved


def _blocking_dependencies(state: ExecutionState, dependency_ids: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(
        dependency_id
        for dependency_id in dependency_ids
        if (step_state := state.steps.get(dependency_id)) is not None
        and step_state.status in {StepRuntimeStatus.FAILED, StepRuntimeStatus.BLOCKED}
    )


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
