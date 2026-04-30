"""Central execution-time ref resolver."""

from __future__ import annotations

from emuchef.domain import (
    ArtifactRuntimeStatus,
    ErrorCode,
    ExecutionState,
    RefKind,
    RuntimeValue,
    RuntimeValueType,
    StepRuntimeStatus,
    parse_reference,
)


class RefResolutionError(ValueError):
    def __init__(self, code: ErrorCode, message: str) -> None:
        super().__init__(message)
        self.code = code


def resolve_runtime_ref(state: ExecutionState, ref: str) -> RuntimeValue:
    try:
        parsed = parse_reference(ref)
    except ValueError as exc:
        raise RefResolutionError(ErrorCode.INVALID_REF_FORMAT, f"Invalid runtime ref: {ref!r}.") from exc

    if parsed.kind is RefKind.INPUT:
        value = state.inputs.get(parsed.target_id)
        if value is None:
            raise RefResolutionError(ErrorCode.UNKNOWN_INPUT_REF, f"Unknown input ref: {ref!r}.")
        return value

    if parsed.kind is RefKind.ARTIFACT_FIELD:
        artifact = state.artifacts.get(parsed.target_id)
        if artifact is None:
            raise RefResolutionError(ErrorCode.UNKNOWN_ARTIFACT_REF, f"Unknown artifact ref: {ref!r}.")
        if artifact.status is not ArtifactRuntimeStatus.RESOLVED:
            raise RefResolutionError(ErrorCode.ARTIFACT_NOT_RESOLVED, f"Artifact is not resolved: {ref!r}.")
        if parsed.field == "status":
            return RuntimeValue(type=RuntimeValueType.STRING, value=artifact.status.value)
        if parsed.field == "local_path":
            return RuntimeValue(type=RuntimeValueType.FILE_PATH, value=artifact.local_path, location="host")
        if parsed.field == "resolved_url":
            return RuntimeValue(type=RuntimeValueType.STRING, value=artifact.resolved_url)
        if parsed.field == "filename":
            return RuntimeValue(type=RuntimeValueType.STRING, value=artifact.filename)
        if parsed.field == "cache_hit":
            return RuntimeValue(type=RuntimeValueType.BOOLEAN, value=artifact.cache_hit)
        if parsed.field == "error":
            return RuntimeValue(type=RuntimeValueType.STRING, value=artifact.error or "")
        raise RefResolutionError(ErrorCode.UNKNOWN_ARTIFACT_FIELD, f"Unknown artifact field in ref: {ref!r}.")

    if parsed.kind is RefKind.STEP_SHORTHAND:
        raise RefResolutionError(ErrorCode.INVALID_REF_FORMAT, f"Step shorthand may not appear at execution time: {ref!r}.")

    step_state = state.steps.get(parsed.target_id)
    if step_state is None:
        raise RefResolutionError(ErrorCode.UNKNOWN_STEP_REF, f"Unknown step ref: {ref!r}.")
    if step_state.status is not StepRuntimeStatus.SUCCEEDED:
        raise RefResolutionError(
            ErrorCode.STEP_OUTPUT_UNAVAILABLE,
            f"Step output is unavailable because step {parsed.target_id!r} did not succeed.",
        )
    value = step_state.outputs.get(parsed.field or "")
    if value is None:
        raise RefResolutionError(ErrorCode.UNKNOWN_STEP_OUTPUT, f"Unknown step output in ref: {ref!r}.")
    return value
