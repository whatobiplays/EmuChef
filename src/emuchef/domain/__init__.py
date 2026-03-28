"""Domain models for EmuChef."""

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
    ExecutionPermissionPlan,
    ExecutionPlan,
    ExecutionPlanSource,
    ExecutionStep,
    PermissionPlanAction,
    PermissionPlanReason,
    PermissionPlanSource,
    ResolvedInputValue,
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
from .refs import Reference, parse_reference
from .step import Step, StepCondition, StepConstraints
from .step_types import StepType
from .validation_result import ValidationResult, ValidationStatus

__all__ = [
    "AndroidVersionRange",
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
    "ParamValue",
    "PermissionPlanAction",
    "PermissionPlanReason",
    "PermissionPlanSource",
    "PermissionPolicy",
    "PermissionSet",
    "PermissionWhen",
    "PlanningResult",
    "PlanningStatus",
    "AppOpGrant",
    "ManualPermissionRequirement",
    "Recipe",
    "RecipeProvides",
    "Reference",
    "ResolvedInputValue",
    "RuntimePermissionGrant",
    "RuntimeCapabilities",
    "ScalarValue",
    "Step",
    "StepCondition",
    "StepConstraints",
    "StepType",
    "WarningCode",
    "WarningMessage",
    "ValidationResult",
    "ValidationStatus",
    "parse_reference",
]
