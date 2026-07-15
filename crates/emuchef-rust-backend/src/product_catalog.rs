//! Product-facing catalog inventory projected from a resolved snapshot.
//!
//! The DTO deliberately omits file paths, YAML text, document ids, dirty state,
//! and authoring commands. A future catalog source can therefore resolve the
//! same snapshot contract without changing this end-user API.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::authored_models::{self, DEVICE_PROFILE_KIND};
use crate::catalog_source::CatalogSnapshot;
use crate::errors::ApiError;

pub(crate) fn describe(snapshot: &CatalogSnapshot) -> Result<Value, ApiError> {
    let recipes = crate::planner::load_top_level_recipes(snapshot.root()).map_err(|error| {
        ApiError::load_failed(
            format!("Failed to load catalog recipes: {error}"),
            json!({ "code": error.code() }),
        )
    })?;
    let recipes = recipes
        .iter()
        .map(|recipe| {
            let required_capabilities = recipe
                .steps
                .iter()
                .flat_map(|step| step.constraints.capabilities.iter().cloned())
                .collect::<BTreeSet<_>>();
            json!({
                "id": recipe.id,
                "name": recipe.name,
                "description": recipe.description,
                "recipeDependencies": recipe.recipe_dependencies,
                "features": recipe.provides.features,
                "requiredCapabilities": required_capabilities,
                "inputs": recipe.inputs.iter().map(|(id, input)| {
                    crate::dto::input_to_dto(&recipe.id, id, input)
                }).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "catalog": snapshot.identity(),
        "devicePlans": authored_inventory(snapshot.root(), "device_plans", "device_plan")?,
        "deviceProfiles": authored_inventory(snapshot.root(), "device_profiles", "device_profile")?,
        "recipes": recipes,
    }))
}

fn authored_inventory(
    root: &Path,
    directory: &str,
    expected_kind: &str,
) -> Result<Vec<Value>, ApiError> {
    let mut paths = fs::read_dir(root.join(directory))
        .map_err(|_| {
            ApiError::load_failed(
                "Catalog inventory directory is unavailable.",
                json!({ "directory": directory }),
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_yaml(path))
        .collect::<Vec<PathBuf>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| inventory_entry(&path, expected_kind))
        .collect()
}

fn inventory_entry(path: &Path, expected_kind: &str) -> Result<Value, ApiError> {
    if expected_kind == DEVICE_PROFILE_KIND {
        return device_profile_inventory_entry(path);
    }
    let bytes = fs::read(path).map_err(|_| {
        ApiError::load_failed(
            "Catalog inventory entry is unreadable.",
            json!({ "kind": expected_kind }),
        )
    })?;
    let yaml: serde_yaml::Value = serde_yaml::from_slice(&bytes).map_err(|_| {
        ApiError::load_failed(
            "Catalog inventory entry is invalid YAML.",
            json!({ "kind": expected_kind }),
        )
    })?;
    let raw = serde_json::to_value(yaml).map_err(|_| {
        ApiError::load_failed(
            "Catalog inventory entry cannot be represented as JSON.",
            json!({ "kind": expected_kind }),
        )
    })?;
    let object = raw.as_object().ok_or_else(|| {
        ApiError::load_failed(
            "Catalog inventory entry must be an object.",
            json!({ "kind": expected_kind }),
        )
    })?;
    if object.get("kind").and_then(Value::as_str) != Some(expected_kind) {
        return Err(ApiError::load_failed(
            "Catalog inventory entry has the wrong kind.",
            json!({ "expectedKind": expected_kind }),
        ));
    }
    let recipes = object
        .get("recipes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|selection| {
            json!({
                "recipeId": selection.get("recipe_ref"),
                "selectedByDefault": selection.get("selected_by_default"),
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "id": object.get("id"),
        "name": object.get("name"),
        "description": object.get("description"),
        "deviceProfileId": object.get("device_profile_ref"),
        "recipes": recipes,
        "showAdvancedSteps": object.get("defaults").and_then(Value::as_object).and_then(|value| value.get("show_advanced_steps")),
        "metadata": object.get("metadata"),
    });
    Ok(value)
}

fn device_profile_inventory_entry(path: &Path) -> Result<Value, ApiError> {
    let profile = authored_models::load_device_profile(path).map_err(|error| {
        ApiError::load_failed(
            error.message(),
            json!({ "kind": DEVICE_PROFILE_KIND, "code": error.code() }),
        )
    })?;
    let diagnostics = authored_models::validate_device_profile(&profile);
    if !diagnostics.is_empty() {
        return Err(ApiError::load_failed(
            "Catalog device profile failed semantic validation.",
            json!({ "kind": DEVICE_PROFILE_KIND, "diagnostics": diagnostics }),
        ));
    }

    Ok(json!({
        "id": profile.id,
        "name": profile.name,
        "description": profile.description,
        "matchCriteria": {
            "manufacturerContains": profile.match_criteria.manufacturer_contains,
            "brandContains": profile.match_criteria.brand_contains,
            "modelPatterns": profile.match_criteria.model_patterns,
            "androidVersion": profile.match_criteria.android_version,
        },
        "capabilities": {
            "adbAvailable": profile.capability_defaults.adb_available,
            "apkInstall": profile.capability_defaults.apk_install,
            "sharedStorageWrite": profile.capability_defaults.shared_storage_write,
            "appLaunch": profile.capability_defaults.app_launch,
            "shellCommand": profile.capability_defaults.shell_command,
            "packageRemoveForUser": profile.capability_defaults.package_remove_for_user,
            "rootShell": profile.capability_defaults.root_shell,
            "appDataWrite": profile.capability_defaults.app_data_write,
        },
        "deviceTags": profile.device_tags,
        "metadata": profile.metadata,
    }))
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("yaml" | "yml")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_source::{
        CatalogIdentity, CatalogSource, CatalogSourceKind, LocalCatalogSource,
    };

    #[test]
    fn product_inventory_exposes_display_contract_without_editor_internals() {
        let temp = tempfile::tempdir().unwrap();
        for directory in ["apps", "recipes", "device_profiles", "device_plans"] {
            fs::create_dir_all(temp.path().join(directory)).unwrap();
        }
        fs::write(
            temp.path().join("recipes/example.yaml"),
            r#"schema_version: 1
kind: recipe
id: recipe.example
name: Example Recipe
description: A product recipe.
recipe_dependencies: []
provides: {features: [example]}
inputs: {}
artifacts: {}
artifact_groups: {}
steps:
  - id: wait
    type: wait
    name: Wait
    user_toggleable: false
    dependencies: []
    constraints: {capabilities: [shell_command], conflicts_with: []}
    skip_if: []
    params: {duration_ms: 1}
    verify: []
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("device_profiles/example.yaml"),
            r#"schema_version: 1
kind: device_profile
id: profile.example
name: Example Profile
description: A product profile.
match: {manufacturer_contains: [Example]}
capability_defaults: {adb_available: true, apk_install: true, shared_storage_write: true, app_launch: true, shell_command: true, package_remove_for_user: true, root_shell: false, app_data_write: false}
device_tags: [example]
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("device_plans/example.yaml"),
            r#"schema_version: 1
kind: device_plan
id: plan.example
name: Example Plan
description: A product plan.
device_profile_ref: profile.example
recipes: [{recipe_ref: recipe.example, selected_by_default: true}]
defaults: {show_advanced_steps: false}
overrides: {}
"#,
        )
        .unwrap();
        let identity = CatalogIdentity {
            source_kind: CatalogSourceKind::Bundled,
            source_id: "catalog.example".to_string(),
            version: Some("1".to_string()),
            cache_key: None,
            content_digest: None,
        };
        let snapshot = LocalCatalogSource::new(temp.path(), identity)
            .resolve()
            .unwrap();
        let result = describe(&snapshot).unwrap();

        assert_eq!(result["catalog"]["sourceId"], "catalog.example");
        assert_eq!(result["devicePlans"][0]["name"], "Example Plan");
        assert_eq!(result["deviceProfiles"][0]["name"], "Example Profile");
        assert_eq!(result["recipes"][0]["name"], "Example Recipe");
        assert_eq!(
            result["recipes"][0]["requiredCapabilities"],
            json!(["shell_command"])
        );
        assert!(result.get("root").is_none());
        assert!(result.get("documentId").is_none());
        assert!(result.get("yaml").is_none());
    }

    #[test]
    fn product_inventory_rejects_semantically_invalid_typed_device_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("invalid-profile.yaml");
        fs::write(
            &path,
            r#"schema_version: 1
kind: device_profile
id: profile.example
name: Example Profile
match: {model_patterns: ['[']}
capability_defaults: {adb_available: true, apk_install: true, shared_storage_write: true, app_launch: true, shell_command: true, package_remove_for_user: true, root_shell: false, app_data_write: false}
device_tags: []
metadata: {}
"#,
        )
        .unwrap();

        let error = inventory_entry(&path, DEVICE_PROFILE_KIND).unwrap_err();
        let value = error.to_value();
        assert_eq!(value["code"], "load_failed");
        assert_eq!(
            value["details"]["diagnostics"][0]["code"],
            "device_model_pattern_invalid"
        );
        assert_eq!(
            value["details"]["diagnostics"][0]["field"],
            "match.model_patterns[0]"
        );
    }
}
