//! Exact Android permission catalog and deterministic authoring rules.
//!
//! This module contains reviewed platform facts only. It never queries a
//! connected device and never constructs commands. Permission names are
//! matched exactly; names absent from the catalog remain unknown.

use crate::apk_manifest::{ApkPermissionDeclaration, ApkPermissionDeclarationKind};

const RUNTIME_PERMISSIONS_API: u32 = 23;

/// Stable permission categories understood by the backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidPermissionClassification {
    RuntimeGrantable,
    RuntimeRestricted,
    AppOpGrantable,
    ManualSpecialAccess,
    InstallTime,
    SignatureOrPrivileged,
    Unknown,
}

/// Whether a declaration can participate in device-independent authoring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidPermissionApplicability {
    Applicable,
    NotApplicable(AndroidPermissionNonApplicability),
    Indeterminate(AndroidPermissionIndeterminacy),
}

/// A proven reason that a declaration cannot produce an applicable action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidPermissionNonApplicability {
    /// The effective maximum is below the effective minimum.
    MaxSdkVersionExceeded {
        maximum: u32,
    },
    /// A target-SDK rule makes this permission ineffective or replaced.
    PermissionReplaced {
        minimum_device_api: u32,
        minimum_target_sdk: u32,
    },
    TargetSdkBelowMinimum {
        minimum_target_sdk: u32,
    },
}

/// A reason the classifier cannot safely decide authoring applicability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidPermissionIndeterminacy {
    InvalidMaxSdkVersion,
    TargetSdkUnavailable {
        minimum_target_sdk: u32,
        state: TargetSdkState,
    },
    ReplacementTargetSdkUnavailable {
        minimum_device_api: u32,
        minimum_target_sdk: u32,
        state: TargetSdkState,
    },
}

/// The two fail-closed forms of unavailable application target SDK metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetSdkState {
    Missing,
    NonNumeric,
}

/// Inclusive Android API bounds copied into generated action conditions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AndroidApiBounds {
    pub(crate) minimum: u32,
    pub(crate) maximum: Option<u32>,
}

/// Explicit reviewed automation metadata. This is data, not command generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidPermissionAutomation {
    RuntimeGrant {
        requires_root: bool,
    },
    AppOp {
        app_op: &'static str,
        mode: AndroidAppOpMode,
        requires_root: bool,
    },
}

/// App-op modes reviewed for use by the catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidAppOpMode {
    Allow,
}

/// All manifest-owned inputs required to classify one declaration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AndroidPermissionClassificationContext<'a> {
    pub(crate) declaration: &'a ApkPermissionDeclaration,
    pub(crate) app_target_sdk: Option<&'a str>,
}

/// Stable backend-owned result for one manifest declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClassifiedAndroidPermission {
    pub(crate) declaration: ApkPermissionDeclaration,
    pub(crate) classification: AndroidPermissionClassification,
    pub(crate) applicability: AndroidPermissionApplicability,
    pub(crate) catalog_introduction_api: Option<u32>,
    pub(crate) api_bounds: Option<AndroidApiBounds>,
    /// Present only when positively eligible for the reviewed mechanism.
    pub(crate) automation: Option<AndroidPermissionAutomation>,
}

#[derive(Clone, Copy, Debug)]
struct PermissionCatalogEntry {
    name: &'static str,
    classification: AndroidPermissionClassification,
    introduction_api: u32,
    maximum_api: Option<u32>,
    runtime_permission_model: bool,
    minimum_target_sdk: Option<u32>,
    maximum_target_sdk: Option<u32>,
    #[cfg_attr(not(test), allow(dead_code))]
    public_dangerous: bool,
    automation: Option<AndroidPermissionAutomation>,
}

const RUNTIME_GRANT_NO_ROOT: AndroidPermissionAutomation =
    AndroidPermissionAutomation::RuntimeGrant {
        requires_root: false,
    };

const fn runtime_permission(
    name: &'static str,
    introduction_api: u32,
    minimum_target_sdk: Option<u32>,
) -> PermissionCatalogEntry {
    PermissionCatalogEntry {
        name,
        classification: AndroidPermissionClassification::RuntimeGrantable,
        introduction_api,
        maximum_api: None,
        runtime_permission_model: true,
        minimum_target_sdk,
        maximum_target_sdk: None,
        public_dangerous: true,
        automation: Some(RUNTIME_GRANT_NO_ROOT),
    }
}

