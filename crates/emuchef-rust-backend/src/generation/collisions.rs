//! Deterministic device-profile collision analysis for authored roots.

use std::fs;
use std::path::Path;

use regex::Regex;
use serde::Serialize;

use crate::authored_models::{load_device_profile, DeviceProfileV1};

use super::device_profile::SafeDetectedDeviceFacts;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum CollisionSeverity {
    Blocking,
    Warning,
}

/// One stable collision or incomplete-scan diagnostic.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollisionDiagnostic {
    severity: CollisionSeverity,
    code: String,
    message: String,
    existing_profile_id: Option<String>,
    file_name: Option<String>,
}

/// Complete collision analysis result for one proposed device profile.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollisionCheckResult {
    pub collisions: Vec<CollisionDiagnostic>,
    pub blocking: bool,
}

/// Scan an authored root without modifying it.
pub(crate) fn check_device_profile_collisions(
    authored_root: &Path,
    facts: &SafeDetectedDeviceFacts,
    proposed: &DeviceProfileV1,
) -> CollisionCheckResult {
    let directory = authored_root.join("device_profiles");
    let mut collisions = Vec::new();
    let proposed_file_name = format!("{}.yaml", proposed.id);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(_) => {
            collisions.push(blocking_scan_diagnostic(None));
            return CollisionCheckResult {
                collisions,
                blocking: true,
            };
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                collisions.push(blocking_scan_diagnostic(None));
                continue;
            }
        };
        let path = entry.path();
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
            })
        {
            paths.push(path);
        }
    }
    paths.sort();

    for path in paths {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(ToString::to_string);
        if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file()) {
            collisions.push(blocking_scan_diagnostic(file_name));
            continue;
        }
        if file_name.as_deref() == Some(proposed_file_name.as_str()) {
            collisions.push(CollisionDiagnostic {
                severity: CollisionSeverity::Blocking,
                code: "device_profile_destination_conflict".to_string(),
                message: "The proposed device-profile destination already exists.".to_string(),
                existing_profile_id: None,
                file_name: file_name.clone(),
            });
        }
        let existing = match load_device_profile(&path) {
            Ok(profile) => profile,
            Err(_) => {
                collisions.push(blocking_scan_diagnostic(file_name));
                continue;
            }
        };
        if existing.id == proposed.id {
            collisions.push(CollisionDiagnostic {
                severity: CollisionSeverity::Blocking,
                code: "device_profile_id_conflict".to_string(),
                message: "A device profile with the proposed id already exists.".to_string(),
                existing_profile_id: Some(existing.id.clone()),
                file_name: file_name.clone(),
            });
        }
        if has_identical_model_pattern(&existing, proposed) {
            collisions.push(CollisionDiagnostic {
                severity: CollisionSeverity::Warning,
                code: "device_profile_model_pattern_overlap".to_string(),
                message: "An existing device profile uses an identical model pattern.".to_string(),
                existing_profile_id: Some(existing.id.clone()),
                file_name: file_name.clone(),
            });
        } else if profile_matches_facts(&existing, facts) && profile_matches_facts(proposed, facts)
        {
            collisions.push(CollisionDiagnostic {
                severity: CollisionSeverity::Warning,
                code: "device_profile_match_overlap".to_string(),
                message: "An existing device profile matches the same detected manufacturer, brand, and model facts.".to_string(),
                existing_profile_id: Some(existing.id.clone()),
                file_name,
            });
        }
    }

    collisions.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.existing_profile_id.cmp(&right.existing_profile_id))
            .then_with(|| left.file_name.cmp(&right.file_name))
    });
    let blocking = collisions
        .iter()
        .any(|collision| collision.severity == CollisionSeverity::Blocking);
    CollisionCheckResult {
        collisions,
        blocking,
    }
}

fn blocking_scan_diagnostic(file_name: Option<String>) -> CollisionDiagnostic {
    CollisionDiagnostic {
        severity: CollisionSeverity::Blocking,
        code: "device_profile_collision_scan_incomplete".to_string(),
        message: "The device-profile collision scan could not be completed.".to_string(),
        existing_profile_id: None,
        file_name,
    }
}

fn has_identical_model_pattern(existing: &DeviceProfileV1, proposed: &DeviceProfileV1) -> bool {
    existing
        .match_criteria
        .model_patterns
        .iter()
        .any(|pattern| {
            proposed
                .match_criteria
                .model_patterns
                .iter()
                .any(|candidate| candidate == pattern)
        })
}

fn profile_matches_facts(profile: &DeviceProfileV1, facts: &SafeDetectedDeviceFacts) -> bool {
    contains_constraint_matches(
        &profile.match_criteria.manufacturer_contains,
        facts.manufacturer.as_deref(),
    ) && contains_constraint_matches(
        &profile.match_criteria.brand_contains,
        facts.brand.as_deref(),
    ) && pattern_constraint_matches(
        &profile.match_criteria.model_patterns,
        facts.model.as_deref(),
    )
}

