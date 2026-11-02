//! Authored-root catalog context used by validation.
//!
//! Validation scans these authored-root-relative top-level patterns:
//! `apps/*.y*ml`, `recipes/*.y*ml`,
//! `device_profiles/*.y*ml`, and `device_plans/*.y*ml`. The catalog models
//! recipe metadata needed for dependency diagnostics and validates typed app
//! definitions, device profiles, and Device Plans. It does not build planner
//! data structures, execution plans, or executor state.

#[cfg(test)]
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::app_definition_admission::contains_app_target_key;
use crate::authored_models::{self, AppDefinitionV1, AuthoredModelDiagnostic};
use crate::model::{ParamValue, Recipe};
use crate::yaml;

pub const AUTHORED_CATALOG_GLOBS: &[&str] = &[
    "apps/*.y*ml",
    "recipes/*.y*ml",
    "device_profiles/*.y*ml",
    "device_plans/*.y*ml",
];

#[derive(Clone, Debug, Default)]
pub(crate) struct ValidationCatalog {
    recipes: HashMap<String, Recipe>,
    recipe_files: HashMap<String, Vec<PathBuf>>,
    app_definitions: Vec<crate::authored_models::AppDefinitionV1>,
    app_definition_diagnostics: Vec<Value>,
}

#[cfg(test)]
thread_local! {
    static VALIDATION_CATALOG_BUILD_COUNT: Cell<usize> = const { Cell::new(0) };
}

fn recipe_references_app(recipe: &Recipe, app_id: &str) -> bool {
    recipe.artifacts.values().any(|artifact| match artifact {
        crate::model::RecipeArtifact::AppArtifact(reference) => reference.app_ref == app_id,
        _ => false,
    }) || recipe
        .steps
        .iter()
        .any(|step| step.app_ref.as_deref() == Some(app_id))
        || recipe
            .config
            .as_ref()
            .is_some_and(|config| config.app_ref == app_id)
}

pub(crate) fn recipe_references_any_app(recipe: &Recipe) -> bool {
    recipe
        .artifacts
        .values()
        .any(|artifact| matches!(artifact, crate::model::RecipeArtifact::AppArtifact(_)))
        || recipe.config.is_some()
        || recipe.steps.iter().any(|step| {
            step.app_ref.is_some()
                || step.params.values().any(|value| match value {
                    ParamValue::AppTarget(_) => true,
                    ParamValue::Literal(value) => contains_app_target_key(value),
                    ParamValue::Ref(_) => false,
                })
                || step
                    .skip_if
                    .iter()
                    .chain(step.verify.iter())
                    .any(|condition| condition.params.values().any(contains_app_target_key))
        })
}

pub fn normalize_authored_root(authored_root: Option<&str>, recipe_path: &Path) -> Option<PathBuf> {
    if let Some(root) = authored_root {
        let root_path = resolved_path(root);
        let authored_candidate = root_path.join("authored");
        if root_path.file_name().and_then(|name| name.to_str()) == Some("authored")
            && root_path.join("recipes").is_dir()
        {
            return Some(root_path);
        }
        if authored_candidate.is_dir() && authored_candidate.join("recipes").is_dir() {
            return Some(authored_candidate);
        }
        return Some(root_path);
    }

    infer_authored_root(recipe_path)
}

pub fn validate_recipe_with_catalog(
    file: &str,
    recipe: &Recipe,
    recipe_path: &Path,
    authored_root: &Path,
) -> Vec<Value> {
    match ValidationCatalog::collect(authored_root) {
        Ok(catalog) => catalog.validate_recipe(file, recipe, recipe_path, authored_root),
        Err(error) => vec![recipe_catalog_unreadable_diagnostic(&error)],
    }
}

/// Every successfully loaded app definition plus catalog-validation diagnostics.
///
/// Callers that only need validation read the diagnostics. Callers that also
/// project app identity read the typed entries, and must treat a non-empty
/// diagnostic list as an invalid catalog.
pub struct AppDefinitionCatalog {
    pub entries: Vec<(PathBuf, AppDefinitionV1)>,
    pub diagnostics: Vec<Value>,
}

/// Editor-facing projection of the App Definition choices a catalog defines.
///
/// The projection is the only catalog representation the Config Editor
/// consumes for structured App-aware controls. A document session computes it
/// once per diagnostics refresh and reuses it for every DTO serialization so
/// serializing a document never re-scans the authored catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppCatalogProjection {
    status: AppCatalogStatus,
    apps: Vec<ProjectedAppDefinition>,
    diagnostics: Vec<Value>,
}

/// Whether structured App choices are currently available.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppCatalogStatus {
    /// No authored root is bound to the document; no App choices exist.
    #[default]
    Unbound,
    /// The catalog loaded without diagnostics; bounded choices are available.
    Ready,
    /// The catalog reported diagnostics; bounded choices are closed.
    Invalid,
}

impl AppCatalogStatus {
    fn name(self) -> &'static str {
        match self {
            AppCatalogStatus::Unbound => "unbound",
            AppCatalogStatus::Ready => "ready",
            AppCatalogStatus::Invalid => "invalid",
        }
    }
}

/// Sanitized selections one loaded App Definition offers to the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedAppDefinition {
    pub id: String,
    pub name: String,
    pub artifacts: Vec<ProjectedAppArtifact>,
    pub permission_sets: Vec<String>,
    pub targets: Vec<ProjectedAppTarget>,
    pub configurations: Vec<ProjectedAppConfiguration>,
    pub launcher_activity: Option<String>,
}
/// One App Configuration Definition exposed through the catalog projection.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedAppConfiguration {
    pub id: String,
    pub format: String,
    pub target: String,
    pub templates: Vec<ProjectedAppConfigurationTemplate>,
    pub managed_properties: Vec<ProjectedAppManagedProperty>,
}
/// One template pointer owned by an App Configuration Definition.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedAppConfigurationTemplate {
    pub id: String,
    pub path: String,
}
/// One typed property and its App Definition constraints.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedAppManagedProperty {
    pub id: String,
    pub path: Vec<String>,
    pub type_name: String,
    pub default: Option<Value>,
    pub clearable: bool,
    pub validators: authored_models::AppManagedPropertyValidatorsV1,
}

/// One App Definition artifact choice.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedAppArtifact {
    pub id: String,
    pub kind: String,
    pub requires_input: bool,
}

/// One App Definition target choice.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedAppTarget {
    pub id: String,
    pub kind: String,
    pub location: String,
    pub path: String,
}

impl AppCatalogProjection {
    /// Serialize the projection into the editor document protocol shape.
    pub fn to_dto(&self) -> Value {
        json!({
            "status": self.status.name(),
            "apps": self
                .apps
                .iter()
                .map(|app| {
                    json!({
                        "id": app.id,
                        "name": app.name,
                        "artifacts": app
                            .artifacts
                            .iter()
                            .map(|artifact| {
                                json!({
                                    "id": artifact.id,
                                    "kind": artifact.kind,
                                    "requiresInput": artifact.requires_input,
                                })
                            })
                            .collect::<Vec<_>>(),
                        "permissionSets": app.permission_sets,
                        "targets": app
                            .targets
                            .iter()
                            .map(|target| {
                                json!({
                                    "id": target.id,
                                    "kind": target.kind,
                                    "location": target.location,
                                    "path": target.path,
                                })
                            })
                            .collect::<Vec<_>>(),
                "configurations": app
                    .configurations
                    .iter()
                    .map(projected_configuration_to_dto)
                    .collect::<Vec<_>>(),
                        "launcherActivity": app.launcher_activity,
                    })
                })
                .collect::<Vec<_>>(),
            "diagnostics": self.diagnostics,
        })
    }
}