const fn restricted_permission(
    name: &'static str,
    introduction_api: u32,
    maximum_api: Option<u32>,
    minimum_target_sdk: Option<u32>,
) -> PermissionCatalogEntry {
    PermissionCatalogEntry {
        name,
        classification: AndroidPermissionClassification::RuntimeRestricted,
        introduction_api,
        maximum_api,
        runtime_permission_model: true,
        minimum_target_sdk,
        maximum_target_sdk: None,
        public_dangerous: true,
        automation: None,
    }
}

const fn non_runtime_permission(
    name: &'static str,
    classification: AndroidPermissionClassification,
    introduction_api: u32,
) -> PermissionCatalogEntry {
    PermissionCatalogEntry {
        name,
        classification,
        introduction_api,
        maximum_api: None,
        runtime_permission_model: false,
        minimum_target_sdk: None,
        maximum_target_sdk: None,
        public_dangerous: false,
        automation: None,
    }
}

/// Reviewed public Android permission registry.
///
/// Dangerous entries are the stable public integer-API permission surface.
/// Hidden, SystemApi-only, preview, feature-flag-only, and extension-only names
/// are deliberately absent because they do not provide a portable recipe
/// condition contract.
const PERMISSION_CATALOG: &[PermissionCatalogEntry] = &[
    // Public dangerous permissions introduced in API 1.
    runtime_permission("android.permission.ACCESS_COARSE_LOCATION", 1, None),
    runtime_permission("android.permission.ACCESS_FINE_LOCATION", 1, None),
    runtime_permission("android.permission.CALL_PHONE", 1, None),
    runtime_permission("android.permission.CAMERA", 1, None),
    runtime_permission("android.permission.GET_ACCOUNTS", 1, None),
    restricted_permission(
        "android.permission.PROCESS_OUTGOING_CALLS",
        1,
        Some(28),
        None,
    ),
    runtime_permission("android.permission.READ_CALENDAR", 1, None),
    runtime_permission("android.permission.READ_CONTACTS", 1, None),
    runtime_permission("android.permission.READ_PHONE_STATE", 1, None),
    restricted_permission("android.permission.READ_SMS", 1, None, None),
    restricted_permission("android.permission.RECEIVE_MMS", 1, None, None),
    restricted_permission("android.permission.RECEIVE_SMS", 1, None, None),
    restricted_permission("android.permission.RECEIVE_WAP_PUSH", 1, None, None),
    runtime_permission("android.permission.RECORD_AUDIO", 1, None),
    restricted_permission("android.permission.SEND_SMS", 1, None, None),
    runtime_permission("android.permission.WRITE_CALENDAR", 1, None),
    runtime_permission("android.permission.WRITE_CONTACTS", 1, None),
    // Legacy storage remains a reviewed exception: target SDK, not device API,
    // limits WRITE_EXTERNAL_STORAGE automation.
    PermissionCatalogEntry {
        name: "android.permission.WRITE_EXTERNAL_STORAGE",
        classification: AndroidPermissionClassification::RuntimeGrantable,
        introduction_api: 4,
        maximum_api: None,
        runtime_permission_model: true,
        minimum_target_sdk: None,
        maximum_target_sdk: Some(29),
        public_dangerous: true,
        automation: Some(RUNTIME_GRANT_NO_ROOT),
    },
    runtime_permission("android.permission.USE_SIP", 9, None),
    runtime_permission("com.android.voicemail.permission.ADD_VOICEMAIL", 14, None),
    restricted_permission("android.permission.READ_CALL_LOG", 16, None, None),
    restricted_permission(
        "android.permission.READ_EXTERNAL_STORAGE",
        16,
        Some(32),
        None,
    ),
    restricted_permission("android.permission.WRITE_CALL_LOG", 16, None, None),
    runtime_permission("android.permission.BODY_SENSORS", 20, None),
    runtime_permission("android.permission.ANSWER_PHONE_CALLS", 26, None),
    runtime_permission("android.permission.READ_PHONE_NUMBERS", 26, None),
    runtime_permission("android.permission.ACCEPT_HANDOVER", 28, None),
    restricted_permission(
        "android.permission.ACCESS_BACKGROUND_LOCATION",
        29,
        None,
        Some(29),
    ),
    runtime_permission("android.permission.ACCESS_MEDIA_LOCATION", 29, Some(29)),
    runtime_permission("android.permission.ACTIVITY_RECOGNITION", 29, Some(29)),
    runtime_permission("android.permission.BLUETOOTH_ADVERTISE", 31, Some(31)),
    runtime_permission("android.permission.BLUETOOTH_CONNECT", 31, Some(31)),
    runtime_permission("android.permission.BLUETOOTH_SCAN", 31, Some(31)),
    runtime_permission("android.permission.UWB_RANGING", 31, Some(31)),
    restricted_permission(
        "android.permission.BODY_SENSORS_BACKGROUND",
        33,
        None,
        Some(33),
    ),
    runtime_permission("android.permission.NEARBY_WIFI_DEVICES", 33, Some(33)),
    runtime_permission("android.permission.POST_NOTIFICATIONS", 33, None),
    runtime_permission("android.permission.READ_MEDIA_AUDIO", 33, Some(33)),
    runtime_permission("android.permission.READ_MEDIA_IMAGES", 33, Some(33)),
    runtime_permission("android.permission.READ_MEDIA_VIDEO", 33, Some(33)),
    runtime_permission(
        "android.permission.READ_MEDIA_VISUAL_USER_SELECTED",
        34,
        Some(34),
    ),
    runtime_permission("android.permission.RANGING", 36, Some(36)),
    runtime_permission("android.permission.ACCESS_LOCAL_NETWORK", 37, Some(37)),
    // Reviewed non-dangerous, special-access, and privileged cases.
    non_runtime_permission(
        "android.permission.INTERNET",
        AndroidPermissionClassification::InstallTime,
        1,
    ),
    non_runtime_permission(
        "android.permission.SYSTEM_ALERT_WINDOW",
        AndroidPermissionClassification::ManualSpecialAccess,
        1,
    ),
    non_runtime_permission(
        "android.permission.WRITE_SETTINGS",
        AndroidPermissionClassification::ManualSpecialAccess,
        1,
    ),
    non_runtime_permission(
        "android.permission.WRITE_SECURE_SETTINGS",
        AndroidPermissionClassification::SignatureOrPrivileged,
        3,
    ),
    non_runtime_permission(
        "android.permission.BIND_DEVICE_ADMIN",
        AndroidPermissionClassification::SignatureOrPrivileged,
        8,
    ),
    non_runtime_permission(
        "android.permission.BIND_VPN_SERVICE",
        AndroidPermissionClassification::SignatureOrPrivileged,
        14,
    ),
    non_runtime_permission(
        "android.permission.BIND_ACCESSIBILITY_SERVICE",
        AndroidPermissionClassification::SignatureOrPrivileged,
        16,
    ),
    non_runtime_permission(
        "android.permission.BIND_NOTIFICATION_LISTENER_SERVICE",
        AndroidPermissionClassification::SignatureOrPrivileged,
        18,
    ),
    non_runtime_permission(
        "android.permission.ACCESS_NOTIFICATION_POLICY",
        AndroidPermissionClassification::ManualSpecialAccess,
        23,
    ),
    PermissionCatalogEntry {
        name: "android.permission.MANAGE_EXTERNAL_STORAGE",
        classification: AndroidPermissionClassification::AppOpGrantable,
        introduction_api: 30,
        maximum_api: None,
        runtime_permission_model: false,
        minimum_target_sdk: None,
        maximum_target_sdk: None,
        public_dangerous: false,
        automation: Some(AndroidPermissionAutomation::AppOp {
            app_op: "MANAGE_EXTERNAL_STORAGE",
            mode: AndroidAppOpMode::Allow,
            requires_root: true,
        }),
    },
    non_runtime_permission(
        "android.permission.SCHEDULE_EXACT_ALARM",
        AndroidPermissionClassification::ManualSpecialAccess,
        31,
    ),
    non_runtime_permission(
        "android.permission.USE_EXACT_ALARM",
        AndroidPermissionClassification::ManualSpecialAccess,
        33,
    ),
];

