//! Resolve named App Definition targets before selecting executable steps.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::authored_models::{
    is_valid_target_path, AppDefinitionV1, AppTargetKind, AppTargetLocation,
};
use crate::model::{OrderedMap, ParamValue, Recipe, RecipeArtifact, Step};
use crate::runtime_refs::{parse_reference, RuntimeRef};
use crate::step_specs;

use super::{make_execution_step_id, reference_value_type, PlannerMessage};

/// A complete, immutable interpretation of one authored target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResolvedTarget {
    pub path: String,
    pub kind: AppTargetKind,
    pub capability: Option<&'static str>,
}

/// Resolve a target using its owning App Definition and the Android user-0 path contract.
pub(super) fn resolve_target(app: &AppDefinitionV1, target_id: &str) -> Option<ResolvedTarget> {
    let target = app.targets.get(target_id)?;
    if !is_valid_target_path(target.location, &target.path) {
        return None;
    }
    let (path, capability) = match target.location {
        AppTargetLocation::AppData => (
            format!("/data/user/0/{}/{}", app.package_id, target.path),
            Some("app_data_write"),
        ),
        AppTargetLocation::ExternalAppData => (
            format!(
                "/storage/emulated/0/Android/data/{}/{}",
                app.package_id, target.path
            ),
            Some("app_data_write"),
        ),
        AppTargetLocation::SharedStorage => (
            format!("/storage/emulated/0/{}", target.path),
            Some("shared_storage_write"),
        ),
        AppTargetLocation::AbsoluteDevicePath => (target.path.clone(), None),
    };
    Some(ResolvedTarget {
        path,
        kind: target.kind,
        capability,
    })
}

/// Resolved target operands and effective constraints for one authored step.
#[derive(Clone, Debug, Default)]
pub(super) struct ResolvedStepUsage {
    pub dest: Option<String>,
    pub dest_kind: Option<AppTargetKind>,
    pub skip_if_paths: Vec<Option<String>>,
    pub verify_paths: Vec<Option<String>>,
    pub capabilities: Vec<String>,
}

