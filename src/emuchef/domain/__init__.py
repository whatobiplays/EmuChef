"""Domain models for EmuChef."""

from .artifacts import ArtifactCacheMode, ArtifactDefinition, ArtifactType, RemoteFileArtifact
from .app_definition import (
    AppArtifacts,
    AppArtifactSupport,
    AppConfigTarget,
    AppDefinition,
    AppInstallSource,
    AppPackage,
    AppProvisioning,
    AppTrackingSource,
)
from .codes import AvailabilityCode, ErrorCode, WarningCode
from .copy_policy import CopyPolicy
from .device_context import DeviceContext
from .device_plan import DevicePlan, DevicePlanRecipeSelection
from .device_profiles import (
    AndroidVersionRange,
    DeviceMatchCriteria,
    DeviceProfile,
    RuntimeCapabilities,
)
from .draft_changes import DraftPlanChanges
from .draft_plan import (
    Availability,
    DraftInputState,
    DraftPlan,
    DraftPlanSource,
    DraftRecipeState,
    DraftStepState,
)
from .draft_update_result import DraftUpdateResult
from .execution_plan import (
    ExecutionArtifact,
    ExecutionInputValue,
    ExecutionPermissionPlan,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    PermissionPlanAction,
    PermissionPlanReason,
    PermissionPlanSource,
)
from .history_entry import HistoryEntry
from .input_declaration import InputDeclaration, InputRole, InputType, InputValidation
from .issues import AvailabilityReason, ErrorMessage, WarningMessage
from .param_values import (
    AuthoredParamValue,
    BoundParamValue,
    JSONValue,
    LiteralParamValue,
    ParamValue,
    RefParamValue,
    ScalarValue,
)
from .planning_result import PlanningResult, PlanningStatus
from .recipe import (
    AppOpGrant,
    ManualPermissionRequirement,
    PermissionPolicy,
    PermissionSet,
    PermissionWhen,
    Recipe,
    RecipeProvides,
    RuntimePermissionGrant,
)
from .refs import RefKind, RuntimeRef, parse_reference
from .runtime_state import (
    ArtifactRuntimeState,
    ArtifactRuntimeStatus,
    ExecutionState,
    RuntimeValue,
    RuntimeValueType,
    StepRuntimeState,
    StepRuntimeStatus,
)
from .step_specs import PRIMARY_OUTPUT_STEP_TYPES, ParamMode, ParamSpec, STEP_SPECS, StepSpec
from .step import Step, StepCondition, StepConstraints
from .step_types import StepType
from .validation_result import ValidationResult, ValidationStatus

__all__ = [
    "AndroidVersionRange",
    "ArtifactCacheMode",
    "ArtifactDefinition",
    "ArtifactRuntimeState",
    "ArtifactRuntimeStatus",
    "ArtifactType",
    "AppArtifacts",
    "AppArtifactSupport",
    "AppConfigTarget",
    "AppDefinition",
    "AppInstallSource",
    "AppPackage",
    "AppProvisioning",
    "AppTrackingSource",
    "AuthoredParamValue",
    "Availability",
    "AvailabilityCode",
    "AvailabilityReason",
    "BoundParamValue",
    "CopyPolicy",
    "DeviceContext",
    "DeviceMatchCriteria",
    "DevicePlan",
    "DevicePlanRecipeSelection",
    "DeviceProfile",
    "DraftPlanChanges",
    "DraftInputState",
    "DraftPlan",
    "DraftPlanSource",
    "DraftRecipeState",
    "DraftStepState",
    "DraftUpdateResult",
    "ErrorCode",
    "ErrorMessage",
    "ExecutionArtifact",
    "ExecutionInputValue",
    "ExecutionState",
    "ExecutionPermissionPlan",
    "ExecutionPlan",
    "ExecutionPlanSource",
    "ExecutionStep",
    "HistoryEntry",
    "InputDeclaration",
    "InputRole",
    "InputType",
    "InputValidation",
    "JSONValue",
    "LiteralParamValue",
    "ParamMode",
    "ParamValue",
    "ParamSpec",
    "PRIMARY_OUTPUT_STEP_TYPES",
    "PermissionPlanAction",
    "PermissionPlanReason",
    "PermissionPlanSource",
    "PermissionPolicy",
    "PermissionSet",
    "PermissionWhen",
    "PlanningResult",
    "PlanningStatus",
    "RefKind",
    "RefParamValue",
    "AppOpGrant",
    "ManualPermissionRequirement",
    "Recipe",
    "RecipeProvides",
    "RemoteFileArtifact",
    "RuntimeRef",
    "RuntimePermissionGrant",
    "RuntimeCapabilities",
    "RuntimeValue",
    "RuntimeValueType",
    "ScalarValue",
    "Step",
    "StepCondition",
    "StepConstraints",
    "STEP_SPECS",
    "StepRuntimeState",
    "StepRuntimeStatus",
    "StepSpec",
    "StepType",
    "WarningCode",
    "WarningMessage",
    "ValidationResult",
    "ValidationStatus",
    "parse_reference",
]
