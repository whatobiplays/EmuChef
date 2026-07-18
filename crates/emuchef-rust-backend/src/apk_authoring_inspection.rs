//! Native authoring-time APK inspection and review-only permission actions.
//!
//! This boundary accepts only a selected APK path and optional Android device
//! API level. It returns manifest-owned metadata, a calculated file hash, and
//! review DTOs. It never verifies signatures, constructs commands, or mutates
//! recipes and devices.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::android_permissions::{
    classify_android_permissions, AndroidAppOpMode, AndroidPermissionApplicability,
    AndroidPermissionAutomation, AndroidPermissionClassification, AndroidPermissionIndeterminacy,
    AndroidPermissionNonApplicability, ClassifiedAndroidPermission, TargetSdkState,
};
use crate::apk_manifest::{
    inspect_apk_manifest, ApkManifestError, ApkManifestFacts, ApkPermissionDeclaration,
    ApkPermissionDeclarationKind,
};

const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// Strict protocol input for native APK inspection.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ApkAuthoringInspectionRequest {
    pub(crate) apk_path: String,
    #[serde(default)]
    pub(crate) connected_device_api: Option<u32>,
}

impl ApkAuthoringInspectionRequest {
    /// Reject empty paths and Android's invalid API level zero.
    pub(crate) fn is_valid(&self) -> bool {
        !self.apk_path.trim().is_empty() && self.connected_device_api != Some(0)
    }
}

/// Stable redacted failures from native APK authoring inspection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApkAuthoringInspectionError {
    Manifest(ApkManifestError),
    FileReadFailed,
}

impl ApkAuthoringInspectionError {
    /// Return the stable machine-readable failure reason.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Manifest(error) => error.code(),
            Self::FileReadFailed => "apk_file_read_failed",
        }
    }

    /// Return a fixed public message that contains no source path or parser detail.
    pub(crate) fn message(self) -> String {
        match self {
            Self::Manifest(error) => error.to_string(),
            Self::FileReadFailed => "APK file could not be read.".to_string(),
        }
    }
}

impl From<ApkManifestError> for ApkAuthoringInspectionError {
    fn from(error: ApkManifestError) -> Self {
        Self::Manifest(error)
    }
}

/// Safe manifest metadata extracted from the APK.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApkManifestMetadataDto {
    package_name: String,
    version_code: Option<String>,
    version_name: Option<String>,
    min_sdk_version: Option<String>,
    target_sdk_version: Option<String>,
}

/// The manifest element that declared a requested permission.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkPermissionDeclarationKindDto {
    UsesPermission,
    UsesPermissionSdk23,
}

/// Stable permission categories exposed for author review.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkPermissionClassificationDto {
    RuntimeGrantable,
    RuntimeRestricted,
    AppOpGrantable,
    ManualSpecialAccess,
    InstallTime,
    SignatureOrPrivileged,
    Unknown,
}

/// Whether a permission declaration applies to the selected device context.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkPermissionApplicabilityStatusDto {
    Applicable,
    NotApplicable,
    Indeterminate,
}

/// Stable reasons for non-applicable and indeterminate declarations.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkPermissionApplicabilityReasonDto {
    DeclarationRequiresApi23,
    MaxSdkVersionExceeded,
    PermissionNotIntroduced,
    PermissionReplaced,
    TargetSdkBelowMinimum,
    InvalidMaxSdkVersion,
    TargetSdkUnavailable,
    ReplacementTargetSdkUnavailable,
}

/// Why required target-SDK context could not be interpreted.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkTargetSdkStateDto {
    Missing,
    NonNumeric,
}

/// Structured applicability plus the reviewed Android API thresholds involved.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApkPermissionApplicabilityDto {
    status: ApkPermissionApplicabilityStatusDto,
    reason: Option<ApkPermissionApplicabilityReasonDto>,
    maximum_sdk_version: Option<u32>,
    introduction_api: Option<u32>,
    minimum_device_api: Option<u32>,
    minimum_target_sdk: Option<u32>,
    target_sdk_state: Option<ApkTargetSdkStateDto>,
}

/// One complete manifest declaration with optional device-context decisions.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApkPermissionReviewDto {
    name: String,
    declaration_kind: ApkPermissionDeclarationKindDto,
    max_sdk_version: Option<String>,
    classification: Option<ApkPermissionClassificationDto>,
    applicability: Option<ApkPermissionApplicabilityDto>,
}

