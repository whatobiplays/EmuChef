//! Save-time validation for App Definitions and their direct Recipe dependents.
//!
//! This module reads a bounded, explicit snapshot of the `apps` and `recipes`
//! directories. It does not run catalog-wide validation or discard files that
//! could affect dependency classification.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::app_definition_persistence::{
    capture_catalog_directory, capture_file_snapshot, file_snapshots_equal, stale_file,
    AppDefinitionPersistenceError, CatalogDirectorySnapshot, OpenedAppDefinitionFile,
};
use crate::authored_models::{self, AppDefinitionV1, AuthoredModelDiagnostic};
use crate::model::{ParamValue, Recipe, RecipeArtifact, Step};
use crate::runtime_refs::{parse_reference, RuntimeRef};

/// The original identity of a catalog-backed App Definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDefinitionCatalogIdentity {
    pub id: String,
    pub package_id: String,
}

/// Selects whether Save replaces the opened file or creates a new catalog App.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppDefinitionSaveDestination {
    OpenedFile,
    NewCatalogApp,
}

/// Inputs captured by the document when it was opened and the intended save.
#[derive(Clone, Debug)]
pub struct AppDefinitionSaveContext {
    pub opened_file: OpenedAppDefinitionFile,
    /// Canonical authored root for a catalog document or a Save As operation.
    pub authored_root: Option<PathBuf>,
    /// Present only when the opened file is an existing canonical catalog App.
    pub opened_catalog_identity: Option<AppDefinitionCatalogIdentity>,
    pub destination: AppDefinitionSaveDestination,
}

/// A deterministic diagnostic produced by App Definition Save admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDefinitionSaveDiagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub file: Option<String>,
    pub object_kind: Option<String>,
    pub object_id: Option<String>,
    pub field: Option<String>,
}

impl AppDefinitionSaveDiagnostic {
    /// Project the diagnostic into the JSON shape used by editor validation.
    pub fn to_value(&self) -> Value {
        json!({
            "severity": self.severity,
            "code": self.code,
            "message": self.message,
            "file": self.file,
            "object_kind": self.object_kind,
            "object_id": self.object_id,
            "field": self.field,
        })
    }
}

/// Complete inputs required to publish one admitted App Definition.
#[derive(Clone, Debug)]
pub struct AdmissionSnapshot {
    pub(crate) candidate: AppDefinitionV1,
    pub(crate) opened_file: OpenedAppDefinitionFile,
    pub(crate) directories: Vec<CatalogDirectorySnapshot>,
    pub(crate) destination: PathBuf,
    pub(crate) publish_kind: AdmissionPublishKind,
    dependent_references: DependentAppReferences,
}

/// Exact App-owned entries referenced by the classified Recipe snapshot.
/// Recipe binding names remain separate from App Artifact identities.
#[derive(Clone, Debug, Default)]
struct DependentAppReferences {
    artifacts: HashSet<String>,
    targets: HashSet<String>,
}

/// Admission diagnostics and the disk snapshot used by the subsequent save.
#[derive(Clone, Debug)]
pub struct AppDefinitionSaveAdmission {
    pub diagnostics: Vec<AppDefinitionSaveDiagnostic>,
    pub snapshot: Option<AdmissionSnapshot>,
}