/// Classify declarations in the manifest model's stable order.
pub(crate) fn classify_android_permissions(
    declarations: &[ApkPermissionDeclaration],
    app_target_sdk: Option<&str>,
) -> Vec<ClassifiedAndroidPermission> {
    let mut results = declarations
        .iter()
        .map(|declaration| {
            classify_android_permission(AndroidPermissionClassificationContext {
                declaration,
                app_target_sdk,
            })
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.declaration.cmp(&right.declaration));
    results
}

/// Classify one declaration using manifest facts and exact catalog metadata.
pub(crate) fn classify_android_permission(
    context: AndroidPermissionClassificationContext<'_>,
) -> ClassifiedAndroidPermission {
    let catalog_entry = catalog_entry(&context.declaration.name);
    let mut classification = catalog_entry
        .map_or(AndroidPermissionClassification::Unknown, |entry| {
            entry.classification
        });
    let (applicability, api_bounds) =
        declaration_applicability(context, catalog_entry, &mut classification);
    let automation = catalog_entry
        .filter(|entry| {
            applicability == AndroidPermissionApplicability::Applicable
                && classification == entry.classification
                && api_bounds.is_some()
        })
        .and_then(|entry| entry.automation);

    ClassifiedAndroidPermission {
        declaration: context.declaration.clone(),
        classification,
        applicability,
        catalog_introduction_api: catalog_entry.map(|entry| entry.introduction_api),
        api_bounds,
        automation,
    }
}

fn catalog_entry(name: &str) -> Option<&'static PermissionCatalogEntry> {
    PERMISSION_CATALOG.iter().find(|entry| entry.name == name)
}

