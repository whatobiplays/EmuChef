use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

/// One catalog-resolved RetroArch configuration destination that a successful
/// configuration-create write probe would authorize.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RetroArchConfigCreateTarget {
    pub(crate) app_id: String,
    pub(crate) package_id: String,
    pub(crate) target_id: String,
    pub(crate) target_kind: String,
    pub(crate) target_location: String,
    pub(crate) target_path: String,
    pub(crate) destination: String,
    pub(crate) catalog_identity: String,
}

const RETROARCH_CONFIG_CREATE_RECIPES: [&str; 2] =
    ["app.retroarch.configure", "app.retroarch.provision"];

/// Resolve only the authored RetroArch create-only target from a planner-owned
/// execution plan. The returned identity includes its catalog and package
/// destination so evidence cannot authorize another App Target.
pub(crate) fn resolve_retroarch_config_create_target(
    plan: &Value,
) -> Result<Option<RetroArchConfigCreateTarget>, &'static str> {
    let resolved_inputs = plan.get("resolved_inputs").and_then(Value::as_array);
    let steps = plan.get("steps").and_then(Value::as_array);
    let selected_retroarch_config_create_recipes = plan
        .pointer("/source/selected_recipe_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|recipe| RETROARCH_CONFIG_CREATE_RECIPES.contains(recipe))
        .collect::<Vec<_>>();
    let Some(resolved_inputs) = resolved_inputs else {
        let mut has_configuration_step = false;
        if let Some(steps) = steps {
            for step in steps {
                if step_is_retroarch_config_create(plan, step)? {
                    has_configuration_step = true;
                    break;
                }
            }
        }
        return if !selected_retroarch_config_create_recipes.is_empty() || has_configuration_step {
            Err("retroarch_config_create_plan_invalid")
        } else {
            Ok(None)
        };
    };
    let steps = steps.ok_or("retroarch_config_create_plan_invalid")?;
    for recipe_id in &selected_retroarch_config_create_recipes {
        if !resolved_inputs.iter().any(|input| {
            input.get("recipeId").and_then(Value::as_str) == Some(recipe_id)
                && input.get("inputId").and_then(Value::as_str) == Some("retroarch_cfg_source")
        }) {
            return Err("retroarch_config_create_plan_invalid");
        }
    }
    let mut selected_sources = HashMap::<String, String>::new();
    for input in resolved_inputs {
        let recipe_id = input
            .get("recipeId")
            .and_then(Value::as_str)
            .filter(|id| RETROARCH_CONFIG_CREATE_RECIPES.contains(id));
        if input.get("inputId").and_then(Value::as_str) != Some("retroarch_cfg_source") {
            continue;
        }
        let Some(recipe_id) = recipe_id else {
            continue;
        };
        match input.get("value").and_then(Value::as_str) {
            Some("managed") | Some("uploaded") => {
                let source = input["value"].as_str().unwrap().to_string();
                if selected_sources
                    .insert(recipe_id.to_string(), source)
                    .is_some()
                {
                    return Err("retroarch_config_create_plan_ambiguous");
                }
            }
            Some("none") => {}
            _ => return Err("retroarch_config_create_plan_invalid"),
        }
    }
    let mut configuration_steps = Vec::new();
    for step in steps {
        if step_is_retroarch_config_create(plan, step)? {
            configuration_steps.push(step);
        }
    }
    if selected_sources.is_empty() && configuration_steps.is_empty() {
        return Ok(None);
    }
    if selected_sources.len() > 1 || configuration_steps.len() > 1 {
        return Err("retroarch_config_create_plan_ambiguous");
    }
    if selected_sources.len() != 1 || configuration_steps.len() != 1 {
        return Err("retroarch_config_create_configuration_step_missing");
    }
    let (recipe_id, source_kind) = selected_sources.iter().next().unwrap();
    let step = configuration_steps[0];
    if step.get("recipe_ref").and_then(Value::as_str) != Some(recipe_id.as_str())
        || step.get("type").and_then(Value::as_str) != Some("copy_files")
        || step.get("app_id").and_then(Value::as_str) != Some("retroarch")
    {
        return Err("retroarch_config_create_target_invalid");
    }
    let source_artifact = literal_step_parameter(step, "source")
        .filter(|value| value.get("type").and_then(Value::as_str) == Some("configuration_artifact"))
        .ok_or("retroarch_config_create_target_invalid")?;
    if !source_artifact.get("sourceIdentity").is_some_and(|value| {
        value.get("recipeId").and_then(Value::as_str) == Some(recipe_id.as_str())
            && value.get("kind").and_then(Value::as_str)
                == Some(if source_kind == "managed" {
                    "generated"
                } else {
                    "uploaded"
                })
    }) {
        return Err("retroarch_config_create_target_invalid");
    }
    let target_identity = source_artifact
        .get("targetIdentity")
        .filter(|value| {
            value.get("appId").and_then(Value::as_str) == Some("retroarch")
                && value.get("targetId").and_then(Value::as_str) == Some("config")
                && value
                    .get("target")
                    .and_then(|target| target.get("kind"))
                    .and_then(Value::as_str)
                    == Some("file")
                && value
                    .get("target")
                    .and_then(|target| target.get("location"))
                    .and_then(Value::as_str)
                    == Some("external_app_data")
                && value
                    .get("target")
                    .and_then(|target| target.get("path"))
                    .and_then(Value::as_str)
                    == Some("files/retroarch.cfg")
        })
        .ok_or("retroarch_config_create_target_invalid")?;
    let package_id = target_identity
        .get("packageId")
        .and_then(Value::as_str)
        .filter(|package_id| !package_id.trim().is_empty())
        .ok_or("retroarch_config_create_target_invalid")?;
    let apps = plan
        .get("apps")
        .and_then(Value::as_array)
        .ok_or("retroarch_config_create_target_invalid")?;
    let matching_apps = apps
        .iter()
        .filter(|app| app.get("id").and_then(Value::as_str) == Some("retroarch"))
        .collect::<Vec<_>>();
    if matching_apps.len() != 1
        || matching_apps[0].get("package_id").and_then(Value::as_str) != Some(package_id)
    {
        return Err("retroarch_config_create_target_invalid");
    }
    let destination = format!("/storage/emulated/0/Android/data/{package_id}/files/retroarch.cfg");
    if !app_target_parameter_matches(literal_step_parameter(step, "dest"), &destination)
        || literal_step_parameter(step, "copy_policy").and_then(Value::as_str)
            != Some("create_only")
    {
        return Err("retroarch_config_create_target_invalid");
    }
    let catalog_identity = plan
        .pointer("/source/catalog")
        .filter(|value| value.is_object())
        .and_then(|value| serde_json::to_string(value).ok())
        .ok_or("retroarch_config_create_target_invalid")?;
    Ok(Some(RetroArchConfigCreateTarget {
        app_id: "retroarch".to_string(),
        package_id: package_id.to_string(),
        target_id: "config".to_string(),
        target_kind: "file".to_string(),
        target_location: "external_app_data".to_string(),
        target_path: "files/retroarch.cfg".to_string(),
        destination,
        catalog_identity,
    }))
}