impl AppDefinitionSaveAdmission {
    /// Return whether every required input was classified and all save checks passed.
    pub fn can_save(&self) -> bool {
        self.snapshot.is_some()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != "error")
    }

    /// Return whether a captured Recipe references this candidate App Artifact.
    ///
    /// Semantic validation failures do not erase typed references. A false
    /// result is authoritative only when `snapshot` is present; an incomplete
    /// classification has no snapshot and must be rejected by the caller.
    pub fn has_dependent_artifact_reference(&self, artifact_id: &str) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .dependent_references
                .artifacts
                .contains(artifact_id)
        })
    }

    /// Return whether a captured Recipe references this candidate App Target,
    /// including typed target operands in skip and verification conditions.
    ///
    /// The target's owner is the enclosing step's App context. As with artifact
    /// queries, a caller must reject an absent classification snapshot.
    pub fn has_dependent_target_reference(&self, target_id: &str) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.dependent_references.targets.contains(target_id))
    }

    /// Return the editor-facing JSON projection of the admission diagnostics.
    pub fn diagnostics_as_values(&self) -> Vec<Value> {
        self.diagnostics
            .iter()
            .map(AppDefinitionSaveDiagnostic::to_value)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AdmissionPublishKind {
    ReplaceOpenedFile,
    CreateCatalogApp,
}

/// Validate the complete App Definition and its directly affected Recipes.
///
/// Structurally parseable semantic errors in unrelated authored documents are
/// ignored. A file that cannot be parsed or whose references cannot be
/// classified makes the snapshot incomplete and blocks publication.
pub fn admit_app_definition_save(
    value: &AppDefinitionV1,
    context: &AppDefinitionSaveContext,
) -> AppDefinitionSaveAdmission {
    let mut diagnostics = authored_models::validate_app_definition(value)
        .into_iter()
        .map(|diagnostic| {
            model_diagnostic(
                diagnostic,
                "app_definition",
                Some(&value.id),
                Some(&context.opened_file.path),
            )
        })
        .collect::<Vec<_>>();

    let mut snapshot = match prepare_save_snapshot(value, context, &mut diagnostics) {
        Some(snapshot) => Some(snapshot),
        None => None,
    };

    if let Some(current) = snapshot.as_ref() {
        if let Err(error) = verify_admission_snapshot(current) {
            diagnostics.push(persistence_diagnostic(error));
            snapshot = None;
        }
    }
    sort_diagnostics(&mut diagnostics);
    AppDefinitionSaveAdmission {
        diagnostics,
        snapshot,
    }
}

fn prepare_save_snapshot(
    value: &AppDefinitionV1,
    context: &AppDefinitionSaveContext,
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
) -> Option<AdmissionSnapshot> {
    let mut directories = Vec::new();
    let mut dependent_references = DependentAppReferences::default();
    let publish_kind = match context.destination {
        AppDefinitionSaveDestination::OpenedFile => AdmissionPublishKind::ReplaceOpenedFile,
        AppDefinitionSaveDestination::NewCatalogApp => AdmissionPublishKind::CreateCatalogApp,
    };

    match (
        context.destination,
        context.opened_catalog_identity.as_ref(),
    ) {
        (AppDefinitionSaveDestination::NewCatalogApp, Some(_)) => {
            diagnostics.push(diagnostic(
                "app_definition_save_as_forbidden",
                "A catalog App Definition cannot be saved as a different catalog document.",
                Some(&context.opened_file.path),
                Some("app_definition"),
                Some(&value.id),
                Some("id"),
            ));
            return None;
        }
        (AppDefinitionSaveDestination::OpenedFile, Some(original)) => {
            if value.id != original.id {
                diagnostics.push(diagnostic(
                    "app_identity_immutable",
                    "A catalog App Definition keeps its opened ID; create a different App Definition or use the later atomic reference-refactor workflow.",
                    Some(&context.opened_file.path),
                    Some("app_definition"),
                    Some(&original.id),
                    Some("id"),
                ));
            }
            if value.package_id != original.package_id {
                diagnostics.push(diagnostic(
                    "app_package_identity_immutable",
                    "A catalog App Definition keeps its opened package identity; create a different App Definition to use another package.",
                    Some(&context.opened_file.path),
                    Some("app_definition"),
                    Some(&original.id),
                    Some("package_id"),
                ));
            }
        }
        _ => {}
    }

    let needs_catalog = context.opened_catalog_identity.is_some()
        || context.destination == AppDefinitionSaveDestination::NewCatalogApp;
    if needs_catalog {
        let Some(root) = context.authored_root.as_deref() else {
            diagnostics.push(diagnostic(
                "app_catalog_path_invalid",
                "A canonical authored root is required to save a catalog App Definition.",
                Some(&context.opened_file.path),
                Some("app_definition"),
                Some(&value.id),
                Some("file"),
            ));
            return None;
        };
        let root = match canonical_authored_root(root) {
            Ok(root) => root,
            Err(error) => {
                diagnostics.push(persistence_diagnostic(error));
                return None;
            }
        };
        let apps = match capture_catalog_directory(&root.join("apps"), &root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                diagnostics.push(persistence_diagnostic(error));
                return None;
            }
        };
        let recipes = match capture_catalog_directory(&root.join("recipes"), &root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                diagnostics.push(persistence_diagnostic(error));
                return None;
            }
        };
        let destination = match context.destination {
            AppDefinitionSaveDestination::OpenedFile => {
                let identity = context.opened_catalog_identity.as_ref()?;
                let expected = root.join("apps").join(format!("{}.yaml", identity.id));
                if context.opened_file.path != expected {
                    diagnostics.push(diagnostic(
                        "app_definition_path_invalid",
                        "A catalog App Definition must be saved at its canonical apps/<id>.yaml path.",
                        Some(&context.opened_file.path),
                        Some("app_definition"),
                        Some(&identity.id),
                        Some("file"),
                    ));
                    return None;
                }
                let opened_identity = match parse_app_bytes(&context.opened_file.bytes) {
                    Ok(opened) => opened,
                    Err(message) => {
                        diagnostics.push(diagnostic(
                            "app_definition_stale_state",
                            format!(
                                "The opened App Definition can no longer be identified: {message}"
                            ),
                            Some(&context.opened_file.path),
                            Some("app_definition"),
                            Some(&identity.id),
                            Some("file"),
                        ));
                        return None;
                    }
                };
                if opened_identity.id != identity.id
                    || opened_identity.package_id != identity.package_id
                {
                    diagnostics.push(diagnostic(
                        "app_definition_stale_state",
                        "The opened file bytes do not match the App Definition identity captured by the document.",
                        Some(&context.opened_file.path),
                        Some("app_definition"),
                        Some(&identity.id),
                        Some("id"),
                    ));
                    return None;
                }
                expected
            }
            AppDefinitionSaveDestination::NewCatalogApp => {
                if !is_safe_app_id(&value.id) {
                    return None;
                }
                root.join("apps").join(format!("{}.yaml", value.id))
            }
        };
        directories.push(apps);
        directories.push(recipes);

        if context.destination == AppDefinitionSaveDestination::OpenedFile
            && !directories[0]
                .files
                .iter()
                .any(|file| file.path == context.opened_file.path)
        {
            diagnostics.push(diagnostic(
                "app_definition_stale_state",
                "The opened catalog App Definition is absent from the current apps directory.",
                Some(&context.opened_file.path),
                Some("app_definition"),
                context
                    .opened_catalog_identity
                    .as_ref()
                    .map(|identity| identity.id.as_str()),
                Some("file"),
            ));
            return None;
        }

        if let Some(identity) = context.opened_catalog_identity.as_ref() {
            let self_file = directories[0]
                .files
                .iter()
                .find(|file| file.path == context.opened_file.path);
            if self_file.is_none_or(|file| {
                file.bytes != context.opened_file.bytes
                    || file.identity != context.opened_file.identity
            }) {
                diagnostics.push(diagnostic(
                    "app_definition_stale_state",
                    "The opened App Definition changed after it was loaded; reopen or refresh it before saving.",
                    Some(&context.opened_file.path),
                    Some("app_definition"),
                    Some(&identity.id),
                    Some("file"),
                ));
                return None;
            }
        }

        if !check_competing_app_identities(
            value,
            &directories[0],
            context
                .opened_catalog_identity
                .as_ref()
                .map(|_| context.opened_file.path.as_path()),
            &root,
            diagnostics,
        ) {
            return None;
        }
        if !validate_recipe_inventory(
            value,
            &directories[0],
            &context.opened_file.path,
            &directories[1],
            &root,
            diagnostics,
            &mut dependent_references,
        ) {
            return None;
        }

        Some(AdmissionSnapshot {
            candidate: value.clone(),
            opened_file: context.opened_file.clone(),
            directories,
            destination,
            publish_kind,
            dependent_references,
        })
    } else {
        if context.destination != AppDefinitionSaveDestination::OpenedFile {
            return None;
        }
        Some(AdmissionSnapshot {
            candidate: value.clone(),
            opened_file: context.opened_file.clone(),
            directories,
            destination: context.opened_file.path.clone(),
            publish_kind,
            dependent_references,
        })
    }
}