fn projected_configuration_to_dto(configuration: &ProjectedAppConfiguration) -> Value {
    let mut dto = json!({
        "id": configuration.id,
        "format": configuration.format,
        "target": configuration.target,
        "templates": configuration.templates.iter().map(|template| json!({
            "id": template.id,
            "path": template.path,
        })).collect::<Vec<_>>(),
    });
    let properties = configuration
        .managed_properties
        .iter()
        .map(|property| {
            let mut validators = json!({
                "allowedValues": property.validators.allowed_values,
            });
            if let Some(minimum) = &property.validators.minimum {
                validators["minimum"] = minimum.clone();
            }
            if let Some(maximum) = &property.validators.maximum {
                validators["maximum"] = maximum.clone();
            }
            if let Some(min_length) = property.validators.min_length {
                validators["minLength"] = json!(min_length);
            }
            if let Some(max_length) = property.validators.max_length {
                validators["maxLength"] = json!(max_length);
            }
            let mut projected = json!({
                "id": property.id,
                "path": property.path,
                "type": property.type_name,
            "clearable": property.clearable,
                "validators": validators,
            });
            if let Some(default) = &property.default {
                projected["default"] = default.clone();
            }
            projected
        })
        .collect::<Vec<_>>();
    dto["managedProperties"] = json!(properties);
    dto
}

/// Project the catalog bound to one document session.
///
/// An unbound session reports no choices. A catalog with any diagnostic is
/// invalid and reports no App choices at all so the editor can never present a
/// partially trusted catalog. App ids are sorted so repeated loads produce
/// identical projections.
pub fn project_app_catalog(authored_root: Option<&Path>) -> AppCatalogProjection {
    let Some(authored_root) = authored_root else {
        return AppCatalogProjection::default();
    };
    let AppDefinitionCatalog {
        entries,
        diagnostics,
    } = load_app_definition_catalog(authored_root);
    if !diagnostics.is_empty() {
        return AppCatalogProjection {
            status: AppCatalogStatus::Invalid,
            apps: Vec::new(),
            diagnostics,
        };
    }
    let mut apps = entries
        .into_iter()
        .map(|(_, app)| project_app_definition(&app))
        .collect::<Vec<_>>();
    apps.sort_by(|left, right| left.id.cmp(&right.id));
    AppCatalogProjection {
        status: AppCatalogStatus::Ready,
        apps,
        diagnostics,
    }
}

fn project_app_definition(app: &AppDefinitionV1) -> ProjectedAppDefinition {
    let mut permission_sets = Vec::new();
    if let Some(sets) = &app.permission_sets {
        if sets.baseline.is_some() {
            permission_sets.push("baseline".to_string());
        }
        if sets.elevated.is_some() {
            permission_sets.push("elevated".to_string());
        }
    }
    ProjectedAppDefinition {
        id: app.id.clone(),
        name: app.name.clone(),
        artifacts: app
            .artifacts
            .iter()
            .map(|(id, artifact)| ProjectedAppArtifact {
                id: id.clone(),
                kind: serialized_enum_name(artifact.kind),
                requires_input: matches!(
                    artifact.source,
                    crate::authored_models::AppArtifactSource::UserProvided
                ),
            })
            .collect(),
        permission_sets,
        targets: app
            .targets
            .iter()
            .map(|(id, target)| ProjectedAppTarget {
                id: id.clone(),
                kind: serialized_enum_name(target.kind),
                location: serialized_enum_name(target.location),
                path: target.path.clone(),
            })
            .collect(),
        configurations: app
            .configurations
            .iter()
            .map(|(id, configuration)| ProjectedAppConfiguration {
                id: id.clone(),
                format: configuration.format.as_str().to_string(),
                target: configuration.target.clone(),
                templates: configuration
                    .templates
                    .iter()
                    .map(|(id, template)| ProjectedAppConfigurationTemplate {
                        id: id.clone(),
                        path: template.path.clone(),
                    })
                    .collect(),
                managed_properties: configuration
                    .managed_properties
                    .iter()
                    .map(|(id, property)| ProjectedAppManagedProperty {
                        id: id.clone(),
                        path: property.path.clone(),
                        type_name: property.type_name.as_str().to_string(),
                        default: property.default.clone(),
                        clearable: property.clearable,
                        validators: property.validators.clone(),
                    })
                    .collect(),
            })
            .collect(),
        launcher_activity: app.launcher_activity.clone(),
    }
}

/// Serialize a catalog enum using its canonical authored name.
fn serialized_enum_name<T: serde::Serialize>(value: T) -> String {
    match serde_json::to_value(value).expect("authored catalog enum should serialize") {
        Value::String(name) => name,
        other => unreachable!("authored catalog enum should serialize to a string, got {other}"),
    }
}

/// Load and validate every top-level app definition in one authored root.
///
/// Validation covers each file's semantic validity plus the catalog-global
/// invariants that app definition IDs and package IDs are unique within the
/// catalog. Diagnostics are ordered by file name and semantic field order, and
/// file fields are relative to the authored root so messages do not expose
/// machine-specific paths.
pub fn load_app_definition_catalog(authored_root: &Path) -> AppDefinitionCatalog {
    let apps_directory = authored_root.join("apps");
    let app_files = match top_level_yaml_files(&apps_directory) {
        Ok(paths) => paths,
        Err(error) => return app_definition_catalog_unreadable(error),
    };
    load_app_definition_catalog_from_paths(authored_root, &app_files)
}

fn app_definition_catalog_unreadable(error: std::io::Error) -> AppDefinitionCatalog {
    AppDefinitionCatalog {
        entries: Vec::new(),
        diagnostics: vec![diagnostic(
            "error",
            "app_definition_catalog_unreadable",
            &format!("The App Definitions directory could not be read: {error}."),
            "apps",
            Some("app_definition"),
            None,
            None,
        )],
    }
}

fn load_app_definition_catalog_from_paths(
    authored_root: &Path,
    app_files: &[PathBuf],
) -> AppDefinitionCatalog {
    let mut entries = Vec::new();
    let mut diagnostics = Vec::new();
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut packages: HashMap<String, (String, String)> = HashMap::new();
    for path in app_files {
        let file = authored_relative_path(authored_root, path);
        let value = match authored_models::load_app_definition(path) {
            Ok(value) => value,
            Err(error) => {
                diagnostics.push(diagnostic(
                    "error",
                    error.code(),
                    error.message(),
                    &file,
                    Some("app_definition"),
                    None,
                    None,
                ));
                continue;
            }
        };
        diagnostics.extend(model_diagnostics(
            authored_root,
            path,
            "app_definition",
            Some(&value.id),
            authored_models::validate_app_definition(&value),
        ));
        diagnostics.extend(app_configuration_template_diagnostics(
            authored_root,
            path,
            &value,
        ));
        if let Some(first_file) = ids.get(&value.id) {
            diagnostics.push(diagnostic(
                "error",
                "app_definition_id_conflict",
                &format!(
                    "Duplicate app definition id {}; already defined in {}.",
                    single_quote(&value.id),
                    first_file
                ),
                &file,
                Some("app_definition"),
                Some(&value.id),
                Some("id"),
            ));
        } else {
            ids.insert(value.id.clone(), file.clone());
        }
        if let Some((first_id, first_file)) = packages.get(&value.package_id) {
            diagnostics.push(diagnostic(
                "error",
                "app_package_id_conflict",
                &format!(
                    "Duplicate app package id {}; already defined in {}.",
                    single_quote(&value.package_id),
                    first_file
                ),
                &file,
                Some("app_definition"),
                Some(&value.id),
                Some("package_id"),
            ));
            diagnostics.push(diagnostic(
                "error",
                "app_package_id_conflict",
                &format!(
                    "App package id {} is also defined in {}.",
                    single_quote(&value.package_id),
                    file
                ),
                first_file,
                Some("app_definition"),
                Some(first_id),
                Some("package_id"),
            ));
        } else {
            packages.insert(value.package_id.clone(), (value.id.clone(), file.clone()));
        }
        entries.push((path.clone(), value));
    }
    AppDefinitionCatalog {
        entries,
        diagnostics,
    }
}

pub(crate) struct AppConfigurationTemplateIssue {
    pub code: &'static str,
    pub message: &'static str,
    pub field: String,
}

