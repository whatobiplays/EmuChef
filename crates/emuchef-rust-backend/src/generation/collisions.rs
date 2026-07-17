//! Deterministic device-profile collision analysis for authored roots.

use std::fs;
use std::path::Path;

use regex::Regex;
use serde::Serialize;

use serde::Deserialize;
use serde_json::Value;

use crate::authored_models::{
    load_app_definition, load_device_profile, AppDefinitionV1, DeviceProfileV1,
};

use super::device_profile::SafeDetectedDeviceFacts;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppRecipeCollisionRequest {
    pub app: AppDefinitionV1,
    pub recipe_id: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppRecipeCollisionDiagnostic {
    severity: CollisionSeverity,
    code: String,
    message: String,
    existing_id: Option<String>,
    relative_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppRecipeCollisionCheckResult {
    pub collisions: Vec<AppRecipeCollisionDiagnostic>,
    pub blocking: bool,
}

/// Scan top-level app and recipe documents for deterministic Phase 3 conflicts.
pub(crate) fn check_app_recipe_collisions(
    authored_root: &Path,
    request: &AppRecipeCollisionRequest,
) -> AppRecipeCollisionCheckResult {
    let mut collisions = Vec::new();
    scan_apps(authored_root, request, &mut collisions);
    scan_recipes(authored_root, request, &mut collisions);
    collisions.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.existing_id.cmp(&right.existing_id))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    let blocking = collisions
        .iter()
        .any(|collision| collision.severity == CollisionSeverity::Blocking);
    AppRecipeCollisionCheckResult {
        collisions,
        blocking,
    }
}

fn scan_apps(
    authored_root: &Path,
    request: &AppRecipeCollisionRequest,
    collisions: &mut Vec<AppRecipeCollisionDiagnostic>,
) {
    let directory = authored_root.join("apps");
    let proposed_file = format!("{}.yaml", request.app.id);
    for path in yaml_paths(
        &directory,
        "app_collision_scan_incomplete",
        "apps",
        collisions,
    ) {
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string);
        let relative_path = file_name.as_ref().map(|name| format!("apps/{name}"));
        if file_name.as_deref() == Some(proposed_file.as_str()) {
            collisions.push(app_collision(
                CollisionSeverity::Blocking,
                "app_destination_conflict",
                "The proposed app-definition destination already exists.",
                None,
                relative_path.clone(),
            ));
        }
        let existing = match load_app_definition(&path) {
            Ok(value) => value,
            Err(_) => {
                collisions.push(app_collision(
                    CollisionSeverity::Blocking,
                    "app_collision_scan_incomplete",
                    "The app-definition collision scan could not be completed.",
                    None,
                    relative_path,
                ));
                continue;
            }
        };
        if existing.id == request.app.id {
            collisions.push(app_collision(
                CollisionSeverity::Blocking,
                "app_id_conflict",
                "An app definition with the proposed id already exists.",
                Some(existing.id.clone()),
                relative_path.clone(),
            ));
        } else if existing.package.primary == request.app.package.primary {
            collisions.push(app_collision(
                CollisionSeverity::Warning,
                "app_package_overlap",
                "An app definition with a different id uses the same primary package.",
                Some(existing.id.clone()),
                relative_path.clone(),
            ));
        }
        let proposed_repository = source_string(&request.app, "repository");
        let existing_repository = source_string(&existing, "repository");
        let proposed_release_tag = source_string(&request.app, "release_tag");
        let existing_release_tag = source_string(&existing, "release_tag");
        let proposed_latest_policy = latest_policy_fingerprint(&request.app);
        let existing_latest_policy = latest_policy_fingerprint(&existing);
        if existing.id != request.app.id
            && proposed_latest_policy.is_some()
            && proposed_latest_policy == existing_latest_policy
        {
            collisions.push(app_collision(
                CollisionSeverity::Blocking,
                "app_latest_policy_conflict",
                "An app definition with a different id uses the same latest-release policy.",
                Some(existing.id.clone()),
                relative_path.clone(),
            ));
        }
        if existing.id != request.app.id
            && proposed_repository.is_some()
            && proposed_repository == existing_repository
        {
            collisions.push(app_collision(
                CollisionSeverity::Warning,
                "app_source_repository_overlap",
                "An app definition with a different id tracks the same release-provider repository.",
                Some(existing.id.clone()),
                relative_path.clone(),
            ));
        }
        if existing.id != request.app.id
            && proposed_repository.is_some()
            && proposed_repository == existing_repository
            && proposed_release_tag.is_some()
            && proposed_release_tag == existing_release_tag
        {
            collisions.push(app_collision(
                CollisionSeverity::Warning,
                "app_source_release_overlap",
                "An app definition with a different id tracks the same release-provider release.",
                Some(existing.id.clone()),
                relative_path.clone(),
            ));
        }
        let proposed_url = source_string(&request.app, "url");
        let existing_url = source_string(&existing, "url");
        if existing.id != request.app.id && proposed_url.is_some() && proposed_url == existing_url {
            let provider_release = request.app.tracking_source.type_name == "provider_release"
                || request.app.tracking_source.type_name == "github_release";
            collisions.push(app_collision(
                CollisionSeverity::Warning,
                "app_source_url_overlap",
                if provider_release {
                    "An app definition with a different id uses the same release-provider APK asset."
                } else {
                    "An app definition with a different id uses the same remote APK URL."
                },
                Some(existing.id.clone()),
                relative_path.clone(),
            ));
        }
        let comparable_source = !request.app.install_source.options.is_empty()
            || !request.app.tracking_source.fields.is_empty();
        if existing.id != request.app.id
            && comparable_source
            && existing.install_source == request.app.install_source
            && existing.tracking_source == request.app.tracking_source
        {
            collisions.push(app_collision(
                CollisionSeverity::Warning,
                "app_source_overlap",
                "An app definition with a different id uses matching non-empty source metadata.",
                Some(existing.id),
                relative_path,
            ));
        }
    }
}