fn contains_constraint_matches(expected: &[String], actual: Option<&str>) -> bool {
    if expected.is_empty() {
        return true;
    }
    let Some(actual) = actual else {
        return false;
    };
    let actual = actual.to_lowercase();
    expected
        .iter()
        .any(|token| actual.contains(&token.to_lowercase()))
}

fn pattern_constraint_matches(patterns: &[String], actual: Option<&str>) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let Some(actual) = actual else {
        return false;
    };
    patterns
        .iter()
        .any(|pattern| Regex::new(pattern).is_ok_and(|regex| regex.is_match(actual)))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::authored_models::{
        AndroidVersionRange, DeviceCapabilityDefaults, DeviceMatchCriteria, OrderedValueMap,
    };

    fn profile(id: &str, model_pattern: &str) -> DeviceProfileV1 {
        DeviceProfileV1 {
            schema_version: 1,
            kind: "device_profile".to_string(),
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            match_criteria: DeviceMatchCriteria {
                manufacturer_contains: vec!["AYANEO".to_string()],
                brand_contains: vec!["AYANEO".to_string()],
                model_patterns: vec![model_pattern.to_string()],
                android_version: Some(AndroidVersionRange {
                    min: Some(13),
                    max: None,
                }),
            },
            capability_defaults: DeviceCapabilityDefaults {
                adb_available: true,
                apk_install: true,
                shared_storage_write: true,
                app_launch: true,
                shell_command: true,
                package_remove_for_user: false,
                root_shell: false,
                app_data_write: false,
            },
            device_tags: Vec::new(),
            metadata: OrderedValueMap::new(),
        }
    }

    fn facts() -> SafeDetectedDeviceFacts {
        SafeDetectedDeviceFacts {
            manufacturer: Some("AYANEO".to_string()),
            brand: Some("AYANEO".to_string()),
            model: Some("Pocket S Mini".to_string()),
            ..SafeDetectedDeviceFacts::default()
        }
    }

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("emuchef-{label}-{nonce}"));
        fs::create_dir_all(root.join("device_profiles")).unwrap();
        root
    }

    #[test]
    fn id_destination_and_pattern_conflicts_are_stably_classified() {
        let root = temp_root("collisions");
        let existing = profile("ayaneo.pocket_s_mini", "^Pocket S Mini$");
        fs::write(
            root.join("device_profiles/ayaneo.pocket_s_mini.yaml"),
            crate::authored_models::emit_device_profile_yaml(&existing).unwrap(),
        )
        .unwrap();
        let result = check_device_profile_collisions(&root, &facts(), &existing);
        assert!(result.blocking);
        assert_eq!(result.collisions[0].severity, CollisionSeverity::Blocking);
        assert!(result
            .collisions
            .iter()
            .any(|item| item.code == "device_profile_id_conflict"));
        assert!(result
            .collisions
            .iter()
            .any(|item| item.code == "device_profile_destination_conflict"));
        assert!(result
            .collisions
            .iter()
            .any(|item| item.code == "device_profile_model_pattern_overlap"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_scan_is_blocking_but_returns_a_normal_result() {
        let root = temp_root("incomplete-collision-scan");
        fs::write(root.join("device_profiles/broken.yaml"), "not: [valid").unwrap();
        let result =
            check_device_profile_collisions(&root, &facts(), &profile("new.profile", "^New$"));
        assert!(result.blocking);
        assert_eq!(
            result.collisions[0].code,
            "device_profile_collision_scan_incomplete"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn matching_facts_warn_without_blocking() {
        let root = temp_root("overlap");
        let existing = profile("existing.profile", "Pocket.*");
        fs::write(
            root.join("device_profiles/existing.yaml"),
            crate::authored_models::emit_device_profile_yaml(&existing).unwrap(),
        )
        .unwrap();
        let proposed = profile("new.profile", "^Pocket S Mini$");
        let result = check_device_profile_collisions(&root, &facts(), &proposed);
        assert!(!result.blocking);
        assert_eq!(result.collisions[0].code, "device_profile_match_overlap");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn profile_symlinks_make_the_scan_blocking_without_following_them() {
        use std::os::unix::fs::symlink;

        let root = temp_root("collision-symlink");
        let outside = temp_root("collision-symlink-outside");
        let outside_file = outside.join("outside.yaml");
        fs::write(&outside_file, "not a device profile").unwrap();
        symlink(&outside_file, root.join("device_profiles/linked.yaml")).unwrap();
        let result =
            check_device_profile_collisions(&root, &facts(), &profile("new.profile", "^New$"));
        assert!(result.blocking);
        assert_eq!(
            result.collisions[0].code,
            "device_profile_collision_scan_incomplete"
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
