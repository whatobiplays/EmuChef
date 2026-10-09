//! App Definition authority checks and projections used by execution planning.
//!
//! This module validates Recipe references against the App Definition catalog
//! and converts app-owned facts into the immutable values carried by an
//! Execution Plan.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::authored_models::{
    AppArtifactKind, AppArtifactSource, AppDefinitionV1, ReleaseProvider,
};
use crate::model::{AppArtifactReference, ParamValue, Recipe, RecipeArtifact, Step, StepCondition};
use crate::step_specs;

use super::{
    expand_artifact_selection, has_dependency_ancestor, ExecutionAppSnapshot,
    ExecutionArtifactAppProvenance, ExecutionArtifactSource, ExecutionStepCondition,
    PlannerMessage,
};

/// Validate the App Definition facts and authored app context used by a plan.
pub(super) fn validate_app_authority(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    selected_steps: &[(String, String, Step)],
    app_definitions: &HashMap<String, AppDefinitionV1>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    let mut invalid_app_ids = HashSet::new();
    for app_id in referenced_app_ids(recipes, expanded_recipe_refs, selected_steps) {
        let Some(app) = app_definitions.get(&app_id) else {
            continue;
        };
        let diagnostics = crate::authored_models::validate_app_definition(app);
        if diagnostics.is_empty() {
            continue;
        }
        invalid_app_ids.insert(app_id.clone());
        errors.push(app_authority_error(
            "app_definition_invalid",
            "A referenced App Definition does not satisfy its authored schema.",
            json!({
                "app_id": app_id,
                "diagnostics": diagnostics.iter().map(|diagnostic| json!({
                    "code": diagnostic.code,
                    "field": diagnostic.field,
                })).collect::<Vec<_>>(),
            }),
        ));
    }
    for recipe_id in expanded_recipe_refs {
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        for (artifact_id, artifact) in &recipe.artifacts {
            let RecipeArtifact::AppArtifact(reference) = artifact else {
                continue;
            };
            if !crate::authored_models::is_valid_identifier(&reference.app_ref) {
                errors.push(app_authority_error(
                    "app_reference_invalid",
                    "A Recipe App Artifact must use a valid App Definition identifier.",
                    json!({ "recipe_ref": recipe_id, "artifact_id": artifact_id }),
                ));
                continue;
            }
            let Some(app) = app_definitions.get(&reference.app_ref) else {
                errors.push(app_authority_error(
                    "app_definition_unknown",
                    "A recipe artifact refers to an App Definition that is not available.",
                    json!({ "recipe_ref": recipe_id, "artifact_id": artifact_id }),
                ));
                continue;
            };
            if invalid_app_ids.contains(&reference.app_ref) {
                continue;
            }
            let Some(app_artifact) = app.artifacts.get(&reference.artifact) else {
                errors.push(app_authority_error(
                    "app_artifact_unknown",
                    "A recipe artifact refers to an unknown App Definition artifact.",
                    json!({ "recipe_ref": recipe_id, "artifact_id": artifact_id }),
                ));
                continue;
            };
            if execution_artifact_source(app_artifact).is_err() {
                errors.push(app_authority_error(
                    "app_artifact_source_unsupported",
                    "The referenced App Definition artifact source is not supported by this execution path.",
                    json!({ "recipe_ref": recipe_id, "artifact_id": artifact_id }),
                ));
            }
            if !matches!(reference.cache.as_str(), "default" | "none") {
                errors.push(app_authority_error(
                    "app_artifact_cache_unsupported",
                    "A Recipe App Artifact cache policy must be 'default' or 'none'.",
                    json!({ "recipe_ref": recipe_id, "artifact_id": artifact_id }),
                ));
            }
        }
    }

    for (_, recipe_id, step) in selected_steps {
        let Some(app_ref) = step.app_ref.as_deref() else {
            let recipe = recipes.get(recipe_id);
            let app_context_required =
                step_specs::step_spec_for(&step.type_name).is_some_and(|spec| {
                    spec.app_context == step_specs::AppContextRequirement::Required
                        && !recipe.is_some_and(|recipe| is_legacy_app_step_form(recipe, step))
                });
            if app_context_required {
                errors.push(app_authority_error(
                    "app_context_required",
                    "This App Definition step form requires an app context.",
                    json!({ "recipe_ref": recipe_id, "step_id": step.id }),
                ));
            }
            if step.skip_if.iter().chain(&step.verify).any(|condition| {
                condition.type_name == "package_installed" && condition.params.is_empty()
            }) {
                errors.push(app_authority_error(
                    "app_condition_context_required",
                    "A parameterless package_installed condition requires an app context on its step.",
                    json!({ "recipe_ref": recipe_id, "step_id": step.id }),
                ));
            }
            continue;
        };
        if !crate::authored_models::is_valid_identifier(app_ref) {
            errors.push(app_authority_error(
                "app_reference_invalid",
                "A step must use a valid App Definition identifier.",
                json!({ "recipe_ref": recipe_id, "step_id": step.id }),
            ));
            continue;
        }
        let Some(app) = app_definitions.get(app_ref) else {
            errors.push(app_authority_error(
                "app_definition_unknown",
                "A step refers to an App Definition that is not available.",
                json!({ "recipe_ref": recipe_id, "step_id": step.id }),
            ));
            continue;
        };
        if invalid_app_ids.contains(app_ref) {
            continue;
        }
        let Some(spec) = step_specs::step_spec_for(&step.type_name) else {
            errors.push(app_authority_error(
                "app_context_inapplicable",
                "This step type does not support an App Definition context.",
                json!({ "recipe_ref": recipe_id, "step_id": step.id }),
            ));
            continue;
        };
        if spec.app_context == step_specs::AppContextRequirement::None {
            errors.push(app_authority_error(
                "app_context_inapplicable",
                "This step type does not support an App Definition context.",
                json!({ "recipe_ref": recipe_id, "step_id": step.id }),
            ));
            continue;
        }
        for condition in step.skip_if.iter().chain(&step.verify) {
            if condition.type_name == "package_installed" && !condition.params.is_empty() {
                errors.push(app_authority_error(
                    "app_condition_package_parameter_forbidden",
                    "An app-context package_installed condition inherits its package from the step and takes no parameters.",
                    json!({ "recipe_ref": recipe_id, "step_id": step.id }),
                ));
            }
        }
        if step.type_name == "install_apk" {
            let Some(recipe) = recipes.get(recipe_id) else {
                continue;
            };
            errors.extend(validate_app_install_binding(
                recipe,
                step,
                app,
                selected_steps,
            ));
        }
    }
    errors
}