/// Review-only candidate for a future Android runtime permission grant.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApkRuntimeGrantCandidateDto {
    permission_name: String,
    requires_root: bool,
    selected: bool,
}

/// App-op modes reviewed by the backend permission catalog.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkAppOpModeDto {
    Allow,
}

/// Review-only candidate for a future explicit app-op action.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApkAppOpCandidateDto {
    permission_name: String,
    operation_name: String,
    mode: ApkAppOpModeDto,
    requires_root: bool,
    selected: bool,
}

/// A stable warning about unavailable classification or non-automated access.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApkPermissionWarningDto {
    code: &'static str,
    message: &'static str,
    permission_name: Option<String>,
    applicability_reason: Option<ApkPermissionApplicabilityReasonDto>,
}

/// A calculated hash that has not been compared with publisher evidence.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkChecksumStatusDto {
    NotCompared,
}

/// EmuChef does not perform APK signature verification in this inspection.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkSignatureVerificationDto {
    NotPerformed,
}

/// Native, backend-owned APK authoring inspection result.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApkAuthoringInspectionResult {
    manifest: ApkManifestMetadataDto,
    permissions: Vec<ApkPermissionReviewDto>,
    runtime_grant_candidates: Vec<ApkRuntimeGrantCandidateDto>,
    app_op_candidates: Vec<ApkAppOpCandidateDto>,
    warnings: Vec<ApkPermissionWarningDto>,
    calculated_sha256: String,
    checksum_status: ApkChecksumStatusDto,
    signature_verification: ApkSignatureVerificationDto,
}

/// Inspect a selected APK and build review-only authoring metadata.
pub(crate) fn inspect_apk_for_authoring(
    apk_path: &Path,
    connected_device_api: Option<u32>,
) -> Result<ApkAuthoringInspectionResult, ApkAuthoringInspectionError> {
    let facts = inspect_apk_manifest(apk_path)?;
    let calculated_sha256 = calculate_sha256(apk_path)?;
    Ok(build_result(facts, connected_device_api, calculated_sha256))
}

fn calculate_sha256(path: &Path) -> Result<String, ApkAuthoringInspectionError> {
    let file = File::open(path).map_err(|_| ApkAuthoringInspectionError::FileReadFailed)?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| ApkAuthoringInspectionError::FileReadFailed)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect())
}

fn build_result(
    facts: ApkManifestFacts,
    connected_device_api: Option<u32>,
    calculated_sha256: String,
) -> ApkAuthoringInspectionResult {
    let manifest = ApkManifestMetadataDto {
        package_name: facts.package_name,
        version_code: facts.version_code,
        version_name: facts.version_name,
        min_sdk_version: facts.min_sdk_version,
        target_sdk_version: facts.target_sdk_version,
    };
    let (permissions, runtime_grant_candidates, app_op_candidates, warnings) =
        match connected_device_api {
            Some(device_api) => classified_permission_review(
                &facts.permissions,
                manifest.target_sdk_version.as_deref(),
                device_api,
            ),
            None => (
                facts
                    .permissions
                    .iter()
                    .map(permission_without_context)
                    .collect(),
                Vec::new(),
                Vec::new(),
                vec![ApkPermissionWarningDto {
                    code: "apk_permission_classification_context_unavailable",
                    message: "Permission classification requires a connected-device API level.",
                    permission_name: None,
                    applicability_reason: None,
                }],
            ),
        };

    ApkAuthoringInspectionResult {
        manifest,
        permissions,
        runtime_grant_candidates,
        app_op_candidates,
        warnings,
        calculated_sha256,
        checksum_status: ApkChecksumStatusDto::NotCompared,
        signature_verification: ApkSignatureVerificationDto::NotPerformed,
    }
}

type PermissionReviewParts = (
    Vec<ApkPermissionReviewDto>,
    Vec<ApkRuntimeGrantCandidateDto>,
    Vec<ApkAppOpCandidateDto>,
    Vec<ApkPermissionWarningDto>,
);