pub(crate) fn app_configuration_template_issues(
    authored_root: &Path,
    app: &AppDefinitionV1,
) -> Vec<AppConfigurationTemplateIssue> {
    let mut issues = Vec::new();
    let apps_root = authored_root.join("apps");
    let app_directory = apps_root.join(&app.id);
    for (configuration_id, configuration) in &app.configurations {
        for (template_id, template) in &configuration.templates {
            if !authored_models::is_valid_app_configuration_template_path(&app.id, &template.path) {
                continue;
            }
            let mut current = apps_root.clone();
            let mut unsafe_path = false;
            let mut missing_path = false;
            for component in template.path.split('/') {
                current.push(component);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        unsafe_path = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        missing_path = true;
                        break;
                    }
                    Err(_) => {
                        unsafe_path = true;
                        break;
                    }
                }
            }
            let canonical_app = fs::canonicalize(&app_directory).ok();
            let canonical_template = fs::canonicalize(&current).ok();
            let contained = canonical_app
                .as_ref()
                .zip(canonical_template.as_ref())
                .is_some_and(|(app_path, template_path)| template_path.starts_with(app_path));
            let is_file = fs::metadata(&current).is_ok_and(|metadata| metadata.is_file());
            if unsafe_path || !contained || !is_file {
                let code = if missing_path {
                    "app_configuration_template_missing"
                } else {
                    "app_configuration_template_path_unsafe"
                };
                let message = if missing_path {
                    "The App Configuration template file does not exist."
                } else {
                    "The App Configuration template must be a regular file inside the owning app directory and may not traverse a symlink."
                };
                issues.push(AppConfigurationTemplateIssue {
                    code,
                    message,
                    field: format!(
                        "configurations[{configuration_id}].templates[{template_id}].path"
                    ),
                });
            }
        }
    }
    issues
}

fn app_configuration_template_diagnostics(
    authored_root: &Path,
    app_file: &Path,
    app: &AppDefinitionV1,
) -> Vec<Value> {
    let relative_file = authored_relative_path(authored_root, app_file);
    let diagnostics = app_configuration_template_issues(authored_root, app)
        .into_iter()
        .map(|issue| {
            diagnostic(
                "error",
                issue.code,
                issue.message,
                &relative_file,
                Some("app_definition"),
                Some(&app.id),
                Some(&issue.field),
            )
        })
        .collect();
    diagnostics
}

/// Validate every top-level schema-v1 app definition, device profile, and Device Plan.
///
/// Diagnostics are ordered by authored directory, file name, and semantic
/// field order. File fields are relative to the authored root so messages do
/// not expose machine-specific paths.
pub fn validate_authored_catalog_models(authored_root: &Path) -> Vec<Value> {
    let mut diagnostics = load_app_definition_catalog(authored_root).diagnostics;
    diagnostics.extend(validate_device_profile_models(authored_root));
    diagnostics.extend(validate_device_plan_models(authored_root));
    diagnostics
}

