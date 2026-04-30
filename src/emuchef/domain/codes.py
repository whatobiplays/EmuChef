"""Typed warning, error, and availability codes."""

from enum import Enum


class WarningCode(str, Enum):
    DEVICE_PROFILE_MISMATCH = "device_profile_mismatch"
    ORPHANED_PERMISSION_ACTIONS = "orphaned_permission_actions"
    OPTIONAL_STEPS_OMITTED_FOR_CAPABILITIES = "optional_steps_omitted_for_capabilities"
    VALIDATION_CONTEXT_LIMITED = "validation_context_limited"
    WARNING_UNKNOWN = "warning_unknown"


class ErrorCode(str, Enum):
    ADB_NOT_FOUND = "adb_not_found"
    APP_DEFINITION_NOT_FOUND = "app_definition_not_found"
    APP_DATA_WRITE_UNAVAILABLE = "app_data_write_unavailable"
    APP_ID_CONFLICT = "app_id_conflict"
    ARTIFACT_DOWNLOAD_FAILED = "artifact_download_failed"
    ARTIFACT_NOT_RESOLVED = "artifact_not_resolved"
    AUTHORED_DATA_INVALID = "authored_data_invalid"
    BINDING_MISSING = "binding_missing"
    BINDING_REF_CONFLICT = "binding_ref_conflict"
    BINDING_VALIDATION_FAILED = "binding_validation_failed"
    CONFLICT_UNRESOLVED = "conflict_unresolved"
    DEPENDENCY_CYCLE = "dependency_cycle"
    DEVICE_PLAN_NOT_FOUND = "device_plan_not_found"
    DEVICE_PLAN_ID_CONFLICT = "device_plan_id_conflict"
    DEVICE_PROFILE_NOT_FOUND = "device_profile_not_found"
    DEVICE_PROFILE_ID_CONFLICT = "device_profile_id_conflict"
    EMPTY_EXECUTION_PLAN = "empty_execution_plan"
    CAPABILITY_REDUCTION_FAILED = "capability_reduction_failed"
    INPUT_NOT_FOUND = "input_not_found"
    INVALID_REF_FORMAT = "invalid_ref_format"
    INVALID_OPERATION = "invalid_operation"
    PARAM_CONTRACT_VIOLATION = "param_contract_violation"
    RECIPE_ID_CONFLICT = "recipe_id_conflict"
    RECIPE_NOT_FOUND = "recipe_not_found"
    STEP_ID_CONFLICT = "step_id_conflict"
    STEP_NOT_FOUND = "step_not_found"
    STEP_NOT_TOGGLEABLE = "step_not_toggleable"
    STEP_OUTPUT_UNAVAILABLE = "step_output_unavailable"
    TLS_VERIFICATION_FAILED = "tls_verification_failed"
    UNKNOWN_ARTIFACT_FIELD = "unknown_artifact_field"
    UNKNOWN_ARTIFACT_REF = "unknown_artifact_ref"
    UNKNOWN_INPUT_REF = "unknown_input_ref"
    UNKNOWN_STEP_OUTPUT = "unknown_step_output"
    UNKNOWN_STEP_REF = "unknown_step_ref"
    ERROR_UNKNOWN = "error_unknown"


class AvailabilityCode(str, Enum):
    OPTIONAL_INPUT_UNBOUND = "optional_input_unbound"
    REQUIRED_CAPABILITY_MISSING = "required_capability_missing"
