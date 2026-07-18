//! Safe source facts and deterministic metadata consumed by app generators.
//!
//! APK inspection is owned by `apk_authoring_inspection`. This boundary accepts
//! only safe, persistable review facts and rejects malformed inspection
//! metadata before an authored document can be emitted.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const CHECKSUM_NOT_COMPARED: &str = "not_compared";
const SIGNATURE_NOT_PERFORMED: &str = "not_performed";
const DECLARATION_KINDS: &[&str] = &["uses_permission", "uses_permission_sdk_23"];
const CLASSIFICATIONS: &[&str] = &[
    "runtime_grantable",
    "runtime_restricted",
    "app_op_grantable",
    "manual_special_access",
    "install_time",
    "signature_or_privileged",
    "unknown",
];
const APPLICABILITY_STATUSES: &[&str] = &["applicable", "not_applicable", "indeterminate"];
const APPLICABILITY_REASONS: &[&str] = &[
    "declaration_requires_api_23",
    "max_sdk_version_exceeded",
    "permission_not_introduced",
    "permission_replaced",
    "target_sdk_below_minimum",
    "invalid_max_sdk_version",
    "target_sdk_unavailable",
    "replacement_target_sdk_unavailable",
];
const TARGET_SDK_STATES: &[&str] = &["missing", "non_numeric"];

/// Safe facts used to propose authored app and recipe fields.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ApkInspectionFacts {
    pub package_name: Option<String>,
    pub application_label: Option<String>,
    pub version_code: Option<String>,
    pub version_name: Option<String>,
    pub min_sdk: Option<i64>,
    pub target_sdk: Option<i64>,
    pub abis: Vec<String>,
    pub launcher_activities: Vec<String>,
    pub requested_permissions: Vec<ApkPermissionDeclarationFacts>,
    pub calculated_sha256: String,
    pub checksum_status: String,
    pub signature_verification: String,
    pub debuggable: Option<bool>,
    pub split: Option<bool>,
    pub base: Option<bool>,
}

/// One reviewed manifest permission declaration safe for authored metadata.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ApkPermissionDeclarationFacts {
    pub name: String,
    pub declaration_kind: String,
    pub max_sdk_version: Option<String>,
    pub classification: Option<String>,
    pub applicability: Option<ApkPermissionApplicabilityFacts>,
}

/// Reviewed applicability facts and the Android bounds that justified them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ApkPermissionApplicabilityFacts {
    pub status: String,
    pub reason: Option<String>,
    pub maximum_sdk_version: Option<i64>,
    pub introduction_api: Option<i64>,
    pub minimum_device_api: Option<i64>,
    pub minimum_target_sdk: Option<i64>,
    pub target_sdk_state: Option<String>,
}

/// Canonical selected runtime-permission metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SelectedRuntimePermissionMetadata {
    pub permission_name: String,
    pub requires_root: bool,
}

/// Canonical selected app-op metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SelectedAppOpMetadata {
    pub permission_name: String,
    pub operation_name: String,
    pub mode: String,
    pub requires_root: bool,
}

/// One stable validation failure for generator-owned inspection metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApkMetadataIssue {
    pub code: &'static str,
    pub message: &'static str,
    pub field: String,
}

