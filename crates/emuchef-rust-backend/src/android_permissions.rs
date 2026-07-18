//! Exact Android permission catalog and deterministic classification rules.
//!
//! This module deliberately contains no ADB commands, executor integration, or
//! protocol types. It records only reviewed Android platform facts and pure
//! decisions that later phases may consume. Permission names are matched
//! exactly; an absent catalog entry always remains unknown.

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

/// Whether a declaration applies to the connected device and application.
///
/// `Indeterminate` is intentionally distinct from `NotApplicable`: missing or
/// non-numeric SDK metadata must disable automation without making an
/// unsupported claim about Android's effective permission behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidPermissionApplicability {
    Applicable,
    NotApplicable(AndroidPermissionNonApplicability),
    Indeterminate(AndroidPermissionIndeterminacy),
}

/// A proven reason that a declaration does not apply to the current context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AndroidPermissionNonApplicability {
    DeclarationRequiresApi23,
    MaxSdkVersionExceeded {
        maximum: u32,
    },
    PermissionNotIntroduced {
        introduction_api: u32,
    },
    PermissionReplaced {
        minimum_device_api: u32,
        minimum_target_sdk: u32,
    },
    TargetSdkBelowMinimum {
        minimum_target_sdk: u32,
    },
}

/// A reason the classifier cannot safely decide current applicability.
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

/// Explicit future automation metadata. This is data, not command generation.
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

/// All inputs required to classify one manifest declaration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AndroidPermissionClassificationContext<'a> {
    pub(crate) declaration: &'a ApkPermissionDeclaration,
    pub(crate) app_target_sdk: Option<&'a str>,
    pub(crate) connected_device_api: u32,
}

/// Stable backend-owned result for a single manifest declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClassifiedAndroidPermission {
    pub(crate) declaration: ApkPermissionDeclaration,
    pub(crate) classification: AndroidPermissionClassification,
    pub(crate) applicability: AndroidPermissionApplicability,
    /// Present only when the result is applicable and positively eligible for
    /// the cataloged future automation mechanism.
    pub(crate) automation: Option<AndroidPermissionAutomation>,
}

#[derive(Clone, Copy)]
struct PermissionCatalogEntry {
    name: &'static str,
    classification: AndroidPermissionClassification,
    introduction_api: u32,
    target_sdk_policy: TargetSdkPolicy,
    replacement: Option<ReplacementCondition>,
    automation: Option<AndroidPermissionAutomation>,
}

#[derive(Clone, Copy)]
enum TargetSdkPolicy {
    None,
    /// Dangerous permissions use runtime grants only on devices and apps using
    /// Android's runtime-permission model. Older contexts remain install-time.
    RuntimePermissionModel {
        minimum_device_api: u32,
        minimum_target_sdk: u32,
    },
    /// The permission is effective only for applications targeting at least
    /// the stated SDK. Unavailable target metadata must remain non-automated.
    MinimumApplicable {
        minimum_target_sdk: u32,
    },
}

/// A permission is replaced only when both thresholds are satisfied.
///
/// Keeping both values in the catalog prevents device-version-only replacement
/// decisions for target-SDK-gated Android behavior changes.
#[derive(Clone, Copy)]
struct ReplacementCondition {
    minimum_device_api: u32,
    minimum_target_sdk: u32,
}

const RUNTIME_GRANT_NO_ROOT: AndroidPermissionAutomation =
    AndroidPermissionAutomation::RuntimeGrant {
        requires_root: false,
    };