/// Validate target and App Artifact use on every expanded step, including steps
/// that device capability selection will later omit. The returned constraints
/// are used by both selection and immutable plan emission. Pass `None` for
/// catalog validation or capability projection, where input bindings do not
/// exist; planning passes effective bindings to validate input-backed policies.
pub(super) fn resolve_usage(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    apps: &HashMap<String, AppDefinitionV1>,
    bindings: Option<&OrderedMap<Value>>,
) -> Result<HashMap<String, ResolvedStepUsage>, Vec<PlannerMessage>> {
    let mut resolved = HashMap::new();
    let mut errors = Vec::new();
    for recipe_id in expanded_recipe_refs {
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        for step in &recipe.steps {
            let mut usage = ResolvedStepUsage {
                capabilities: step.constraints.capabilities.clone(),
                ..ResolvedStepUsage::default()
            };
            let app = step.app_ref.as_deref().and_then(|id| apps.get(id));
            if let Some(app_id) = step.app_ref.as_deref() {
                if step_specs::step_spec_for(&step.type_name)
                    .is_none_or(|spec| spec.app_context == step_specs::AppContextRequirement::None)
                {
                    errors.push(message("app_context_inapplicable", recipe_id, step));
                }
                if !crate::authored_models::is_valid_identifier(app_id) {
                    errors.push(message("app_reference_invalid", recipe_id, step));
                } else if app.is_none() {
                    errors.push(message("app_definition_unknown", recipe_id, step));
                } else if !crate::authored_models::validate_app_definition(app.unwrap()).is_empty()
                {
                    errors.push(message("app_definition_invalid", recipe_id, step));
                }
            }
            let mut consumed = false;
            if step.type_name == "copy_files" {
                if let Some(ParamValue::Ref(source)) = step.params.get("source") {
                    for source_app in source_app_refs(recipe, source, &mut HashSet::new()) {
                        consumed = true;
                        if step.app_ref.as_deref() != Some(source_app.as_str()) {
                            errors.push(message("app_source_provenance_mismatch", recipe_id, step));
                        }
                    }
                }
            }
            for (name, value) in &step.params {
                if let ParamValue::AppTarget(target_id) = value {
                    if step.type_name != "copy_files" || name != "dest" {
                        errors.push(message("app_target_inapplicable", recipe_id, step));
                    } else {
                        consumed = true;
                        match resolve_step_target(app, target_id, recipe_id, step) {
                            Ok(target) => {
                                check_copy_kind(
                                    recipe,
                                    step,
                                    bindings,
                                    &target,
                                    recipe_id,
                                    &mut errors,
                                );
                                add_capability(&mut usage.capabilities, target.capability);
                                usage.dest_kind = Some(target.kind);
                                usage.dest = Some(target.path);
                            }
                            Err(error) => errors.push(error),
                        }
                    }
                } else if let ParamValue::Literal(Value::Object(object)) = value {
                    if object.contains_key("app_target") {
                        errors.push(message("app_target_malformed", recipe_id, step));
                    }
                }
            }
            for (conditions, paths) in [
                (&step.skip_if, &mut usage.skip_if_paths),
                (&step.verify, &mut usage.verify_paths),
            ] {
                for condition in conditions {
                    if condition.params.contains_key("app_ref") {
                        errors.push(message(
                            "app_condition_context_inapplicable",
                            recipe_id,
                            step,
                        ));
                    }
                    let mut resolved_path = None;
                    for (name, value) in &condition.params {
                        if let Value::Object(object) = value {
                            if object.contains_key("app_target") {
                                consumed = true;
                                if name != "path"
                                    || !matches!(
                                        condition.type_name.as_str(),
                                        "path_exists" | "file_exists"
                                    )
                                {
                                    errors.push(message(
                                        "app_target_inapplicable",
                                        recipe_id,
                                        step,
                                    ));
                                } else if object.len() != 1
                                    || object.get("app_target").and_then(Value::as_str).is_none_or(
                                        |id| !crate::authored_models::is_valid_identifier(id),
                                    )
                                {
                                    errors.push(message("app_target_malformed", recipe_id, step));
                                } else {
                                    let target_id = object["app_target"].as_str().unwrap();
                                    match resolve_step_target(app, target_id, recipe_id, step) {
                                        Ok(target) => {
                                            if condition.type_name == "file_exists"
                                                && target.kind != AppTargetKind::File
                                            {
                                                errors.push(message(
                                                    "app_target_kind_invalid",
                                                    recipe_id,
                                                    step,
                                                ));
                                            }
                                            add_capability(
                                                &mut usage.capabilities,
                                                target.capability,
                                            );
                                            resolved_path = Some(target.path);
                                        }
                                        Err(error) => errors.push(error),
                                    }
                                }
                            }
                        }
                    }
                    paths.push(resolved_path);
                }
            }
            if step.type_name == "copy_files" && step.app_ref.is_some() && !consumed {
                errors.push(message("app_context_inapplicable", recipe_id, step));
            }
            resolved.insert(make_execution_step_id(recipe_id, &step.id), usage);
        }
    }
    if errors.is_empty() {
        Ok(resolved)
    } else {
        Err(errors)
    }
}

/// Follow a copy source through producer steps to the App Artifacts that
/// contributed its bytes. Generic recipe inputs and remote files return none.
fn source_app_refs(
    recipe: &Recipe,
    reference: &str,
    visited: &mut HashSet<String>,
) -> HashSet<String> {
    match parse_reference(reference) {
        Ok(RuntimeRef::ArtifactField { target_id, .. }) => match recipe.artifacts.get(&target_id) {
            Some(RecipeArtifact::AppArtifact(artifact)) => {
                HashSet::from([artifact.app_ref.clone()])
            }
            _ => HashSet::new(),
        },
        Ok(RuntimeRef::StepShorthand { target_id })
        | Ok(RuntimeRef::StepOutput { target_id, .. }) => {
            if !visited.insert(target_id.clone()) {
                return HashSet::new();
            }
            let Some(producer) = recipe.steps.iter().find(|step| step.id == target_id) else {
                return HashSet::new();
            };
            let mut apps = producer
                .params
                .values()
                .filter_map(|value| match value {
                    ParamValue::Ref(reference) => Some(reference),
                    _ => None,
                })
                .flat_map(|reference| source_app_refs(recipe, reference, visited))
                .collect::<HashSet<_>>();
            if producer.type_name == "extract_artifacts" {
                for artifact_id in super::expand_artifact_selection(
                    recipe,
                    producer.params.get("artifacts"),
                    producer.params.get("artifact_groups"),
                ) {
                    if let Some(RecipeArtifact::AppArtifact(artifact)) =
                        recipe.artifacts.get(&artifact_id)
                    {
                        apps.insert(artifact.app_ref.clone());
                    }
                }
            }
            apps
        }
        _ => HashSet::new(),
    }
}