/// Validate app authority across every authored step in one Recipe.
///
/// Catalog-backed authoring validation uses the same checks as execution
/// planning while keeping this interface available from the planner module.
pub(crate) fn validate_recipe_app_authority(
    recipe: &Recipe,
    app_definitions: &[AppDefinitionV1],
) -> Vec<PlannerMessage> {
    let recipes = HashMap::from([(recipe.id.clone(), recipe.clone())]);
    let expanded_recipe_refs = vec![recipe.id.clone()];
    let selected_steps = recipe
        .steps
        .iter()
        .cloned()
        .map(|step| (recipe.id.clone(), recipe.id.clone(), step))
        .collect::<Vec<_>>();
    let app_definitions = app_definitions
        .iter()
        .cloned()
        .map(|app| (app.id.clone(), app))
        .collect::<HashMap<_, _>>();

    validate_app_authority(
        &recipes,
        &expanded_recipe_refs,
        &selected_steps,
        &app_definitions,
    )
}

fn is_legacy_app_step_form(recipe: &Recipe, step: &Step) -> bool {
    match step.type_name.as_str() {
        "install_apk" => !step.params.get("app").is_some_and(|value| {
            let ParamValue::Ref(reference) = value else {
                return false;
            };
            let Some(artifact_id) = reference
                .strip_prefix("artifacts.")
                .and_then(|value| value.strip_suffix(".local_path"))
            else {
                return false;
            };
            matches!(
                recipe.artifacts.get(artifact_id),
                Some(RecipeArtifact::AppArtifact(_))
            )
        }),
        "grant_permissions" => ["runtime", "appops", "policy"]
            .iter()
            .any(|parameter| step.params.contains_key(*parameter)),
        "launch_app" | "force_stop_app" => step.params.contains_key("package_name"),
        _ => false,
    }
}