/// Validate trusted inspection facts and build the fixed authored metadata map.
pub(crate) fn build_apk_inspection_metadata(
    facts: &ApkInspectionFacts,
    runtime_permissions: &[SelectedRuntimePermissionMetadata],
    app_ops: &[SelectedAppOpMetadata],
) -> Result<Value, Vec<ApkMetadataIssue>> {
    let mut issues = validate_inspection_facts(facts);
    if !issues.is_empty() {
        issues.sort_by(|left, right| {
            left.code
                .cmp(right.code)
                .then_with(|| left.field.cmp(&right.field))
        });
        issues.dedup();
        return Err(issues);
    }

    let mut permissions = facts.requested_permissions.clone();
    permissions.sort();
    permissions.dedup();

    let mut result = Map::new();
    result.insert(
        "package_name".to_string(),
        option_string(&facts.package_name),
    );
    result.insert(
        "version_code".to_string(),
        option_string(&facts.version_code),
    );
    result.insert(
        "version_name".to_string(),
        option_string(&facts.version_name),
    );
    result.insert("min_sdk".to_string(), option_i64(facts.min_sdk));
    result.insert("target_sdk".to_string(), option_i64(facts.target_sdk));
    result.insert(
        "calculated_sha256".to_string(),
        Value::String(facts.calculated_sha256.to_ascii_uppercase()),
    );
    result.insert(
        "checksum_status".to_string(),
        Value::String(CHECKSUM_NOT_COMPARED.to_string()),
    );
    result.insert(
        "signature_verification".to_string(),
        Value::String(SIGNATURE_NOT_PERFORMED.to_string()),
    );
    result.insert(
        "requested_permissions".to_string(),
        Value::Array(permissions.iter().map(permission_value).collect()),
    );
    result.insert(
        "selected_runtime_permissions".to_string(),
        Value::Array(
            runtime_permissions
                .iter()
                .map(|permission| {
                    ordered_object([
                        (
                            "permission_name",
                            Value::String(permission.permission_name.clone()),
                        ),
                        ("requires_root", Value::Bool(permission.requires_root)),
                    ])
                })
                .collect(),
        ),
    );
    result.insert(
        "selected_app_ops".to_string(),
        Value::Array(
            app_ops
                .iter()
                .map(|action| {
                    ordered_object([
                        (
                            "permission_name",
                            Value::String(action.permission_name.clone()),
                        ),
                        (
                            "operation_name",
                            Value::String(action.operation_name.clone()),
                        ),
                        ("mode", Value::String(action.mode.clone())),
                        ("requires_root", Value::Bool(action.requires_root)),
                    ])
                })
                .collect(),
        ),
    );
    Ok(Value::Object(result))
}

fn validate_inspection_facts(facts: &ApkInspectionFacts) -> Vec<ApkMetadataIssue> {
    let mut issues = Vec::new();
    if facts.calculated_sha256.len() != 64
        || !facts
            .calculated_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        issues.push(issue(
            "apk_inspection_metadata_sha256_invalid",
            "Calculated APK SHA-256 must contain exactly 64 hexadecimal characters.",
            "metadata.apk_inspection.calculated_sha256",
        ));
    }
    if facts.checksum_status != CHECKSUM_NOT_COMPARED {
        issues.push(issue(
            "apk_inspection_metadata_checksum_status_invalid",
            "APK inspection checksum status must be 'not_compared'.",
            "metadata.apk_inspection.checksum_status",
        ));
    }
    if facts.signature_verification != SIGNATURE_NOT_PERFORMED {
        issues.push(issue(
            "apk_inspection_metadata_signature_verification_invalid",
            "APK signature verification state must be 'not_performed'.",
            "metadata.apk_inspection.signature_verification",
        ));
    }
    if facts.min_sdk.is_some_and(|value| value < 0)
        || facts.target_sdk.is_some_and(|value| value < 0)
        || facts
            .min_sdk
            .zip(facts.target_sdk)
            .is_some_and(|(minimum, target)| minimum > target)
    {
        issues.push(issue(
            "apk_inspection_metadata_sdk_bounds_invalid",
            "APK minimum and target SDK values must be non-negative and internally consistent.",
            "metadata.apk_inspection",
        ));
    }
    for (index, permission) in facts.requested_permissions.iter().enumerate() {
        validate_permission(permission, index, &mut issues);
    }
    issues
}

fn validate_permission(
    permission: &ApkPermissionDeclarationFacts,
    index: usize,
    issues: &mut Vec<ApkMetadataIssue>,
) {
    let field = format!("metadata.apk_inspection.requested_permissions[{index}]");
    if permission.name.trim().is_empty()
        || !DECLARATION_KINDS.contains(&permission.declaration_kind.as_str())
    {
        issues.push(issue(
            "apk_inspection_metadata_permission_invalid",
            "APK permission declarations require a name and supported declaration kind.",
            &field,
        ));
    }
    if permission
        .classification
        .as_deref()
        .is_some_and(|value| !CLASSIFICATIONS.contains(&value))
    {
        issues.push(issue(
            "apk_inspection_metadata_permission_classification_invalid",
            "APK permission classification must use a supported value.",
            &format!("{field}.classification"),
        ));
    }
    if permission.classification.is_some() != permission.applicability.is_some() {
        issues.push(issue(
            "apk_inspection_metadata_permission_context_invalid",
            "APK permission classification and applicability must both be present or both be unavailable.",
            &field,
        ));
    }
    if let Some(applicability) = &permission.applicability {
        validate_applicability(applicability, &format!("{field}.applicability"), issues);
    }
}