fn declaration_applicability(
    context: AndroidPermissionClassificationContext<'_>,
    catalog_entry: Option<&PermissionCatalogEntry>,
    classification: &mut AndroidPermissionClassification,
) -> (AndroidPermissionApplicability, Option<AndroidApiBounds>) {
    let manifest_maximum = match context.declaration.max_sdk_version.as_deref() {
        Some(value) => match value.parse::<u32>() {
            Ok(value) => Some(value),
            Err(_) => {
                return (
                    AndroidPermissionApplicability::Indeterminate(
                        AndroidPermissionIndeterminacy::InvalidMaxSdkVersion,
                    ),
                    None,
                );
            }
        },
        None => None,
    };

    let Some(entry) = catalog_entry else {
        return (AndroidPermissionApplicability::Applicable, None);
    };

    if let Some(minimum_target_sdk) = entry.minimum_target_sdk {
        match numeric_target_sdk(context.app_target_sdk) {
            Ok(target_sdk) if target_sdk < minimum_target_sdk => {
                return (
                    AndroidPermissionApplicability::NotApplicable(
                        AndroidPermissionNonApplicability::TargetSdkBelowMinimum {
                            minimum_target_sdk,
                        },
                    ),
                    None,
                );
            }
            Ok(_) => {}
            Err(state) => {
                return (
                    AndroidPermissionApplicability::Indeterminate(
                        AndroidPermissionIndeterminacy::TargetSdkUnavailable {
                            minimum_target_sdk,
                            state,
                        },
                    ),
                    None,
                );
            }
        }
    }

    if let Some(maximum_target_sdk) = entry.maximum_target_sdk {
        match numeric_target_sdk(context.app_target_sdk) {
            Ok(target_sdk) if target_sdk > maximum_target_sdk => {
                return (
                    AndroidPermissionApplicability::NotApplicable(
                        AndroidPermissionNonApplicability::PermissionReplaced {
                            minimum_device_api: maximum_target_sdk.saturating_add(1),
                            minimum_target_sdk: maximum_target_sdk.saturating_add(1),
                        },
                    ),
                    None,
                );
            }
            Ok(_) => {}
            Err(state) => {
                return (
                    AndroidPermissionApplicability::Indeterminate(
                        AndroidPermissionIndeterminacy::ReplacementTargetSdkUnavailable {
                            minimum_device_api: maximum_target_sdk.saturating_add(1),
                            minimum_target_sdk: maximum_target_sdk.saturating_add(1),
                            state,
                        },
                    ),
                    None,
                );
            }
        }
    }

    if entry.runtime_permission_model
        && matches!(
            entry.automation,
            Some(AndroidPermissionAutomation::RuntimeGrant { .. })
        )
    {
        match numeric_target_sdk(context.app_target_sdk) {
            Ok(target_sdk) if target_sdk < RUNTIME_PERMISSIONS_API => {
                *classification = AndroidPermissionClassification::InstallTime;
                return (AndroidPermissionApplicability::Applicable, None);
            }
            Ok(_) => {}
            Err(state) => {
                *classification = AndroidPermissionClassification::Unknown;
                return (
                    AndroidPermissionApplicability::Indeterminate(
                        AndroidPermissionIndeterminacy::TargetSdkUnavailable {
                            minimum_target_sdk: RUNTIME_PERMISSIONS_API,
                            state,
                        },
                    ),
                    None,
                );
            }
        }
    }

    let mut minimum = entry.introduction_api;
    if entry.runtime_permission_model {
        minimum = minimum.max(RUNTIME_PERMISSIONS_API);
    }
    if context.declaration.kind == ApkPermissionDeclarationKind::UsesPermissionSdk23 {
        minimum = minimum.max(RUNTIME_PERMISSIONS_API);
    }
    let maximum = minimum_option(entry.maximum_api, manifest_maximum);
    if maximum.is_some_and(|maximum| maximum < minimum) {
        return (
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::MaxSdkVersionExceeded {
                    maximum: maximum.unwrap_or_default(),
                },
            ),
            None,
        );
    }

    (
        AndroidPermissionApplicability::Applicable,
        Some(AndroidApiBounds { minimum, maximum }),
    )
}