fn source_string<'a>(app: &'a AppDefinitionV1, key: &str) -> Option<&'a str> {
    app.install_source
        .options
        .get(key)
        .or_else(|| app.tracking_source.fields.get(key))
        .and_then(Value::as_str)
}

fn latest_policy_fingerprint(app: &AppDefinitionV1) -> Option<String> {
    if !matches!(
        app.install_source.resolver.as_str(),
        "github_latest_release" | "provider_latest_release"
    ) {
        return None;
    }
    let provider = source_string(app, "provider").unwrap_or("github");
    let base_url = source_string(app, "base_url").unwrap_or("https://github.com");
    let repository = source_string(app, "repository")?;
    let pattern = source_string(app, "asset_pattern")?;
    let include_prereleases = app
        .install_source
        .options
        .get("include_prereleases")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(format!(
        "provider_latest_release\nprovider={provider}\nbase_url={base_url}\nrepository={repository}\nasset_pattern={pattern}\ninclude_prereleases={include_prereleases}"
    ))
}

fn scan_recipes(
    authored_root: &Path,
    request: &AppRecipeCollisionRequest,
    collisions: &mut Vec<AppRecipeCollisionDiagnostic>,
) {
    let directory = authored_root.join("recipes");
    let proposed_file = format!("{}.yaml", request.recipe_id);
    for path in yaml_paths(
        &directory,
        "recipe_collision_scan_incomplete",
        "recipes",
        collisions,
    ) {
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string);
        let relative_path = file_name.as_ref().map(|name| format!("recipes/{name}"));
        if file_name.as_deref() == Some(proposed_file.as_str()) {
            collisions.push(app_collision(
                CollisionSeverity::Blocking,
                "recipe_destination_conflict",
                "The proposed recipe destination already exists.",
                None,
                relative_path.clone(),
            ));
        }
        let existing = match crate::yaml::load_recipe_from_path(&path) {
            Ok(value) => value,
            Err(_) => {
                collisions.push(app_collision(
                    CollisionSeverity::Blocking,
                    "recipe_collision_scan_incomplete",
                    "The recipe collision scan could not be completed.",
                    None,
                    relative_path,
                ));
                continue;
            }
        };
        if existing.id == request.recipe_id {
            collisions.push(app_collision(
                CollisionSeverity::Blocking,
                "recipe_id_conflict",
                "A recipe with the proposed id already exists.",
                Some(existing.id),
                relative_path,
            ));
        }
    }
}