fn literal_step_parameter<'a>(step: &'a Value, name: &str) -> Option<&'a Value> {
    let parameter = step.get("params")?.get(name)?;
    parameter
        .get("Literal")
        .or_else(|| parameter.get("literal"))
        .and_then(|literal| literal.get("value"))
        .or_else(|| parameter.get("value"))
}

fn step_parameter_value<'a>(plan: &'a Value, step: &'a Value, name: &str) -> Option<&'a Value> {
    if let Some(value) = literal_step_parameter(step, name) {
        return Some(value);
    }

    let reference = step
        .get("params")?
        .get(name)?
        .get("ref")?
        .as_str()?
        .strip_prefix("inputs.")?;
    let mut components = reference.split('/');
    let recipe_id = components.next()?;
    let input_id = components.next()?;
    if components.next().is_some() {
        return None;
    }
    if !authored_identifier_is_valid(recipe_id) || !authored_identifier_is_valid(input_id) {
        return None;
    }

    let recipe_ref = step.get("recipe_ref").and_then(Value::as_str);
    if recipe_ref.is_some_and(|recipe| recipe != recipe_id) {
        return None;
    }

    let mut matches = plan
        .get("resolved_inputs")?
        .as_array()?
        .iter()
        .filter(|input| input.get("key").and_then(Value::as_str) == Some(reference));
    let input = matches.next()?;
    if matches.next().is_some()
        || input.get("recipeId").and_then(Value::as_str) != Some(recipe_id)
        || input.get("inputId").and_then(Value::as_str) != Some(input_id)
    {
        return None;
    }
    resolved_input_snapshot_value(input)
}

/// Mirrors the authored catalog's lowercase identifiers with single separators.
fn authored_identifier_is_valid(value: &str) -> bool {
    let mut segment_needs_alphanumeric = true;
    for character in value.chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            segment_needs_alphanumeric = false;
        } else if matches!(character, '.' | '_' | '-') && !segment_needs_alphanumeric {
            segment_needs_alphanumeric = true;
        } else {
            return false;
        }
    }
    !segment_needs_alphanumeric
}

fn resolved_input_snapshot_value(input: &Value) -> Option<&Value> {
    let input_type = input
        .get("type")
        .and_then(Value::as_str)
        .filter(|input_type| !input_type.trim().is_empty())?;
    let source_is_valid = input.get("source").is_some_and(|source| {
        source.is_null()
            || source.as_str().is_some_and(|source| {
                matches!(
                    source,
                    "explicit" | "user_configuration" | "device_plan" | "recipe_default"
                )
            })
    });
    if !source_is_valid
        || input.get("label").and_then(Value::as_str).is_none()
        || input.get("required").and_then(Value::as_bool).is_none()
        || input.get("sensitive").and_then(Value::as_bool).is_none()
        || input
            .get("description")
            .is_some_and(|description| !description.is_string())
        || input
            .get("selectedEnumLabel")
            .is_some_and(|label| !label.is_string())
    {
        return None;
    }

    let value = input.get("value").filter(|value| !value.is_null())?;
    input_value_matches_type(input_type, value).then_some(value)
}

fn input_value_matches_type(input_type: &str, value: &Value) -> bool {
    let scalar_matches = |value: &Value| match input_type {
        "string" | "enum" | "file" | "directory" | "path" | "device_path"
        | "setup_location_ref" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value
            .as_number()
            .and_then(serde_json::Number::as_f64)
            .is_some_and(f64::is_finite),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        _ => false,
    };

    match input_type {
        "string_list" | "path_list" => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        _ => {
            scalar_matches(value)
                || value
                    .as_array()
                    .is_some_and(|items| items.iter().all(scalar_matches))
        }
    }
}

fn step_parameter_string<'a>(plan: &'a Value, step: &'a Value, name: &str) -> Option<&'a str> {
    let value = step_parameter_value(plan, step, name)?;
    value
        .as_str()
        .or_else(|| value.get("value").and_then(Value::as_str))
}

fn plan_or_step_uses_retroarch_config_recipe(plan: &Value, step: &Value) -> bool {
    if step.get("app_id").and_then(Value::as_str) == Some("retroarch")
        || step
            .get("recipe_ref")
            .and_then(Value::as_str)
            .is_some_and(|recipe| RETROARCH_CONFIG_CREATE_RECIPES.contains(&recipe))
    {
        return true;
    }

    let source = plan.get("source");
    if ["selected_recipe_refs", "expanded_recipe_refs"]
        .into_iter()
        .filter_map(|field| source.and_then(|source| source.get(field)))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .any(|recipe| RETROARCH_CONFIG_CREATE_RECIPES.contains(&recipe))
    {
        return true;
    }

    plan.get("apps")
        .and_then(Value::as_array)
        .is_some_and(|apps| {
            apps.iter()
                .any(|app| app.get("id").and_then(Value::as_str) == Some("retroarch"))
        })
}

fn app_target_parameter_matches(value: Option<&Value>, destination: &str) -> bool {
    let Some(value) = value else {
        return false;
    };
    value.as_str() == Some(destination)
        || value.get("app_target").and_then(Value::as_str) == Some("config")
        || value.get("appTarget").and_then(Value::as_str) == Some("config")
}