fn canonical_authored_root(root: &Path) -> Result<PathBuf, AppDefinitionPersistenceError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_catalog_path_invalid",
            "The authored root could not be inspected",
            root,
            error,
        )
    })?;
    if !metadata.is_dir() {
        return Err(AppDefinitionPersistenceError::new(
            "app_catalog_path_invalid",
            "The authored root must be a directory.",
            Some(root),
        ));
    }
    fs::canonicalize(root).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_catalog_path_invalid",
            "The authored root could not be canonicalized",
            root,
            error,
        )
    })
}

fn is_safe_app_id(value: &str) -> bool {
    let mut segment_has_character = false;
    for character in value.chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            segment_has_character = true;
        } else if matches!(character, '.' | '_' | '-') && segment_has_character {
            segment_has_character = false;
        } else {
            return false;
        }
    }
    segment_has_character
}

fn check_competing_app_identities(
    candidate: &AppDefinitionV1,
    apps: &CatalogDirectorySnapshot,
    opened_path: Option<&Path>,
    root: &Path,
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
) -> bool {
    let mut complete = true;
    for file in &apps.files {
        if opened_path.is_some_and(|opened| opened == file.path) {
            continue;
        }
        let other = match parse_app_bytes(&file.bytes) {
            Ok(other) => other,
            Err(message) => {
                diagnostics.push(diagnostic(
                    "app_identity_classification_incomplete",
                    format!("This App Definition cannot be checked for ID or package conflicts: {message}"),
                    Some(&file.path),
                    Some("app_definition"),
                    None,
                    Some("id/package_id"),
                ));
                complete = false;
                continue;
            }
        };
        let relative = relative_path(root, &file.path);
        if other.id == candidate.id {
            diagnostics.push(diagnostic(
                "app_definition_id_conflict",
                format!(
                    "App ID '{}' is already defined in {relative}.",
                    candidate.id
                ),
                Some(&file.path),
                Some("app_definition"),
                Some(&candidate.id),
                Some("id"),
            ));
        }
        if other.package_id == candidate.package_id {
            diagnostics.push(diagnostic(
                "app_package_id_conflict",
                format!(
                    "Package '{}' is already defined in {relative}.",
                    candidate.package_id
                ),
                Some(&file.path),
                Some("app_definition"),
                Some(&other.id),
                Some("package_id"),
            ));
        }
    }
    complete
}