fn resolve_step_target(
    app: Option<&AppDefinitionV1>,
    target_id: &str,
    recipe_id: &str,
    step: &Step,
) -> Result<ResolvedTarget, PlannerMessage> {
    if !crate::authored_models::is_valid_identifier(target_id) {
        return Err(message("app_target_malformed", recipe_id, step));
    }
    let Some(app) = app else {
        let code = if step.app_ref.is_some() {
            "app_definition_unknown"
        } else {
            "app_context_required"
        };
        return Err(message(code, recipe_id, step));
    };
    if !crate::authored_models::validate_app_definition(app).is_empty() {
        return Err(message("app_definition_invalid", recipe_id, step));
    }
    resolve_target(app, target_id).ok_or_else(|| message("app_target_unknown", recipe_id, step))
}

fn add_capability(capabilities: &mut Vec<String>, capability: Option<&'static str>) {
    if let Some(capability) = capability {
        if !capabilities.iter().any(|item| item == capability) {
            capabilities.push(capability.to_string());
        }
    }
}

fn check_copy_kind(
    recipe: &Recipe,
    step: &Step,
    bindings: Option<&OrderedMap<Value>>,
    target: &ResolvedTarget,
    recipe_id: &str,
    errors: &mut Vec<PlannerMessage>,
) {
    if target.kind == AppTargetKind::Directory {
        return;
    }
    let source_kind = match step.params.get("source") {
        Some(ParamValue::Ref(source)) => parse_reference(source)
            .ok()
            .and_then(|reference| reference_value_type(recipe, reference)),
        _ => None,
    };
    if source_kind.as_deref() != Some("file_path") {
        errors.push(message("app_target_kind_invalid", recipe_id, step));
    }
    let policy = match step.params.get("copy_policy") {
        Some(ParamValue::Literal(value)) => value.as_str(),
        Some(ParamValue::Ref(reference)) => match parse_reference(reference) {
            Ok(RuntimeRef::Input { target_id }) => bindings
                .and_then(|bindings| bindings.get(&format!("{recipe_id}/{target_id}")))
                .and_then(Value::as_str),
            _ => None,
        },
        _ => Some("merge"),
    };
    if policy == Some("sync") {
        errors.push(message(
            "app_target_sync_requires_directory",
            recipe_id,
            step,
        ));
    } else if policy.is_none() && bindings.is_some() {
        errors.push(message(
            "app_target_copy_policy_unresolved",
            recipe_id,
            step,
        ));
    }
}

