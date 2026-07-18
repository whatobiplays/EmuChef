//! Native authoring-time APK inspection and review-only permission actions.
//!
//! This boundary accepts only a selected APK path. It returns manifest-owned
//! metadata, a calculated file hash, and device-independent review DTOs. It
//! never verifies signatures, constructs commands, or mutates recipes or
//! devices.

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
}

impl ApkAuthoringInspectionRequest {
    /// Reject empty selected paths.
    pub(crate) fn is_valid(&self) -> bool {
        !self.apk_path.trim().is_empty()
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

/// Whether a permission declaration has a usable authoring-time API range.
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
    MaxSdkVersionExceeded,
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
    android_api_min: u32,
    android_api_max: Option<u32>,
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
    android_api_min: u32,
    android_api_max: Option<u32>,
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
) -> Result<ApkAuthoringInspectionResult, ApkAuthoringInspectionError> {
    let facts = inspect_apk_manifest(apk_path)?;
    let calculated_sha256 = calculate_sha256(apk_path)?;
    Ok(build_result(facts, calculated_sha256))
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
        classified_permission_review(&facts.permissions, manifest.target_sdk_version.as_deref());

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
) -> PermissionReviewParts {
    let classified = classify_android_permissions(declarations, target_sdk);
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
                let bounds = permission
                    .api_bounds
                    .expect("automated permissions must have reviewed API bounds");
                runtime_candidates.push(ApkRuntimeGrantCandidateDto {
                    permission_name: permission.declaration.name.clone(),
                    requires_root,
                    android_api_min: bounds.minimum,
                    android_api_max: bounds.maximum,
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
                let bounds = permission
                    .api_bounds
                    .expect("automated permissions must have reviewed API bounds");
                app_op_candidates.push(ApkAppOpCandidateDto {
                    permission_name: permission.declaration.name.clone(),
                    operation_name: app_op.to_string(),
                    mode: app_op_mode(mode),
                    requires_root,
                    android_api_min: bounds.minimum,
                    android_api_max: bounds.maximum,
                    selected: false,
                });
            }
            _ => {}
        }
        permissions.push(permission_with_context(permission));
    }

    (permissions, runtime_candidates, app_op_candidates, warnings)
}