fn validate_app_install_binding(
    recipe: &Recipe,
    step: &Step,
    app: &AppDefinitionV1,
    selected_steps: &[(String, String, Step)],
) -> Vec<PlannerMessage> {
    let duplicated_identity_fields = ["expected_package_name", "expected_sha256"]
        .into_iter()
        .filter(|field| step.params.contains_key(*field))
        .collect::<Vec<_>>();
    if !duplicated_identity_fields.is_empty() {
        return vec![app_authority_error(
            "app_install_authority_override",
            "An App Definition install must not duplicate package or checksum authority in Recipe parameters.",
            json!({
                "recipe_ref": recipe.id,
                "step_id": step.id,
                "fields": duplicated_identity_fields,
            }),
        )];
    }
    let artifact_ref = step.params.get("app").and_then(|value| match value {
        ParamValue::Ref(value) => value
            .strip_prefix("artifacts.")?
            .strip_suffix(".local_path"),
        ParamValue::Literal(_) => None,
    });
    let Some(artifact_id) = artifact_ref else {
        return vec![app_authority_error(
            "app_install_artifact_provenance_invalid",
            "An app-context APK install must use the local path of its declared App Definition artifact.",
            json!({ "recipe_ref": recipe.id, "step_id": step.id }),
        )];
    };
    let Some(RecipeArtifact::AppArtifact(reference)) = recipe.artifacts.get(artifact_id) else {
        return vec![app_authority_error(
            "app_install_artifact_provenance_invalid",
            "An app-context APK install must use the local path of its declared App Definition artifact.",
            json!({ "recipe_ref": recipe.id, "step_id": step.id }),
        )];
    };
    if reference.app_ref != app.id
        || app
            .artifacts
            .get(&reference.artifact)
            .is_none_or(|artifact| artifact.kind != AppArtifactKind::Apk)
    {
        return vec![app_authority_error(
            "app_install_artifact_provenance_invalid",
            "The APK install step and its artifact must identify the same App Definition and APK artifact.",
            json!({ "recipe_ref": recipe.id, "step_id": step.id }),
        )];
    }
    let step_by_id = selected_steps
        .iter()
        .filter(|(_, selected_recipe_id, _)| selected_recipe_id == &recipe.id)
        .map(|(_, _, selected_step)| (selected_step.id.as_str(), selected_step))
        .collect::<HashMap<_, _>>();
    let has_resolver_ancestor = selected_steps.iter().any(|(_, recipe_id, candidate)| {
        if recipe_id != &recipe.id || candidate.type_name != "resolve_artifacts" {
            return false;
        }
        let selects_artifact = expand_artifact_selection(
            recipe,
            candidate.params.get("artifacts"),
            candidate.params.get("artifact_groups"),
        )
        .iter()
        .any(|selected_artifact| selected_artifact == artifact_id);
        selects_artifact
            && has_dependency_ancestor(step, &candidate.id, &step_by_id, &mut HashSet::new())
    });
    if !has_resolver_ancestor {
        return vec![app_authority_error(
            "app_install_resolution_dependency_missing",
            "An app-context APK install must have a dependency ancestor that resolves its App Definition artifact.",
            json!({ "recipe_ref": recipe.id, "step_id": step.id }),
        )];
    }
    Vec::new()
}