fn classified_permission_review(
    declarations: &[ApkPermissionDeclaration],
    target_sdk: Option<&str>,
    device_api: u32,
) -> PermissionReviewParts {
    let classified = classify_android_permissions(declarations, target_sdk, device_api);
    let mut permissions = Vec::with_capacity(classified.len());
    let mut runtime_candidates = Vec::new();
    let mut app_op_candidates = Vec::new();
    let mut warnings = Vec::new();

    for permission in classified {
        if let Some(warning) = warning_for_permission(&permission) {
            warnings.push(warning);
        }
        match (permission.classification, permission.automation) {
            (
                AndroidPermissionClassification::RuntimeGrantable,
                Some(AndroidPermissionAutomation::RuntimeGrant { requires_root }),
            ) if permission.applicability == AndroidPermissionApplicability::Applicable => {
                runtime_candidates.push(ApkRuntimeGrantCandidateDto {
                    permission_name: permission.declaration.name.clone(),
                    requires_root,
                    selected: false,
                });
            }
            (
                AndroidPermissionClassification::AppOpGrantable,
                Some(AndroidPermissionAutomation::AppOp {
                    app_op,
                    mode,
                    requires_root,
                }),
            ) if permission.applicability == AndroidPermissionApplicability::Applicable => {
                app_op_candidates.push(ApkAppOpCandidateDto {
                    permission_name: permission.declaration.name.clone(),
                    operation_name: app_op.to_string(),
                    mode: app_op_mode(mode),
                    requires_root,
                    selected: false,
                });
            }
            _ => {}
        }
        permissions.push(permission_with_context(permission));
    }

    (permissions, runtime_candidates, app_op_candidates, warnings)
}

fn permission_without_context(declaration: &ApkPermissionDeclaration) -> ApkPermissionReviewDto {
    ApkPermissionReviewDto {
        name: declaration.name.clone(),
        declaration_kind: declaration_kind(declaration.kind),
        max_sdk_version: declaration.max_sdk_version.clone(),
        classification: None,
        applicability: None,
    }
}

fn permission_with_context(permission: ClassifiedAndroidPermission) -> ApkPermissionReviewDto {
    ApkPermissionReviewDto {
        name: permission.declaration.name,
        declaration_kind: declaration_kind(permission.declaration.kind),
        max_sdk_version: permission.declaration.max_sdk_version,
        classification: Some(classification(permission.classification)),
        applicability: Some(applicability(permission.applicability)),
    }
}

fn declaration_kind(kind: ApkPermissionDeclarationKind) -> ApkPermissionDeclarationKindDto {
    match kind {
        ApkPermissionDeclarationKind::UsesPermission => {
            ApkPermissionDeclarationKindDto::UsesPermission
        }
        ApkPermissionDeclarationKind::UsesPermissionSdk23 => {
            ApkPermissionDeclarationKindDto::UsesPermissionSdk23
        }
    }
}

fn classification(value: AndroidPermissionClassification) -> ApkPermissionClassificationDto {
    match value {
        AndroidPermissionClassification::RuntimeGrantable => {
            ApkPermissionClassificationDto::RuntimeGrantable
        }
        AndroidPermissionClassification::RuntimeRestricted => {
            ApkPermissionClassificationDto::RuntimeRestricted
        }
        AndroidPermissionClassification::AppOpGrantable => {
            ApkPermissionClassificationDto::AppOpGrantable
        }
        AndroidPermissionClassification::ManualSpecialAccess => {
            ApkPermissionClassificationDto::ManualSpecialAccess
        }
        AndroidPermissionClassification::InstallTime => ApkPermissionClassificationDto::InstallTime,
        AndroidPermissionClassification::SignatureOrPrivileged => {
            ApkPermissionClassificationDto::SignatureOrPrivileged
        }
        AndroidPermissionClassification::Unknown => ApkPermissionClassificationDto::Unknown,
    }
}

fn applicability(value: AndroidPermissionApplicability) -> ApkPermissionApplicabilityDto {
    let mut result = ApkPermissionApplicabilityDto {
        status: ApkPermissionApplicabilityStatusDto::Applicable,
        reason: None,
        maximum_sdk_version: None,
        introduction_api: None,
        minimum_device_api: None,
        minimum_target_sdk: None,
        target_sdk_state: None,
    };
    match value {
        AndroidPermissionApplicability::Applicable => result,
        AndroidPermissionApplicability::NotApplicable(reason) => {
            result.status = ApkPermissionApplicabilityStatusDto::NotApplicable;
            apply_non_applicable_reason(&mut result, reason);
            result
        }
        AndroidPermissionApplicability::Indeterminate(reason) => {
            result.status = ApkPermissionApplicabilityStatusDto::Indeterminate;
            apply_indeterminate_reason(&mut result, reason);
            result
        }
    }
}