/// Deliberately small Phase 5B3 catalog. Each entry documents one reviewed
/// Android platform assumption; additional permissions require explicit review.
const PERMISSION_CATALOG: &[PermissionCatalogEntry] = &[
    PermissionCatalogEntry {
        // API 33 dangerous permission that is hard restricted by the installer.
        name: "android.permission.BODY_SENSORS_BACKGROUND",
        classification: AndroidPermissionClassification::RuntimeRestricted,
        introduction_api: 33,
        target_sdk_policy: TargetSdkPolicy::None,
        replacement: None,
        automation: None,
    },
    PermissionCatalogEntry {
        // API 1 dangerous permission; runtime granting requires the API 23 model.
        name: "android.permission.CAMERA",
        classification: AndroidPermissionClassification::RuntimeGrantable,
        introduction_api: 1,
        target_sdk_policy: TargetSdkPolicy::RuntimePermissionModel {
            minimum_device_api: RUNTIME_PERMISSIONS_API,
            minimum_target_sdk: RUNTIME_PERMISSIONS_API,
        },
        replacement: None,
        automation: Some(RUNTIME_GRANT_NO_ROOT),
    },
    PermissionCatalogEntry {
        // API 1 normal permission granted as part of installation.
        name: "android.permission.INTERNET",
        classification: AndroidPermissionClassification::InstallTime,
        introduction_api: 1,
        target_sdk_policy: TargetSdkPolicy::None,
        replacement: None,
        automation: None,
    },
    PermissionCatalogEntry {
        // API 30 special permission with the sole Phase 5B3 app-op mapping.
        name: "android.permission.MANAGE_EXTERNAL_STORAGE",
        classification: AndroidPermissionClassification::AppOpGrantable,
        introduction_api: 30,
        target_sdk_policy: TargetSdkPolicy::None,
        replacement: None,
        automation: Some(AndroidPermissionAutomation::AppOp {
            app_op: "MANAGE_EXTERNAL_STORAGE",
            mode: AndroidAppOpMode::Allow,
            requires_root: true,
        }),
    },
    PermissionCatalogEntry {
        // API 16 soft-restricted permission. Android 13 replaces it only for
        // applications that also target API 33 or newer.
        name: "android.permission.READ_EXTERNAL_STORAGE",
        classification: AndroidPermissionClassification::RuntimeRestricted,
        introduction_api: 16,
        target_sdk_policy: TargetSdkPolicy::None,
        replacement: Some(ReplacementCondition {
            minimum_device_api: 33,
            minimum_target_sdk: 33,
        }),
        automation: None,
    },
    PermissionCatalogEntry {
        // API 33 dangerous granular-media permission, effective for target 33+.
        name: "android.permission.READ_MEDIA_IMAGES",
        classification: AndroidPermissionClassification::RuntimeGrantable,
        introduction_api: 33,
        target_sdk_policy: TargetSdkPolicy::MinimumApplicable {
            minimum_target_sdk: 33,
        },
        replacement: None,
        automation: Some(RUNTIME_GRANT_NO_ROOT),
    },
    PermissionCatalogEntry {
        // API 1 Settings-mediated overlay access; never automated here.
        name: "android.permission.SYSTEM_ALERT_WINDOW",
        classification: AndroidPermissionClassification::ManualSpecialAccess,
        introduction_api: 1,
        target_sdk_policy: TargetSdkPolicy::None,
        replacement: None,
        automation: None,
    },
    PermissionCatalogEntry {
        // API 3 secure-settings permission unavailable to ordinary third-party apps.
        name: "android.permission.WRITE_SECURE_SETTINGS",
        classification: AndroidPermissionClassification::SignatureOrPrivileged,
        introduction_api: 3,
        target_sdk_policy: TargetSdkPolicy::None,
        replacement: None,
        automation: None,
    },
];

