//! Authored-root catalog context used by validation.
//!
//! Validation scans these authored-root-relative top-level patterns:
//! `apps/*.y*ml`, `recipes/*.y*ml`,
//! `device_profiles/*.y*ml`, and `device_plans/*.y*ml`. The catalog models
//! recipe metadata needed for dependency diagnostics and validates typed app
//! definitions and device profiles. It does not build planner data structures,
//! execution plans, or executor state.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::authored_models::{self, AppDefinitionV1, AuthoredModelDiagnostic};
use crate::model::Recipe;
use crate::yaml;

pub const AUTHORED_CATALOG_GLOBS: &[&str] = &[
    "apps/*.y*ml",
    "recipes/*.y*ml",
    "device_profiles/*.y*ml",
    "device_plans/*.y*ml",
];

#[derive(Clone, Debug, Default)]
struct ValidationCatalog {
    recipes: HashMap<String, Recipe>,
    recipe_files: HashMap<String, Vec<PathBuf>>,
    app_definitions: Vec<crate::authored_models::AppDefinitionV1>,
    authored_model_diagnostics: Vec<Value>,
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
    let catalog = ValidationCatalog::collect(authored_root);
    catalog.validate_recipe(file, recipe, recipe_path, authored_root)
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
    pub launcher_activity: Option<String>,
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
                        "launcherActivity": app.launcher_activity,
                    })
                })
                .collect::<Vec<_>>(),
            "diagnostics": self.diagnostics,
        })
    }
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
    let mut entries = Vec::new();
    let mut diagnostics = Vec::new();
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut packages: HashMap<String, String> = HashMap::new();
    let apps_directory = authored_root.join("apps");
    let app_files = match top_level_yaml_files(&apps_directory) {
        Ok(paths) => paths,
        Err(error) => {
            // An unreadable directory must not look like an empty catalog,
            // because callers treat a diagnostic-free result as complete.
            diagnostics.push(diagnostic(
                "error",
                "app_definition_catalog_unreadable",
                &format!("The App Definitions directory could not be read: {error}."),
                "apps",
                Some("app_definition"),
                None,
                None,
            ));
            return AppDefinitionCatalog {
                entries,
                diagnostics,
            };
        }
    };
    for path in app_files {
        let file = authored_relative_path(authored_root, &path);
        let value = match authored_models::load_app_definition(&path) {
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
            &path,
            "app_definition",
            Some(&value.id),
            authored_models::validate_app_definition(&value),
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
        if let Some(first_file) = packages.get(&value.package_id) {
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
        } else {
            packages.insert(value.package_id.clone(), file.clone());
        }
        entries.push((path, value));
    }
    AppDefinitionCatalog {
        entries,
        diagnostics,
    }
}

/// Validate every top-level schema-v1 app definition and device profile.
///
/// Diagnostics are ordered by authored directory, file name, and semantic
/// field order. File fields are relative to the authored root so messages do
/// not expose machine-specific paths.
pub fn validate_authored_catalog_models(authored_root: &Path) -> Vec<Value> {
    let mut diagnostics = load_app_definition_catalog(authored_root).diagnostics;
    diagnostics.extend(validate_device_profile_models(authored_root));
    diagnostics
}

fn validate_device_profile_models(authored_root: &Path) -> Vec<Value> {
    let mut diagnostics = Vec::new();
    for path in top_level_yaml_files(&authored_root.join("device_profiles")).unwrap_or_default() {
        diagnostics.extend(validate_device_profile_path(authored_root, &path));
    }
    diagnostics
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
    fn collect(authored_root: &Path) -> Self {
        let AppDefinitionCatalog {
            entries,
            diagnostics,
        } = load_app_definition_catalog(authored_root);
        let app_definitions = entries.into_iter().map(|(_, app)| app).collect::<Vec<_>>();
        let mut authored_model_diagnostics = diagnostics;
        authored_model_diagnostics.extend(validate_device_profile_models(authored_root));
        let mut catalog = Self {
            app_definitions,
            authored_model_diagnostics,
            ..Self::default()
        };
        for path in top_level_yaml_files(&authored_root.join("recipes")).unwrap_or_default() {
            catalog.collect_recipe(path);
        }
        catalog
    }

    fn collect_recipe(&mut self, path: PathBuf) {
        let Ok(raw) = yaml::load_yaml_mapping(&path) else {
            return;
        };
        let Ok(recipe) = yaml::parse_recipe_mapping(&raw, &path) else {
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

    fn validate_recipe(
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

        // Authored-model diagnostics are additive. Recipe diagnostics continue
        // to run even when an app definition or device profile is malformed.
        let mut errors = self.authored_model_diagnostics.clone();
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

        errors.extend(
            crate::planner::validate_recipe_app_authority(recipe, &self.app_definitions)
                .iter()
                .map(|message| app_authority_diagnostic(file, recipe, message)),
        );
        errors.extend(permission_selection_diagnostics(
            file,
            recipe,
            &self.app_definitions,
        ));

        for (index, dependency_ref) in recipe.recipe_dependencies.iter().enumerate() {
            if !recipes.contains_key(dependency_ref) {
                errors.push(diagnostic(
                    "error",
                    "recipe_not_found",
                    &format!(
                        "Recipe dependency {} was not found.",
                        single_quote(dependency_ref)
                    ),
                    file,
                    Some("recipe"),
                    Some(&recipe.id),
                    Some(&format!("recipe_dependencies[{index}]")),
                ));
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

/// Read the top-level YAML files of one authored directory.
///
/// A directory that does not exist is an empty authored set. Any other read
/// failure is reported so a validating caller cannot mistake an unreadable
/// directory for an empty one.
fn top_level_yaml_files(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_yaml_extension(path))
        .collect::<Vec<_>>();
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