/// Validate every top-level Recipe and catalog model in an authored root.
///
/// Diagnostics are deduplicated because per-Recipe validation includes errors
/// for referenced App Definitions. Root-level diagnostics also include Device
/// Profile catalog errors, but unrelated profile damage does not block Recipe
/// admission. The returned paths are resolved paths suitable for CLI summaries;
/// diagnostic file fields retain the stable authored-root-relative form.
pub fn validate_authored_root(authored_root: &Path) -> (Vec<String>, Vec<Value>) {
    let recipe_paths_result = top_level_yaml_files(&authored_root.join("recipes"));
    let app_paths_result = top_level_yaml_files(&authored_root.join("apps"));
    let profile_paths_result = top_level_yaml_files(&authored_root.join("device_profiles"));
    let plan_paths_result = top_level_yaml_files(&authored_root.join("device_plans"));

    let mut validated_path_files = recipe_paths_result
        .as_ref()
        .map_or_else(|_| Vec::new(), |paths| paths.clone());
    for paths in [&app_paths_result, &profile_paths_result, &plan_paths_result]
        .into_iter()
        .flatten()
    {
        validated_path_files.extend(paths.iter().cloned());
    }
    let mut validated_paths = validated_path_files
        .iter()
        .map(|path| yaml::resolved_path_string(path))
        .collect::<Vec<_>>();
    validated_paths.sort();
    validated_paths.dedup();

    let app_catalog = match app_paths_result {
        Ok(paths) => load_app_definition_catalog_from_paths(authored_root, &paths),
        Err(error) => app_definition_catalog_unreadable(error),
    };
    let (recipe_paths, recipe_directory_error) = match recipe_paths_result {
        Ok(paths) => (paths, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let catalog = ValidationCatalog::from_app_definition_catalog(app_catalog, recipe_paths.clone());
    let mut diagnostics = catalog.app_definition_diagnostics.clone();
    diagnostics.extend(validate_device_profile_models_from_paths(
        authored_root,
        profile_paths_result,
    ));
    diagnostics.extend(validate_device_plan_models_from_paths(
        authored_root,
        plan_paths_result,
    ));
    if let Some(error) = recipe_directory_error {
        diagnostics.push(recipe_catalog_unreadable_diagnostic(&error));
    }
    let mut seen = diagnostics.iter().cloned().collect::<HashSet<_>>();
    for path in recipe_paths {
        let result = crate::validation::validate_recipe_path_result_with_catalog(
            &path,
            authored_root,
            &catalog,
        );
        for diagnostic in result["diagnostics"].as_array().into_iter().flatten() {
            if seen.insert(diagnostic.clone()) {
                diagnostics.push(diagnostic.clone());
            }
        }
    }
    let invalid_device_plan_files = diagnostics
        .iter()
        .filter(|issue| issue["severity"] == "error")
        .filter_map(|issue| issue["file"].as_str())
        .filter(|file| Path::new(file).starts_with("device_plans"))
        .map(|file| yaml::resolved_path_string(&authored_root.join(file)))
        .collect::<HashSet<_>>();
    validated_paths.retain(|path| !invalid_device_plan_files.contains(path));

    (validated_paths, diagnostics)
}

fn validate_device_profile_models(authored_root: &Path) -> Vec<Value> {
    validate_device_profile_models_from_paths(
        authored_root,
        top_level_yaml_files(&authored_root.join("device_profiles")),
    )
}

fn validate_device_profile_models_from_paths(
    authored_root: &Path,
    profile_paths: std::io::Result<Vec<PathBuf>>,
) -> Vec<Value> {
    let profile_paths = match profile_paths {
        Ok(paths) => paths,
        Err(error) => return vec![device_profile_catalog_unreadable_diagnostic(&error)],
    };
    let mut diagnostics = Vec::new();
    for path in profile_paths {
        diagnostics.extend(validate_device_profile_path(authored_root, &path));
    }
    diagnostics
}

fn device_profile_catalog_unreadable_diagnostic(error: &std::io::Error) -> Value {
    diagnostic(
        "error",
        "device_profile_catalog_unreadable",
        &format!("The device profile directory could not be read: {error}."),
        "device_profiles",
        Some("device_profile"),
        None,
        None,
    )
}

fn validate_device_plan_models(authored_root: &Path) -> Vec<Value> {
    match crate::planner_device_plan::validate_device_plan_files(authored_root) {
        Ok(issues) => device_plan_issue_diagnostics(authored_root, issues),
        Err(error) => device_plan_load_error_diagnostic(error),
    }
}

fn device_plan_load_error_diagnostic(error: crate::planner::PlannerLoadError) -> Vec<Value> {
    let (code, message) = if error.code() == "io" {
        (
            "device_plan_catalog_unreadable",
            "The Device Plan directory or one of its files could not be read.".to_string(),
        )
    } else {
        (error.code(), error.to_string())
    };
    vec![diagnostic(
        "error",
        code,
        &message,
        "device_plans",
        Some("device_plan"),
        None,
        None,
    )]
}

fn validate_device_plan_models_from_paths(
    authored_root: &Path,
    plan_paths: std::io::Result<Vec<PathBuf>>,
) -> Vec<Value> {
    match plan_paths {
        Ok(paths) => {
            match crate::planner_device_plan::validate_device_plan_paths(authored_root, paths) {
                Ok(issues) => device_plan_issue_diagnostics(authored_root, issues),
                Err(error) => device_plan_load_error_diagnostic(error),
            }
        }
        Err(error) => vec![device_plan_catalog_unreadable_diagnostic(&error)],
    }
}

fn device_plan_catalog_unreadable_diagnostic(_error: &std::io::Error) -> Value {
    diagnostic(
        "error",
        "device_plan_catalog_unreadable",
        "The Device Plan directory or one of its files could not be read.",
        "device_plans",
        Some("device_plan"),
        None,
        None,
    )
}

fn device_plan_issue_diagnostics(
    authored_root: &Path,
    issues: Vec<(PathBuf, crate::planner::PlannerLoadError)>,
) -> Vec<Value> {
    issues
        .into_iter()
        .map(|(path, error)| {
            let relative_path = path.strip_prefix(authored_root).unwrap_or(&path);
            diagnostic(
                "error",
                error.code(),
                &error.to_string(),
                &relative_path.to_string_lossy(),
                Some("device_plan"),
                None,
                None,
            )
        })
        .collect()
}

fn infer_authored_root(recipe_path: &Path) -> Option<PathBuf> {
    let target_path = PathBuf::from(yaml::resolved_path_string(recipe_path));
    for parent in target_path.ancestors().skip(1) {
        if parent.file_name().and_then(|name| name.to_str()) == Some("authored")
            && parent.join("recipes").is_dir()
        {
            return Some(parent.to_path_buf());
        }
    }
    None
}

fn resolved_path(path: impl AsRef<Path>) -> PathBuf {
    PathBuf::from(yaml::resolved_path_string(path.as_ref()))
}

impl ValidationCatalog {
    fn collect(authored_root: &Path) -> std::io::Result<Self> {
        let recipe_files = top_level_yaml_files(&authored_root.join("recipes"))?;
        let app_catalog = load_app_definition_catalog(authored_root);
        Ok(Self::from_app_definition_catalog(app_catalog, recipe_files))
    }

    fn from_app_definition_catalog(
        app_catalog: AppDefinitionCatalog,
        recipe_files: Vec<PathBuf>,
    ) -> Self {
        #[cfg(test)]
        VALIDATION_CATALOG_BUILD_COUNT.with(|count| count.set(count.get() + 1));

        let AppDefinitionCatalog {
            entries,
            diagnostics,
        } = app_catalog;
        let app_definitions = entries.into_iter().map(|(_, app)| app).collect::<Vec<_>>();
        let mut catalog = Self {
            app_definitions,
            app_definition_diagnostics: diagnostics,
            ..Self::default()
        };
        for path in recipe_files {
            catalog.collect_recipe(path);
        }
        catalog
    }

    fn collect_recipe(&mut self, path: PathBuf) {
        let Ok(raw) = yaml::load_yaml_mapping(&path) else {
            return;
        };
        let Ok(recipe) = yaml::parse_recipe_mapping(&raw, &path) else {
            if let Some(declared_id) = yaml::declared_recipe_id(&raw) {
                self.recipe_files
                    .entry(declared_id)
                    .or_default()
                    .push(resolved_path(&path));
            }
            return;
        };
        self.recipe_files
            .entry(recipe.id.clone())
            .or_default()
            .push(resolved_path(&path));
        if self.recipes.contains_key(&recipe.id) {
            return;
        }
        self.recipes.insert(recipe.id.clone(), recipe);
    }

    pub(crate) fn validate_recipe(
        &self,
        file: &str,
        recipe: &Recipe,
        recipe_path: &Path,
        authored_root: &Path,
    ) -> Vec<Value> {
        let target_path = resolved_path(recipe_path);
        let resolved_authored_root = resolved_path(authored_root);
        let target_in_catalog_root = target_path.starts_with(&resolved_authored_root);
        let mut recipes = self.recipes.clone();

        let replaced_recipe_id = first_recipe_id_for_path(&self.recipe_files, &target_path);
        if let Some(existing_id) = &replaced_recipe_id {
            recipes.remove(existing_id);
        }

        // Recipe admission includes catalog diagnostics only for App
        // Definitions referenced by the selected Recipe or its dependencies.
        let mut errors = Vec::new();
        let existing_path = (replaced_recipe_id.as_deref() != Some(recipe.id.as_str()))
            .then(|| {
                self.recipe_files
                    .get(&recipe.id)
                    .and_then(|paths| paths.first())
            })
            .flatten();
        if target_in_catalog_root && existing_path.is_some_and(|path| path != &target_path) {
            errors.push(diagnostic(
                "error",
                "recipe_id_conflict",
                &format!("Duplicate recipe id {}.", single_quote(&recipe.id)),
                file,
                Some("recipe"),
                Some(&recipe.id),
                Some("id"),
            ));
        }

        recipes.insert(recipe.id.clone(), recipe.clone());

        let mut pending_recipe_ids = VecDeque::from([recipe.id.clone()]);
        let mut visited_recipe_ids = HashSet::new();
        let mut seen_app_definition_diagnostics = HashSet::new();
        while let Some(current_recipe_id) = pending_recipe_ids.pop_front() {
            if !visited_recipe_ids.insert(current_recipe_id.clone()) {
                continue;
            }

            let Some(current_recipe) = recipes.get(&current_recipe_id) else {
                continue;
            };
            let current_file = if current_recipe_id == recipe.id {
                file.to_string()
            } else {
                self.recipe_files
                    .get(&current_recipe_id)
                    .and_then(|paths| paths.first())
                    .map(|path| yaml::resolved_path_string(path))
                    .unwrap_or_else(|| file.to_string())
            };

            if current_recipe_id != recipe.id {
                if self
                    .recipe_files
                    .get(&current_recipe_id)
                    .is_some_and(|paths| paths.len() > 1)
                {
                    errors.push(diagnostic(
                        "error",
                        "recipe_id_conflict",
                        &format!(
                            "Duplicate recipe id {} among reachable dependencies.",
                            single_quote(&current_recipe_id)
                        ),
                        &current_file,
                        Some("recipe"),
                        Some(&current_recipe_id),
                        Some("id"),
                    ));
                    continue;
                }
                errors.extend(crate::validation::validate_recipe_local_content(
                    &current_file,
                    current_recipe,
                    false,
                ));
            }
            let recipe_has_app_references = recipe_references_any_app(current_recipe);
            errors.extend(
                self.app_definition_diagnostics
                    .iter()
                    .filter(|diagnostic| {
                        diagnostic["objectKind"] == "app_definition"
                            && diagnostic["objectId"]
                                .as_str()
                                .map_or(recipe_has_app_references, |app_id| {
                                    recipe_references_app(current_recipe, app_id)
                                })
                            && seen_app_definition_diagnostics.insert((*diagnostic).clone())
                    })
                    .cloned(),
            );
            errors.extend(
                crate::planner::validate_recipe_app_authority(
                    current_recipe,
                    &self.app_definitions,
                )
                .iter()
                .map(|message| app_authority_diagnostic(&current_file, current_recipe, message)),
            );
            errors.extend(validate_recipe_configuration(
                &current_file,
                current_recipe,
                &self.app_definitions,
            ));
            errors.extend(permission_selection_diagnostics(
                &current_file,
                current_recipe,
                &self.app_definitions,
            ));

            for (index, dependency_ref) in current_recipe.recipe_dependencies.iter().enumerate() {
                if recipes.contains_key(dependency_ref) {
                    pending_recipe_ids.push_back(dependency_ref.clone());
                } else {
                    errors.push(diagnostic(
                        "error",
                        "recipe_not_found",
                        &format!(
                            "Recipe dependency {} was not found.",
                            single_quote(dependency_ref)
                        ),
                        &current_file,
                        Some("recipe"),
                        Some(&current_recipe.id),
                        Some(&format!("recipe_dependencies[{index}]")),
                    ));
                }
            }
        }

        if dependency_cycle_reachable(&recipes, &recipe.id) {
            errors.push(diagnostic(
                "error",
                "dependency_cycle",
                &format!(
                    "Recipe dependency cycle detected in recipe {}.",
                    single_quote(&recipe.id)
                ),
                file,
                Some("recipe"),
                Some(&recipe.id),
                Some("recipe_dependencies"),
            ));
        }

        errors
    }
}

fn app_authority_diagnostic(
    file: &str,
    recipe: &Recipe,
    message: &crate::planner::PlannerMessage,
) -> Value {
    if let Some(step_id) = message.details.get("step_id").and_then(Value::as_str) {
        return diagnostic(
            "error",
            &message.code,
            &message.message,
            file,
            Some("step"),
            Some(step_id),
            Some("app_ref"),
        );
    }
    if let Some(artifact_id) = message.details.get("artifact_id").and_then(Value::as_str) {
        return diagnostic(
            "error",
            &message.code,
            &message.message,
            file,
            Some("artifact"),
            Some(artifact_id),
            Some("app_ref"),
        );
    }
    diagnostic(
        "error",
        &message.code,
        &message.message,
        file,
        Some("recipe"),
        Some(&recipe.id),
        Some("steps"),
    )
}

/// Reports permission selection violations for `grant_permissions` steps.
///
/// Recipe validation reuses the save-time App Definition admission rules so a
/// Recipe reports the same diagnostics whether it is edited directly or is
/// referenced by an App Definition. The App Definition is resolved through
/// the catalog by identifier, so a step whose app reference does not resolve
/// keeps its existing app authority diagnostics. Invalid App Definitions are
/// reported once by the app authority check and skipped here.
pub(crate) fn validate_recipe_configuration(
    file: &str,
    recipe: &Recipe,
    app_definitions: &[AppDefinitionV1],
) -> Vec<Value> {
    let Some(config) = recipe.config.as_ref() else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    let Some(app) = app_definitions.iter().find(|app| app.id == config.app_ref) else {
        errors.push(recipe_configuration_diagnostic(
            file,
            recipe,
            "config.app_ref",
            "recipe_config_app_unknown",
            "Recipe configuration references an App Definition that is not in the catalog.",
        ));
        return errors;
    };

    if !authored_models::validate_app_definition(app).is_empty() {
        return errors;
    }
    let Some(configuration) = app.configurations.get(&config.configuration) else {
        let exists_elsewhere = app_definitions.iter().any(|other| {
            other.id != app.id && other.configurations.contains_key(&config.configuration)
        });
        errors.push(recipe_configuration_diagnostic(
            file,
            recipe,
            "config.configuration",
            if exists_elsewhere {
                "recipe_config_cross_app_reference"
            } else {
                "recipe_config_document_unknown"
            },
            if exists_elsewhere {
                "Recipe configuration references a document owned by another App Definition."
            } else {
                "Recipe configuration references an undeclared App Configuration document."
            },
        ));
        return errors;
    };
    if config.target != configuration.target {
        let target_is_other_app = app_definitions
            .iter()
            .any(|other| other.id != app.id && other.targets.contains_key(&config.target));
        errors.push(recipe_configuration_diagnostic(
            file,
            recipe,
            "config.target",
            if target_is_other_app {
                "recipe_config_cross_app_reference"
            } else {
                "recipe_config_target_mismatch"
            },
            if target_is_other_app {
                "Recipe configuration references an App Target owned by another App Definition."
            } else {
                "Recipe configuration target must match the App Configuration document target."
            },
        ));
    }

    match &config.base {
        crate::model::RecipeConfigurationBaseV1::Template(template_id) => {
            if !configuration.templates.contains_key(template_id) {
                let exists_elsewhere = app_definitions.iter().any(|other| {
                    other.id != app.id
                        && other.configurations.values().any(|other_configuration| {
                            other_configuration.templates.contains_key(template_id)
                        })
                });
                errors.push(recipe_configuration_diagnostic(
                    file,
                    recipe,
                    "config.base.template",
                    if exists_elsewhere {
                        "recipe_config_cross_app_reference"
                    } else {
                        "recipe_config_template_unknown"
                    },
                    if exists_elsewhere {
                        "Recipe base template is owned by another App Definition."
                    } else {
                        "Recipe base template is not declared by its App Configuration document."
                    },
                ));
            }
        }
        crate::model::RecipeConfigurationBaseV1::Input(input_id) => {
            match recipe.inputs.get(input_id) {
                Some(input)
                    if matches!(input.type_name.as_str(), "file" | "path") && input.multiple =>
                {
                    errors.push(recipe_configuration_diagnostic(
                        file,
                        recipe,
                        "config.base.input",
                        "recipe_config_base_input_multiple_invalid",
                        "Recipe configuration base input must select one file or path.",
                    ));
                }
                Some(input) if matches!(input.type_name.as_str(), "file" | "path") => {}
                Some(_) => errors.push(recipe_configuration_diagnostic(
                    file,
                    recipe,
                    "config.base.input",
                    "recipe_config_base_input_type_invalid",
                    "Recipe configuration base input must have type file or path.",
                )),
                None => errors.push(recipe_configuration_diagnostic(
                    file,
                    recipe,
                    "config.base.input",
                    "recipe_config_base_input_unknown",
                    "Recipe configuration base input is not declared by the Recipe.",
                )),
            }
        }
    }

    for (property_id, exposure) in &config.properties {
        let property_field = format!("config.properties[{property_id}]");
        let Some(property) = configuration.managed_properties.get(property_id) else {
            let exists_elsewhere = app_definitions.iter().any(|other| {
                other.id != app.id
                    && other.configurations.values().any(|other_configuration| {
                        other_configuration
                            .managed_properties
                            .contains_key(property_id)
                    })
            });
            errors.push(recipe_configuration_diagnostic(
                file,
                recipe,
                &property_field,
                if exists_elsewhere {
                    "recipe_config_cross_app_reference"
                } else {
                    "recipe_config_property_unknown"
                },
                if exists_elsewhere {
                    "Recipe exposed property is owned by another App Definition."
                } else {
                    "Recipe exposes a property that is not declared by its App Configuration document."
                },
            ));
            continue;
        };
        match &exposure.override_operation {
            Some(crate::model::RecipeConfigOverrideV1::Value(value))
                if !authored_models::managed_property_value_is_valid(property, value) =>
            {
                errors.push(recipe_configuration_diagnostic(
                    file,
                    recipe,
                    &format!("{property_field}.override.value"),
                    "recipe_config_override_invalid",
                    "Recipe override must match the App Definition type and validators.",
                ));
            }
            Some(crate::model::RecipeConfigOverrideV1::Clear) if !property.clearable => {
                errors.push(recipe_configuration_diagnostic(
                    file,
                    recipe,
                    &format!("{property_field}.override.clear"),
                    "recipe_config_override_invalid",
                    "App Definition does not permit clearing this managed property.",
                ));
            }
            _ => {}
        }
    }

    if !config.unmanaged_literals.is_empty()
        && !matches!(
            &config.base,
            crate::model::RecipeConfigurationBaseV1::Input(_)
        )
    {
        errors.push(recipe_configuration_diagnostic(
            file,
            recipe,
            "config.unmanaged_literals",
            "recipe_config_unmanaged_requires_input_base",
            "Unmanaged template literals require a user-supplied base input.",
        ));
    }
    let mut unmanaged_paths: Vec<Vec<String>> = Vec::new();
    for (index, literal) in config.unmanaged_literals.iter().enumerate() {
        let literal_field = format!("config.unmanaged_literals[{index}].path");
        if !literal.type_name.matches(&literal.value) {
            errors.push(recipe_configuration_diagnostic(
                file,
                recipe,
                &format!("config.unmanaged_literals[{index}].value"),
                "recipe_config_unmanaged_value_invalid",
                "Unmanaged template literal value must match its declared type.",
            ));
        }
        let Some(tokens) = authored_models::configuration_property_path_tokens(
            configuration.format,
            &literal.path,
        ) else {
            errors.push(recipe_configuration_diagnostic(
                file,
                recipe,
                &literal_field,
                "recipe_config_unmanaged_path_invalid",
                "Unmanaged template literal path is invalid for the App Configuration format.",
            ));
            continue;
        };
        for (managed_id, managed) in &configuration.managed_properties {
            if authored_models::configuration_paths_overlap(
                configuration.format,
                &literal.path,
                &managed.path,
            ) {
                errors.push(recipe_configuration_diagnostic(
                    file,
                    recipe,
                    &literal_field,
                    "recipe_config_unmanaged_path_collision",
                    &format!(
                        "Unmanaged template literal overlaps managed property {managed_id:?}."
                    ),
                ));
            }
        }
        if unmanaged_paths
            .iter()
            .any(|previous| path_token_sets_overlap(previous, &tokens))
        {
            errors.push(recipe_configuration_diagnostic(
                file,
                recipe,
                &literal_field,
                "recipe_config_unmanaged_path_overlap",
                "Unmanaged template literal paths may not duplicate or contain one another.",
            ));
        }
        unmanaged_paths.push(tokens);
    }
    errors
}

fn recipe_configuration_diagnostic(
    file: &str,
    recipe: &Recipe,
    field: &str,
    code: &str,
    message: &str,
) -> Value {
    diagnostic(
        "error",
        code,
        message,
        file,
        Some("recipe"),
        Some(&recipe.id),
        Some(field),
    )
}

fn path_token_sets_overlap(left: &[String], right: &[String]) -> bool {
    let common = left.len().min(right.len());
    left[..common] == right[..common]
}

fn permission_selection_diagnostics(
    file: &str,
    recipe: &Recipe,
    app_definitions: &[AppDefinitionV1],
) -> Vec<Value> {
    let mut errors = Vec::new();
    for step in recipe
        .steps
        .iter()
        .filter(|step| step.type_name == "grant_permissions")
    {
        let Some(app) = step
            .app_ref
            .as_deref()
            .and_then(|app_ref| app_definitions.iter().find(|app| app.id == app_ref))
        else {
            continue;
        };
        if !authored_models::validate_app_definition(app).is_empty() {
            continue;
        }
        for violation in crate::app_definition_admission::permission_selection_violations(app, step)
        {
            errors.push(diagnostic(
                "error",
                violation.code,
                &violation.message,
                file,
                Some("step"),
                Some(&step.id),
                Some(&violation.field),
            ));
        }
    }
    errors
}

fn validate_device_profile_path(authored_root: &Path, path: &Path) -> Vec<Value> {
    match authored_models::load_device_profile(path) {
        Ok(value) => model_diagnostics(
            authored_root,
            path,
            "device_profile",
            Some(&value.id),
            authored_models::validate_device_profile(&value),
        ),
        Err(error) => vec![diagnostic(
            "error",
            error.code(),
            error.message(),
            &authored_relative_path(authored_root, path),
            Some("device_profile"),
            None,
            None,
        )],
    }
}

fn model_diagnostics(
    authored_root: &Path,
    path: &Path,
    object_kind: &str,
    object_id: Option<&str>,
    diagnostics: Vec<AuthoredModelDiagnostic>,
) -> Vec<Value> {
    let file = authored_relative_path(authored_root, path);
    diagnostics
        .into_iter()
        .map(|item| {
            diagnostic(
                "error",
                &item.code,
                &item.message,
                &file,
                Some(object_kind),
                object_id,
                Some(&item.field),
            )
        })
        .collect()
}

fn authored_relative_path(authored_root: &Path, path: &Path) -> String {
    path.strip_prefix(authored_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn recipe_catalog_unreadable_diagnostic(error: &std::io::Error) -> Value {
    diagnostic(
        "error",
        "recipe_catalog_unreadable",
        &format!("The Recipe directory could not be read: {error}."),
        "recipes",
        Some("recipe"),
        None,
        None,
    )
}

/// Read the top-level YAML files of one authored directory.
///
/// A directory that does not exist is an empty authored set. Any other read
/// failure is reported so a validating caller cannot mistake an unreadable
/// directory for an empty one.
fn top_level_yaml_files(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let directory_metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let followed_metadata = if directory_metadata.file_type().is_symlink() {
        fs::metadata(directory)?
    } else {
        directory_metadata
    };
    if !followed_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!("{} is not a directory", directory.display()),
        ));
    }
    let entries = fs::read_dir(directory)?;
    top_level_yaml_paths(entries.map(|entry| entry.map(|entry| entry.path())))
}

/// Select YAML files from a directory-entry stream.
///
/// Entry and metadata failures are returned to the caller so a partial
/// directory scan cannot be mistaken for a complete authored catalog.
fn top_level_yaml_paths(
    entries: impl IntoIterator<Item = std::io::Result<PathBuf>>,
) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry?;
        if is_yaml_extension(&path) && fs::metadata(&path)?.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_yaml_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    )
}