fn apply_non_applicable_reason(
    result: &mut ApkPermissionApplicabilityDto,
    reason: AndroidPermissionNonApplicability,
) {
    match reason {
        AndroidPermissionNonApplicability::DeclarationRequiresApi23 => {
            result.reason = Some(ApkPermissionApplicabilityReasonDto::DeclarationRequiresApi23);
        }
        AndroidPermissionNonApplicability::MaxSdkVersionExceeded { maximum } => {
            result.reason = Some(ApkPermissionApplicabilityReasonDto::MaxSdkVersionExceeded);
            result.maximum_sdk_version = Some(maximum);
        }
        AndroidPermissionNonApplicability::PermissionNotIntroduced { introduction_api } => {
            result.reason = Some(ApkPermissionApplicabilityReasonDto::PermissionNotIntroduced);
            result.introduction_api = Some(introduction_api);
        }
        AndroidPermissionNonApplicability::PermissionReplaced {
            minimum_device_api,
            minimum_target_sdk,
        } => {
            result.reason = Some(ApkPermissionApplicabilityReasonDto::PermissionReplaced);
            result.minimum_device_api = Some(minimum_device_api);
            result.minimum_target_sdk = Some(minimum_target_sdk);
        }
        AndroidPermissionNonApplicability::TargetSdkBelowMinimum { minimum_target_sdk } => {
            result.reason = Some(ApkPermissionApplicabilityReasonDto::TargetSdkBelowMinimum);
            result.minimum_target_sdk = Some(minimum_target_sdk);
        }
    }
}

fn apply_indeterminate_reason(
    result: &mut ApkPermissionApplicabilityDto,
    reason: AndroidPermissionIndeterminacy,
) {
    match reason {
        AndroidPermissionIndeterminacy::InvalidMaxSdkVersion => {
            result.reason = Some(ApkPermissionApplicabilityReasonDto::InvalidMaxSdkVersion);
        }
        AndroidPermissionIndeterminacy::TargetSdkUnavailable {
            minimum_target_sdk,
            state,
        } => {
            result.reason = Some(ApkPermissionApplicabilityReasonDto::TargetSdkUnavailable);
            result.minimum_target_sdk = Some(minimum_target_sdk);
            result.target_sdk_state = Some(target_sdk_state(state));
        }
        AndroidPermissionIndeterminacy::ReplacementTargetSdkUnavailable {
            minimum_device_api,
            minimum_target_sdk,
            state,
        } => {
            result.reason =
                Some(ApkPermissionApplicabilityReasonDto::ReplacementTargetSdkUnavailable);
            result.minimum_device_api = Some(minimum_device_api);
            result.minimum_target_sdk = Some(minimum_target_sdk);
            result.target_sdk_state = Some(target_sdk_state(state));
        }
    }
}

fn target_sdk_state(state: TargetSdkState) -> ApkTargetSdkStateDto {
    match state {
        TargetSdkState::Missing => ApkTargetSdkStateDto::Missing,
        TargetSdkState::NonNumeric => ApkTargetSdkStateDto::NonNumeric,
    }
}

fn app_op_mode(mode: AndroidAppOpMode) -> ApkAppOpModeDto {
    match mode {
        AndroidAppOpMode::Allow => ApkAppOpModeDto::Allow,
    }
}

fn warning_for_permission(
    permission: &ClassifiedAndroidPermission,
) -> Option<ApkPermissionWarningDto> {
    let (code, message, applicability_reason) = match permission.applicability {
        AndroidPermissionApplicability::NotApplicable(reason) => (
            "apk_permission_not_applicable",
            "This permission does not apply to the selected Android context.",
            Some(non_applicable_reason(reason)),
        ),
        AndroidPermissionApplicability::Indeterminate(reason) => (
            "apk_permission_applicability_indeterminate",
            "This permission's applicability could not be determined safely.",
            Some(indeterminate_reason(reason)),
        ),
        AndroidPermissionApplicability::Applicable => match permission.classification {
            AndroidPermissionClassification::RuntimeRestricted => (
                "apk_permission_runtime_restricted",
                "This restricted runtime permission is not offered for automation.",
                None,
            ),
            AndroidPermissionClassification::ManualSpecialAccess => (
                "apk_permission_manual_special_access",
                "This special access must be reviewed and enabled manually.",
                None,
            ),
            AndroidPermissionClassification::SignatureOrPrivileged => (
                "apk_permission_signature_or_privileged",
                "This signature or privileged permission is not offered for automation.",
                None,
            ),
            AndroidPermissionClassification::Unknown => (
                "apk_permission_unknown",
                "This permission is not in the reviewed Android permission catalog.",
                None,
            ),
            _ => return None,
        },
    };
    Some(ApkPermissionWarningDto {
        code,
        message,
        permission_name: Some(permission.declaration.name.clone()),
        applicability_reason,
    })
}