fn validate_applicability(
    value: &ApkPermissionApplicabilityFacts,
    field: &str,
    issues: &mut Vec<ApkMetadataIssue>,
) {
    if !APPLICABILITY_STATUSES.contains(&value.status.as_str())
        || value
            .reason
            .as_deref()
            .is_some_and(|reason| !APPLICABILITY_REASONS.contains(&reason))
        || value
            .target_sdk_state
            .as_deref()
            .is_some_and(|state| !TARGET_SDK_STATES.contains(&state))
    {
        issues.push(issue(
            "apk_inspection_metadata_applicability_invalid",
            "APK permission applicability must use supported status, reason, and target-SDK-state values.",
            field,
        ));
    }
    if (value.status == "applicable") != value.reason.is_none()
        || (value.status == "indeterminate")
            != value.reason.as_deref().is_some_and(|reason| {
                matches!(
                    reason,
                    "invalid_max_sdk_version"
                        | "target_sdk_unavailable"
                        | "replacement_target_sdk_unavailable"
                )
            })
    {
        issues.push(issue(
            "apk_inspection_metadata_applicability_state_invalid",
            "APK permission applicability status and reason are inconsistent.",
            field,
        ));
    }
    let bounds = [
        value.maximum_sdk_version,
        value.introduction_api,
        value.minimum_device_api,
        value.minimum_target_sdk,
    ];
    if bounds.into_iter().flatten().any(|bound| bound < 0)
        || value
            .maximum_sdk_version
            .zip(value.introduction_api)
            .is_some_and(|(maximum, introduction)| maximum < introduction)
        || value
            .maximum_sdk_version
            .zip(value.minimum_device_api)
            .is_some_and(|(maximum, minimum)| maximum < minimum)
    {
        issues.push(issue(
            "apk_inspection_metadata_api_bounds_invalid",
            "APK permission applicability API bounds must be non-negative and internally consistent.",
            field,
        ));
    }
    let target_state_required = matches!(
        value.reason.as_deref(),
        Some("target_sdk_unavailable" | "replacement_target_sdk_unavailable")
    );
    if target_state_required != value.target_sdk_state.is_some() {
        issues.push(issue(
            "apk_inspection_metadata_target_sdk_state_invalid",
            "APK permission target-SDK state must match its applicability reason.",
            field,
        ));
    }
}

fn permission_value(permission: &ApkPermissionDeclarationFacts) -> Value {
    ordered_object([
        ("name", Value::String(permission.name.clone())),
        (
            "declaration_kind",
            Value::String(permission.declaration_kind.clone()),
        ),
        (
            "max_sdk_version",
            option_string(&permission.max_sdk_version),
        ),
        ("classification", option_string(&permission.classification)),
        (
            "applicability",
            permission
                .applicability
                .as_ref()
                .map(applicability_value)
                .unwrap_or(Value::Null),
        ),
    ])
}

fn applicability_value(value: &ApkPermissionApplicabilityFacts) -> Value {
    ordered_object([
        ("status", Value::String(value.status.clone())),
        ("reason", option_string(&value.reason)),
        ("maximum_sdk_version", option_i64(value.maximum_sdk_version)),
        ("introduction_api", option_i64(value.introduction_api)),
        ("minimum_device_api", option_i64(value.minimum_device_api)),
        ("minimum_target_sdk", option_i64(value.minimum_target_sdk)),
        ("target_sdk_state", option_string(&value.target_sdk_state)),
    ])
}

fn ordered_object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn option_string(value: &Option<String>) -> Value {
    value.clone().map(Value::String).unwrap_or(Value::Null)
}

fn option_i64(value: Option<i64>) -> Value {
    value.map(Value::from).unwrap_or(Value::Null)
}

fn issue(code: &'static str, message: &'static str, field: &str) -> ApkMetadataIssue {
    ApkMetadataIssue {
        code,
        message,
        field: field.to_string(),
    }
}
