//! Deterministic device-profile collision analysis for authored roots.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use regex::Regex;
use serde::Serialize;

use serde_json::Value;

use crate::authored_models::{
    load_app_definition, load_device_profile, AppDefinitionV1, DeviceProfileV1,
};
use crate::model::{ParamValue, Recipe, Step};
use crate::validation::normalize_expected_sha256;

use super::device_profile::SafeDetectedDeviceFacts;

pub(crate) struct AppRecipeCollisionRequest {
    pub app: AppDefinitionV1,
    pub recipe_id: String,
    pub recipe: Option<Recipe>,
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
    let proposed_fingerprint = request
        .recipe
        .as_ref()
        .and_then(apk_security_automation_fingerprint);
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
        } else if let (Some(proposed), Some(existing_fingerprint)) = (
            proposed_fingerprint.as_ref(),
            apk_security_automation_fingerprint(&existing).as_ref(),
        ) {
            if proposed == existing_fingerprint {
                collisions.push(app_collision(
                    CollisionSeverity::Blocking,
                    "apk_security_automation_fingerprint_conflict",
                    "An existing recipe enforces the same APK security and permission-automation behavior.",
                    Some(existing.id),
                    relative_path,
                ));
            } else if proposed.expected_package_name == existing_fingerprint.expected_package_name {
                collisions.push(app_collision(
                    CollisionSeverity::Warning,
                    "apk_expected_package_overlap",
                    "An existing recipe enforces the same APK package with different checksum or permission automation.",
                    Some(existing.id),
                    relative_path,
                ));
            } else if proposed.expected_sha256.is_some()
                && proposed.expected_sha256 == existing_fingerprint.expected_sha256
            {
                collisions.push(app_collision(
                    CollisionSeverity::Warning,
                    "apk_expected_sha256_overlap",
                    "An existing recipe trusts the same APK checksum for a different expected package.",
                    Some(existing.id),
                    relative_path,
                ));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ApkSecurityAutomationFingerprint {
    expected_package_name: String,
    expected_sha256: Option<String>,
    runtime_permissions: Vec<RuntimePermissionFingerprint>,
    app_ops: Vec<AppOpFingerprint>,
    policy: Option<PermissionPolicyFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RuntimePermissionFingerprint {
    package_name: String,
    permission_name: String,
    required: bool,
    when: PermissionWhenFingerprint,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AppOpFingerprint {
    package_name: String,
    operation_name: String,
    mode: String,
    required: bool,
    when: PermissionWhenFingerprint,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct PermissionWhenFingerprint {
    rooted: Option<bool>,
    android_api_min: Option<i64>,
    android_api_max: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PermissionPolicyFingerprint {
    on_failure: String,
    require_all: bool,
}

fn apk_security_automation_fingerprint(
    recipe: &Recipe,
) -> Option<ApkSecurityAutomationFingerprint> {
    let install_steps = recipe
        .steps
        .iter()
        .filter(|step| step.type_name == "install_apk")
        .collect::<Vec<_>>();
    let [install] = install_steps.as_slice() else {
        return None;
    };
    let expected_package_name =
        literal_non_empty_string(install.params.get("expected_package_name")?)?.to_string();
    let expected_sha256 = match install.params.get("expected_sha256") {
        None => None,
        Some(ParamValue::Literal(Value::String(value))) => normalize_expected_sha256(value),
        Some(_) => return None,
    };
    if install.params.contains_key("expected_sha256") && expected_sha256.is_none() {
        return None;
    }

    let ancestors = dependency_ancestors(recipe)?;
    let associated_permission_steps = recipe
        .steps
        .iter()
        .filter(|step| {
            step.type_name == "grant_permissions"
                && ancestors
                    .get(step.id.as_str())
                    .is_some_and(|dependencies| dependencies.contains(install.id.as_str()))
        })
        .collect::<Vec<_>>();
    let permission_step = match associated_permission_steps.as_slice() {
        [] => None,
        [step] => Some(*step),
        _ => return None,
    };
    let (runtime_permissions, app_ops, policy) = match permission_step {
        Some(step) => permission_fingerprint(step, &expected_package_name)?,
        None => (Vec::new(), Vec::new(), None),
    };
    Some(ApkSecurityAutomationFingerprint {
        expected_package_name,
        expected_sha256,
        runtime_permissions,
        app_ops,
        policy,
    })
}

fn dependency_ancestors(recipe: &Recipe) -> Option<HashMap<&str, BTreeSet<&str>>> {
    let mut steps = HashMap::new();
    for step in &recipe.steps {
        if step.id.trim().is_empty() || steps.insert(step.id.as_str(), step).is_some() {
            return None;
        }
    }
    let mut state = HashMap::new();
    let mut ancestors = HashMap::new();
    for step in &recipe.steps {
        collect_ancestors(step.id.as_str(), &steps, &mut state, &mut ancestors)?;
    }
    Some(ancestors)
}

fn collect_ancestors<'a>(
    step_id: &'a str,
    steps: &HashMap<&'a str, &'a Step>,
    state: &mut HashMap<&'a str, u8>,
    ancestors: &mut HashMap<&'a str, BTreeSet<&'a str>>,
) -> Option<BTreeSet<&'a str>> {
    match state.get(step_id) {
        Some(1) => return None,
        Some(2) => return ancestors.get(step_id).cloned(),
        _ => {}
    }
    state.insert(step_id, 1);
    let step = steps.get(step_id)?;
    let mut result = BTreeSet::new();
    for dependency in &step.dependencies {
        let dependency = dependency.as_str();
        if !steps.contains_key(dependency) {
            return None;
        }
        result.insert(dependency);
        result.extend(collect_ancestors(dependency, steps, state, ancestors)?);
    }
    state.insert(step_id, 2);
    ancestors.insert(step_id, result.clone());
    Some(result)
}

type PermissionFingerprintParts = (
    Vec<RuntimePermissionFingerprint>,
    Vec<AppOpFingerprint>,
    Option<PermissionPolicyFingerprint>,
);

fn permission_fingerprint(
    step: &Step,
    expected_package_name: &str,
) -> Option<PermissionFingerprintParts> {
    if step
        .params
        .keys()
        .any(|key| !matches!(key.as_str(), "runtime" | "appops" | "policy"))
    {
        return None;
    }
    let runtime_values = literal_array(step.params.get("runtime"))?;
    let app_op_values = literal_array(step.params.get("appops"))?;
    let mut runtime_by_identity = BTreeMap::new();
    for value in runtime_values {
        let action = runtime_permission_fingerprint(value, expected_package_name)?;
        let identity = (action.package_name.clone(), action.permission_name.clone());
        match runtime_by_identity.get(&identity) {
            Some(existing) if existing != &action => return None,
            Some(_) => {}
            None => {
                runtime_by_identity.insert(identity, action);
            }
        }
    }
    let mut app_ops_by_identity = BTreeMap::new();
    for value in app_op_values {
        let action = app_op_fingerprint(value, expected_package_name)?;
        let identity = (
            action.package_name.clone(),
            action.operation_name.clone(),
            action.mode.clone(),
        );
        match app_ops_by_identity.get(&identity) {
            Some(existing) if existing != &action => return None,
            Some(_) => {}
            None => {
                app_ops_by_identity.insert(identity, action);
            }
        }
    }
    let mut runtime_permissions = runtime_by_identity.into_values().collect::<Vec<_>>();
    runtime_permissions.sort();
    let mut app_ops = app_ops_by_identity.into_values().collect::<Vec<_>>();
    app_ops.sort();
    let policy = if runtime_permissions.is_empty() && app_ops.is_empty() {
        None
    } else {
        Some(permission_policy_fingerprint(step.params.get("policy"))?)
    };
    Some((runtime_permissions, app_ops, policy))
}

fn runtime_permission_fingerprint(
    value: &Value,
    expected_package_name: &str,
) -> Option<RuntimePermissionFingerprint> {
    let object = strict_object(value, &["package_name", "name", "required", "when"])?;
    let package_name = non_empty_string(object.get("package_name")?)?;
    if package_name != expected_package_name {
        return None;
    }
    Some(RuntimePermissionFingerprint {
        package_name: package_name.to_string(),
        permission_name: non_empty_string(object.get("name")?)?.to_string(),
        required: optional_bool(object.get("required"), true)?,
        when: permission_when_fingerprint(object.get("when"))?,
    })
}

fn app_op_fingerprint(value: &Value, expected_package_name: &str) -> Option<AppOpFingerprint> {
    let object = strict_object(value, &["package_name", "op", "mode", "required", "when"])?;
    let package_name = non_empty_string(object.get("package_name")?)?;
    if package_name != expected_package_name {
        return None;
    }
    let mode = non_empty_string(object.get("mode")?)?;
    if mode != "allow" {
        return None;
    }
    Some(AppOpFingerprint {
        package_name: package_name.to_string(),
        operation_name: non_empty_string(object.get("op")?)?.to_string(),
        mode: mode.to_string(),
        required: optional_bool(object.get("required"), true)?,
        when: permission_when_fingerprint(object.get("when"))?,
    })
}

fn permission_when_fingerprint(value: Option<&Value>) -> Option<PermissionWhenFingerprint> {
    let Some(value) = value else {
        return Some(PermissionWhenFingerprint::default());
    };
    let object = strict_object(value, &["rooted", "android_api_min", "android_api_max"])?;
    let rooted = optional_bool_value(object.get("rooted"))?;
    let android_api_min = optional_non_negative_i64(object.get("android_api_min"))?;
    let android_api_max = optional_non_negative_i64(object.get("android_api_max"))?;
    if android_api_min
        .zip(android_api_max)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        return None;
    }
    Some(PermissionWhenFingerprint {
        rooted,
        android_api_min,
        android_api_max,
    })
}

fn permission_policy_fingerprint(
    value: Option<&ParamValue>,
) -> Option<PermissionPolicyFingerprint> {
    let Some(value) = value else {
        return Some(PermissionPolicyFingerprint {
            on_failure: "warn".to_string(),
            require_all: false,
        });
    };
    let ParamValue::Literal(value) = value else {
        return None;
    };
    let object = strict_object(value, &["on_failure", "require_all"])?;
    let on_failure = match object.get("on_failure") {
        None => "warn",
        Some(Value::String(value)) if matches!(value.as_str(), "warn" | "fail") => value,
        Some(_) => return None,
    };
    Some(PermissionPolicyFingerprint {
        on_failure: on_failure.to_string(),
        require_all: optional_bool(object.get("require_all"), false)?,
    })
}

fn literal_array(value: Option<&ParamValue>) -> Option<&[Value]> {
    match value {
        None => Some(&[]),
        Some(ParamValue::Literal(Value::Array(values))) => Some(values),
        Some(_) => None,
    }
}

fn literal_non_empty_string(value: &ParamValue) -> Option<&str> {
    match value {
        ParamValue::Literal(value) => non_empty_string(value),
        ParamValue::Ref(_) => None,
    }
}

fn non_empty_string(value: &Value) -> Option<&str> {
    value.as_str().filter(|value| !value.trim().is_empty())
}

fn optional_bool(value: Option<&Value>, default: bool) -> Option<bool> {
    value.map(Value::as_bool).unwrap_or(Some(default))
}

fn optional_bool_value(value: Option<&Value>) -> Option<Option<bool>> {
    match value {
        None => Some(None),
        Some(value) => value.as_bool().map(Some),
    }
}

fn optional_non_negative_i64(value: Option<&Value>) -> Option<Option<i64>> {
    match value {
        None => Some(None),
        Some(value) => value.as_i64().filter(|value| *value >= 0).map(Some),
    }
}

fn strict_object<'a>(
    value: &'a Value,
    allowed_fields: &[&str],
) -> Option<&'a serde_json::Map<String, Value>> {
    let object = value.as_object()?;
    object
        .keys()
        .all(|key| allowed_fields.contains(&key.as_str()))
        .then_some(object)
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

    fn fingerprint_recipe(id: &str, package: &str, checksum: &str) -> Recipe {
        let raw = format!(
            r#"schema_version: 1
kind: recipe
id: {id}
name: Fingerprint
provides:
  features: []
steps:
- id: install
  type: install_apk
  name: Install
  user_toggleable: false
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  skip_if: []
  params:
    app: {{ref: inputs.apk}}
    expected_package_name: {package}
    expected_sha256: {checksum}
  verify: []
- id: permissions
  type: grant_permissions
  name: Permissions
  user_toggleable: false
  dependencies: [install]
  constraints:
    capabilities: []
    conflicts_with: []
  skip_if: []
  params:
    runtime:
    - package_name: {package}
      name: android.permission.CAMERA
      required: false
      when:
        android_api_min: 23
    appops:
    - package_name: {package}
      op: MANAGE_EXTERNAL_STORAGE
      mode: allow
      required: false
      when:
        rooted: true
    policy:
      on_failure: warn
      require_all: false
  verify: []
"#
        );
        let serde_yaml::Value::Mapping(mapping) = serde_yaml::from_str(&raw).unwrap() else {
            panic!("recipe fixture must be a mapping");
        };
        crate::yaml::parse_recipe_mapping(&mapping, Path::new("fingerprint.yaml")).unwrap()
    }

    fn permission_step_mut(recipe: &mut Recipe) -> &mut Step {
        recipe
            .steps
            .iter_mut()
            .find(|step| step.type_name == "grant_permissions")
            .unwrap()
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
                recipe: None,
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
                recipe: None,
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
                recipe: None,
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
                recipe: None,
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
    fn latest_policy_fingerprint_separates_provider_and_base_url() {
        let root = app_recipe_root("provider-aware-latest-policy");
        let mut existing = app("latest.gitlab", "com.example.gitlab");
        existing.install_source.type_name = "remote_release".to_string();
        existing.install_source.resolver = "provider_latest_release".to_string();
        for (key, value) in [
            ("provider", Value::String("gitlab".to_string())),
            ("base_url", Value::String("https://gitlab.com".to_string())),
            ("repository", Value::String("example/project".to_string())),
            ("asset_pattern", Value::String("^app-.*\\.apk$".to_string())),
            ("include_prereleases", Value::Bool(false)),
        ] {
            existing
                .install_source
                .options
                .insert(key.to_string(), value);
        }
        existing.tracking_source.type_name = "provider_release".to_string();
        existing.tracking_source.fields = existing.install_source.options.clone();
        fs::write(
            root.join("apps/latest.gitlab.yaml"),
            crate::authored_models::emit_app_definition_yaml(&existing).unwrap(),
        )
        .unwrap();

        let mut different_provider = existing.clone();
        different_provider.id = "latest.forgejo".to_string();
        different_provider.package.primary = "com.example.forgejo".to_string();
        different_provider
            .install_source
            .options
            .insert("provider".to_string(), Value::String("forgejo".to_string()));
        different_provider.install_source.options.insert(
            "base_url".to_string(),
            Value::String("https://codeberg.org".to_string()),
        );
        different_provider.tracking_source.fields =
            different_provider.install_source.options.clone();
        let result = check_app_recipe_collisions(
            &root,
            &AppRecipeCollisionRequest {
                app: different_provider,
                recipe_id: "app.latest.forgejo.install".to_string(),
                recipe: None,
            },
        );
        assert!(!result
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
                recipe: None,
            },
        );
        assert!(result.blocking);
        assert_eq!(result.collisions[0].code, "app_collision_scan_incomplete");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fingerprint_changes_for_every_security_component_and_ignores_action_order() {
        let checksum = "A".repeat(64);
        let mut base = fingerprint_recipe("app.base.install", "com.example.app", &checksum);
        for (section, field, value) in [
            ("runtime", "name", "android.permission.RECORD_AUDIO"),
            ("appops", "op", "LEGACY_STORAGE"),
        ] {
            let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut base)
                .params
                .get_mut(section)
                .unwrap()
            else {
                panic!("permission actions must be literal arrays");
            };
            let mut second = actions[0].clone();
            second[field] = Value::String(value.to_string());
            actions.push(second);
        }
        let base_fingerprint = apk_security_automation_fingerprint(&base).unwrap();

        let mut reordered = base.clone();
        for key in ["runtime", "appops"] {
            let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut reordered)
                .params
                .get_mut(key)
                .unwrap()
            else {
                panic!("permission actions must be literal arrays");
            };
            actions.reverse();
        }
        assert_eq!(
            apk_security_automation_fingerprint(&reordered).unwrap(),
            base_fingerprint
        );

        let mut variants = Vec::new();
        let mut package = base.clone();
        package.steps[0].params.insert(
            "expected_package_name".to_string(),
            ParamValue::Literal(Value::String("com.example.other".to_string())),
        );
        for section in ["runtime", "appops"] {
            let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut package)
                .params
                .get_mut(section)
                .unwrap()
            else {
                panic!("permission actions must be literal arrays");
            };
            for action in actions {
                action["package_name"] = Value::String("com.example.other".to_string());
            }
        }
        variants.push(package);
        let mut checksum_variant = base.clone();
        checksum_variant.steps[0].params.insert(
            "expected_sha256".to_string(),
            ParamValue::Literal(Value::String("B".repeat(64))),
        );
        variants.push(checksum_variant);
        for (section, field, replacement) in [
            ("runtime", "name", "android.permission.RECORD_AUDIO"),
            ("appops", "op", "LEGACY_STORAGE"),
        ] {
            let mut variant = base.clone();
            let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut variant)
                .params
                .get_mut(section)
                .unwrap()
            else {
                panic!("permission actions must be literal arrays");
            };
            actions[0][field] = Value::String(replacement.to_string());
            variants.push(variant);
        }
        let mut rooted = base.clone();
        let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut rooted)
            .params
            .get_mut("appops")
            .unwrap()
        else {
            panic!("app-op actions must be a literal array");
        };
        actions[0]["when"]["rooted"] = Value::Bool(false);
        variants.push(rooted);
        let mut api = base.clone();
        let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut api)
            .params
            .get_mut("runtime")
            .unwrap()
        else {
            panic!("runtime actions must be a literal array");
        };
        actions[0]["when"]["android_api_min"] = Value::from(24);
        variants.push(api);
        let mut required = base.clone();
        let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut required)
            .params
            .get_mut("runtime")
            .unwrap()
        else {
            panic!("runtime actions must be a literal array");
        };
        actions[0]["required"] = Value::Bool(true);
        variants.push(required);
        let mut policy = base.clone();
        permission_step_mut(&mut policy).params.insert(
            "policy".to_string(),
            ParamValue::Literal(serde_json::json!({ "on_failure": "fail", "require_all": false })),
        );
        variants.push(policy);

        for variant in variants {
            assert_ne!(
                apk_security_automation_fingerprint(&variant).unwrap(),
                base_fingerprint
            );
        }
    }

    #[test]
    fn malformed_referenced_or_ambiguous_recipes_are_not_comparable() {
        let checksum = "C".repeat(64);
        let base = fingerprint_recipe("app.base.install", "com.example.app", &checksum);
        let mut variants = Vec::new();
        let mut referenced = base.clone();
        referenced.steps[0].params.insert(
            "expected_package_name".to_string(),
            ParamValue::Ref("inputs.package".to_string()),
        );
        variants.push(referenced);
        let mut unknown_condition = base.clone();
        let ParamValue::Literal(Value::Array(actions)) =
            permission_step_mut(&mut unknown_condition)
                .params
                .get_mut("runtime")
                .unwrap()
        else {
            panic!("runtime actions must be a literal array");
        };
        actions[0]["when"]["future_api"] = Value::from(1);
        variants.push(unknown_condition);
        let mut negative_condition = base.clone();
        let ParamValue::Literal(Value::Array(actions)) =
            permission_step_mut(&mut negative_condition)
                .params
                .get_mut("runtime")
                .unwrap()
        else {
            panic!("runtime actions must be a literal array");
        };
        actions[0]["when"]["android_api_min"] = Value::from(-1);
        variants.push(negative_condition);
        let mut package_mismatch = base.clone();
        let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut package_mismatch)
            .params
            .get_mut("runtime")
            .unwrap()
        else {
            panic!("runtime actions must be a literal array");
        };
        actions[0]["package_name"] = Value::String("com.example.other".to_string());
        variants.push(package_mismatch);
        let mut cycle = base.clone();
        cycle.steps[0].dependencies = vec!["permissions".to_string()];
        variants.push(cycle);
        let mut multiple = base.clone();
        let mut second_permission_step = multiple.steps[1].clone();
        second_permission_step.id = "permissions_two".to_string();
        multiple.steps.push(second_permission_step);
        variants.push(multiple);
        let mut local = base.clone();
        local.steps[0].params.shift_remove("expected_package_name");
        variants.push(local);

        for variant in variants {
            assert!(apk_security_automation_fingerprint(&variant).is_none());
        }
    }

    #[test]
    fn unsupported_app_op_mode_is_not_comparable() {
        let checksum = "F".repeat(64);
        let mut recipe = fingerprint_recipe("app.mode.install", "com.example.app", &checksum);
        let ParamValue::Literal(Value::Array(actions)) = permission_step_mut(&mut recipe)
            .params
            .get_mut("appops")
            .unwrap()
        else {
            panic!("app-op actions must be a literal array");
        };
        actions[0]["mode"] = Value::String("deny".to_string());
        assert!(apk_security_automation_fingerprint(&recipe).is_none());
    }

    #[test]
    fn exact_and_partial_fingerprint_collisions_have_stable_diagnostics() {
        let checksum = "D".repeat(64);
        let cases = [
            (
                "exact-fingerprint",
                fingerprint_recipe("app.existing.install", "com.example.app", &checksum),
                fingerprint_recipe("app.proposed.install", "com.example.app", &checksum),
                "apk_security_automation_fingerprint_conflict",
                true,
            ),
            (
                "package-overlap",
                fingerprint_recipe("app.existing.install", "com.example.app", &checksum),
                fingerprint_recipe("app.proposed.install", "com.example.app", &"E".repeat(64)),
                "apk_expected_package_overlap",
                false,
            ),
            (
                "checksum-overlap",
                fingerprint_recipe("app.existing.install", "com.example.app", &checksum),
                fingerprint_recipe("app.proposed.install", "com.example.other", &checksum),
                "apk_expected_sha256_overlap",
                false,
            ),
        ];
        for (label, existing, proposed, expected_code, blocking) in cases {
            let root = app_recipe_root(label);
            fs::write(
                root.join("recipes/existing.yaml"),
                crate::yaml::emit_recipe_yaml(&existing).unwrap(),
            )
            .unwrap();
            let result = check_app_recipe_collisions(
                &root,
                &AppRecipeCollisionRequest {
                    app: app("proposed", "com.example.proposed"),
                    recipe_id: proposed.id.clone(),
                    recipe: Some(proposed),
                },
            );
            assert_eq!(result.blocking, blocking, "{label}");
            assert_eq!(
                result
                    .collisions
                    .iter()
                    .filter(|collision| collision.code.starts_with("apk_"))
                    .map(|collision| collision.code.as_str())
                    .collect::<Vec<_>>(),
                vec![expected_code],
                "{label}"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }
}