fn minimum_option(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn numeric_target_sdk(target_sdk: Option<&str>) -> Result<u32, TargetSdkState> {
    let target_sdk = target_sdk.ok_or(TargetSdkState::Missing)?;
    target_sdk
        .parse::<u32>()
        .map_err(|_| TargetSdkState::NonNumeric)
}

#[cfg(test)]
fn catalog_integrity_issues(entries: &[PermissionCatalogEntry]) -> Vec<&'static str> {
    use std::collections::HashSet;

    let mut issues = Vec::new();
    let mut names = HashSet::new();
    for entry in entries {
        if !names.insert(entry.name) {
            issues.push("duplicate_name");
        }
        if entry.introduction_api == 0 {
            issues.push("api_zero");
        }
        if entry
            .maximum_api
            .is_some_and(|maximum| maximum < entry.introduction_api)
        {
            issues.push("api_bounds_impossible");
        }
        if entry
            .minimum_target_sdk
            .zip(entry.maximum_target_sdk)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            issues.push("target_sdk_bounds_impossible");
        }
        match (entry.classification, entry.automation) {
            (
                AndroidPermissionClassification::RuntimeGrantable,
                Some(AndroidPermissionAutomation::RuntimeGrant { .. }),
            ) if entry.runtime_permission_model => {}
            (
                AndroidPermissionClassification::AppOpGrantable,
                Some(AndroidPermissionAutomation::AppOp {
                    app_op: "MANAGE_EXTERNAL_STORAGE",
                    mode: AndroidAppOpMode::Allow,
                    requires_root: true,
                }),
            ) if entry.name == "android.permission.MANAGE_EXTERNAL_STORAGE" => {}
            (
                AndroidPermissionClassification::AppOpGrantable,
                Some(AndroidPermissionAutomation::AppOp { .. }),
            ) => {
                issues.push("app_op_mapping_unreviewed");
            }
            (AndroidPermissionClassification::RuntimeGrantable, _) => {
                issues.push("runtime_automation_mismatch");
            }
            (AndroidPermissionClassification::AppOpGrantable, _) => {
                issues.push("app_op_metadata_invalid");
            }
            (_, Some(AndroidPermissionAutomation::RuntimeGrant { .. })) => {
                issues.push("runtime_classification_invalid");
            }
            (_, Some(AndroidPermissionAutomation::AppOp { .. })) => {
                issues.push("app_op_mapping_unreviewed");
            }
            _ => {}
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const EXPECTED_PUBLIC_DANGEROUS_PERMISSIONS: &[&str] = &[
        "android.permission.ACCEPT_HANDOVER",
        "android.permission.ACCESS_BACKGROUND_LOCATION",
        "android.permission.ACCESS_COARSE_LOCATION",
        "android.permission.ACCESS_FINE_LOCATION",
        "android.permission.ACCESS_LOCAL_NETWORK",
        "android.permission.ACCESS_MEDIA_LOCATION",
        "android.permission.ACTIVITY_RECOGNITION",
        "android.permission.ANSWER_PHONE_CALLS",
        "android.permission.BLUETOOTH_ADVERTISE",
        "android.permission.BLUETOOTH_CONNECT",
        "android.permission.BLUETOOTH_SCAN",
        "android.permission.BODY_SENSORS",
        "android.permission.BODY_SENSORS_BACKGROUND",
        "android.permission.CALL_PHONE",
        "android.permission.CAMERA",
        "android.permission.GET_ACCOUNTS",
        "android.permission.NEARBY_WIFI_DEVICES",
        "android.permission.POST_NOTIFICATIONS",
        "android.permission.PROCESS_OUTGOING_CALLS",
        "android.permission.RANGING",
        "android.permission.READ_CALENDAR",
        "android.permission.READ_CALL_LOG",
        "android.permission.READ_CONTACTS",
        "android.permission.READ_EXTERNAL_STORAGE",
        "android.permission.READ_MEDIA_AUDIO",
        "android.permission.READ_MEDIA_IMAGES",
        "android.permission.READ_MEDIA_VIDEO",
        "android.permission.READ_MEDIA_VISUAL_USER_SELECTED",
        "android.permission.READ_PHONE_NUMBERS",
        "android.permission.READ_PHONE_STATE",
        "android.permission.READ_SMS",
        "android.permission.RECEIVE_MMS",
        "android.permission.RECEIVE_SMS",
        "android.permission.RECEIVE_WAP_PUSH",
        "android.permission.RECORD_AUDIO",
        "android.permission.SEND_SMS",
        "android.permission.USE_SIP",
        "android.permission.UWB_RANGING",
        "android.permission.WRITE_CALENDAR",
        "android.permission.WRITE_CALL_LOG",
        "android.permission.WRITE_CONTACTS",
        "android.permission.WRITE_EXTERNAL_STORAGE",
        "com.android.voicemail.permission.ADD_VOICEMAIL",
    ];

    fn declaration(name: &str) -> ApkPermissionDeclaration {
        ApkPermissionDeclaration {
            name: name.to_string(),
            kind: ApkPermissionDeclarationKind::UsesPermission,
            max_sdk_version: None,
        }
    }

    fn classify(name: &str, target_sdk: Option<&str>) -> ClassifiedAndroidPermission {
        let declaration = declaration(name);
        classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: target_sdk,
        })
    }

    #[test]
    fn android_permissions_catalog_is_complete_and_internally_valid() {
        assert_eq!(
            catalog_integrity_issues(PERMISSION_CATALOG),
            Vec::<&str>::new()
        );
        let actual = PERMISSION_CATALOG
            .iter()
            .filter(|entry| entry.public_dangerous)
            .map(|entry| entry.name)
            .collect::<BTreeSet<_>>();
        let expected = EXPECTED_PUBLIC_DANGEROUS_PERMISSIONS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn android_permissions_integrity_rejects_each_invalid_registry_shape() {
        let base = runtime_permission("android.permission.EXAMPLE", 23, None);
        let cases = [
            (vec![base, base], "duplicate_name"),
            (
                vec![PermissionCatalogEntry {
                    introduction_api: 0,
                    ..base
                }],
                "api_zero",
            ),
            (
                vec![PermissionCatalogEntry {
                    introduction_api: 24,
                    maximum_api: Some(23),
                    ..base
                }],
                "api_bounds_impossible",
            ),
            (
                vec![PermissionCatalogEntry {
                    minimum_target_sdk: Some(30),
                    maximum_target_sdk: Some(29),
                    ..base
                }],
                "target_sdk_bounds_impossible",
            ),
            (
                vec![PermissionCatalogEntry {
                    classification: AndroidPermissionClassification::InstallTime,
                    ..base
                }],
                "runtime_classification_invalid",
            ),
            (
                vec![PermissionCatalogEntry {
                    classification: AndroidPermissionClassification::AppOpGrantable,
                    runtime_permission_model: false,
                    automation: None,
                    ..base
                }],
                "app_op_metadata_invalid",
            ),
            (
                vec![PermissionCatalogEntry {
                    classification: AndroidPermissionClassification::AppOpGrantable,
                    runtime_permission_model: false,
                    automation: Some(AndroidPermissionAutomation::AppOp {
                        app_op: "CAMERA",
                        mode: AndroidAppOpMode::Allow,
                        requires_root: false,
                    }),
                    ..base
                }],
                "app_op_mapping_unreviewed",
            ),
        ];
        for (entries, expected) in cases {
            let issues = catalog_integrity_issues(&entries);
            assert!(
                issues.contains(&expected),
                "expected {expected}, got {issues:?}"
            );
        }
    }

    #[test]
    fn android_permissions_every_automated_entry_has_an_effective_minimum() {
        for entry in PERMISSION_CATALOG
            .iter()
            .filter(|entry| entry.automation.is_some())
        {
            let result = classify(entry.name, Some("37"));
            if entry.maximum_target_sdk.is_some() {
                continue;
            }
            assert!(
                result.api_bounds.is_some(),
                "missing bounds for {}",
                entry.name
            );
            assert!(
                result.api_bounds.expect("checked").minimum > 0,
                "zero minimum for {}",
                entry.name
            );
        }
    }

    #[test]
    fn android_permissions_unknown_names_are_exact_and_never_automated() {
        for name in [
            "android.permission.CAMERA_EXTRA",
            "prefix.android.permission.CAMERA",
            "com.example.permission.CUSTOM",
        ] {
            let result = classify(name, Some("35"));
            assert_eq!(
                result.classification,
                AndroidPermissionClassification::Unknown
            );
            assert_eq!(
                result.applicability,
                AndroidPermissionApplicability::Applicable
            );
            assert_eq!(result.api_bounds, None);
            assert_eq!(result.automation, None);
        }
    }

    #[test]
    fn android_permissions_representative_runtime_bounds_are_device_independent() {
        for (name, expected_minimum) in [
            ("android.permission.CAMERA", 23),
            ("android.permission.RECORD_AUDIO", 23),
            ("android.permission.POST_NOTIFICATIONS", 33),
            ("android.permission.RANGING", 36),
            ("android.permission.ACCESS_LOCAL_NETWORK", 37),
        ] {
            let result = classify(name, Some("37"));
            assert_eq!(
                result.api_bounds,
                Some(AndroidApiBounds {
                    minimum: expected_minimum,
                    maximum: None,
                }),
                "unexpected bounds for {name}"
            );
            assert_eq!(result.automation, Some(RUNTIME_GRANT_NO_ROOT));
        }
    }

    #[test]
    fn android_permissions_uses_permission_sdk_23_and_manifest_max_combine() {
        let declaration = ApkPermissionDeclaration {
            name: "android.permission.CAMERA".to_string(),
            kind: ApkPermissionDeclarationKind::UsesPermissionSdk23,
            max_sdk_version: Some("33".to_string()),
        };
        let result = classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: Some("35"),
        });
        assert_eq!(
            result.api_bounds,
            Some(AndroidApiBounds {
                minimum: 23,
                maximum: Some(33),
            })
        );
    }

    #[test]
    fn android_permissions_impossible_and_invalid_manifest_bounds_fail_closed() {
        let mut declaration = declaration("android.permission.POST_NOTIFICATIONS");
        declaration.max_sdk_version = Some("32".to_string());
        let impossible = classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: Some("35"),
        });
        assert_eq!(
            impossible.applicability,
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::MaxSdkVersionExceeded { maximum: 32 }
            )
        );
        assert_eq!(impossible.api_bounds, None);
        assert_eq!(impossible.automation, None);

        declaration.max_sdk_version = Some("Tiramisu".to_string());
        let invalid = classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: Some("35"),
        });
        assert_eq!(
            invalid.applicability,
            AndroidPermissionApplicability::Indeterminate(
                AndroidPermissionIndeterminacy::InvalidMaxSdkVersion
            )
        );
        assert_eq!(invalid.automation, None);
    }

    #[test]
    fn android_permissions_write_external_storage_uses_target_not_device_maximum() {
        let target_29 = classify("android.permission.WRITE_EXTERNAL_STORAGE", Some("29"));
        assert_eq!(
            target_29.api_bounds,
            Some(AndroidApiBounds {
                minimum: 23,
                maximum: None,
            })
        );
        assert_eq!(target_29.automation, Some(RUNTIME_GRANT_NO_ROOT));

        let target_30 = classify("android.permission.WRITE_EXTERNAL_STORAGE", Some("30"));
        assert!(matches!(
            target_30.applicability,
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::PermissionReplaced {
                    minimum_device_api: 30,
                    minimum_target_sdk: 30,
                }
            )
        ));
        assert_eq!(target_30.automation, None);

        for (target_sdk, state) in [
            (None, TargetSdkState::Missing),
            (Some("R"), TargetSdkState::NonNumeric),
        ] {
            let result = classify("android.permission.WRITE_EXTERNAL_STORAGE", target_sdk);
            assert_eq!(
                result.applicability,
                AndroidPermissionApplicability::Indeterminate(
                    AndroidPermissionIndeterminacy::ReplacementTargetSdkUnavailable {
                        minimum_device_api: 30,
                        minimum_target_sdk: 30,
                        state,
                    }
                )
            );
            assert_eq!(result.automation, None);
        }
    }

    #[test]
    fn android_permissions_legacy_read_storage_stays_restricted_through_api_32() {
        let result = classify("android.permission.READ_EXTERNAL_STORAGE", Some("35"));
        assert_eq!(
            result.classification,
            AndroidPermissionClassification::RuntimeRestricted
        );
        assert_eq!(
            result.api_bounds,
            Some(AndroidApiBounds {
                minimum: 23,
                maximum: Some(32),
            })
        );
        assert_eq!(result.automation, None);
    }

    #[test]
    fn android_permissions_target_rules_fail_closed_without_numeric_metadata() {
        for (target_sdk, state) in [
            (None, TargetSdkState::Missing),
            (Some("Tiramisu"), TargetSdkState::NonNumeric),
        ] {
            let result = classify("android.permission.READ_MEDIA_IMAGES", target_sdk);
            assert_eq!(
                result.applicability,
                AndroidPermissionApplicability::Indeterminate(
                    AndroidPermissionIndeterminacy::TargetSdkUnavailable {
                        minimum_target_sdk: 33,
                        state,
                    }
                )
            );
            assert_eq!(result.automation, None);
        }
    }

    #[test]
    fn android_permissions_restricted_and_manual_permissions_never_automate() {
        for name in [
            "android.permission.SEND_SMS",
            "android.permission.READ_CALL_LOG",
            "android.permission.ACCESS_BACKGROUND_LOCATION",
            "android.permission.BODY_SENSORS_BACKGROUND",
            "android.permission.READ_EXTERNAL_STORAGE",
            "android.permission.SYSTEM_ALERT_WINDOW",
            "android.permission.SCHEDULE_EXACT_ALARM",
            "android.permission.WRITE_SECURE_SETTINGS",
        ] {
            assert_eq!(classify(name, Some("37")).automation, None, "{name}");
        }
    }

    #[test]
    fn android_permissions_manage_external_storage_is_the_only_app_op_mapping() {
        let app_ops = PERMISSION_CATALOG
            .iter()
            .filter_map(|entry| match entry.automation {
                Some(AndroidPermissionAutomation::AppOp { app_op, .. }) => {
                    Some((entry.name, app_op))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            app_ops,
            vec![(
                "android.permission.MANAGE_EXTERNAL_STORAGE",
                "MANAGE_EXTERNAL_STORAGE"
            )]
        );
        let result = classify("android.permission.MANAGE_EXTERNAL_STORAGE", None);
        assert_eq!(
            result.api_bounds,
            Some(AndroidApiBounds {
                minimum: 30,
                maximum: None,
            })
        );
        assert_eq!(
            result.automation,
            Some(AndroidPermissionAutomation::AppOp {
                app_op: "MANAGE_EXTERNAL_STORAGE",
                mode: AndroidAppOpMode::Allow,
                requires_root: true,
            })
        );
    }
}