fn yaml_paths(
    directory: &Path,
    code: &str,
    relative_directory: &str,
    collisions: &mut Vec<AppRecipeCollisionDiagnostic>,
) -> Vec<std::path::PathBuf> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => {
            collisions.push(app_collision(
                CollisionSeverity::Blocking,
                code,
                "The generated catalog collision scan could not be completed.",
                None,
                Some(relative_directory.to_string()),
            ));
            return Vec::new();
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            collisions.push(app_collision(
                CollisionSeverity::Blocking,
                code,
                "The generated catalog collision scan could not be completed.",
                None,
                Some(relative_directory.to_string()),
            ));
            continue;
        };
        let path = entry.path();
        let yaml = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                value.eq_ignore_ascii_case("yaml") || value.eq_ignore_ascii_case("yml")
            });
        if !yaml {
            continue;
        }
        if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file()) {
            let relative_path = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|name| format!("{relative_directory}/{name}"));
            collisions.push(app_collision(
                CollisionSeverity::Blocking,
                code,
                "The generated catalog collision scan could not be completed.",
                None,
                relative_path,
            ));
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    paths
}

fn app_collision(
    severity: CollisionSeverity,
    code: &str,
    message: &str,
    existing_id: Option<String>,
    relative_path: Option<String>,
) -> AppRecipeCollisionDiagnostic {
    AppRecipeCollisionDiagnostic {
        severity,
        code: code.to_string(),
        message: message.to_string(),
        existing_id,
        relative_path,
    }
}

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

    fn app(id: &str, package: &str) -> AppDefinitionV1 {
        crate::authored_models::parse_app_definition_yaml(&format!(
            r#"schema_version: 1
kind: app_definition
id: {id}
name: Example
category: utility
package:
  primary: {package}
  aliases: []
install_source:
  type: user_provided_apk
  resolver: none
  options: {{}}
tracking_source:
  type: local_apk
artifacts:
  apk:
    required: false
  shared_storage_config:
    supported: false
  app_data_config:
    supported: false
  byo_apk:
    required: true
provisioning:
  launch_once_recommended: false
  shared_storage_paths: []
  app_data_paths: []
  config_targets: []
inputs: []
metadata: {{}}
"#
        ))
        .unwrap()
    }

    fn app_recipe_root(label: &str) -> std::path::PathBuf {
        let root = temp_root(label);
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::create_dir_all(root.join("recipes")).unwrap();
        root
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

    #[test]
    fn app_and_recipe_collisions_block_ids_and_destinations_and_warn_for_package_overlap() {
        let root = app_recipe_root("app-recipe-collisions");
        let existing = app("existing", "com.example.app");
        fs::write(
            root.join("apps/existing.yaml"),
            crate::authored_models::emit_app_definition_yaml(&existing).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("recipes/app.existing.install.yaml"),
            "schema_version: 1\nkind: recipe\nid: app.existing.install\nname: Install Example\nprovides:\n  features: []\nsteps: []\n",
        )
        .unwrap();

        let exact = check_app_recipe_collisions(
            &root,
            &AppRecipeCollisionRequest {
                app: existing.clone(),
                recipe_id: "app.existing.install".to_string(),
            },
        );
        assert!(exact.blocking);
        assert!(exact
            .collisions
            .iter()
            .any(|item| item.code == "app_id_conflict"));
        assert!(exact
            .collisions
            .iter()
            .any(|item| item.code == "recipe_id_conflict"));

        let overlap = check_app_recipe_collisions(
            &root,
            &AppRecipeCollisionRequest {
                app: app("different", "com.example.app"),
                recipe_id: "app.different.install".to_string(),
            },
        );
        assert!(!overlap.blocking);
        assert_eq!(overlap.collisions[0].code, "app_package_overlap");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_source_collisions_distinguish_repository_release_and_download_url() {
        let root = app_recipe_root("remote-source-collisions");
        let mut existing = app("remote.example", "com.example.remote");
        existing.install_source.type_name = "remote_apk".to_string();
        existing.install_source.resolver = "direct_url".to_string();
        existing.install_source.options.insert(
            "url".to_string(),
            Value::String(
                "https://github.com/example/project/releases/download/v1/app.apk".to_string(),
            ),
        );
        existing.install_source.options.insert(
            "repository".to_string(),
            Value::String("example/project".to_string()),
        );
        existing
            .install_source
            .options
            .insert("release_tag".to_string(), Value::String("v1".to_string()));
        existing.tracking_source.type_name = "github_release".to_string();
        existing.tracking_source.fields = existing.install_source.options.clone();
        fs::write(
            root.join("apps/remote.example.yaml"),
            crate::authored_models::emit_app_definition_yaml(&existing).unwrap(),
        )
        .unwrap();

        let mut proposed = app("remote.other", "com.example.other");
        proposed.install_source = existing.install_source.clone();
        proposed.tracking_source = existing.tracking_source.clone();
        let result = check_app_recipe_collisions(
            &root,
            &AppRecipeCollisionRequest {
                app: proposed,
                recipe_id: "app.remote.other.install".to_string(),
            },
        );
        assert!(!result.blocking);
        assert!(result
            .collisions
            .iter()
            .any(|item| item.code == "app_source_repository_overlap"));
        assert!(result
            .collisions
            .iter()
            .any(|item| item.code == "app_source_release_overlap"));
        assert!(result
            .collisions
            .iter()
            .any(|item| item.code == "app_source_url_overlap"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identical_latest_release_policies_are_blocking() {
        let root = app_recipe_root("latest-policy-collisions");
        let mut existing = app("latest.example", "com.example.latest");
        existing.install_source.type_name = "remote_release".to_string();
        existing.install_source.resolver = "github_latest_release".to_string();
        existing.install_source.options.insert(
            "repository".to_string(),
            Value::String("example/project".to_string()),
        );
        existing.install_source.options.insert(
            "asset_pattern".to_string(),
            Value::String("^app-v.*-arm64\\.apk$".to_string()),
        );
        existing
            .install_source
            .options
            .insert("include_prereleases".to_string(), Value::Bool(false));
        existing.tracking_source.type_name = "github_release".to_string();
        existing.tracking_source.fields.insert(
            "repository".to_string(),
            Value::String("example/project".to_string()),
        );
        fs::write(
            root.join("apps/latest.example.yaml"),
            crate::authored_models::emit_app_definition_yaml(&existing).unwrap(),
        )
        .unwrap();

        let mut proposed = existing.clone();
        proposed.id = "latest.other".to_string();
        proposed.package.primary = "com.example.other".to_string();
        let result = check_app_recipe_collisions(
            &root,
            &AppRecipeCollisionRequest {
                app: proposed,
                recipe_id: "app.latest.other.install".to_string(),
            },
        );
        assert!(result.blocking);
        assert!(result
            .collisions
            .iter()
            .any(|item| item.code == "app_latest_policy_conflict"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreadable_app_or_recipe_scan_is_blocking() {
        let root = app_recipe_root("app-recipe-incomplete");
        fs::write(root.join("apps/broken.yaml"), "not: [valid").unwrap();
        let result = check_app_recipe_collisions(
            &root,
            &AppRecipeCollisionRequest {
                app: app("different", "com.example.app"),
                recipe_id: "app.different.install".to_string(),
            },
        );
        assert!(result.blocking);
        assert_eq!(result.collisions[0].code, "app_collision_scan_incomplete");
        fs::remove_dir_all(root).unwrap();
    }
}
