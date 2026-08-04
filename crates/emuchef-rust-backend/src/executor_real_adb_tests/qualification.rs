use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FIXTURE_PACKAGE: &str = "com.emuchef.fixture";
pub const GLOBAL_OPT_IN: &str = "EMUCHEF_RUN_REAL_ADB_TESTS";
const SHARED_STORAGE_ROOT: &str = "/sdcard/EmuChefQualification/com.emuchef.fixture/";
const APP_SPECIFIC_EXTERNAL_STORAGE_ROOT: &str = "/sdcard/Android/data/com.emuchef.fixture/files/";
const STAGING_ROOT: &str = "/sdcard/EmuChefQualification/com.emuchef.fixture/staging/";
const DESTINATION_ROOT: &str = "/sdcard/EmuChefQualification/com.emuchef.fixture/output/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualificationGroup {
    InstallPackage,
    CopyExtraction,
    PermissionAppop,
    LaunchForceStop,
    CleanupFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationClassification {
    Supported,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationOutcome {
    pub classification: QualificationClassification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limitation: Option<&'static str>,
}

impl QualificationGroup {
    pub const ALL: [Self; 5] = [
        Self::InstallPackage,
        Self::CopyExtraction,
        Self::PermissionAppop,
        Self::LaunchForceStop,
        Self::CleanupFailure,
    ];

    pub const fn env_name(self) -> &'static str {
        match self {
            Self::InstallPackage => "EMUCHEF_RUN_REAL_ADB_INSTALL_PACKAGE_TESTS",
            Self::CopyExtraction => "EMUCHEF_RUN_REAL_ADB_COPY_EXTRACTION_TESTS",
            Self::PermissionAppop => "EMUCHEF_RUN_REAL_ADB_PERMISSION_APPOP_TESTS",
            Self::LaunchForceStop => "EMUCHEF_RUN_REAL_ADB_LAUNCH_FORCE_STOP_TESTS",
            Self::CleanupFailure => "EMUCHEF_RUN_REAL_ADB_CLEANUP_FAILURE_TESTS",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationContract {
    pub schema_version: u32,
    pub package_name: String,
    pub shared_storage_root: String,
    pub app_specific_external_storage_root: String,
    pub staging_root: String,
    pub destination_root: String,
    pub app_specific_external_storage_requires_capability: bool,
}

pub fn qualification_contract_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate should live beneath the repository root")
        .join("tests/fixtures/phase-6c/non-root/qualification-contract.json")
}

pub fn load_contract() -> Result<QualificationContract, String> {
    let bytes = std::fs::read(qualification_contract_path())
        .map_err(|_| "qualification contract is unavailable".to_string())?;
    let contract = serde_json::from_slice::<QualificationContract>(&bytes)
        .map_err(|_| "qualification contract is invalid".to_string())?;
    validate_contract(&contract)?;
    Ok(contract)
}

fn validate_contract(contract: &QualificationContract) -> Result<(), String> {
    if contract.schema_version != 1
        || contract.package_name != FIXTURE_PACKAGE
        || contract.shared_storage_root != SHARED_STORAGE_ROOT
        || contract.app_specific_external_storage_root != APP_SPECIFIC_EXTERNAL_STORAGE_ROOT
        || contract.staging_root != STAGING_ROOT
        || contract.destination_root != DESTINATION_ROOT
        || !contract.app_specific_external_storage_requires_capability
    {
        return Err(
            "qualification contract identity or fixed root authority is invalid".to_string(),
        );
    }
    for root in contract.owned_roots() {
        normalize_absolute_device_path(root)?;
    }
    Ok(())
}

pub fn validate_group_opt_ins(
    global_value: Option<&str>,
    enabled_groups: &[QualificationGroup],
    required: QualificationGroup,
) -> Result<(), String> {
    if global_value != Some("1") {
        return Err(format!("{GLOBAL_OPT_IN}=1 is required"));
    }
    if enabled_groups != [required] {
        return Err(format!(
            "exactly {}=1 must be enabled for this qualification case",
            required.env_name()
        ));
    }
    Ok(())
}

pub fn validate_package(package: &str, allowlist: Option<&str>) -> Result<(), String> {
    let system_like = package == "android"
        || package.starts_with("android.")
        || package.starts_with("com.android.")
        || package.starts_with("com.google.android.");
    if package != FIXTURE_PACKAGE || system_like {
        return Err("qualification package is not the exact fixture package".to_string());
    }
    if allowlist.map(str::trim) != Some(package) {
        return Err("fixture package is not exactly allowlisted".to_string());
    }
    Ok(())
}

pub fn classify_preflight(
    group: QualificationGroup,
    android_api_level: Option<u32>,
    package_manager_available: bool,
    activity_manager_available: bool,
    shared_storage_available: bool,
) -> QualificationOutcome {
    let limitation = if android_api_level.is_none_or(|value| value < 30) {
        Some("android_api_level_unsupported")
    } else if matches!(
        group,
        QualificationGroup::InstallPackage | QualificationGroup::PermissionAppop
    ) && !package_manager_available
    {
        Some("package_manager_unavailable")
    } else if group == QualificationGroup::LaunchForceStop && !activity_manager_available {
        Some("activity_manager_unavailable")
    } else if matches!(
        group,
        QualificationGroup::CopyExtraction | QualificationGroup::CleanupFailure
    ) && !shared_storage_available
    {
        Some("shared_storage_unavailable")
    } else {
        None
    };
    QualificationOutcome {
        classification: if limitation.is_some() {
            QualificationClassification::Unsupported
        } else {
            QualificationClassification::Supported
        },
        limitation,
    }
}

pub fn classify_permission_appop_support(
    camera_permission_available: bool,
    appop_available: bool,
) -> QualificationOutcome {
    let limitation = if !camera_permission_available {
        Some("camera_permission_unavailable")
    } else if !appop_available {
        Some("app_op_unavailable")
    } else {
        None
    };
    QualificationOutcome {
        classification: if limitation.is_some() {
            QualificationClassification::Unsupported
        } else {
            QualificationClassification::Supported
        },
        limitation,
    }
}

pub fn validate_owned_destination(
    contract: &QualificationContract,
    destination: &str,
    allow_root_equality: bool,
) -> Result<String, String> {
    validate_contract(contract)?;
    let destination = normalize_absolute_device_path(destination)?;
    let roots = contract
        .owned_roots()
        .into_iter()
        .map(normalize_absolute_device_path)
        .collect::<Result<Vec<_>, _>>()?;
    let equals_root = roots.contains(&destination);
    let owned = roots
        .iter()
        .any(|root| destination == *root || destination.starts_with(&format!("{root}/")));
    if !owned {
        return Err("qualification destination is outside manifest-owned roots".to_string());
    }
    if equals_root && !allow_root_equality {
        return Err("qualification destination must be below the owned root".to_string());
    }
    Ok(destination)
}

impl QualificationContract {
    fn owned_roots(&self) -> [&str; 4] {
        [
            &self.shared_storage_root,
            &self.app_specific_external_storage_root,
            &self.staging_root,
            &self.destination_root,
        ]
    }
}

fn normalize_absolute_device_path(path: &str) -> Result<String, String> {
    if !path.starts_with('/') || path == "/" || path.contains('\\') {
        return Err("qualification device path must be an absolute owned path".to_string());
    }
    let mut components = Vec::new();
    for component in path.split('/').skip(1) {
        match component {
            "" => {}
            "." | ".." => {
                return Err("qualification device path traversal is not allowed".to_string());
            }
            value if value.contains('\0') => {
                return Err("qualification device path contains an invalid component".to_string());
            }
            value => components.push(value),
        }
    }
    if components.is_empty() {
        return Err("qualification device path must name an owned path".to_string());
    }
    Ok(format!("/{}", components.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> QualificationContract {
        QualificationContract {
            schema_version: 1,
            package_name: FIXTURE_PACKAGE.to_string(),
            shared_storage_root: "/sdcard/EmuChefQualification/com.emuchef.fixture/".to_string(),
            app_specific_external_storage_root: "/sdcard/Android/data/com.emuchef.fixture/files/"
                .to_string(),
            staging_root: "/sdcard/EmuChefQualification/com.emuchef.fixture/staging/".to_string(),
            destination_root: "/sdcard/EmuChefQualification/com.emuchef.fixture/output/"
                .to_string(),
            app_specific_external_storage_requires_capability: true,
        }
    }

    #[test]
    fn fixed_contract_is_strict_and_names_only_fixture_owned_roots() {
        let actual = load_contract().unwrap();
        assert_eq!(actual, contract());
    }

    #[test]
    fn individual_tests_require_global_and_exactly_one_matching_group() {
        let required = QualificationGroup::CopyExtraction;
        assert!(validate_group_opt_ins(Some("1"), &[required], required).is_ok());
        assert!(validate_group_opt_ins(None, &[required], required).is_err());
        assert!(validate_group_opt_ins(Some("true"), &[required], required).is_err());
        assert!(validate_group_opt_ins(Some("1"), &[], required).is_err());
        assert!(validate_group_opt_ins(
            Some("1"),
            &[required, QualificationGroup::InstallPackage],
            required
        )
        .is_err());
        assert!(
            validate_group_opt_ins(Some("1"), &[QualificationGroup::InstallPackage], required)
                .is_err()
        );
    }

    #[test]
    fn package_guard_accepts_only_the_exact_allowlisted_fixture() {
        assert!(validate_package(FIXTURE_PACKAGE, Some(FIXTURE_PACKAGE)).is_ok());
        for package in [
            "android",
            "android.test",
            "com.android.settings",
            "com.google.android.gms",
            "com.example.other",
        ] {
            assert!(validate_package(package, Some(package)).is_err());
        }
        assert!(validate_package(FIXTURE_PACKAGE, None).is_err());
        assert!(validate_package(FIXTURE_PACKAGE, Some("com.example.other")).is_err());
        assert!(
            validate_package(FIXTURE_PACKAGE, Some("com.emuchef.fixture,com.example")).is_err()
        );
    }

    #[test]
    fn destination_guard_normalizes_children_and_rejects_unsafe_paths() {
        let contract = contract();
        assert_eq!(
            validate_owned_destination(
                &contract,
                "/sdcard/EmuChefQualification/com.emuchef.fixture/output/nested/file.txt",
                false,
            )
            .unwrap(),
            "/sdcard/EmuChefQualification/com.emuchef.fixture/output/nested/file.txt"
        );
        for destination in [
            "/sdcard/EmuChefQualification/com.emuchef.fixture/output/../escape",
            "/sdcard/Unrelated/file.txt",
            "relative/path",
            "/",
            "",
        ] {
            assert!(validate_owned_destination(&contract, destination, false).is_err());
        }
        assert!(validate_owned_destination(
            &contract,
            "/sdcard/EmuChefQualification/com.emuchef.fixture/output/",
            false,
        )
        .is_err());
        assert!(validate_owned_destination(
            &contract,
            "/sdcard/EmuChefQualification/com.emuchef.fixture/output/",
            true,
        )
        .is_ok());
    }

    #[test]
    fn malformed_contract_roots_are_rejected() {
        for invalid in [
            "../relative",
            "/sdcard/EmuChefQualification/com.emuchef.fixture/../escape/",
            "/",
            "",
        ] {
            let mut candidate = contract();
            candidate.destination_root = invalid.to_string();
            assert!(validate_owned_destination(
                &candidate,
                "/sdcard/EmuChefQualification/com.emuchef.fixture/output/file",
                false,
            )
            .is_err());
        }
    }

    #[test]
    fn contract_rejects_well_formed_roots_outside_the_fixed_fixture_namespaces() {
        let replacements = [
            ("shared", "/sdcard/EmuChefQualification/com.example.other/"),
            (
                "app-specific",
                "/sdcard/Android/data/com.example.other/files/",
            ),
            (
                "staging",
                "/sdcard/EmuChefQualification/com.example.other/staging/",
            ),
            (
                "destination",
                "/sdcard/EmuChefQualification/com.example.other/output/",
            ),
        ];
        for (field, replacement) in replacements {
            let mut candidate = contract();
            match field {
                "shared" => candidate.shared_storage_root = replacement.to_string(),
                "app-specific" => {
                    candidate.app_specific_external_storage_root = replacement.to_string()
                }
                "staging" => candidate.staging_root = replacement.to_string(),
                "destination" => candidate.destination_root = replacement.to_string(),
                _ => unreachable!("test field is fixed"),
            }
            assert!(
                validate_contract(&candidate).is_err(),
                "{field} root must be fixed"
            );
            assert!(
                validate_owned_destination(&candidate, replacement, true).is_err(),
                "{field} root must not become cleanup authority"
            );
        }
    }

    #[test]
    fn unsupported_capabilities_keep_a_distinct_sanitized_classification() {
        let cases = [
            (
                QualificationGroup::InstallPackage,
                None,
                true,
                true,
                true,
                "android_api_level_unsupported",
            ),
            (
                QualificationGroup::InstallPackage,
                Some(30),
                false,
                true,
                true,
                "package_manager_unavailable",
            ),
            (
                QualificationGroup::LaunchForceStop,
                Some(30),
                true,
                false,
                true,
                "activity_manager_unavailable",
            ),
            (
                QualificationGroup::CopyExtraction,
                Some(30),
                true,
                true,
                false,
                "shared_storage_unavailable",
            ),
        ];
        for (group, api, package_manager, activity_manager, storage, limitation) in cases {
            let outcome =
                classify_preflight(group, api, package_manager, activity_manager, storage);
            assert_eq!(
                outcome.classification,
                QualificationClassification::Unsupported
            );
            assert_eq!(outcome.limitation, Some(limitation));
            let projection = serde_json::to_value(outcome).unwrap();
            assert_eq!(projection["classification"], "unsupported");
            assert!(projection.get("deviceSerial").is_none());
            assert!(projection.get("hostPath").is_none());
        }
        assert_eq!(
            classify_preflight(
                QualificationGroup::PermissionAppop,
                Some(35),
                true,
                true,
                true,
            )
            .classification,
            QualificationClassification::Supported
        );
        let unavailable_permission = classify_permission_appop_support(false, true);
        assert_eq!(
            unavailable_permission.classification,
            QualificationClassification::Unsupported
        );
        assert_eq!(
            unavailable_permission.limitation,
            Some("camera_permission_unavailable")
        );
        let unavailable_appop = classify_permission_appop_support(true, false);
        assert_eq!(
            unavailable_appop.classification,
            QualificationClassification::Unsupported
        );
        assert_eq!(unavailable_appop.limitation, Some("app_op_unavailable"));
        assert_eq!(
            classify_permission_appop_support(true, true).classification,
            QualificationClassification::Supported
        );
        let projection = serde_json::to_value(unavailable_appop).unwrap();
        assert_eq!(projection["classification"], "unsupported");
        assert!(projection.get("deviceSerial").is_none());
        assert!(projection.get("hostPath").is_none());
    }
}