/// Classify declarations and return them in the manifest model's stable order.
pub(crate) fn classify_android_permissions(
    declarations: &[ApkPermissionDeclaration],
    app_target_sdk: Option<&str>,
    connected_device_api: u32,
) -> Vec<ClassifiedAndroidPermission> {
    let mut results = declarations
        .iter()
        .map(|declaration| {
            classify_android_permission(AndroidPermissionClassificationContext {
                declaration,
                app_target_sdk,
                connected_device_api,
            })
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.declaration.cmp(&right.declaration));
    results
}

/// Classify one declaration using only exact catalog data and numeric SDK rules.
pub(crate) fn classify_android_permission(
    context: AndroidPermissionClassificationContext<'_>,
) -> ClassifiedAndroidPermission {
    let catalog_entry = catalog_entry(&context.declaration.name);
    let mut classification = catalog_entry
        .map_or(AndroidPermissionClassification::Unknown, |entry| {
            entry.classification
        });

    let applicability = declaration_applicability(context, catalog_entry, &mut classification);
    let automation = catalog_entry
        .filter(|entry| {
            applicability == AndroidPermissionApplicability::Applicable
                && classification == entry.classification
        })
        .and_then(|entry| entry.automation);

    ClassifiedAndroidPermission {
        declaration: context.declaration.clone(),
        classification,
        applicability,
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
) -> AndroidPermissionApplicability {
    if context.declaration.kind == ApkPermissionDeclarationKind::UsesPermissionSdk23
        && context.connected_device_api < RUNTIME_PERMISSIONS_API
    {
        return AndroidPermissionApplicability::NotApplicable(
            AndroidPermissionNonApplicability::DeclarationRequiresApi23,
        );
    }

    if let Some(max_sdk_version) = context.declaration.max_sdk_version.as_deref() {
        let Ok(maximum) = max_sdk_version.parse::<u32>() else {
            return AndroidPermissionApplicability::Indeterminate(
                AndroidPermissionIndeterminacy::InvalidMaxSdkVersion,
            );
        };
        if maximum < context.connected_device_api {
            return AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::MaxSdkVersionExceeded { maximum },
            );
        }
    }

    let Some(catalog_entry) = catalog_entry else {
        return AndroidPermissionApplicability::Applicable;
    };

    if context.connected_device_api < catalog_entry.introduction_api {
        return AndroidPermissionApplicability::NotApplicable(
            AndroidPermissionNonApplicability::PermissionNotIntroduced {
                introduction_api: catalog_entry.introduction_api,
            },
        );
    }

    if let Some(replacement) = catalog_entry.replacement {
        if context.connected_device_api >= replacement.minimum_device_api {
            match numeric_target_sdk(context.app_target_sdk) {
                Ok(target_sdk) if target_sdk >= replacement.minimum_target_sdk => {
                    return AndroidPermissionApplicability::NotApplicable(
                        AndroidPermissionNonApplicability::PermissionReplaced {
                            minimum_device_api: replacement.minimum_device_api,
                            minimum_target_sdk: replacement.minimum_target_sdk,
                        },
                    );
                }
                Ok(_) => {}
                Err(state) => {
                    return AndroidPermissionApplicability::Indeterminate(
                        AndroidPermissionIndeterminacy::ReplacementTargetSdkUnavailable {
                            minimum_device_api: replacement.minimum_device_api,
                            minimum_target_sdk: replacement.minimum_target_sdk,
                            state,
                        },
                    );
                }
            }
        }
    }

    match catalog_entry.target_sdk_policy {
        TargetSdkPolicy::None => AndroidPermissionApplicability::Applicable,
        TargetSdkPolicy::RuntimePermissionModel {
            minimum_device_api,
            minimum_target_sdk,
        } => {
            if context.connected_device_api < minimum_device_api {
                *classification = AndroidPermissionClassification::InstallTime;
                return AndroidPermissionApplicability::Applicable;
            }
            match numeric_target_sdk(context.app_target_sdk) {
                Ok(target_sdk) if target_sdk >= minimum_target_sdk => {
                    AndroidPermissionApplicability::Applicable
                }
                Ok(_) => {
                    *classification = AndroidPermissionClassification::InstallTime;
                    AndroidPermissionApplicability::Applicable
                }
                Err(state) => {
                    *classification = AndroidPermissionClassification::Unknown;
                    AndroidPermissionApplicability::Indeterminate(
                        AndroidPermissionIndeterminacy::TargetSdkUnavailable {
                            minimum_target_sdk,
                            state,
                        },
                    )
                }
            }
        }
        TargetSdkPolicy::MinimumApplicable { minimum_target_sdk } => {
            match numeric_target_sdk(context.app_target_sdk) {
                Ok(target_sdk) if target_sdk >= minimum_target_sdk => {
                    AndroidPermissionApplicability::Applicable
                }
                Ok(_) => AndroidPermissionApplicability::NotApplicable(
                    AndroidPermissionNonApplicability::TargetSdkBelowMinimum { minimum_target_sdk },
                ),
                Err(state) => {
                    *classification = AndroidPermissionClassification::Unknown;
                    AndroidPermissionApplicability::Indeterminate(
                        AndroidPermissionIndeterminacy::TargetSdkUnavailable {
                            minimum_target_sdk,
                            state,
                        },
                    )
                }
            }
        }
    }
}

fn numeric_target_sdk(target_sdk: Option<&str>) -> Result<u32, TargetSdkState> {
    let target_sdk = target_sdk.ok_or(TargetSdkState::Missing)?;
    target_sdk
        .parse::<u32>()
        .map_err(|_| TargetSdkState::NonNumeric)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(name: &str) -> ApkPermissionDeclaration {
        ApkPermissionDeclaration {
            name: name.to_string(),
            kind: ApkPermissionDeclarationKind::UsesPermission,
            max_sdk_version: None,
        }
    }

    fn classify(
        name: &str,
        target_sdk: Option<&str>,
        device_api: u32,
    ) -> ClassifiedAndroidPermission {
        let declaration = declaration(name);
        classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: target_sdk,
            connected_device_api: device_api,
        })
    }

    #[test]
    fn android_permissions_catalog_covers_every_classification() {
        let cases = [
            (
                "android.permission.CAMERA",
                Some("35"),
                35,
                AndroidPermissionClassification::RuntimeGrantable,
            ),
            (
                "android.permission.BODY_SENSORS_BACKGROUND",
                Some("35"),
                35,
                AndroidPermissionClassification::RuntimeRestricted,
            ),
            (
                "android.permission.MANAGE_EXTERNAL_STORAGE",
                Some("35"),
                35,
                AndroidPermissionClassification::AppOpGrantable,
            ),
            (
                "android.permission.SYSTEM_ALERT_WINDOW",
                Some("35"),
                35,
                AndroidPermissionClassification::ManualSpecialAccess,
            ),
            (
                "android.permission.INTERNET",
                Some("35"),
                35,
                AndroidPermissionClassification::InstallTime,
            ),
            (
                "android.permission.WRITE_SECURE_SETTINGS",
                Some("35"),
                35,
                AndroidPermissionClassification::SignatureOrPrivileged,
            ),
            (
                "com.example.permission.CUSTOM",
                Some("35"),
                35,
                AndroidPermissionClassification::Unknown,
            ),
        ];

        for (name, target_sdk, device_api, expected) in cases {
            assert_eq!(
                classify(name, target_sdk, device_api).classification,
                expected,
                "unexpected classification for {name}"
            );
        }
    }

    #[test]
    fn android_permissions_unknown_names_are_exact_and_never_automated() {
        for name in [
            "android.permission.CAMERA_EXTRA",
            "prefix.android.permission.CAMERA",
            "manage_external_storage",
        ] {
            let result = classify(name, Some("35"), 35);
            assert_eq!(
                result.classification,
                AndroidPermissionClassification::Unknown
            );
            assert_eq!(
                result.applicability,
                AndroidPermissionApplicability::Applicable
            );
            assert_eq!(result.automation, None);
        }
    }

    #[test]
    fn android_permissions_manage_external_storage_has_the_only_app_op_mapping() {
        let applicable = classify("android.permission.MANAGE_EXTERNAL_STORAGE", Some("30"), 30);
        assert_eq!(
            applicable.classification,
            AndroidPermissionClassification::AppOpGrantable
        );
        assert_eq!(
            applicable.applicability,
            AndroidPermissionApplicability::Applicable
        );
        assert_eq!(
            applicable.automation,
            Some(AndroidPermissionAutomation::AppOp {
                app_op: "MANAGE_EXTERNAL_STORAGE",
                mode: AndroidAppOpMode::Allow,
                requires_root: true,
            })
        );

        let below_introduction =
            classify("android.permission.MANAGE_EXTERNAL_STORAGE", Some("30"), 29);
        assert_eq!(
            below_introduction.classification,
            AndroidPermissionClassification::AppOpGrantable
        );
        assert_eq!(
            below_introduction.applicability,
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::PermissionNotIntroduced {
                    introduction_api: 30,
                }
            )
        );
        assert_eq!(below_introduction.automation, None);

        assert_eq!(
            PERMISSION_CATALOG
                .iter()
                .filter(|entry| matches!(
                    entry.automation,
                    Some(AndroidPermissionAutomation::AppOp { .. })
                ))
                .count(),
            1
        );
    }

    #[test]
    fn android_permissions_sdk_23_declaration_is_not_applicable_below_api_23() {
        let declaration = ApkPermissionDeclaration {
            name: "android.permission.CAMERA".to_string(),
            kind: ApkPermissionDeclarationKind::UsesPermissionSdk23,
            max_sdk_version: None,
        };
        let result = classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: Some("23"),
            connected_device_api: 22,
        });

        assert_eq!(
            result.classification,
            AndroidPermissionClassification::RuntimeGrantable
        );
        assert_eq!(
            result.applicability,
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::DeclarationRequiresApi23
            )
        );
        assert_eq!(result.automation, None);
    }

    #[test]
    fn android_permissions_max_sdk_is_inclusive_and_expires_afterward() {
        let declaration = ApkPermissionDeclaration {
            name: "android.permission.CAMERA".to_string(),
            kind: ApkPermissionDeclarationKind::UsesPermission,
            max_sdk_version: Some("33".to_string()),
        };

        let at_maximum = classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: Some("33"),
            connected_device_api: 33,
        });
        assert_eq!(
            at_maximum.applicability,
            AndroidPermissionApplicability::Applicable
        );
        assert!(at_maximum.automation.is_some());

        let after_maximum = classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: Some("33"),
            connected_device_api: 34,
        });
        assert_eq!(
            after_maximum.applicability,
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::MaxSdkVersionExceeded { maximum: 33 }
            )
        );
        assert_eq!(after_maximum.automation, None);
    }

    #[test]
    fn android_permissions_invalid_direct_max_sdk_fails_closed() {
        let declaration = ApkPermissionDeclaration {
            name: "android.permission.CAMERA".to_string(),
            kind: ApkPermissionDeclarationKind::UsesPermission,
            max_sdk_version: Some("Tiramisu".to_string()),
        };
        let result = classify_android_permission(AndroidPermissionClassificationContext {
            declaration: &declaration,
            app_target_sdk: Some("33"),
            connected_device_api: 33,
        });

        assert_eq!(
            result.applicability,
            AndroidPermissionApplicability::Indeterminate(
                AndroidPermissionIndeterminacy::InvalidMaxSdkVersion
            )
        );
        assert_eq!(result.automation, None);
    }

    #[test]
    fn android_permissions_introduction_api_is_inclusive() {
        let before = classify("android.permission.BODY_SENSORS_BACKGROUND", Some("33"), 32);
        assert_eq!(
            before.applicability,
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::PermissionNotIntroduced {
                    introduction_api: 33,
                }
            )
        );

        let introduced = classify("android.permission.BODY_SENSORS_BACKGROUND", Some("33"), 33);
        assert_eq!(
            introduced.applicability,
            AndroidPermissionApplicability::Applicable
        );
    }

    #[test]
    fn android_permissions_camera_uses_numeric_runtime_model_boundaries() {
        let device_before_runtime_model = classify("android.permission.CAMERA", Some("35"), 22);
        assert_eq!(
            device_before_runtime_model.classification,
            AndroidPermissionClassification::InstallTime
        );
        assert_eq!(device_before_runtime_model.automation, None);

        let legacy_target = classify("android.permission.CAMERA", Some("22"), 23);
        assert_eq!(
            legacy_target.classification,
            AndroidPermissionClassification::InstallTime
        );
        assert_eq!(legacy_target.automation, None);

        let runtime_target = classify("android.permission.CAMERA", Some("23"), 23);
        assert_eq!(
            runtime_target.classification,
            AndroidPermissionClassification::RuntimeGrantable
        );
        assert_eq!(runtime_target.automation, Some(RUNTIME_GRANT_NO_ROOT));
    }

    #[test]
    fn android_permissions_camera_unavailable_target_falls_back_to_unknown() {
        for (target_sdk, state) in [
            (None, TargetSdkState::Missing),
            (Some("VanillaIceCream"), TargetSdkState::NonNumeric),
        ] {
            let result = classify("android.permission.CAMERA", target_sdk, 35);
            assert_eq!(
                result.classification,
                AndroidPermissionClassification::Unknown
            );
            assert_eq!(
                result.applicability,
                AndroidPermissionApplicability::Indeterminate(
                    AndroidPermissionIndeterminacy::TargetSdkUnavailable {
                        minimum_target_sdk: 23,
                        state,
                    }
                )
            );
            assert_eq!(result.automation, None);
        }
    }

    #[test]
    fn android_permissions_read_external_storage_replacement_is_compound() {
        let cases = [
            (32, Some("33"), AndroidPermissionApplicability::Applicable),
            (33, Some("32"), AndroidPermissionApplicability::Applicable),
            (
                33,
                Some("33"),
                AndroidPermissionApplicability::NotApplicable(
                    AndroidPermissionNonApplicability::PermissionReplaced {
                        minimum_device_api: 33,
                        minimum_target_sdk: 33,
                    },
                ),
            ),
            (
                33,
                None,
                AndroidPermissionApplicability::Indeterminate(
                    AndroidPermissionIndeterminacy::ReplacementTargetSdkUnavailable {
                        minimum_device_api: 33,
                        minimum_target_sdk: 33,
                        state: TargetSdkState::Missing,
                    },
                ),
            ),
            (
                33,
                Some("VanillaIceCream"),
                AndroidPermissionApplicability::Indeterminate(
                    AndroidPermissionIndeterminacy::ReplacementTargetSdkUnavailable {
                        minimum_device_api: 33,
                        minimum_target_sdk: 33,
                        state: TargetSdkState::NonNumeric,
                    },
                ),
            ),
        ];

        for (device_api, target_sdk, expected_applicability) in cases {
            let result = classify(
                "android.permission.READ_EXTERNAL_STORAGE",
                target_sdk,
                device_api,
            );
            assert_eq!(
                result.classification,
                AndroidPermissionClassification::RuntimeRestricted
            );
            assert_eq!(result.applicability, expected_applicability);
            assert_eq!(result.automation, None);
        }
    }

    #[test]
    fn android_permissions_read_media_images_requires_device_and_numeric_target_33() {
        let before_introduction = classify("android.permission.READ_MEDIA_IMAGES", Some("33"), 32);
        assert_eq!(
            before_introduction.applicability,
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::PermissionNotIntroduced {
                    introduction_api: 33,
                }
            )
        );
        assert_eq!(before_introduction.automation, None);

        let legacy_target = classify("android.permission.READ_MEDIA_IMAGES", Some("32"), 33);
        assert_eq!(
            legacy_target.classification,
            AndroidPermissionClassification::RuntimeGrantable
        );
        assert_eq!(
            legacy_target.applicability,
            AndroidPermissionApplicability::NotApplicable(
                AndroidPermissionNonApplicability::TargetSdkBelowMinimum {
                    minimum_target_sdk: 33,
                }
            )
        );
        assert_eq!(legacy_target.automation, None);

        let eligible = classify("android.permission.READ_MEDIA_IMAGES", Some("33"), 33);
        assert_eq!(
            eligible.classification,
            AndroidPermissionClassification::RuntimeGrantable
        );
        assert_eq!(
            eligible.applicability,
            AndroidPermissionApplicability::Applicable
        );
        assert_eq!(eligible.automation, Some(RUNTIME_GRANT_NO_ROOT));

        for (target_sdk, state) in [
            (None, TargetSdkState::Missing),
            (Some("VanillaIceCream"), TargetSdkState::NonNumeric),
        ] {
            let unresolved = classify("android.permission.READ_MEDIA_IMAGES", target_sdk, 33);
            assert_eq!(
                unresolved.classification,
                AndroidPermissionClassification::Unknown
            );
            assert_eq!(
                unresolved.applicability,
                AndroidPermissionApplicability::Indeterminate(
                    AndroidPermissionIndeterminacy::TargetSdkUnavailable {
                        minimum_target_sdk: 33,
                        state,
                    }
                )
            );
            assert_eq!(unresolved.automation, None);
        }
    }

    #[test]
    fn android_permissions_results_use_manifest_order_without_deduplication() {
        let declarations = [
            ApkPermissionDeclaration {
                name: "android.permission.CAMERA".to_string(),
                kind: ApkPermissionDeclarationKind::UsesPermissionSdk23,
                max_sdk_version: Some("34".to_string()),
            },
            declaration("android.permission.INTERNET"),
            declaration("android.permission.CAMERA"),
            declaration("android.permission.CAMERA"),
        ];

        let results = classify_android_permissions(&declarations, Some("33"), 33);
        assert_eq!(results.len(), declarations.len());
        assert_eq!(
            results
                .iter()
                .map(|result| &result.declaration)
                .collect::<Vec<_>>(),
            [
                &declarations[2],
                &declarations[3],
                &declarations[0],
                &declarations[1],
            ]
        );
    }
}