fn validate_recipe_inventory(
    app: &AppDefinitionV1,
    apps: &CatalogDirectorySnapshot,
    opened_path: &Path,
    recipes: &CatalogDirectorySnapshot,
    root: &Path,
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
    references: &mut DependentAppReferences,
) -> bool {
    // Parse only the captured bytes. Preserve duplicate identities until each
    // affected Recipe has established which App identities it needs.
    let mut definitions = Vec::new();
    for file in apps.files.iter().filter(|file| file.path != opened_path) {
        match parse_app_bytes(&file.bytes) {
            Ok(app) => definitions.push(app),
            Err(message) => {
                diagnostics.push(diagnostic(
                    "app_dependency_classification_incomplete",
                    format!("App references cannot be classified from this file: {message}"),
                    Some(&file.path),
                    Some("app_definition"),
                    None,
                    Some("file"),
                ));
                return false;
            }
        }
    }
    definitions.push(app.clone());
    let mut complete = true;
    for file in &recipes.files {
        let recipe = match parse_recipe_bytes(&file.bytes, &file.path) {
            Ok(recipe) => recipe,
            Err(message) => {
                diagnostics.push(diagnostic(
                    "app_dependency_classification_incomplete",
                    format!("Recipe references cannot be classified from this file: {message}"),
                    Some(&file.path),
                    Some("recipe"),
                    None,
                    Some("file"),
                ));
                complete = false;
                continue;
            }
        };
        let affected = match recipe_references_app(&recipe, &app.id, &file.path) {
            Ok(affected) => affected,
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                complete = false;
                continue;
            }
        };
        if affected {
            capture_dependent_references(&recipe, &app.id, references);
            validate_dependent_recipe(&definitions, &recipe, &file.path, root, diagnostics);
        }
    }
    complete
}