fn first_recipe_id_for_path(
    recipe_files: &HashMap<String, Vec<PathBuf>>,
    target_path: &Path,
) -> Option<String> {
    recipe_files.iter().find_map(|(recipe_id, recipe_path)| {
        recipe_path
            .first()
            .is_some_and(|path| path == target_path)
            .then(|| recipe_id.clone())
    })
}

fn dependency_cycle_reachable(recipes: &HashMap<String, Recipe>, selected_recipe_id: &str) -> bool {
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    visit_recipe_dependency(recipes, selected_recipe_id, &mut visiting, &mut visited)
}

fn visit_recipe_dependency(
    recipes: &HashMap<String, Recipe>,
    recipe_id: &str,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
) -> bool {
    if visiting.contains(recipe_id) {
        return true;
    }
    if visited.contains(recipe_id) {
        return false;
    }
    let Some(recipe) = recipes.get(recipe_id) else {
        return false;
    };

    visiting.insert(recipe_id.to_string());
    for dependency_ref in &recipe.recipe_dependencies {
        if visit_recipe_dependency(recipes, dependency_ref, visiting, visited) {
            return true;
        }
    }
    visiting.remove(recipe_id);
    visited.insert(recipe_id.to_string());
    false
}

fn diagnostic(
    severity: &str,
    code: &str,
    message: &str,
    file: &str,
    object_kind: Option<&str>,
    object_id: Option<&str>,
    field: Option<&str>,
) -> Value {
    json!({
        "severity": severity,
        "code": code,
        "message": message,
        "file": file,
        "objectKind": object_kind,
        "objectId": object_id,
        "field": field,
    })
}

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod specification_regression_tests {
    use super::*;
    use crate::model::{
        OrderedMap, RecipeConfigOverrideV1, RecipeConfigPropertyV1, RecipeConfigurationBaseV1,
        RecipeProvides, Step, StepCondition, StepConstraints, UnmanagedTemplateLiteralV1,
    };
    use std::{fs, path::Path};
    use tempfile::TempDir;

    fn classifier_step(
        params: OrderedMap<ParamValue>,
        skip_if: Vec<StepCondition>,
        verify: Vec<StepCondition>,
    ) -> Step {
        Step {
            id: "condition".to_string(),
            type_name: "wait".to_string(),
            name: "Condition".to_string(),
            description: None,
            progress_note: None,
            user_toggleable: false,
            app_ref: None,
            dependencies: Vec::new(),
            constraints: StepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            skip_if,
            params,
            verify,
        }
    }

    fn classifier_recipe(step: Step) -> Recipe {
        Recipe {
            schema_version: 1,
            kind: "recipe".to_string(),
            id: "test.condition_classifier".to_string(),
            name: "Condition Classifier".to_string(),
            description: None,
            recipe_dependencies: Vec::new(),
            provides: RecipeProvides {
                features: Vec::new(),
            },
            inputs: OrderedMap::new(),
            config: None,
            artifacts: OrderedMap::new(),
            artifact_groups: OrderedMap::new(),
            steps: vec![step],
        }
    }

    fn configuration_app() -> AppDefinitionV1 {
        authored_models::parse_app_definition_yaml(include_str!(
            "../fixtures/configuration_contracts/app-key-value.yaml"
        ))
        .expect("configuration fixture should parse")
    }
    fn configuration_recipe() -> Recipe {
        yaml::load_recipe_from_bytes(
            include_bytes!("../fixtures/configuration_contracts/recipe-key-value.yaml"),
            Path::new("recipes/configuration.yaml"),
        )
        .expect("Recipe configuration fixture should parse")
    }
    fn configuration_diagnostic_codes(recipe: &Recipe, apps: &[AppDefinitionV1]) -> Vec<String> {
        validate_recipe_configuration("recipes/configuration.yaml", recipe, apps)
            .iter()
            .filter_map(|diagnostic| diagnostic.get("code")?.as_str().map(str::to_string))
            .collect()
    }
    fn write_configuration_catalog(root: &Path) -> (PathBuf, PathBuf) {
        let apps = root.join("apps");
        let recipes = root.join("recipes");
        fs::create_dir_all(apps.join("retroarch")).expect("App template directory");
        fs::create_dir_all(&recipes).expect("Recipe directory");
        fs::write(
            apps.join("retroarch.yaml"),
            include_bytes!("../fixtures/configuration_contracts/app-key-value.yaml"),
        )
        .expect("App Definition fixture");
        fs::write(apps.join("retroarch/default.cfg"), "audio_driver=alsa\n")
            .expect("template fixture");
        let recipe_path = recipes.join("recipe.retroarch.config.yaml");
        fs::write(
            &recipe_path,
            include_bytes!("../fixtures/configuration_contracts/recipe-key-value.yaml"),
        )
        .expect("Recipe fixture");
        (apps, recipe_path)
    }
    #[test]
    fn configuration_catalog_validation_and_projection_keep_typed_contracts() {
        let root = TempDir::new().expect("temporary authored root");
        let (_, recipe_path) = write_configuration_catalog(root.path());
        let recipe = configuration_recipe();
        let diagnostics = validate_recipe_with_catalog(
            "recipes/recipe.retroarch.config.yaml",
            &recipe,
            &recipe_path,
            root.path(),
        );
        assert!(
            diagnostics.is_empty(),
            "valid contract diagnostics: {diagnostics:?}"
        );

        let projection = project_app_catalog(Some(root.path()));
        let projected = projection.to_dto();
        assert_eq!(projected["status"], "ready");
        assert!(projected["apps"][0]["configurations"].is_array());
        assert!(projected["apps"][0]["configurations"][0]["templates"].is_array());
        assert!(projected["apps"][0]["configurations"][0]["managedProperties"].is_array());
        assert_eq!(
            projected["apps"][0]["configurations"][0]["format"],
            "key_value/v1"
        );
        assert_eq!(
            projected["apps"][0]["configurations"][0]["managedProperties"][0]["path"],
            json!(["audio_driver"])
        );
        assert_eq!(
            projected["apps"][0]["configurations"][0]["managedProperties"][1]["validators"]
                ["minLength"],
            1
        );
        assert_eq!(
            projected["apps"][0]["configurations"][0]["managedProperties"][2]["clearable"],
            true
        );
    }

    #[test]
    fn recipe_configuration_base_rejects_multiple_file_inputs() {
        let app = configuration_app();
        let mut recipe = configuration_recipe();
        recipe
            .inputs
            .get_mut("user_template")
            .expect("fixture declares base input")
            .multiple = true;

        let diagnostics =
            validate_recipe_configuration("recipes/configuration.yaml", &recipe, &[app]);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic["code"] == "recipe_config_base_input_multiple_invalid"
        }));
    }
    #[cfg(unix)]
    #[test]
    fn configuration_catalog_rejects_symlinked_template_files() {
        let root = TempDir::new().expect("temporary authored root");
        let (apps, _) = write_configuration_catalog(root.path());
        let template = apps.join("retroarch/default.cfg");
        fs::remove_file(&template).expect("remove regular template");
        let external = root.path().join("external.cfg");
        fs::write(&external, "audio_driver=alsa\n").expect("external template target");
        std::os::unix::fs::symlink(&external, &template).expect("symlink template path");
        let projection = project_app_catalog(Some(root.path()));
        assert_eq!(projection.to_dto()["status"], "invalid");
        assert!(projection.to_dto()["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "app_configuration_template_path_unsafe"));
    }
    #[test]
    fn recipe_configuration_accepts_valid_app_owned_exposure_without_override() {
        let app = configuration_app();
        let recipe = configuration_recipe();
        assert!(configuration_diagnostic_codes(&recipe, &[app]).is_empty());
        assert!(recipe.config.as_ref().unwrap().properties["audio_driver"]
            .override_operation
            .is_none());
    }
    #[test]
    fn recipe_configuration_rejects_cross_app_documents_templates_and_properties() {
        let app = configuration_app();
        let mut other_app = app.clone();
        other_app.id = "other_app".to_string();
        for configuration in other_app.configurations.values_mut() {
            for template in configuration.templates.values_mut() {
                template.path = template.path.replacen("retroarch/", "other_app/", 1);
            }
        }
        let mut foreign_configuration = other_app.configurations["main"].clone();
        foreign_configuration.templates.insert(
            "foreign_template".to_string(),
            authored_models::AppConfigurationTemplateV1 {
                path: "other_app/foreign.cfg".to_string(),
            },
        );
        let mut foreign_property = foreign_configuration.managed_properties["audio_driver"].clone();
        foreign_property.path = vec!["foreign_only".to_string()];
        foreign_configuration
            .managed_properties
            .insert("foreign_only".to_string(), foreign_property);
        other_app
            .configurations
            .insert("foreign_document".to_string(), foreign_configuration);

        let mut recipe = configuration_recipe();
        recipe.config.as_mut().unwrap().configuration = "foreign_document".to_string();
        assert!(
            configuration_diagnostic_codes(&recipe, &[app.clone(), other_app.clone()])
                .contains(&"recipe_config_cross_app_reference".to_string())
        );

        let mut recipe = configuration_recipe();
        recipe.config.as_mut().unwrap().base =
            RecipeConfigurationBaseV1::Template("foreign_template".to_string());
        assert!(
            configuration_diagnostic_codes(&recipe, &[app.clone(), other_app.clone()])
                .contains(&"recipe_config_cross_app_reference".to_string())
        );

        let mut recipe = configuration_recipe();
        recipe.config.as_mut().unwrap().properties.insert(
            "foreign_only".to_string(),
            RecipeConfigPropertyV1 {
                label: None,
                description: None,
                required: false,
                advanced: false,
                override_operation: None,
            },
        );
        assert!(configuration_diagnostic_codes(&recipe, &[app, other_app])
            .contains(&"recipe_config_cross_app_reference".to_string()));
    }
    #[test]
    fn recipe_configuration_preserves_coercion_rejection_and_reset_intent_without_app_default() {
        let app = configuration_app();
        let mut recipe = configuration_recipe();
        recipe
            .config
            .as_mut()
            .unwrap()
            .properties
            .get_mut("audio_driver")
            .unwrap()
            .override_operation = Some(RecipeConfigOverrideV1::Value(json!(42)));
        assert!(
            configuration_diagnostic_codes(&recipe, std::slice::from_ref(&app))
                .contains(&"recipe_config_override_invalid".to_string())
        );

        let mut app_without_default = app;
        app_without_default
            .configurations
            .get_mut("main")
            .unwrap()
            .managed_properties
            .get_mut("frame_limit")
            .unwrap()
            .default = None;
        let recipe = configuration_recipe();
        assert!(configuration_diagnostic_codes(&recipe, &[app_without_default]).is_empty());
        assert!(matches!(
            recipe.config.as_ref().unwrap().properties["frame_limit"].override_operation,
            Some(RecipeConfigOverrideV1::Reset)
        ));
    }

    #[test]
    fn recipe_clear_requires_app_owned_clearability() {
        let app = configuration_app();
        let recipe = configuration_recipe();
        assert!(configuration_diagnostic_codes(&recipe, std::slice::from_ref(&app)).is_empty());

        let mut forbidden = recipe;
        forbidden
            .config
            .as_mut()
            .unwrap()
            .properties
            .get_mut("audio_driver")
            .unwrap()
            .override_operation = Some(RecipeConfigOverrideV1::Clear);
        assert!(configuration_diagnostic_codes(&forbidden, &[app])
            .contains(&"recipe_config_override_invalid".to_string()));
    }
    #[test]
    fn unmanaged_literals_reject_managed_collisions_and_unsafe_paths() {
        let app = configuration_app();
        let mut recipe = configuration_recipe();
        recipe
            .config
            .as_mut()
            .unwrap()
            .unmanaged_literals
            .push(UnmanagedTemplateLiteralV1 {
                path: vec!["audio_driver".to_string()],
                type_name: authored_models::UnmanagedTemplateLiteralTypeV1::String,
                value: json!("manual"),
            });
        assert!(
            configuration_diagnostic_codes(&recipe, std::slice::from_ref(&app))
                .contains(&"recipe_config_unmanaged_path_collision".to_string())
        );

        let mut recipe = configuration_recipe();
        recipe.config.as_mut().unwrap().unmanaged_literals[0].path = vec!["bad=key".to_string()];
        assert!(configuration_diagnostic_codes(&recipe, &[app])
            .contains(&"recipe_config_unmanaged_path_invalid".to_string()));
    }
    #[test]
    fn json_unmanaged_literal_collision_uses_literal_path_segments() {
        let app = authored_models::parse_app_definition_yaml(include_str!(
            "../fixtures/configuration_contracts/app-nested-json.yaml"
        ))
        .expect("nested JSON App Definition fixture");
        let mut recipe = yaml::load_recipe_from_bytes(
            include_bytes!("../fixtures/configuration_contracts/recipe-nested-json.yaml"),
            Path::new("recipes/json-configuration.yaml"),
        )
        .expect("nested JSON Recipe fixture");
        let config = recipe.config.as_mut().unwrap();
        config.base = RecipeConfigurationBaseV1::Input("user_template".to_string());
        config.unmanaged_literals.push(UnmanagedTemplateLiteralV1 {
            path: vec![
                "ui".to_string(),
                "overlays".to_string(),
                "primary".to_string(),
                "path".to_string(),
            ],
            type_name: authored_models::UnmanagedTemplateLiteralTypeV1::String,
            value: json!("manual"),
        });
        assert!(configuration_diagnostic_codes(&recipe, &[app])
            .contains(&"recipe_config_unmanaged_path_collision".to_string()));
    }
    fn condition(type_name: &str, path: Value) -> StepCondition {
        let mut params = OrderedMap::new();
        params.insert("path".to_string(), path);
        StepCondition {
            type_name: type_name.to_string(),
            params,
        }
    }

    #[test]
    fn recipe_references_any_app_detects_condition_only_app_targets() {
        let recipe = classifier_recipe(classifier_step(
            OrderedMap::new(),
            vec![condition("path_exists", json!({"app_target": "system"}))],
            vec![condition(
                "file_exists",
                json!({"path": {"app_target": "system"}}),
            )],
        ));

        assert!(recipe_references_any_app(&recipe));
    }

    #[test]
    fn recipe_references_any_app_detects_nested_literal_app_targets() {
        let mut params = OrderedMap::new();
        params.insert(
            "payload".to_string(),
            ParamValue::Literal(json!({"nested": [{"app_target": "system"}]})),
        );
        let recipe = classifier_recipe(classifier_step(params, Vec::new(), Vec::new()));

        assert!(recipe_references_any_app(&recipe));
    }

    #[test]
    fn recipe_references_any_app_ignores_literal_device_paths() {
        let mut params = OrderedMap::new();
        params.insert(
            "path".to_string(),
            ParamValue::Literal(json!("/sdcard/EmuChef")),
        );
        let recipe = classifier_recipe(classifier_step(
            params,
            vec![condition("path_exists", json!({"path": "/sdcard/EmuChef"}))],
            vec![condition(
                "file_exists",
                json!({"path": "/storage/emulated/0/ROMs"}),
            )],
        ));

        assert!(!recipe_references_any_app(&recipe));
    }

    #[cfg(unix)]
    #[test]
    fn top_level_yaml_files_propagates_yaml_metadata_errors() {
        let directory = TempDir::new().expect("temporary YAML directory");
        std::os::unix::fs::symlink("missing-target", directory.path().join("missing.yaml"))
            .expect("create dangling YAML symlink");
        let error = top_level_yaml_files(directory.path())
            .expect_err("dangling YAML entry metadata must fail closed");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn top_level_yaml_files_preserves_empty_sets_and_propagates_entry_errors() {
        let directory = TempDir::new().expect("temporary YAML directory");
        assert!(top_level_yaml_files(&directory.path().join("missing"))
            .expect("missing directory is an empty authored set")
            .is_empty());

        let empty = directory.path().join("empty");
        fs::create_dir(&empty).expect("create empty authored directory");
        assert!(top_level_yaml_files(&empty)
            .expect("empty directory is an empty authored set")
            .is_empty());

        let entries: [std::io::Result<PathBuf>; 1] = [Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected entry failure",
        ))];
        let error =
            top_level_yaml_paths(entries).expect_err("directory-entry errors must propagate");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn authored_root_validation_builds_catalog_once_for_all_recipes() {
        let root = TempDir::new().expect("temporary authored root");
        let recipes = root.path().join("recipes");
        fs::create_dir(&recipes).expect("create Recipe directory");
        let minimal = include_str!("../tests/fixtures/recipes/minimal_recipe.yaml");
        fs::write(
            recipes.join("first.yaml"),
            minimal.replace("phase6e.minimal", "test.catalog_once.first"),
        )
        .expect("write first Recipe");
        fs::write(
            recipes.join("second.yaml"),
            minimal.replace("phase6e.minimal", "test.catalog_once.second"),
        )
        .expect("write second Recipe");

        VALIDATION_CATALOG_BUILD_COUNT.with(|count| count.set(0));
        let (_, diagnostics) = validate_authored_root(root.path());

        assert_eq!(
            VALIDATION_CATALOG_BUILD_COUNT.with(Cell::get),
            1,
            "all Recipes in one authored-root validation must share one catalog"
        );
        assert!(
            diagnostics.iter().all(|issue| issue["severity"] != "error"),
            "the shared catalog must preserve clean validation: {diagnostics:?}"
        );
    }

    fn duplicate_package_catalog(retroarch_first: bool) -> (TempDir, ValidationCatalog) {
        let root = TempDir::new().expect("duplicate package catalog");
        let apps = root.path().join("apps");
        let recipes = root.path().join("recipes");
        fs::create_dir_all(&apps).expect("create App Definition directory");
        fs::create_dir_all(&recipes).expect("create Recipe directory");

        let retroarch = include_str!("../../../authored/apps/retroarch.yaml");
        let duplicate_retroarch = retroarch.replacen("id: retroarch\n", "id: retroarch-copy\n", 1);
        let (retroarch_file, duplicate_file) = if retroarch_first {
            ("a-retroarch.yaml", "z-retroarch-copy.yaml")
        } else {
            ("z-retroarch.yaml", "a-retroarch-copy.yaml")
        };
        fs::write(apps.join(retroarch_file), retroarch).expect("write first App Definition");
        fs::write(apps.join(duplicate_file), duplicate_retroarch)
            .expect("write duplicate package App Definition");

        let app_recipe = include_str!("../../../authored/recipes/app.retroarch.provision.yaml");
        fs::write(recipes.join("app.retroarch.provision.yaml"), app_recipe)
            .expect("write Recipe bound to first App");
        let duplicate_app_recipe = app_recipe
            .replace(
                "id: app.retroarch.provision",
                "id: app.retroarch.copy.provision",
            )
            .replace("app_ref: retroarch", "app_ref: retroarch-copy");
        fs::write(
            recipes.join("app.retroarch.copy.provision.yaml"),
            duplicate_app_recipe,
        )
        .expect("write Recipe bound to second App");
        fs::write(
            recipes.join("generic.yaml"),
            include_str!("../../../templates/authored/recipe.blank.template.yaml"),
        )
        .expect("write unrelated generic Recipe");

        let catalog = ValidationCatalog::collect(root.path())
            .expect("duplicate package catalog should be readable");
        (root, catalog)
    }

    fn recipe_has_package_conflict(
        catalog: &ValidationCatalog,
        authored_root: &Path,
        file_name: &str,
    ) -> bool {
        let path = authored_root.join("recipes").join(file_name);
        let recipe = yaml::load_recipe_from_path(&path).expect("load Recipe fixture");
        catalog
            .validate_recipe(file_name, &recipe, &path, authored_root)
            .iter()
            .any(|diagnostic| diagnostic["code"] == "app_package_id_conflict")
    }

    #[test]
    fn duplicate_package_conflict_blocks_recipes_for_either_app_when_retroarch_loads_first() {
        let (root, catalog) = duplicate_package_catalog(true);

        for recipe in [
            "app.retroarch.provision.yaml",
            "app.retroarch.copy.provision.yaml",
        ] {
            assert!(
                recipe_has_package_conflict(&catalog, root.path(), recipe),
                "package conflict should block Recipe {recipe}"
            );
        }
    }

    #[test]
    fn duplicate_package_conflict_blocks_recipes_for_either_app_when_retroarch_loads_last() {
        let (root, catalog) = duplicate_package_catalog(false);

        for recipe in [
            "app.retroarch.provision.yaml",
            "app.retroarch.copy.provision.yaml",
        ] {
            assert!(
                recipe_has_package_conflict(&catalog, root.path(), recipe),
                "package conflict should block Recipe {recipe}"
            );
        }
    }

    #[test]
    fn duplicate_package_conflict_does_not_block_an_unrelated_generic_recipe() {
        let (root, catalog) = duplicate_package_catalog(true);

        assert!(!recipe_has_package_conflict(
            &catalog,
            root.path(),
            "generic.yaml"
        ));
    }
}