pub(super) fn app_authority_error(code: &str, message: &str, details: Value) -> PlannerMessage {
    PlannerMessage {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

fn execution_artifact_source(
    artifact: &crate::authored_models::AppArtifactV1,
) -> Result<ExecutionArtifactSource, ()> {
    match &artifact.source {
        AppArtifactSource::DirectUrl { url, sha256 }
            if artifact.kind == crate::authored_models::AppArtifactKind::Apk =>
        {
            Ok(ExecutionArtifactSource::DirectUrl {
                url: url.clone(),
                sha256: sha256.clone(),
            })
        }
        AppArtifactSource::LatestRelease {
            provider: ReleaseProvider::Github,
            base_url,
            repository,
            asset_pattern: Some(asset_pattern),
            invert_asset_pattern,
            prerelease: false,
        } if base_url == "https://github.com"
            && !repository.is_empty()
            && invert_asset_pattern != &Some(true)
            && regex::Regex::new(asset_pattern).is_ok() =>
        {
            Ok(ExecutionArtifactSource::RemoteRelease {
                provider: "github".to_string(),
                service_origin: base_url.clone(),
                repository: repository.clone(),
                include_prereleases: false,
                asset_pattern: asset_pattern.clone(),
            })
        }
        _ => Err(()),
    }
}

fn referenced_app_ids(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    ordered_steps: &[(String, String, Step)],
) -> Vec<String> {
    let mut ids = Vec::new();
    for recipe_id in expanded_recipe_refs {
        if let Some(recipe) = recipes.get(recipe_id) {
            for artifact in recipe.artifacts.values() {
                if let RecipeArtifact::AppArtifact(reference) = artifact {
                    if !ids.contains(&reference.app_ref) {
                        ids.push(reference.app_ref.clone());
                    }
                }
            }
        }
    }
    for (_, _, step) in ordered_steps {
        if let Some(app_ref) = &step.app_ref {
            if !ids.contains(app_ref) {
                ids.push(app_ref.clone());
            }
        }
    }
    ids
}

/// Capture the referenced Apps in the same stable order used by the plan.
pub(super) fn execution_app_snapshots(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    ordered_steps: &[(String, String, Step)],
    app_definitions: &HashMap<String, AppDefinitionV1>,
) -> Vec<ExecutionAppSnapshot> {
    referenced_app_ids(recipes, expanded_recipe_refs, ordered_steps)
        .into_iter()
        .filter_map(|app_id| app_definitions.get(&app_id))
        .map(|app| ExecutionAppSnapshot {
            id: app.id.clone(),
            name: app.name.clone(),
            description: app.description.clone(),
            category: app.category.clone(),
            package_id: app.package_id.clone(),
        })
        .collect()
}

/// Project a Recipe-owned App Artifact reference into its reviewed source and provenance.
pub(super) fn execution_app_artifact_projection(
    reference: &AppArtifactReference,
    app_definitions: &HashMap<String, AppDefinitionV1>,
) -> Option<(ExecutionArtifactSource, ExecutionArtifactAppProvenance)> {
    let app = app_definitions.get(&reference.app_ref)?;
    let app_artifact = app.artifacts.get(&reference.artifact)?;
    let source = execution_artifact_source(app_artifact).ok()?;
    let provenance = ExecutionArtifactAppProvenance {
        app_id: app.id.clone(),
        artifact_id: reference.artifact.clone(),
        kind: match app_artifact.kind {
            AppArtifactKind::Apk => "apk",
            AppArtifactKind::File => "file",
        }
        .to_string(),
    };
    Some((source, provenance))
}

/// Attach the enclosing step's app identity to parameterless package conditions.
pub(super) fn execution_condition(
    condition: &StepCondition,
    app_ref: Option<&str>,
    app_definitions: &HashMap<String, AppDefinitionV1>,
) -> ExecutionStepCondition {
    ExecutionStepCondition {
        type_name: condition.type_name.clone(),
        app_id: if condition.type_name == "package_installed" && condition.params.is_empty() {
            app_ref.and_then(|app_ref| app_definitions.get(app_ref).map(|app| app.id.clone()))
        } else {
            None
        },
        params: condition.params.clone(),
    }
}