/// Record typed references while visiting each classified Recipe, before its
/// semantic validation. No additional disk reads are required by later queries.
fn capture_dependent_references(
    recipe: &Recipe,
    app_id: &str,
    references: &mut DependentAppReferences,
) {
    for artifact in recipe.artifacts.values() {
        if let RecipeArtifact::AppArtifact(reference) = artifact {
            if reference.app_ref == app_id {
                references.artifacts.insert(reference.artifact.clone());
            }
        }
    }
    for step in recipe
        .steps
        .iter()
        .filter(|step| step.app_ref.as_deref() == Some(app_id))
    {
        for value in step.params.values() {
            if let ParamValue::AppTarget(id) = value {
                references.targets.insert(id.clone());
            }
        }
        for value in step
            .skip_if
            .iter()
            .chain(&step.verify)
            .flat_map(|condition| condition.params.values())
        {
            if let Some(id) = value
                .as_object()
                .filter(|object| object.len() == 1)
                .and_then(|object| object.get("app_target"))
                .and_then(Value::as_str)
            {
                references.targets.insert(id.to_string());
            }
        }
    }
}

fn recipe_references_app(
    recipe: &Recipe,
    app_id: &str,
    path: &Path,
) -> Result<bool, AppDefinitionSaveDiagnostic> {
    let mut affected = recipe.artifacts.values().any(|artifact| match artifact {
        RecipeArtifact::AppArtifact(reference) => reference.app_ref == app_id,
        RecipeArtifact::RemoteFile(_) => false,
    });

    for step in &recipe.steps {
        if step.type_name == "copy_files" && copy_source_app_refs(recipe, step).contains(app_id) {
            affected = true;
        }
        let has_app_target = step.params.values().any(|value| match value {
            ParamValue::AppTarget(_) => true,
            ParamValue::Literal(value) => contains_app_target_key(value),
            ParamValue::Ref(_) => false,
        }) || step
            .skip_if
            .iter()
            .chain(&step.verify)
            .flat_map(|condition| condition.params.values())
            .any(contains_app_target_key);

        if has_app_target && step.app_ref.is_none() {
            return Err(diagnostic(
                "app_dependency_classification_incomplete",
                "This Recipe contains an App Target reference without an App Definition context, so its owner cannot be identified safely.",
                Some(path),
                Some("step"),
                Some(&step.id),
                Some("app_ref"),
            ));
        }
        if step.type_name == "grant_permissions"
            && step.params.contains_key("permission_set")
            && step.app_ref.is_none()
        {
            return Err(diagnostic(
                "app_dependency_classification_incomplete",
                "This Recipe selects an App Definition permission set without naming its app_ref, so its owner cannot be identified safely.",
                Some(path),
                Some("step"),
                Some(&step.id),
                Some("app_ref"),
            ));
        }

        if step.app_ref.as_deref() == Some(app_id) {
            affected = true;
        }
    }
    Ok(affected)
}

fn contains_app_target_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("app_target") || object.values().any(contains_app_target_key)
        }
        Value::Array(items) => items.iter().any(contains_app_target_key),
        _ => false,
    }
}