fn non_applicable_reason(
    reason: AndroidPermissionNonApplicability,
) -> ApkPermissionApplicabilityReasonDto {
    match reason {
        AndroidPermissionNonApplicability::DeclarationRequiresApi23 => {
            ApkPermissionApplicabilityReasonDto::DeclarationRequiresApi23
        }
        AndroidPermissionNonApplicability::MaxSdkVersionExceeded { .. } => {
            ApkPermissionApplicabilityReasonDto::MaxSdkVersionExceeded
        }
        AndroidPermissionNonApplicability::PermissionNotIntroduced { .. } => {
            ApkPermissionApplicabilityReasonDto::PermissionNotIntroduced
        }
        AndroidPermissionNonApplicability::PermissionReplaced { .. } => {
            ApkPermissionApplicabilityReasonDto::PermissionReplaced
        }
        AndroidPermissionNonApplicability::TargetSdkBelowMinimum { .. } => {
            ApkPermissionApplicabilityReasonDto::TargetSdkBelowMinimum
        }
    }
}

fn indeterminate_reason(
    reason: AndroidPermissionIndeterminacy,
) -> ApkPermissionApplicabilityReasonDto {
    match reason {
        AndroidPermissionIndeterminacy::InvalidMaxSdkVersion => {
            ApkPermissionApplicabilityReasonDto::InvalidMaxSdkVersion
        }
        AndroidPermissionIndeterminacy::TargetSdkUnavailable { .. } => {
            ApkPermissionApplicabilityReasonDto::TargetSdkUnavailable
        }
        AndroidPermissionIndeterminacy::ReplacementTargetSdkUnavailable { .. } => {
            ApkPermissionApplicabilityReasonDto::ReplacementTargetSdkUnavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn declaration(
        name: &str,
        kind: ApkPermissionDeclarationKind,
        max_sdk_version: Option<&str>,
    ) -> ApkPermissionDeclaration {
        ApkPermissionDeclaration {
            name: name.to_string(),
            kind,
            max_sdk_version: max_sdk_version.map(str::to_string),
        }
    }

    fn facts(
        target_sdk: Option<&str>,
        permissions: Vec<ApkPermissionDeclaration>,
    ) -> ApkManifestFacts {
        ApkManifestFacts {
            package_name: "com.example.review".to_string(),
            version_code: Some("42".to_string()),
            version_name: Some("2.5".to_string()),
            min_sdk_version: Some("23".to_string()),
            target_sdk_version: target_sdk.map(str::to_string),
            permissions,
        }
    }

    #[test]
    fn apk_authoring_inspection_reads_manifest_and_streams_uppercase_sha256() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        let apk = crate::apk_manifest::tests::write_valid_test_apk(&workspace);
        let result = inspect_apk_for_authoring(&apk, Some(35)).expect("APK should inspect");
        let expected = Sha256::digest(fs::read(&apk).expect("fixture should be readable"))
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();

        assert_eq!(result.manifest.package_name, "com.example.qualified");
        assert_eq!(result.manifest.version_code.as_deref(), Some("42"));
        assert_eq!(result.permissions.len(), 3);
        assert_eq!(result.calculated_sha256, expected);
        assert_eq!(result.calculated_sha256.len(), 64);
        assert!(result
            .calculated_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte)));
        assert_eq!(result.checksum_status, ApkChecksumStatusDto::NotCompared);
        assert_eq!(
            result.signature_verification,
            ApkSignatureVerificationDto::NotPerformed
        );
    }

    #[test]
    fn apk_authoring_inspection_without_device_context_preserves_only_declarations() {
        let result = build_result(
            facts(
                Some("35"),
                vec![
                    declaration(
                        "android.permission.CAMERA",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.UNKNOWN_EXAMPLE",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                ],
            ),
            None,
            "A".repeat(64),
        );

        assert_eq!(result.permissions.len(), 2);
        assert!(result
            .permissions
            .iter()
            .all(|permission| permission.classification.is_none()
                && permission.applicability.is_none()));
        assert!(result.runtime_grant_candidates.is_empty());
        assert!(result.app_op_candidates.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(
            result.warnings[0].code,
            "apk_permission_classification_context_unavailable"
        );
        assert_eq!(result.warnings[0].permission_name, None);
    }

    #[test]
    fn apk_authoring_inspection_exposes_only_applicable_review_candidates() {
        let result = build_result(
            facts(
                Some("35"),
                vec![
                    declaration(
                        "android.permission.CAMERA",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.INTERNET",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.MANAGE_EXTERNAL_STORAGE",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.SYSTEM_ALERT_WINDOW",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.WRITE_SECURE_SETTINGS",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.BODY_SENSORS_BACKGROUND",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.UNKNOWN_EXAMPLE",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                ],
            ),
            Some(35),
            "B".repeat(64),
        );

        assert_eq!(result.runtime_grant_candidates.len(), 1);
        assert_eq!(
            result.runtime_grant_candidates[0].permission_name,
            "android.permission.CAMERA"
        );
        assert!(!result.runtime_grant_candidates[0].requires_root);
        assert!(!result.runtime_grant_candidates[0].selected);
        assert_eq!(result.app_op_candidates.len(), 1);
        assert_eq!(
            result.app_op_candidates[0].operation_name,
            "MANAGE_EXTERNAL_STORAGE"
        );
        assert!(result.app_op_candidates[0].requires_root);
        assert!(!result.app_op_candidates[0].selected);
        assert_eq!(
            result
                .warnings
                .iter()
                .map(|warning| warning.code)
                .collect::<Vec<_>>(),
            [
                "apk_permission_runtime_restricted",
                "apk_permission_manual_special_access",
                "apk_permission_unknown",
                "apk_permission_signature_or_privileged",
            ]
        );
        assert!(result
            .warnings
            .iter()
            .all(|warning| warning.permission_name.is_some()));
    }

    #[test]
    fn apk_authoring_inspection_warns_for_every_non_applicable_reason() {
        let cases = [
            (
                declaration(
                    "android.permission.CAMERA",
                    ApkPermissionDeclarationKind::UsesPermissionSdk23,
                    None,
                ),
                "35",
                22,
                ApkPermissionApplicabilityReasonDto::DeclarationRequiresApi23,
            ),
            (
                declaration(
                    "android.permission.CAMERA",
                    ApkPermissionDeclarationKind::UsesPermission,
                    Some("34"),
                ),
                "35",
                35,
                ApkPermissionApplicabilityReasonDto::MaxSdkVersionExceeded,
            ),
            (
                declaration(
                    "android.permission.READ_MEDIA_IMAGES",
                    ApkPermissionDeclarationKind::UsesPermission,
                    None,
                ),
                "35",
                32,
                ApkPermissionApplicabilityReasonDto::PermissionNotIntroduced,
            ),
            (
                declaration(
                    "android.permission.READ_EXTERNAL_STORAGE",
                    ApkPermissionDeclarationKind::UsesPermission,
                    None,
                ),
                "35",
                35,
                ApkPermissionApplicabilityReasonDto::PermissionReplaced,
            ),
            (
                declaration(
                    "android.permission.READ_MEDIA_IMAGES",
                    ApkPermissionDeclarationKind::UsesPermission,
                    None,
                ),
                "32",
                35,
                ApkPermissionApplicabilityReasonDto::TargetSdkBelowMinimum,
            ),
        ];

        for (declaration, target_sdk, device_api, expected_reason) in cases {
            let permission_name = declaration.name.clone();
            let result = build_result(
                facts(Some(target_sdk), vec![declaration]),
                Some(device_api),
                "C".repeat(64),
            );
            assert!(result.runtime_grant_candidates.is_empty());
            assert!(result.app_op_candidates.is_empty());
            assert_eq!(result.warnings.len(), 1);
            assert_eq!(result.warnings[0].code, "apk_permission_not_applicable");
            assert_eq!(
                result.warnings[0].permission_name.as_deref(),
                Some(permission_name.as_str())
            );
            assert_eq!(
                result.warnings[0].applicability_reason,
                Some(expected_reason)
            );
        }
    }

    #[test]
    fn apk_authoring_inspection_warns_for_every_indeterminate_reason() {
        let cases = [
            (
                declaration(
                    "android.permission.UNKNOWN_EXAMPLE",
                    ApkPermissionDeclarationKind::UsesPermission,
                    Some("Preview"),
                ),
                Some("35"),
                ApkPermissionApplicabilityReasonDto::InvalidMaxSdkVersion,
            ),
            (
                declaration(
                    "android.permission.CAMERA",
                    ApkPermissionDeclarationKind::UsesPermission,
                    None,
                ),
                None,
                ApkPermissionApplicabilityReasonDto::TargetSdkUnavailable,
            ),
            (
                declaration(
                    "android.permission.READ_EXTERNAL_STORAGE",
                    ApkPermissionDeclarationKind::UsesPermission,
                    None,
                ),
                Some("Preview"),
                ApkPermissionApplicabilityReasonDto::ReplacementTargetSdkUnavailable,
            ),
        ];

        for (declaration, target_sdk, expected_reason) in cases {
            let result = build_result(
                facts(target_sdk, vec![declaration]),
                Some(35),
                "D".repeat(64),
            );
            assert!(result.runtime_grant_candidates.is_empty());
            assert!(result.app_op_candidates.is_empty());
            assert_eq!(result.warnings.len(), 1);
            assert_eq!(
                result.warnings[0].code,
                "apk_permission_applicability_indeterminate"
            );
            assert_eq!(
                result.warnings[0].applicability_reason,
                Some(expected_reason)
            );
        }
    }

    #[test]
    fn apk_authoring_inspection_serialization_contains_no_legacy_or_command_fields() {
        let result = build_result(
            facts(
                Some("35"),
                vec![declaration(
                    "android.permission.CAMERA",
                    ApkPermissionDeclarationKind::UsesPermission,
                    None,
                )],
            ),
            Some(35),
            "E".repeat(64),
        );
        let value = serde_json::to_value(result).expect("result should serialize");
        assert_eq!(value["checksumStatus"], "not_compared");
        assert_eq!(value["signatureVerification"], "not_performed");
        assert_eq!(value["runtimeGrantCandidates"][0]["selected"], false);
        for forbidden in [
            "analyzer",
            "certificate",
            "apkPath",
            "command",
            "pm grant",
            "appops",
        ] {
            assert!(!value.to_string().contains(forbidden), "{value:#}");
        }
    }

    #[test]
    fn apk_authoring_inspection_errors_are_stable_and_redacted() {
        let private_path = Path::new("/Users/private/secret-source-name.apk");
        let manifest_error =
            inspect_apk_for_authoring(private_path, Some(35)).expect_err("missing APK should fail");
        assert_eq!(manifest_error.code(), "apk_manifest_inspection_failed");
        assert_eq!(
            manifest_error.message(),
            "APK manifest could not be inspected."
        );
        let read_error = calculate_sha256(private_path).expect_err("missing file should fail");
        assert_eq!(read_error.code(), "apk_file_read_failed");
        assert_eq!(read_error.message(), "APK file could not be read.");
        for error in [manifest_error, read_error] {
            let public = format!("{} {}", error.code(), error.message());
            assert!(!public.contains("/Users/private"));
            assert!(!public.contains("secret-source-name"));
        }
    }

    #[test]
    fn apk_authoring_inspection_request_rejects_legacy_and_invalid_inputs() {
        let request = serde_json::from_value::<ApkAuthoringInspectionRequest>(json!({
            "apkPath": "/tmp/example.apk",
            "connectedDeviceApi": 35
        }))
        .expect("native request should parse");
        assert!(request.is_valid());

        for invalid in [
            json!({ "apkPath": "" }),
            json!({ "apkPath": "/tmp/example.apk", "connectedDeviceApi": 0 }),
        ] {
            let request = serde_json::from_value::<ApkAuthoringInspectionRequest>(invalid)
                .expect("structurally valid request should parse");
            assert!(!request.is_valid());
        }
        for legacy in [
            json!({ "analyzer": "apkanalyzer", "facts": {} }),
            json!({ "apkPath": "/tmp/example.apk", "facts": {} }),
        ] {
            assert!(serde_json::from_value::<ApkAuthoringInspectionRequest>(legacy).is_err());
        }
    }
}