fn step_is_retroarch_config_create(plan: &Value, step: &Value) -> Result<bool, &'static str> {
    let retroarch_app_step = step.get("app_id").and_then(Value::as_str) == Some("retroarch");
    let selected_retroarch_recipe = step
        .get("recipe_ref")
        .and_then(Value::as_str)
        .filter(|recipe| RETROARCH_CONFIG_CREATE_RECIPES.contains(recipe))
        .is_some_and(|recipe| {
            plan.pointer("/source/selected_recipe_refs")
                .and_then(Value::as_array)
                .is_some_and(|selected| selected.iter().any(|value| value.as_str() == Some(recipe)))
        });
    let step_app_id_missing = step.get("app_id").is_none_or(Value::is_null);
    let selected_recipe_config_target = (retroarch_app_step
        || (step_app_id_missing && selected_retroarch_recipe))
        && step_parameter_value(plan, step, "dest").is_some_and(|destination| {
            destination.get("app_target").and_then(Value::as_str) == Some("config")
                || destination.get("appTarget").and_then(Value::as_str) == Some("config")
        });
    let destination_may_write_configuration = step_may_write_retroarch_configuration(plan, step)?;
    Ok(retroarch_app_step
        && step
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id.ends_with("seed_retroarch_cfg"))
        || selected_recipe_config_target
        || literal_step_parameter(step, "source")
            .and_then(|source| source.get("targetIdentity"))
            .is_some_and(|identity| {
                identity.get("appId").and_then(Value::as_str) == Some("retroarch")
                    && identity.get("targetId").and_then(Value::as_str) == Some("config")
                    && identity.pointer("/target/path").and_then(Value::as_str)
                        == Some("files/retroarch.cfg")
            })
        || destination_may_write_configuration)
}