fn permission_with_context(permission: ClassifiedAndroidPermission) -> ApkPermissionReviewDto {
    let applicability = applicability(&permission);
    ApkPermissionReviewDto {
        name: permission.declaration.name,
        declaration_kind: declaration_kind(permission.declaration.kind),
        max_sdk_version: permission.declaration.max_sdk_version,
        classification: Some(classification(permission.classification)),
        applicability: Some(applicability),
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

fn applicability(permission: &ClassifiedAndroidPermission) -> ApkPermissionApplicabilityDto {
    let mut result = ApkPermissionApplicabilityDto {
        status: ApkPermissionApplicabilityStatusDto::Applicable,
        reason: None,
        maximum_sdk_version: permission.api_bounds.and_then(|bounds| bounds.maximum),
        introduction_api: permission.catalog_introduction_api,
        minimum_device_api: permission.api_bounds.map(|bounds| bounds.minimum),
        minimum_target_sdk: None,
        target_sdk_state: None,
    };
    match permission.applicability {
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
        AndroidPermissionNonApplicability::MaxSdkVersionExceeded { maximum } => {
            result.reason = Some(ApkPermissionApplicabilityReasonDto::MaxSdkVersionExceeded);
            result.maximum_sdk_version = Some(maximum);
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
            "This permission has no supported authoring-time Android API range.",
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
        AndroidPermissionNonApplicability::MaxSdkVersionExceeded { .. } => {
            ApkPermissionApplicabilityReasonDto::MaxSdkVersionExceeded
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

    use serde_json::{json, Value};
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
        let result = inspect_apk_for_authoring(&apk).expect("APK should inspect");
        let expected = Sha256::digest(fs::read(&apk).expect("fixture should be readable"))
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();

        assert_eq!(result.manifest.package_name, "com.example.qualified");
        assert_eq!(result.manifest.version_code.as_deref(), Some("42"));
        assert_eq!(result.permissions.len(), 3);
        assert_eq!(result.calculated_sha256, expected);
        assert_eq!(result.checksum_status, ApkChecksumStatusDto::NotCompared);
        assert_eq!(
            result.signature_verification,
            ApkSignatureVerificationDto::NotPerformed
        );
    }

    #[test]
    fn apk_authoring_inspection_builds_candidates_without_device_context() {
        let result = build_result(
            facts(
                Some("29"),
                vec![
                    declaration(
                        "android.permission.RECORD_AUDIO",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.POST_NOTIFICATIONS",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.WRITE_EXTERNAL_STORAGE",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.MANAGE_EXTERNAL_STORAGE",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                ],
            ),
            "A".repeat(64),
        );

        assert_eq!(result.runtime_grant_candidates.len(), 3);
        assert_eq!(
            result
                .runtime_grant_candidates
                .iter()
                .map(|candidate| (
                    candidate.permission_name.as_str(),
                    candidate.android_api_min,
                    candidate.android_api_max,
                ))
                .collect::<Vec<_>>(),
            vec![
                ("android.permission.POST_NOTIFICATIONS", 33, None),
                ("android.permission.RECORD_AUDIO", 23, None),
                ("android.permission.WRITE_EXTERNAL_STORAGE", 23, None),
            ]
        );
        assert!(result
            .runtime_grant_candidates
            .iter()
            .all(|candidate| !candidate.requires_root && !candidate.selected));
        assert_eq!(result.app_op_candidates.len(), 1);
        assert_eq!(
            result.app_op_candidates[0].permission_name,
            "android.permission.MANAGE_EXTERNAL_STORAGE"
        );
        assert_eq!(
            result.app_op_candidates[0].operation_name,
            "MANAGE_EXTERNAL_STORAGE"
        );
        assert_eq!(result.app_op_candidates[0].android_api_min, 30);
        assert_eq!(result.app_op_candidates[0].android_api_max, None);
        assert!(result.app_op_candidates[0].requires_root);
        assert!(!result.app_op_candidates[0].selected);
    }

    #[test]
    fn apk_authoring_inspection_combines_manifest_and_catalog_bounds() {
        let result = build_result(
            facts(
                Some("29"),
                vec![
                    declaration(
                        "android.permission.WRITE_EXTERNAL_STORAGE",
                        ApkPermissionDeclarationKind::UsesPermissionSdk23,
                        Some("34"),
                    ),
                    declaration(
                        "android.permission.READ_EXTERNAL_STORAGE",
                        ApkPermissionDeclarationKind::UsesPermission,
                        Some("31"),
                    ),
                ],
            ),
            "B".repeat(64),
        );

        assert_eq!(result.runtime_grant_candidates.len(), 1);
        let write = &result.runtime_grant_candidates[0];
        assert_eq!(write.android_api_min, 23);
        assert_eq!(write.android_api_max, Some(34));
        let read = result
            .permissions
            .iter()
            .find(|permission| permission.name == "android.permission.READ_EXTERNAL_STORAGE")
            .expect("restricted read permission should remain reviewable");
        assert_eq!(
            read.classification,
            Some(ApkPermissionClassificationDto::RuntimeRestricted)
        );
        assert_eq!(
            read.applicability
                .as_ref()
                .and_then(|applicability| applicability.maximum_sdk_version),
            Some(31)
        );
    }

    #[test]
    fn apk_authoring_inspection_target_30_omits_legacy_write_candidate() {
        let result = build_result(
            facts(
                Some("30"),
                vec![declaration(
                    "android.permission.WRITE_EXTERNAL_STORAGE",
                    ApkPermissionDeclarationKind::UsesPermission,
                    None,
                )],
            ),
            "C".repeat(64),
        );

        assert!(result.runtime_grant_candidates.is_empty());
        assert_eq!(
            result.permissions[0].applicability.as_ref().unwrap().status,
            ApkPermissionApplicabilityStatusDto::NotApplicable
        );
        assert_eq!(
            result.permissions[0].applicability.as_ref().unwrap().reason,
            Some(ApkPermissionApplicabilityReasonDto::PermissionReplaced)
        );
        assert_eq!(result.warnings[0].code, "apk_permission_not_applicable");
    }

    #[test]
    fn apk_authoring_inspection_missing_target_fails_closed_for_target_rules() {
        for target_sdk in [None, Some("R")] {
            let result = build_result(
                facts(
                    target_sdk,
                    vec![declaration(
                        "android.permission.WRITE_EXTERNAL_STORAGE",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    )],
                ),
                "D".repeat(64),
            );
            assert!(result.runtime_grant_candidates.is_empty());
            assert_eq!(
                result.permissions[0].applicability.as_ref().unwrap().status,
                ApkPermissionApplicabilityStatusDto::Indeterminate
            );
            assert_eq!(
                result.warnings[0].code,
                "apk_permission_applicability_indeterminate"
            );
        }
    }

    #[test]
    fn apk_authoring_inspection_restricted_manual_and_unknown_names_never_automate() {
        let result = build_result(
            facts(
                Some("37"),
                vec![
                    declaration(
                        "android.permission.SEND_SMS",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "android.permission.SYSTEM_ALERT_WINDOW",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                    declaration(
                        "com.example.permission.CUSTOM",
                        ApkPermissionDeclarationKind::UsesPermission,
                        None,
                    ),
                ],
            ),
            "E".repeat(64),
        );

        assert!(result.runtime_grant_candidates.is_empty());
        assert!(result.app_op_candidates.is_empty());
        assert_eq!(result.permissions.len(), 3);
        assert_eq!(result.warnings.len(), 3);
        assert_eq!(
            result
                .warnings
                .iter()
                .map(|warning| warning.code)
                .collect::<Vec<_>>(),
            vec![
                "apk_permission_runtime_restricted",
                "apk_permission_manual_special_access",
                "apk_permission_unknown",
            ]
        );
    }

    #[test]
    fn apk_authoring_inspection_impossible_range_has_stable_reason() {
        let result = build_result(
            facts(
                Some("35"),
                vec![declaration(
                    "android.permission.POST_NOTIFICATIONS",
                    ApkPermissionDeclarationKind::UsesPermission,
                    Some("32"),
                )],
            ),
            "F".repeat(64),
        );

        assert!(result.runtime_grant_candidates.is_empty());
        let applicability = result.permissions[0].applicability.as_ref().unwrap();
        assert_eq!(
            applicability.status,
            ApkPermissionApplicabilityStatusDto::NotApplicable
        );
        assert_eq!(
            applicability.reason,
            Some(ApkPermissionApplicabilityReasonDto::MaxSdkVersionExceeded)
        );
        assert_eq!(applicability.maximum_sdk_version, Some(32));
    }

    #[test]
    fn apk_authoring_inspection_serializes_bounds_without_commands() {
        let result = build_result(
            facts(
                Some("35"),
                vec![declaration(
                    "android.permission.POST_NOTIFICATIONS",
                    ApkPermissionDeclarationKind::UsesPermission,
                    None,
                )],
            ),
            "A".repeat(64),
        );
        let value = serde_json::to_value(result).expect("result should serialize");
        assert_eq!(value["runtimeGrantCandidates"][0]["androidApiMin"], 33);
        assert_eq!(
            value["runtimeGrantCandidates"][0]["androidApiMax"],
            Value::Null
        );
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
            inspect_apk_for_authoring(private_path).expect_err("missing APK should fail");
        assert_eq!(manifest_error.code(), "apk_manifest_inspection_failed");
        assert_eq!(
            manifest_error.message(),
            "APK manifest could not be inspected."
        );
        let read_error = calculate_sha256(private_path).expect_err("missing file should fail");
        assert_eq!(read_error.code(), "apk_file_read_failed");
        for error in [manifest_error, read_error] {
            let public = format!("{} {}", error.code(), error.message());
            assert!(!public.contains("/Users/private"));
            assert!(!public.contains("secret-source-name"));
        }
    }

    #[test]
    fn apk_authoring_inspection_request_accepts_only_the_apk_path() {
        let request = serde_json::from_value::<ApkAuthoringInspectionRequest>(json!({
            "apkPath": "/tmp/example.apk"
        }))
        .expect("native request should parse");
        assert!(request.is_valid());

        let empty = serde_json::from_value::<ApkAuthoringInspectionRequest>(json!({
            "apkPath": ""
        }))
        .expect("empty path is structurally valid");
        assert!(!empty.is_valid());

        for legacy in [
            json!({ "apkPath": "/tmp/example.apk", "connectedDeviceApi": 35 }),
            json!({ "analyzer": "apkanalyzer", "facts": {} }),
            json!({ "apkPath": "/tmp/example.apk", "facts": {} }),
        ] {
            assert!(serde_json::from_value::<ApkAuthoringInspectionRequest>(legacy).is_err());
        }
    }
}