fn message(code: &str, recipe_id: &str, step: &Step) -> PlannerMessage {
    PlannerMessage {
        code: code.to_string(),
        message: format!(
            "App-owned target use in step '{}' is invalid ({code}).",
            step.id
        ),
        details: json!({ "recipe_ref": recipe_id, "step_id": step.id }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authored_models::load_app_definition;
    use crate::authored_models::{AppTargetKind, AppTargetLocation, AppTargetV1};
    use crate::model::ParamValue;
    use crate::model::{AppArtifactReference, RecipeArtifact};
    use crate::yaml::load_recipe_from_path;
    use std::collections::HashMap;
    use std::path::Path;

    #[test]
    fn retroarch_targets_resolve_to_user_zero_paths_and_capabilities() {
        let app = load_app_definition(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored/apps/retroarch.yaml"),
        )
        .unwrap();
        let system = resolve_target(&app, "system").unwrap();
        assert_eq!(system.path, "/storage/emulated/0/RetroArch/system");
        assert_eq!(system.capability, Some("shared_storage_write"));
        let assets = resolve_target(&app, "assets").unwrap();
        assert_eq!(assets.path, "/data/user/0/com.retroarch.aarch64/assets");
        assert_eq!(assets.capability, Some("app_data_write"));
        let config = resolve_target(&app, "config").unwrap();
        assert_eq!(
            config.path,
            "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg"
        );
        assert_eq!(config.capability, Some("app_data_write"));
    }

    #[test]
    fn bios_target_resolution_supplies_selection_and_plan_constraints() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/feature.copy_bios.yaml")).unwrap();
        let step = &mut recipe.steps[0];
        step.app_ref = Some("retroarch".to_string());
        step.constraints.capabilities.clear();
        step.params.insert(
            "dest".to_string(),
            ParamValue::AppTarget("system".to_string()),
        );
        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);
        let resolved =
            resolve_usage(&recipes, &["feature.copy_bios".to_string()], &apps, None).unwrap();
        let step = &resolved["feature.copy_bios/copy_bios_dir"];
        assert_eq!(
            step.dest.as_deref(),
            Some("/storage/emulated/0/RetroArch/system")
        );
        assert_eq!(step.capabilities, ["shared_storage_write"]);
    }

    #[test]
    fn absolute_target_is_literal_and_invalid_path_is_rejected() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        app.targets.insert(
            "exact".to_string(),
            AppTargetV1 {
                kind: AppTargetKind::File,
                location: AppTargetLocation::AbsoluteDevicePath,
                path: "/opt/retroarch/config.ini".to_string(),
            },
        );
        let exact = resolve_target(&app, "exact").unwrap();
        assert_eq!(exact.path, "/opt/retroarch/config.ini");
        assert_eq!(exact.capability, None);
        app.targets.get_mut("exact").unwrap().path = "/opt/../config.ini".to_string();
        assert!(resolve_target(&app, "exact").is_none());
    }

    #[test]
    fn source_provenance_rejects_direct_cross_app_artifact() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/feature.copy_bios.yaml")).unwrap();
        recipe.artifacts.insert(
            "other_file".to_string(),
            RecipeArtifact::AppArtifact(AppArtifactReference {
                app_ref: "other_app".to_string(),
                artifact: "file".to_string(),
                input: None,
                cache: "default".to_string(),
            }),
        );
        recipe.steps[0].params.insert(
            "source".to_string(),
            ParamValue::Ref("artifacts.other_file.local_path".to_string()),
        );
        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);
        let errors =
            resolve_usage(&recipes, &["feature.copy_bios".to_string()], &apps, None).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "app_source_provenance_mismatch"));
    }

    #[test]
    fn source_provenance_allows_same_app_artifact() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/feature.copy_bios.yaml")).unwrap();
        recipe.artifacts.insert(
            "same_app_file".to_string(),
            RecipeArtifact::AppArtifact(AppArtifactReference {
                app_ref: "retroarch".to_string(),
                artifact: "file".to_string(),
                input: None,
                cache: "default".to_string(),
            }),
        );
        recipe.steps[0].params.insert(
            "source".to_string(),
            ParamValue::Ref("artifacts.same_app_file.local_path".to_string()),
        );
        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);

        let usage =
            resolve_usage(&recipes, &["feature.copy_bios".to_string()], &apps, None).unwrap();
        assert!(usage.contains_key("feature.copy_bios/copy_bios_dir"));
    }

    #[test]
    fn source_provenance_allows_generic_input_for_app_aware_consumer() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let recipe = load_recipe_from_path(root.join("recipes/feature.copy_bios.yaml")).unwrap();
        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);

        let usage =
            resolve_usage(&recipes, &["feature.copy_bios".to_string()], &apps, None).unwrap();
        assert!(usage.contains_key("feature.copy_bios/copy_bios_dir"));
    }

    #[test]
    fn source_provenance_rejects_cross_app_artifact_group_through_extraction_output() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/feature.copy_bios.yaml")).unwrap();
        recipe.artifacts.insert(
            "foreign_file".to_string(),
            RecipeArtifact::AppArtifact(AppArtifactReference {
                app_ref: "other_app".to_string(),
                artifact: "file".to_string(),
                input: None,
                cache: "default".to_string(),
            }),
        );
        recipe.artifact_groups.insert(
            "foreign_group".to_string(),
            vec!["foreign_file".to_string()],
        );

        let mut extraction = recipe.steps[0].clone();
        extraction.id = "extract_foreign_group".to_string();
        extraction.type_name = "extract_artifacts".to_string();
        extraction.app_ref = None;
        extraction.params.clear();
        extraction.params.insert(
            "artifact_groups".to_string(),
            ParamValue::Literal(json!(["foreign_group"])),
        );

        let mut consumer = recipe.steps[0].clone();
        consumer.params.insert(
            "source".to_string(),
            ParamValue::Ref("steps.extract_foreign_group.outputs.files".to_string()),
        );
        recipe.steps = vec![extraction, consumer];

        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);
        let errors =
            resolve_usage(&recipes, &["feature.copy_bios".to_string()], &apps, None).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "app_source_provenance_mismatch"));
    }

    #[test]
    fn source_provenance_rejects_cross_app_artifact_through_extraction_output() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/feature.copy_bios.yaml")).unwrap();
        recipe.artifacts.insert(
            "foreign_file".to_string(),
            RecipeArtifact::AppArtifact(AppArtifactReference {
                app_ref: "other_app".to_string(),
                artifact: "file".to_string(),
                input: None,
                cache: "default".to_string(),
            }),
        );

        let mut extraction = recipe.steps[0].clone();
        extraction.id = "extract_foreign_file".to_string();
        extraction.type_name = "extract_artifacts".to_string();
        extraction.app_ref = None;
        extraction.params.clear();
        extraction.params.insert(
            "artifacts".to_string(),
            ParamValue::Literal(json!(["foreign_file"])),
        );

        let mut consumer = recipe.steps[0].clone();
        consumer.params.insert(
            "source".to_string(),
            ParamValue::Ref("steps.extract_foreign_file.outputs.files".to_string()),
        );
        recipe.steps = vec![extraction, consumer];

        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);
        let errors =
            resolve_usage(&recipes, &["feature.copy_bios".to_string()], &apps, None).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "app_source_provenance_mismatch"));
    }

    #[test]
    fn source_provenance_rejects_cross_app_artifact_through_extract_archive_output() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/feature.copy_bios.yaml")).unwrap();
        recipe.artifacts.insert(
            "foreign_archive".to_string(),
            RecipeArtifact::AppArtifact(AppArtifactReference {
                app_ref: "other_app".to_string(),
                artifact: "file".to_string(),
                input: None,
                cache: "default".to_string(),
            }),
        );

        let mut extraction = recipe.steps[0].clone();
        extraction.id = "extract_foreign_archive".to_string();
        extraction.type_name = "extract_archive".to_string();
        extraction.app_ref = None;
        extraction.params.clear();
        extraction.params.insert(
            "archive".to_string(),
            ParamValue::Ref("artifacts.foreign_archive.local_path".to_string()),
        );

        let mut consumer = recipe.steps[0].clone();
        consumer.params.insert(
            "source".to_string(),
            ParamValue::Ref("steps.extract_foreign_archive.outputs.extracted_path".to_string()),
        );
        recipe.steps = vec![extraction, consumer];

        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);
        let errors =
            resolve_usage(&recipes, &["feature.copy_bios".to_string()], &apps, None).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "app_source_provenance_mismatch"));
    }

    #[test]
    fn input_backed_sync_rejects_exact_file_target() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/app.retroarch.provision.yaml")).unwrap();
        recipe
            .steps
            .iter_mut()
            .find(|step| step.id == "seed_retroarch_cfg")
            .unwrap()
            .params
            .insert(
                "copy_policy".to_string(),
                ParamValue::Ref("inputs.policy".to_string()),
            );
        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);
        let bindings = [("app.retroarch.provision/policy".to_string(), json!("sync"))]
            .into_iter()
            .collect();
        let errors = resolve_usage(
            &recipes,
            &["app.retroarch.provision".to_string()],
            &apps,
            Some(&bindings),
        )
        .unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "app_target_sync_requires_directory"));
    }

    #[test]
    fn generic_rom_copy_retains_its_authored_capability_and_destination_reference() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let recipe = load_recipe_from_path(root.join("recipes/feature.copy_roms.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let usage = resolve_usage(
            &recipes,
            &["feature.copy_roms".to_string()],
            &HashMap::new(),
            None,
        )
        .unwrap();
        let step = &usage["feature.copy_roms/copy_rom_library"];
        assert_eq!(step.capabilities, ["shared_storage_write"]);
        assert!(step.dest.is_none());
    }

    #[test]
    fn unknown_and_malformed_condition_targets_are_rejected_before_selection() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/feature.copy_bios.yaml")).unwrap();
        let step = &mut recipe.steps[0];
        step.params.insert(
            "dest".to_string(),
            ParamValue::AppTarget("absent".to_string()),
        );
        step.verify[0].params.insert(
            "path".to_string(),
            json!({ "app_target": "system", "unexpected": true }),
        );
        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);
        let errors =
            resolve_usage(&recipes, &["feature.copy_bios".to_string()], &apps, None).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "app_target_unknown"));
        assert!(errors
            .iter()
            .any(|error| error.code == "app_target_malformed"));
    }

    #[test]
    fn copy_app_context_requires_an_app_owned_operand() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
        let mut recipe =
            load_recipe_from_path(root.join("recipes/feature.copy_roms.yaml")).unwrap();
        recipe.steps[0].app_ref = Some("retroarch".to_string());
        let app = load_app_definition(root.join("apps/retroarch.yaml")).unwrap();
        let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
        let apps = HashMap::from([(app.id.clone(), app)]);
        let errors =
            resolve_usage(&recipes, &["feature.copy_roms".to_string()], &apps, None).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "app_context_inapplicable"));
    }
}