fn step_may_write_retroarch_configuration(
    plan: &Value,
    step: &Value,
) -> Result<bool, &'static str> {
    let Some(destination) = step_parameter_string(plan, step, "dest") else {
        let has_destination_reference = step
            .pointer("/params/dest")
            .is_some_and(|destination| destination.get("ref").is_some());
        let is_device_writer = matches!(
            step.get("type").and_then(Value::as_str),
            Some("copy_files" | "extract_archive")
        );
        if has_destination_reference
            && is_device_writer
            && plan_or_step_uses_retroarch_config_recipe(plan, step)
        {
            return Err("retroarch_config_create_destination_unresolved");
        }
        return Ok(false);
    };
    if step.get("type").and_then(Value::as_str) == Some("extract_archive")
        && step_parameter_string(plan, step, "extract_on") == Some("host")
    {
        return Ok(false);
    }
    if retroarch_config_destination_matches(plan, destination)? {
        return Ok(true);
    }
    let destination = normalize_retroarch_device_path(destination)?;
    let destination_path = Path::new(&destination);
    let step_type = step.get("type").and_then(Value::as_str);

    for app in plan
        .get("apps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|app| app.get("id").and_then(Value::as_str) == Some("retroarch"))
    {
        let Some(package_id) = app
            .get("package_id")
            .and_then(Value::as_str)
            .filter(|package_id| !package_id.trim().is_empty())
        else {
            continue;
        };
        let configuration =
            format!("/storage/emulated/0/Android/data/{package_id}/files/retroarch.cfg");
        let configuration_path = Path::new(&configuration);
        let Some(configuration_parent) = configuration_path.parent() else {
            continue;
        };
        if !configuration_path.starts_with(destination_path) {
            continue;
        }

        // Device-capable extraction uses its destination as a tree root, so an
        // archive placed at this directory or any ancestor can contain retroarch.cfg.
        // Only an explicit host mode proves extraction cannot write to the device.
        if step_type == Some("extract_archive")
            && step_parameter_string(plan, step, "extract_on") != Some("host")
        {
            return Ok(true);
        }
        if destination_path == configuration_parent
            && step_type == Some("copy_files")
            && copy_files_source_may_write_config_at_destination(plan, step)
        {
            return Ok(true);
        }
        if step_type == Some("copy_files")
            && copy_files_source_may_contain_config_under_destination(plan, step)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn copy_files_source_may_write_config_at_destination(plan: &Value, step: &Value) -> bool {
    let Some(source) = step_parameter_value(plan, step, "source") else {
        return true;
    };
    let source_type = source
        .get("type")
        .or_else(|| source.get("type_name"))
        .or_else(|| source.get("typeName"))
        .and_then(Value::as_str);
    match source_type {
        Some("file_path") => source_path(source).is_none_or(path_targets_retroarch_config_basename),
        Some("path_list") => {
            if step_parameter_string(plan, step, "copy_policy") == Some("replace") {
                return true;
            }
            source
                .get("value")
                .and_then(Value::as_array)
                .is_none_or(|paths| {
                    paths.iter().any(|path| {
                        path.as_str()
                            .is_none_or(path_targets_retroarch_config_basename)
                    })
                })
        }
        Some("directory_path") => true,
        _ => true,
    }
}

fn copy_files_source_may_contain_config_under_destination(plan: &Value, step: &Value) -> bool {
    let Some(source) = step_parameter_value(plan, step, "source") else {
        return true;
    };
    match source
        .get("type")
        .or_else(|| source.get("type_name"))
        .or_else(|| source.get("typeName"))
        .and_then(Value::as_str)
    {
        Some("file_path") => false,
        Some("directory_path" | "path_list") => true,
        _ => true,
    }
}

fn source_path(source: &Value) -> Option<&str> {
    source
        .as_str()
        .or_else(|| source.get("value").and_then(Value::as_str))
}

fn path_targets_retroarch_config_basename(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|basename| basename.to_str())
        .is_none_or(|basename| basename == "retroarch.cfg")
}

fn retroarch_config_destination_matches(
    plan: &Value,
    destination: &str,
) -> Result<bool, &'static str> {
    let destination = normalize_retroarch_device_path(destination)?;
    Ok(plan
        .get("apps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|app| app.get("id").and_then(Value::as_str) == Some("retroarch"))
        .filter_map(|app| app.get("package_id").and_then(Value::as_str))
        .filter(|package_id| !package_id.trim().is_empty())
        .any(|package_id| {
            destination
                == format!("/storage/emulated/0/Android/data/{package_id}/files/retroarch.cfg")
        }))
}

// Root slashless destinations during comparison so equivalent paths reach the same target check.
fn normalize_retroarch_device_path(path: &str) -> Result<String, &'static str> {
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err("retroarch_config_create_destination_escapes_root");
                }
            }
            _ => components.push(component),
        }
    }
    let mut normalized = format!("/{}", components.join("/"));
    for alias in ["/storage/self/primary", "/sdcard"] {
        if normalized == alias {
            normalized = "/storage/emulated/0".to_string();
            break;
        }
        if let Some(suffix) = normalized.strip_prefix(&format!("{alias}/")) {
            normalized = format!("/storage/emulated/0/{suffix}");
            break;
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const TEST_KONKR_PROFILE_REF: &str = "konkr.pocket_advance";

    fn retroarch_config_create_plan(recipe_id: &str, source: &str) -> Value {
        let package_id = "com.retroarch.aarch64";
        let source_kind = if source == "managed" {
            "generated"
        } else {
            "uploaded"
        };
        json!({
            "source": {
                "device_profile_ref": TEST_KONKR_PROFILE_REF,
                "selected_recipe_refs": [recipe_id],
                "catalog": {
                    "sourceKind": "bundled",
                    "sourceId": "test-catalog",
                    "version": "1",
                    "contentDigest": {"algorithm": "sha256", "value": "catalog-digest"}
                }
            },
            "resolved_inputs": [{
                "recipeId": recipe_id,
                "inputId": "retroarch_cfg_source",
                "value": source
            }],
            "apps": [{"id": "retroarch", "package_id": package_id}],
            "steps": [{
                "id": format!("{recipe_id}/seed_retroarch_cfg"),
                "recipe_ref": recipe_id,
                "app_id": "retroarch",
                "type": "copy_files",
                "params": {
                    "source": {"Literal": {"value": {
                        "type": "configuration_artifact",
                        "sourceIdentity": {"recipeId": recipe_id, "kind": source_kind},
                        "targetIdentity": {
                            "appId": "retroarch",
                            "packageId": package_id,
                            "targetId": "config",
                            "target": {
                                "kind": "file",
                                "location": "external_app_data",
                                "path": "files/retroarch.cfg"
                            }
                        }
                    }}},
                    "dest": {"Literal": {"value": {"app_target": "config"}}},
                    "copy_policy": {"Literal": {"value": "create_only"}}
                }
            }]
        })
    }

    #[test]
    fn retroarch_config_target_is_derived_for_both_profiles_sources_and_recipes() {
        for profile_ref in [TEST_KONKR_PROFILE_REF, "konkr.pocket_fit"] {
            for recipe_id in RETROARCH_CONFIG_CREATE_RECIPES {
                for source in ["managed", "uploaded"] {
                    let mut plan = retroarch_config_create_plan(recipe_id, source);
                    plan["source"]["device_profile_ref"] = json!(profile_ref);
                    let target = resolve_retroarch_config_create_target(&plan)
                        .expect("authored configuration plan should validate")
                        .expect("selected configuration should require write evidence");
                    assert_eq!(target.app_id, "retroarch");
                    assert_eq!(target.package_id, "com.retroarch.aarch64");
                    assert_eq!(target.target_path, "files/retroarch.cfg");
                    assert_eq!(
                        target.destination,
                        "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg"
                    );
                    assert!(target.catalog_identity.contains("catalog-digest"));
                }
            }
        }
    }

    #[test]
    fn configuration_admission_does_not_depend_on_profile_identity() {
        let mut plan = retroarch_config_create_plan("app.retroarch.configure", "managed");
        plan["source"]
            .as_object_mut()
            .unwrap()
            .remove("device_profile_ref");

        assert!(resolve_retroarch_config_create_target(&plan)
            .unwrap()
            .is_some());
    }

    #[test]
    fn retroarch_configuration_target_is_gated_independent_of_profile() {
        let mut plan = retroarch_config_create_plan("app.retroarch.provision", "managed");
        plan["source"]["device_profile_ref"] = json!("konkr.pocket_fit");

        assert!(resolve_retroarch_config_create_target(&plan)
            .unwrap()
            .is_some());
    }

    #[test]
    fn config_create_target_under_an_unlisted_recipe_fails_closed() {
        let plan = retroarch_config_create_plan("app.retroarch.profile_specific_config", "managed");

        assert!(resolve_retroarch_config_create_target(&plan).is_err());
    }

    #[test]
    fn selected_configuration_recipe_with_pruned_input_and_step_fails_closed() {
        let mut plan = retroarch_config_create_plan("app.retroarch.provision", "managed");
        plan["resolved_inputs"] = json!([]);
        plan["steps"] = json!([]);

        assert!(resolve_retroarch_config_create_target(&plan).is_err());
    }

    #[test]
    fn only_explicit_none_can_opt_out_of_configuration_create_admission() {
        for missing_value in [false, true] {
            let mut plan = retroarch_config_create_plan("app.retroarch.provision", "managed");
            if missing_value {
                plan["resolved_inputs"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("value");
            } else {
                plan["resolved_inputs"][0]["value"] = Value::Null;
            }
            plan["steps"] = json!([]);

            assert_eq!(
                resolve_retroarch_config_create_target(&plan),
                Err("retroarch_config_create_plan_invalid"),
                "a missing or null resolved source must not behave like explicit none"
            );
        }
    }

    #[test]
    fn unrelated_plan_without_profile_identity_does_not_require_configuration_write_evidence() {
        let plan = json!({"steps": []});

        assert_eq!(resolve_retroarch_config_create_target(&plan), Ok(None));
    }

    #[test]
    fn explicit_none_keeps_unrelated_provisioning_outside_configuration_create_admission() {
        for profile_ref in [TEST_KONKR_PROFILE_REF, "konkr.pocket_fit"] {
            let plan = json!({
                "source": {
                    "device_profile_ref": profile_ref,
                    "selected_recipe_refs": ["app.retroarch.provision"]
                },
                "resolved_inputs": [{
                    "recipeId": "app.retroarch.provision",
                    "inputId": "retroarch_cfg_source",
                    "value": "none"
                }],
                "steps": [{"recipe_ref": "app.retroarch.provision", "id": "install_retroarch"}]
            });
            assert_eq!(resolve_retroarch_config_create_target(&plan), Ok(None));
        }
    }

    #[test]
    fn explicit_none_does_not_exempt_a_renamed_copy_to_the_config_destination() {
        let plan = json!({
            "source": {
                "selected_recipe_refs": ["app.retroarch.provision"],
                "catalog": {"sourceKind": "bundled", "sourceId": "test"}
            },
            "resolved_inputs": [{
                "recipeId": "app.retroarch.provision",
                "inputId": "retroarch_cfg_source",
                "value": "none"
            }],
            "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
            "steps": [{
                "id": "app.retroarch.provision/copy_cores",
                "recipe_ref": "app.retroarch.provision",
                "app_id": "retroarch",
                "type": "copy_files",
                "params": {
                    "source": {"Literal": {"value": {"type": "path_list", "value": []}}},
                    "dest": {"Literal": {"value": "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg"}},
                    "copy_policy": {"Literal": {"value": "merge"}}
                }
            }]
        });

        assert_eq!(
            resolve_retroarch_config_create_target(&plan),
            Err("retroarch_config_create_configuration_step_missing")
        );
    }

    #[test]
    fn generic_writers_to_the_config_parent_cannot_bypass_admission() {
        let package_id = "com.retroarch.aarch64";
        let destination = format!("/storage/emulated/0/Android/data/{package_id}/files");
        for (step_type, params) in [
            (
                "copy_files",
                json!({
                    "source": {"Literal": {"value": {
                        "type": "file_path",
                        "value": "/tmp/retroarch.cfg"
                    }}},
                    "copy_policy": {"Literal": {"value": "merge"}}
                }),
            ),
            (
                "extract_archive",
                json!({
                    "archive": {"Literal": {"value": "/tmp/config.zip"}},
                    "extract_on": {"Literal": {"value": "device"}}
                }),
            ),
        ] {
            let mut params = params;
            params["dest"] = json!({"Literal": {"value": destination.clone()}});
            let plan = json!({
                "source": {
                    "selected_recipe_refs": ["app.retroarch.provision"],
                    "catalog": {"sourceKind": "bundled", "sourceId": "test"}
                },
                "resolved_inputs": [{
                    "recipeId": "app.retroarch.provision",
                    "inputId": "retroarch_cfg_source",
                    "value": "none"
                }],
                "apps": [{"id": "retroarch", "package_id": package_id}],
                "steps": [{
                    "id": format!("app.retroarch.provision/{step_type}"),
                    "recipe_ref": "app.retroarch.provision",
                    "type": step_type,
                    "params": params
                }]
            });

            assert_eq!(
                resolve_retroarch_config_create_target(&plan),
                Err("retroarch_config_create_configuration_step_missing"),
                "{step_type} can create retroarch.cfg below its destination"
            );
        }
    }

    #[test]
    fn generic_copy_input_reference_to_config_parent_cannot_bypass_admission() {
        let package_id = "com.retroarch.aarch64";
        let plan = json!({
            "source": {
                "selected_recipe_refs": ["app.retroarch.provision"],
                "catalog": {"sourceKind": "bundled", "sourceId": "test"}
            },
            "resolved_inputs": [
                {
                    "recipeId": "app.retroarch.provision",
                    "inputId": "retroarch_cfg_source",
                    "value": "none"
                },
                {
                    "recipeId": "app.retroarch.provision",
                    "inputId": "config_parent",
            "key": "app.retroarch.provision/config_parent",
                    "type": "directory",
                    "value": format!(
                        "/storage/emulated/0/Android/data/{package_id}/files"
                    ),
                    "source": "explicit",
                    "label": "Configuration parent",
                    "required": false,
                    "sensitive": false
                }
            ],
            "apps": [{"id": "retroarch", "package_id": package_id}],
            "steps": [{
                "id": "app.retroarch.provision/copy_config_file",
                "recipe_ref": "app.retroarch.provision",
                "app_id": "retroarch",
                "type": "copy_files",
                "params": {
                    "source": {"Literal": {"value": {
                        "type": "file_path",
                        "value": "/tmp/retroarch.cfg"
                    }}},
                "dest": {"ref": "inputs.app.retroarch.provision/config_parent"},
                    "copy_policy": {"Literal": {"value": "merge"}}
                }
            }]
        });

        assert_eq!(
            resolve_retroarch_config_create_target(&plan),
            Err("retroarch_config_create_configuration_step_missing")
        );
    }

    #[test]
    fn generic_copy_with_unresolved_destination_reference_fails_closed() {
        let plan = json!({
            "source": {
                "selected_recipe_refs": ["app.retroarch.provision"],
                "catalog": {"sourceKind": "bundled", "sourceId": "test"}
            },
            "resolved_inputs": [{
                "recipeId": "app.retroarch.provision",
                "inputId": "retroarch_cfg_source",
                "value": "none"
            }],
            "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
            "steps": [{
                "id": "app.retroarch.provision/copy_dynamic",
                "recipe_ref": "app.retroarch.provision",
                "app_id": "retroarch",
                "type": "copy_files",
                "params": {
                    "source": {"Literal": {"value": {
                        "type": "file_path",
                        "value": "/tmp/retroarch.cfg"
                    }}},
                    "dest": {"ref": "steps.resolve.outputs.device_path"},
                    "copy_policy": {"Literal": {"value": "merge"}}
                }
            }]
        });

        assert_eq!(
            resolve_retroarch_config_create_target(&plan),
            Err("retroarch_config_create_destination_unresolved")
        );
    }

    #[test]
    fn unresolved_destination_reference_is_rejected_for_a_selected_retroarch_recipe() {
        let plan = json!({
            "source": {
                "selected_recipe_refs": ["app.retroarch.provision"],
                "catalog": {"sourceKind": "bundled", "sourceId": "test"}
            },
            "resolved_inputs": [{
                "recipeId": "app.retroarch.provision",
                "inputId": "retroarch_cfg_source",
                "value": "none"
            }],
            "steps": [{
                "id": "app.retroarch.provision/copy_dynamic",
                "recipe_ref": "app.retroarch.provision",
                "type": "copy_files",
                "params": {
                    "source": {"Literal": {"value": {
                        "type": "file_path",
                        "value": "/tmp/retroarch.cfg"
                    }}},
                    "dest": {"ref": "steps.resolve.outputs.device_path"},
                    "copy_policy": {"Literal": {"value": "merge"}}
                }
            }]
        });

        assert_eq!(
            resolve_retroarch_config_create_target(&plan),
            Err("retroarch_config_create_destination_unresolved")
        );
    }

    #[test]
    fn generic_copy_with_input_reference_to_config_parent_keeps_unrelated_files_outside_gate() {
        let plan = json!({
            "source": {
                "selected_recipe_refs": ["app.retroarch.provision"],
                "catalog": {"sourceKind": "bundled", "sourceId": "test"}
            },
            "resolved_inputs": [
                {
                    "recipeId": "app.retroarch.provision",
                    "inputId": "retroarch_cfg_source",
                    "value": "none"
                },
                {
                    "recipeId": "app.retroarch.provision",
                    "inputId": "config_parent",
                "key": "app.retroarch.provision/config_parent",
            "type": "directory",
            "value": "/storage/emulated/0/Android/data/com.retroarch.aarch64/files",
            "source": "explicit",
            "label": "Configuration parent",
            "required": false,
            "sensitive": false
                }
            ],
            "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
            "steps": [{
                "id": "app.retroarch.provision/copy_core",
                "recipe_ref": "app.retroarch.provision",
                "app_id": "retroarch",
                "type": "copy_files",
                "params": {
                    "source": {"Literal": {"value": {
                        "type": "file_path",
                        "value": "/tmp/core.so"
                    }}},
                "dest": {"ref": "inputs.app.retroarch.provision/config_parent"},
                    "copy_policy": {"Literal": {"value": "merge"}}
                }
            }]
        });

        assert_eq!(resolve_retroarch_config_create_target(&plan), Ok(None));
    }

    #[test]
    fn generic_copy_to_config_parent_with_an_unrelated_filename_keeps_legacy_admission() {
        let plan = json!({
            "source": {
                "selected_recipe_refs": ["app.retroarch.provision"],
                "catalog": {"sourceKind": "bundled", "sourceId": "test"}
            },
            "resolved_inputs": [{
                "recipeId": "app.retroarch.provision",
                "inputId": "retroarch_cfg_source",
                "value": "none"
            }],
            "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
            "steps": [{
                "id": "app.retroarch.provision/copy_core",
                "recipe_ref": "app.retroarch.provision",
                "type": "copy_files",
                "params": {
                    "source": {"Literal": {"value": {
                        "type": "file_path",
                        "value": "/tmp/core.so"
                    }}},
                    "dest": {"Literal": {"value":
                        "/storage/emulated/0/Android/data/com.retroarch.aarch64/files"
                    }},
                    "copy_policy": {"Literal": {"value": "merge"}}
                }
            }]
        });

        assert_eq!(resolve_retroarch_config_create_target(&plan), Ok(None));
    }

    #[test]
    fn recursive_generic_copy_to_an_ancestor_of_the_config_is_rejected() {
        let plan = json!({
            "source": {
                "selected_recipe_refs": ["app.retroarch.provision"],
                "catalog": {"sourceKind": "bundled", "sourceId": "test"}
            },
            "resolved_inputs": [{
                "recipeId": "app.retroarch.provision",
                "inputId": "retroarch_cfg_source",
                "value": "none"
            }],
            "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
            "steps": [{
                "id": "app.retroarch.provision/copy_tree",
                "recipe_ref": "app.retroarch.provision",
                "type": "copy_files",
                "params": {
                    "source": {"Literal": {"value": {
                        "type": "directory_path",
                        "value": "/tmp/retroarch-data"
                    }}},
                    "dest": {"Literal": {"value":
                        "/storage/emulated/0/Android/data/com.retroarch.aarch64"
                    }},
                    "copy_policy": {"Literal": {"value": "merge"}}
                }
            }]
        });

        assert_eq!(
            resolve_retroarch_config_create_target(&plan),
            Err("retroarch_config_create_configuration_step_missing")
        );
    }

    #[test]
    fn device_capable_archive_ancestors_are_treated_as_configuration_writers() {
        let package_id = "com.retroarch.aarch64";
        let archive_plan = |destination: &str, extract_on: Option<&str>, unresolved: bool| {
            let mut plan = json!({
                "source": {
                    "selected_recipe_refs": ["app.retroarch.provision"],
                    "catalog": {"sourceKind": "bundled", "sourceId": "test"}
                },
                "resolved_inputs": [{
                    "recipeId": "app.retroarch.provision",
                    "inputId": "retroarch_cfg_source",
                    "value": "none"
                }],
                "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
                "steps": [{
                    "id": "app.retroarch.provision/extract_archive",
                    "recipe_ref": "app.retroarch.provision",
                    "type": "extract_archive",
                    "params": {
                        "archive": {"Literal": {"value": "/tmp/retroarch-data.zip"}},
                        "dest": {"Literal": {"value": destination}}
                    }
                }]
            });
            if unresolved {
                plan["steps"][0]["params"]["dest"] =
                    json!({"ref": "steps.resolve.outputs.device_path"});
            }
            if let Some(extract_on) = extract_on {
                plan["steps"][0]["params"]["extract_on"] =
                    json!({"Literal": {"value": extract_on}});
            }
            plan
        };

        let destinations = [
            format!("/storage/emulated/0/Android/data/{package_id}/files/retroarch.cfg"),
            format!("/storage/emulated/0/Android/data/{package_id}/files"),
            format!("/storage/emulated/0/Android/data/{package_id}"),
            "/storage/emulated/0/Android/data".to_string(),
            format!("/sdcard/Android/data/{package_id}/files/cache/.."),
            format!("/storage/self/primary/Android/data/{package_id}/files/./cache/.."),
        ];

        for destination in &destinations {
            for extract_on in [Some("device"), Some("future_mode"), None] {
                assert_eq!(
                    resolve_retroarch_config_create_target(&archive_plan(
                        destination,
                        extract_on,
                        false
                    )),
                    Err("retroarch_config_create_configuration_step_missing"),
                    "archive extraction mode {extract_on:?} at {destination:?} can write below its destination"
                );
            }
        }

        for destination in &destinations {
            assert_eq!(
                resolve_retroarch_config_create_target(&archive_plan(
                    destination,
                    Some("host"),
                    false
                )),
                Ok(None),
                "explicit host extraction does not write to {destination:?} on the device"
            );
        }

        assert_eq!(
            resolve_retroarch_config_create_target(&archive_plan(
                "/../storage/emulated/0/Android/data/com.retroarch.aarch64/files",
                Some("device"),
                false
            )),
            Err("retroarch_config_create_destination_escapes_root")
        );
        assert_eq!(
            resolve_retroarch_config_create_target(&archive_plan(
                "/storage/emulated/0/Android/data/com.retroarch.aarch64",
                Some("device"),
                true
            )),
            Err("retroarch_config_create_destination_unresolved")
        );
    }

    #[test]
    fn conflicting_and_malformed_input_snapshots_fail_closed() {
        let destination = "/storage/emulated/0/Android/data/com.retroarch.aarch64/files";
        let make_plan = |destination_ref: &str, conflicting_snapshot: Value| {
            let mut plan = json!({
                "source": {
                    "selected_recipe_refs": ["app.retroarch.provision"],
                    "catalog": {"sourceKind": "bundled", "sourceId": "test"}
                },
                "resolved_inputs": [
                    {
                        "recipeId": "app.retroarch.provision",
                        "inputId": "retroarch_cfg_source",
                        "value": "none"
                    },
                    {
                        "recipeId": "app.retroarch.provision",
                        "inputId": "config_parent",
                        "key": "app.retroarch.provision/config_parent",
                        "value": destination
                    }
                ],
                "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
                "steps": [{
                    "id": "app.retroarch.provision/copy_core",
                    "recipe_ref": "app.retroarch.provision",
                    "app_id": "retroarch",
                    "type": "copy_files",
                    "params": {
                        "source": {"Literal": {"value": {
                            "type": "file_path",
                            "value": "/tmp/core.so"
                        }}},
                        "dest": {"ref": destination_ref},
                        "copy_policy": {"Literal": {"value": "merge"}}
                    }
                }]
            });
            plan["resolved_inputs"]
                .as_array_mut()
                .expect("resolved-input snapshot should be an array")
                .push(conflicting_snapshot);
            plan
        };

        let conflicting_key = make_plan(
            "inputs.app.retroarch.provision/config_parent",
            json!({
                "recipeId": "app.retroarch.provision",
                "inputId": "different_input",
                "key": "app.retroarch.provision/config_parent",
                "value": "/tmp/unrelated"
            }),
        );
        assert_eq!(
            resolve_retroarch_config_create_target(&conflicting_key),
            Err("retroarch_config_create_destination_unresolved")
        );

        let malformed_reference = make_plan(
            "inputs.app.retroarch.provision/config_parent/extra",
            json!({
                "recipeId": "app.retroarch.provision",
                "inputId": "config_parent/extra",
                "key": "app.retroarch.provision/config_parent/extra",
                "value": destination
            }),
        );
        assert_eq!(
            resolve_retroarch_config_create_target(&malformed_reference),
            Err("retroarch_config_create_destination_unresolved")
        );
    }

    #[test]
    fn qualified_input_references_require_a_production_shaped_consistent_snapshot() {
        let recipe_id = "app.retroarch.provision";
        let input_id = "config_parent";
        let reference = format!("inputs.{recipe_id}/{input_id}");
        let key = format!("{recipe_id}/{input_id}");
        let destination = "/storage/emulated/0/Android/data/com.retroarch.aarch64/files";
        let resolved_input = |row_recipe: &str, row_input: &str, input_type: &str, value: Value| {
            json!({
                "recipeId": row_recipe,
                "inputId": row_input,
                "key": key,
            "type": input_type,
            "value": value,
            "source": "explicit",
            "label": "Resolved input",
            "required": false,
            "sensitive": false
            })
        };
        let make_plan = |destination_ref: &str, rows: Vec<Value>, step_recipe: &str| {
            let mut resolved_inputs = vec![json!({
            "key": format!("{recipe_id}/retroarch_cfg_source"),
                "recipeId": recipe_id,
                "inputId": "retroarch_cfg_source",
            "type": "enum",
            "value": "none",
            "source": "recipe_default",
            "label": "Configuration source",
            "required": false,
            "sensitive": false
            })];
            resolved_inputs.extend(rows);
            json!({
                "source": {
                    "selected_recipe_refs": [recipe_id],
                    "catalog": {"sourceKind": "bundled", "sourceId": "test"}
                },
                "resolved_inputs": resolved_inputs,
                "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
                "steps": [{
                    "id": "app.retroarch.provision/copy_core",
                    "recipe_ref": step_recipe,
                    "app_id": "retroarch",
                    "type": "copy_files",
                    "params": {
                        "source": {"Literal": {"value": {
                            "type": "file_path",
                            "value": "/tmp/core.so"
                        }}},
                        "dest": {"ref": destination_ref},
                        "copy_policy": {"Literal": {"value": "merge"}}
                    }
                }]
            })
        };
        let valid_row = resolved_input(recipe_id, input_id, "directory", json!(destination));
        let mut missing_type = valid_row.clone();
        missing_type.as_object_mut().unwrap().remove("type");
        let mut missing_source = valid_row.clone();
        missing_source.as_object_mut().unwrap().remove("source");
        let resolved_input_with_canonical_key =
            |row_recipe: &str, row_input: &str, input_type: &str, value: Value| {
                json!({
                    "recipeId": row_recipe,
                    "inputId": row_input,
                    "key": format!("{row_recipe}/{row_input}"),
                    "type": input_type,
                    "value": value,
                    "source": "explicit",
                    "label": "Resolved input",
                    "required": false,
                    "sensitive": false
                })
            };
        let malformed_recipe_id = "app..retroarch.provision";
        let malformed_input_id = "config--parent";

        assert_eq!(
            resolve_retroarch_config_create_target(&make_plan(
                &reference,
                vec![valid_row.clone()],
                recipe_id
            )),
            Ok(None),
            "a valid unrelated destination input retains its existing behavior"
        );

        let cases = [
            (
                "duplicate identical key",
                reference.clone(),
                vec![valid_row.clone(), valid_row.clone()],
                recipe_id,
            ),
            (
                "wrong recipe identifier",
                reference.clone(),
                vec![resolved_input(
                    "app.retroarch.configure",
                    input_id,
                    "directory",
                    json!(destination),
                )],
                recipe_id,
            ),
            (
                "wrong input identifier",
                reference.clone(),
                vec![resolved_input(
                    recipe_id,
                    "different_input",
                    "directory",
                    json!(destination),
                )],
                recipe_id,
            ),
            ("missing row", reference.clone(), Vec::new(), recipe_id),
            (
                "null value",
                reference.clone(),
                vec![resolved_input(
                    recipe_id,
                    input_id,
                    "directory",
                    Value::Null,
                )],
                recipe_id,
            ),
            (
                "malformed value shape",
                reference.clone(),
                vec![resolved_input(
                    recipe_id,
                    input_id,
                    "directory",
                    json!([destination]),
                )],
                recipe_id,
            ),
            (
                "type and value mismatch",
                reference.clone(),
                vec![resolved_input(
                    recipe_id,
                    input_id,
                    "boolean",
                    json!(destination),
                )],
                recipe_id,
            ),
            (
                "missing snapshot type",
                reference.clone(),
                vec![missing_type],
                recipe_id,
            ),
            (
                "missing required snapshot provenance",
                reference.clone(),
                vec![missing_source],
                recipe_id,
            ),
            (
                "malformed recipe identifier",
                format!("inputs.{malformed_recipe_id}/{input_id}"),
                vec![resolved_input_with_canonical_key(
                    malformed_recipe_id,
                    input_id,
                    "directory",
                    json!(destination),
                )],
                malformed_recipe_id,
            ),
            (
                "malformed input identifier",
                format!("inputs.{recipe_id}/{malformed_input_id}"),
                vec![resolved_input_with_canonical_key(
                    recipe_id,
                    malformed_input_id,
                    "directory",
                    json!(destination),
                )],
                recipe_id,
            ),
            (
                "empty recipe identifier",
                "inputs./config_parent".to_string(),
                vec![valid_row.clone()],
                recipe_id,
            ),
            (
                "empty input identifier",
                format!("inputs.{recipe_id}/"),
                vec![valid_row.clone()],
                recipe_id,
            ),
            (
                "extra separator",
                format!("{reference}/extra"),
                vec![valid_row.clone()],
                recipe_id,
            ),
            (
                "owning step recipe mismatch",
                reference,
                vec![valid_row],
                "app.retroarch.configure",
            ),
        ];

        for (case, destination_ref, rows, step_recipe) in cases {
            assert_eq!(
                resolve_retroarch_config_create_target(&make_plan(
                    &destination_ref,
                    rows,
                    step_recipe
                )),
                Err("retroarch_config_create_destination_unresolved"),
                "{case} must fail closed"
            );
        }
    }

    #[test]
    fn malformed_and_aliased_config_destinations_never_look_unrelated() {
        let package_id = "com.retroarch.aarch64";
        let destinations = [
            format!("/../storage/emulated/0/Android/data/{package_id}/files/retroarch.cfg"),
            format!("/../../storage/emulated/0/Android/data/{package_id}/files/retroarch.cfg"),
            format!("/storage/emulated/0/Android/data/{package_id}/files/cache/../retroarch.cfg"),
            format!("/storage//emulated/0/Android/data/{package_id}/files/./retroarch.cfg"),
            format!("/sdcard/Android/data/{package_id}/files/retroarch.cfg"),
            format!("/storage/self/primary/Android/data/{package_id}/files/retroarch.cfg"),
            format!("../storage/emulated/0/Android/data/{package_id}/files/retroarch.cfg"),
            format!("storage/emulated/0/Android/data/{package_id}/files/retroarch.cfg"),
            format!("storage/emulated/0/Android/data/{package_id}/files/cache/../retroarch.cfg"),
        ];

        for destination in destinations {
            let mut plan = json!({
                "source": {
                    "selected_recipe_refs": ["app.retroarch.provision"],
                    "catalog": {"sourceKind": "bundled", "sourceId": "test"}
                },
                "resolved_inputs": [{
                    "recipeId": "app.retroarch.provision",
                    "inputId": "retroarch_cfg_source",
                    "value": "none"
                }],
                "apps": [{"id": "retroarch", "package_id": package_id}],
                "steps": [{
                    "id": "app.retroarch.provision/copy_cores",
                    "recipe_ref": "app.retroarch.provision",
                    "type": "copy_files",
                    "params": {
                        "source": {"Literal": {"value": {"type": "path_list", "value": []}}},
                        "dest": {"Literal": {"value": destination}},
                        "copy_policy": {"Literal": {"value": "merge"}}
                    }
                }]
            });
            plan["steps"][0].as_object_mut().unwrap().remove("app_id");

            assert!(
                resolve_retroarch_config_create_target(&plan).is_err(),
                "destination must not be treated as unrelated: {destination}"
            );
        }
    }

    #[test]
    fn malformed_step_metadata_and_path_aliases_cannot_exempt_config_destination() {
        for app_id_present in [true, false] {
            for destination in [
                "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg",
                "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/./retroarch.cfg",
            ] {
                let mut plan = json!({
                "source": {
                "selected_recipe_refs": ["app.retroarch.provision"],
                "catalog": {"sourceKind": "bundled", "sourceId": "test"}
                },
                "resolved_inputs": [{
                "recipeId": "app.retroarch.provision",
                "inputId": "retroarch_cfg_source",
                "value": "none"
                }],
                "apps": [{"id": "retroarch", "package_id": "com.retroarch.aarch64"}],
                "steps": [{
                "id": "app.retroarch.provision/copy_cores",
                "recipe_ref": "app.retroarch.provision",
                "app_id": "retroarch",
                "type": "copy_files",
                "params": {
                "source": {"Literal": {"value": {"type": "path_list", "value": []}}},
                "dest": {"Literal": {"value": destination}},
                "copy_policy": {"Literal": {"value": "merge"}}
                }
                }]
                });
                if !app_id_present {
                    plan["steps"][0].as_object_mut().unwrap().remove("app_id");
                }
                assert_eq!(
                    resolve_retroarch_config_create_target(&plan),
                    Err("retroarch_config_create_configuration_step_missing"),
                    "app_id_present={app_id_present} destination={destination}"
                );
            }
        }
    }
    #[test]
    fn selected_configuration_create_fails_closed_on_pruned_or_mismatched_target() {
        let mut missing_step = retroarch_config_create_plan("app.retroarch.configure", "managed");
        missing_step["steps"] = json!([]);
        assert_eq!(
            resolve_retroarch_config_create_target(&missing_step),
            Err("retroarch_config_create_configuration_step_missing")
        );

        let mut wrong_destination =
            retroarch_config_create_plan("app.retroarch.provision", "uploaded");
        wrong_destination["steps"][0]["params"]["dest"]["Literal"]["value"] =
            json!({"app_target": "other"});
        assert_eq!(
            resolve_retroarch_config_create_target(&wrong_destination),
            Err("retroarch_config_create_target_invalid")
        );

        let mut wrong_artifact = retroarch_config_create_plan("app.retroarch.configure", "managed");
        wrong_artifact["steps"][0]["params"]["source"]["Literal"]["value"]["sourceIdentity"]
            ["kind"] = json!("uploaded");
        assert_eq!(
            resolve_retroarch_config_create_target(&wrong_artifact),
            Err("retroarch_config_create_target_invalid")
        );

        let mut mixed = retroarch_config_create_plan("app.retroarch.provision", "managed");
        mixed["resolved_inputs"] = json!([
            {
                "recipeId": "app.retroarch.provision",
                "inputId": "retroarch_cfg_source",
                "value": "managed"
            },
            {
                "recipeId": "app.retroarch.configure",
                "inputId": "retroarch_cfg_source",
                "value": "uploaded"
            }
        ]);
        assert_eq!(
            resolve_retroarch_config_create_target(&mixed),
            Err("retroarch_config_create_plan_ambiguous")
        );
    }
}