/// Validate every authored contract in an affected Recipe from the captured
/// catalog bytes. No catalog loader or device permission plan participates.
fn validate_dependent_recipe(
    apps: &[AppDefinitionV1],
    recipe: &Recipe,
    path: &Path,
    root: &Path,
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
) {
    let file = relative_path(root, path);
    let result = crate::validation::validate_loaded_recipe_result(recipe, path, None);
    if let Some(values) = result.get("diagnostics").and_then(Value::as_array) {
        for value in values.iter().filter(|value| value["severity"] == "error") {
            let string = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
            diagnostics.push(AppDefinitionSaveDiagnostic {
                severity: "error".into(),
                code: string("code").unwrap_or_else(|| "invalid_recipe".into()),
                message: string("message")
                    .unwrap_or_else(|| "The affected Recipe is invalid.".into()),
                file: Some(file.clone()),
                object_kind: string("object_kind"),
                object_id: string("object_id"),
                field: string("field"),
            });
        }
    }
    let referenced = recipe
        .steps
        .iter()
        .filter_map(|step| step.app_ref.as_deref())
        .chain(
            recipe
                .artifacts
                .values()
                .filter_map(|artifact| match artifact {
                    RecipeArtifact::AppArtifact(reference) => Some(reference.app_ref.as_str()),
                    _ => None,
                }),
        )
        .collect::<HashSet<_>>();
    let mut ambiguous = false;
    for id in &referenced {
        if apps.iter().filter(|app| app.id == *id).count() > 1 {
            diagnostics.push(diagnostic("app_id_conflict",
                format!("The affected Recipe references ambiguous App ID '{id}'; remove the competing definitions."),
                Some(path), Some("recipe"), Some(&recipe.id), Some("app_ref")));
            ambiguous = true;
        }
    }
    // The planner's authority validator builds an ID map internally. Never
    // pass an ambiguous referenced identity where map insertion could hide it.
    if !ambiguous {
        let relevant_apps = apps
            .iter()
            .filter(|app| referenced.contains(app.id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for error in crate::planner::validate_recipe_app_authority(recipe, &relevant_apps) {
            push_planner_diagnostic(error, &file, &recipe.id, diagnostics);
        }
    }
    validate_artifact_selector_shapes(recipe, &file, diagnostics);
    for error in crate::planner::validate_recipe_static_contracts(recipe) {
        push_planner_diagnostic(error, &file, &recipe.id, diagnostics);
    }
    // Set existence is static, whereas action/API applicability belongs to
    // execution planning. An API-unknown permission action remains authorable.
    for step in recipe
        .steps
        .iter()
        .filter(|step| step.type_name == "grant_permissions")
    {
        if let Some(app) = step
            .app_ref
            .as_deref()
            .and_then(|id| apps.iter().find(|app| app.id == id))
        {
            validate_permission_selection(app, step, &file, diagnostics);
        }
    }
}

/// The planner selection-expansion helper reads string members and ignores
/// other literal shapes. Require complete literal selectors before allowing
/// that expansion to establish a save-time install prerequisite.
fn validate_artifact_selector_shapes(
    recipe: &Recipe,
    file: &str,
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
) {
    for step in recipe.steps.iter().filter(|step| {
        matches!(
            step.type_name.as_str(),
            "resolve_artifacts" | "extract_artifacts"
        )
    }) {
        for parameter in ["artifacts", "artifact_groups"] {
            if let Some(value) = step.params.get(parameter) {
                if !matches!(value, ParamValue::Literal(Value::Array(values)) if values.iter().all(Value::is_string))
                {
                    push_recipe_diagnostic(diagnostics, "invalid_param_value_type",
                        "Artifact selectors must be literal lists containing only artifact or group names.",
                        file, &step.id, parameter);
                }
            }
        }
    }
}

fn push_planner_diagnostic(
    error: crate::planner::PlannerMessage,
    file: &str,
    recipe_id: &str,
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
) {
    let field = error
        .details
        .get("field")
        .or_else(|| error.details.get("param"))
        .and_then(Value::as_str);
    let artifact = error.details.get("artifact_id").and_then(Value::as_str);
    let step = error
        .details
        .get("step_id")
        .or_else(|| error.details.get("step_ref"))
        .and_then(Value::as_str);
    diagnostics.push(diagnostic(
        &error.code,
        error.message,
        Some(Path::new(file)),
        Some(if step.is_some() {
            "step"
        } else if artifact.is_some() {
            "artifact"
        } else {
            "recipe"
        }),
        Some(step.or(artifact).unwrap_or(recipe_id)),
        field,
    ));
}

fn validate_permission_selection(
    app: &AppDefinitionV1,
    step: &Step,
    file: &str,
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
) {
    validate_permission_policy(step, file, diagnostics);
    let selected = match step.params.get("permission_set") {
        Some(ParamValue::Literal(Value::String(name)))
            if matches!(name.as_str(), "baseline" | "elevated") =>
        {
            name.as_str()
        }
        Some(_) => {
            push_recipe_diagnostic(
                diagnostics,
                "permission_set_required",
                "The app-aware permission step must select the literal set name 'baseline' or 'elevated'.",
                file,
                &step.id,
                "permission_set",
            );
            return;
        }
        None => {
            push_recipe_diagnostic(
                diagnostics,
                "permission_set_required",
                "The app-aware permission step must select a permission set.",
                file,
                &step.id,
                "permission_set",
            );
            return;
        }
    };
    let exists = app
        .permission_sets
        .as_ref()
        .is_some_and(|sets| match selected {
            "baseline" => sets.baseline.is_some(),
            "elevated" => sets.elevated.is_some(),
            _ => false,
        });
    if !exists {
        push_recipe_diagnostic(
            diagnostics,
            "permission_set_missing",
            &format!(
                "The selected '{selected}' permission set is not defined by this App Definition."
            ),
            file,
            &step.id,
            "permission_set",
        );
    }
}

/// Follow source-producing steps using the same authored reference grammar as
/// execution planning. Only App Artifacts contribute App ownership.
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
                for artifact_id in selected_artifacts(recipe, producer) {
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

fn copy_source_app_refs(recipe: &Recipe, step: &Step) -> HashSet<String> {
    match step.params.get("source") {
        Some(ParamValue::Ref(reference)) => source_app_refs(recipe, reference, &mut HashSet::new()),
        _ => HashSet::new(),
    }
}

/// Expand only literal artifact and group selectors; input bindings are not
/// available during static catalog admission.
fn selected_artifacts(recipe: &Recipe, step: &Step) -> Vec<String> {
    let strings = |name: &str| match step.params.get(name) {
        Some(ParamValue::Literal(Value::Array(values))) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let mut artifacts = strings("artifacts");
    for group in strings("artifact_groups") {
        if let Some(members) = recipe.artifact_groups.get(&group) {
            artifacts.extend(members.clone());
        }
    }
    artifacts
}

/// Check the required authored failure policy without selecting device-specific
/// permission actions or evaluating Android API applicability.
fn validate_permission_policy(
    step: &Step,
    file: &str,
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
) {
    let policy = match step.params.get("policy") {
        Some(ParamValue::Literal(Value::Object(policy))) => policy,
        Some(_) => {
            push_recipe_diagnostic(
                diagnostics,
                "invalid_param_value",
                "Permission policy must be a literal object.",
                file,
                &step.id,
                "policy",
            );
            return;
        }
        None => {
            push_recipe_diagnostic(
                diagnostics,
                "missing_required_param",
                "App-aware permission steps require policy.on_failure.",
                file,
                &step.id,
                "policy",
            );
            return;
        }
    };
    for field in policy.keys().filter(|field| field.as_str() != "on_failure") {
        push_recipe_diagnostic(
            diagnostics,
            "invalid_param_value",
            "App-aware permission policy contains an unsupported field.",
            file,
            &step.id,
            &format!("policy.{field}"),
        );
    }
    match policy.get("on_failure") {
        Some(Value::String(value)) if matches!(value.as_str(), "warn" | "fail") => {}
        Some(_) => push_recipe_diagnostic(
            diagnostics,
            "invalid_enum_value",
            "Permission policy.on_failure must be warn or fail.",
            file,
            &step.id,
            "policy.on_failure",
        ),
        None => push_recipe_diagnostic(
            diagnostics,
            "missing_required_param",
            "App-aware permission steps require policy.on_failure.",
            file,
            &step.id,
            "policy.on_failure",
        ),
    }
}

fn parse_app_bytes(bytes: &[u8]) -> Result<AppDefinitionV1, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the file is not UTF-8 YAML".to_string())?;
    authored_models::parse_app_definition_yaml(text).map_err(|error| error.to_string())
}

fn parse_recipe_bytes(bytes: &[u8], path: &Path) -> Result<Recipe, String> {
    let raw: serde_yaml::Value =
        serde_yaml::from_slice(bytes).map_err(|_| "the file is not valid YAML".to_string())?;
    let mapping = raw
        .as_mapping()
        .ok_or_else(|| "the Recipe document root must be a mapping".to_string())?;
    crate::yaml::parse_recipe_mapping(mapping, path).map_err(|error| error.to_string())
}

pub(crate) fn verify_admission_snapshot(
    snapshot: &AdmissionSnapshot,
) -> Result<(), AppDefinitionPersistenceError> {
    let opened = capture_file_snapshot(&snapshot.opened_file.path)?;
    if opened.path != snapshot.opened_file.path
        || opened.bytes != snapshot.opened_file.bytes
        || opened.identity != snapshot.opened_file.identity
    {
        return Err(stale_file(&snapshot.opened_file.path));
    }
    for directory in &snapshot.directories {
        let current = capture_catalog_directory(
            &directory.path,
            &directory.path.parent().unwrap_or(&directory.path),
        )?;
        if current.path != directory.path
            || current.identity != directory.identity
            || current.files.len() != directory.files.len()
            || !current
                .files
                .iter()
                .zip(&directory.files)
                .all(|(actual, expected)| file_snapshots_equal(actual, expected))
        {
            return Err(stale_file(&directory.path));
        }
    }
    Ok(())
}

fn model_diagnostic(
    diagnostic: AuthoredModelDiagnostic,
    object_kind: &str,
    object_id: Option<&str>,
    file: Option<&Path>,
) -> AppDefinitionSaveDiagnostic {
    AppDefinitionSaveDiagnostic {
        severity: "error".to_string(),
        code: diagnostic.code,
        message: diagnostic.message,
        file: file.map(|path| path.to_string_lossy().into_owned()),
        object_kind: Some(object_kind.to_string()),
        object_id: object_id.map(str::to_string),
        field: Some(diagnostic.field),
    }
}

fn persistence_diagnostic(error: AppDefinitionPersistenceError) -> AppDefinitionSaveDiagnostic {
    diagnostic(
        &error.code,
        error.message,
        error.path.as_deref(),
        Some("app_definition"),
        None,
        None,
    )
}

fn diagnostic(
    code: &str,
    message: impl Into<String>,
    path: Option<&Path>,
    object_kind: Option<&str>,
    object_id: Option<&str>,
    field: Option<&str>,
) -> AppDefinitionSaveDiagnostic {
    AppDefinitionSaveDiagnostic {
        severity: "error".to_string(),
        code: code.to_string(),
        message: message.into(),
        file: path.map(|path| path.to_string_lossy().into_owned()),
        object_kind: object_kind.map(str::to_string),
        object_id: object_id.map(str::to_string),
        field: field.map(str::to_string),
    }
}

fn push_recipe_diagnostic(
    diagnostics: &mut Vec<AppDefinitionSaveDiagnostic>,
    code: &str,
    message: &str,
    file: &str,
    object_id: &str,
    field: &str,
) {
    diagnostics.push(diagnostic(
        code,
        message,
        Some(Path::new(file)),
        Some("step"),
        Some(object_id),
        Some(field),
    ));
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sort_diagnostics(diagnostics: &mut [AppDefinitionSaveDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        (
            &left.severity,
            &left.file,
            &left.object_kind,
            &left.object_id,
            &left.field,
            &left.code,
            &left.message,
        )
            .cmp(&(
                &right.severity,
                &right.file,
                &right.object_kind,
                &right.object_id,
                &right.field,
                &right.code,
                &right.message,
            ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_save_diagnostics_have_a_stable_json_projection() {
        let diagnostic = diagnostic(
            "app_target_unknown",
            "Target is missing.",
            Some(Path::new("recipes/a.yaml")),
            Some("step"),
            Some("install"),
            Some("dest"),
        );

        assert_eq!(
            diagnostic.to_value(),
            json!({
                "severity": "error",
                "code": "app_target_unknown",
                "message": "Target is missing.",
                "file": "recipes/a.yaml",
                "object_kind": "step",
                "object_id": "install",
                "field": "dest",
            })
        );
    }
}
