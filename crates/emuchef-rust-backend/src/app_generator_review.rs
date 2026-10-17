//! Stateful App Generator review and publication service.
//!
//! The service owns every opaque handle the Config Editor native layer uses
//! for existing-App selection and immutable review publication. Captured file
//! bytes and filesystem identity stay inside this process: the native layer
//! receives only short-lived handles and catalog-relative paths.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::app_definition_admission::{
    admit_generated_app_pair_save, classify_affected_recipes, AdmissionPublishKind,
    AppDefinitionSaveDiagnostic, GeneratedAppPairSnapshot, GeneratedAppSaveContext,
};
use crate::app_definition_persistence::{
    capture_opened_app_definition_file, persist_generated_app_pair_for_operation,
    GeneratedPairOutcome, GeneratedPairPersistFailure, OpenedAppDefinitionFile,
};
use crate::authored_models::{self, AppDefinitionV1};
use crate::catalog_change::{
    CatalogChangeCoordinator, OperationPhase as DurableOperationPhase,
    OperationRecord as DurableOperationRecord,
};
use crate::errors::ApiError;
use crate::generation::{AppRecipeDraft, ProposedDestination};
use crate::yaml;

/// One retained existing-App selection.
#[derive(Clone, Debug)]
pub(crate) struct SelectedExistingApp {
    /// Canonical authored root the selection was captured against.
    pub(crate) authored_root: PathBuf,
    /// Canonical path of the selected App Definition file.
    pub(crate) app_path: PathBuf,
    /// Catalog-relative path shown to the author.
    pub(crate) relative_path: String,
    /// Exact bytes and identity captured when the App was selected.
    pub(crate) opened: OpenedAppDefinitionFile,
    /// Parsed App Definition retained for review generation.
    pub(crate) app: AppDefinitionV1,
}

/// One immutable review the author may publish or discard.
#[derive(Debug)]
pub(crate) struct StoredReview {
    pub(crate) request_id: i64,
    pub(crate) blocking: bool,
    /// Exact App/Recipe capture; save never re-reads authored intent.
    pub(crate) pair: GeneratedAppPairSnapshot,
    /// Catalog-relative App path for the publication report.
    pub(crate) app_relative_path: String,
    /// Catalog-relative Recipe path for the publication report.
    pub(crate) recipe_relative_path: String,
    pub(crate) authored_root: PathBuf,
}

/// One recorded publication attempt keyed by native operation identity.
#[derive(Clone, Debug)]
pub(crate) struct OperationRecord {
    pub(crate) outcome: &'static str,
    pub(crate) confirmed: bool,
    pub(crate) recorded_at: String,
    pub(crate) detail: Value,
}

/// In-memory handles and publication records for the App Generator.
#[derive(Debug, Default)]
pub(crate) struct AppGeneratorReviewState {
    selects: IndexMap<String, SelectedExistingApp>,
    reviews: IndexMap<String, StoredReview>,
    operations: IndexMap<String, OperationRecord>,
    next_id: u64,
    catalog_storage_base: Option<PathBuf>,
}

impl AppGeneratorReviewState {
    pub(crate) fn with_catalog_storage(storage_base: impl AsRef<Path>) -> Self {
        Self {
            catalog_storage_base: Some(storage_base.as_ref().to_path_buf()),
            ..Self::default()
        }
    }

    fn allocate(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}_{}", self.next_id)
    }

    fn insert_select(&mut self, selected: SelectedExistingApp) -> String {
        let handle = self.allocate("app_snapshot");
        // The newest selection for one App path replaces older handles, so the
        // author can re-select the same document without stale captures.
        let stale = self
            .selects
            .iter()
            .filter(|(_, existing)| existing.app_path == selected.app_path)
            .map(|(handle, _)| handle.clone())
            .collect::<Vec<_>>();
        for handle in stale {
            self.selects.shift_remove(&handle);
        }
        self.selects.insert(handle.clone(), selected);
        handle
    }

    /// Return a retained existing-App selection without consuming it.
    pub(crate) fn select(&self, handle: &str) -> Option<&SelectedExistingApp> {
        self.selects.get(handle)
    }

    /// Record one publication result under the native operation identity.
    pub(crate) fn record_operation(&mut self, operation_id: &str, record: OperationRecord) {
        self.operations.insert(operation_id.to_string(), record);
    }

    /// Return one recorded publication result.
    pub(crate) fn operation(&self, operation_id: &str) -> Option<&OperationRecord> {
        self.operations.get(operation_id)
    }

    /// Return one retained review without consuming it.
    pub(crate) fn review(&self, review_id: &str) -> Option<&StoredReview> {
        self.reviews.get(review_id)
    }

    /// Remove one review; a review handle is consumed by its first attempt.
    pub(crate) fn take_review(&mut self, review_id: &str) -> Option<StoredReview> {
        self.reviews.shift_remove(review_id)
    }

    /// Drop one operation record after a pre-publication refusal.
    pub(crate) fn remove_operation(&mut self, operation_id: &str) {
        self.operations.shift_remove(operation_id);
    }

    /// Drop every retained review candidate.
    ///
    /// A new existing-App selection invalidates outstanding reviews because
    /// each review was captured against the earlier snapshot. Publication
    /// operation records are retained so native status queries keep reporting
    /// the outcome of attempts that already began.
    pub(crate) fn clear_reviews(&mut self) {
        self.reviews.clear();
    }
}

/// List the catalog Apps that one authored root offers for extension.
pub(crate) fn list_existing_apps(root: &Path) -> Result<Value, ApiError> {
    let canonical_root = canonical_authored_root(root)?;
    let scan = scan_existing_apps(&canonical_root);
    Ok(json!({
        "apps": scan.summaries,
        "inventory": {
            "state": if scan.complete { "complete" } else { "incomplete" },
            "diagnostics": scan.diagnostics,
        },
    }))
}

/// Capture one existing-App selection and return its opaque handle.
pub(crate) fn select_existing_app(
    state: &mut AppGeneratorReviewState,
    root: &Path,
    app_path: &str,
) -> Result<Value, ApiError> {
    let canonical_root = canonical_authored_root(root)?;
    let scan = scan_existing_apps(&canonical_root);
    ensure_inventory_complete(&scan)?;
    let resolved = resolve_catalog_relative_path(&canonical_root, app_path)?;
    let relative = relative_path(&canonical_root, &resolved);
    let opened = capture_opened_app_definition_file(&resolved).map_err(|error| {
        let code = if error.code == "app_definition_path_invalid" {
            crate::errors::ApiErrorCode::ExistingAppUnavailable
        } else {
            crate::errors::ApiErrorCode::ExistingAppUnavailable
        };
        ApiError::new(
            code,
            error.message.clone(),
            json!({ "appPath": relative, "reason": error.code }),
        )
    })?;
    let text = std::str::from_utf8(opened.raw_bytes()).map_err(|_| {
        ApiError::new(
            crate::errors::ApiErrorCode::ExistingAppInvalid,
            "The selected App Definition is not UTF-8 YAML.",
            json!({ "appPath": relative }),
        )
    })?;
    let app = authored_models::parse_app_definition_yaml(text).map_err(|error| {
        ApiError::new(
            crate::errors::ApiErrorCode::ExistingAppInvalid,
            format!("The selected App Definition could not be parsed: {error}"),
            json!({ "appPath": relative }),
        )
    })?;
    let permission_set_count = |set: Option<&authored_models::AppPermissionSet>| {
        set.map_or(0, |set| set.runtime.len() + set.app_ops.len())
    };
    let sets = app.permission_sets.as_ref();
    let summary = json!({
        "appPath": relative,
        "appId": app.id,
        "name": app.name,
        "packageId": app.package_id,
        "artifactIds": app.artifacts.keys().cloned().collect::<Vec<_>>(),
        "permissionSetSummary": {
            "baseline": permission_set_count(sets.and_then(|sets| sets.baseline.as_ref())),
            "elevated": permission_set_count(sets.and_then(|sets| sets.elevated.as_ref())),
        },
        "launcherActivity": app.launcher_activity,
    });
    let handle = state.insert_select(SelectedExistingApp {
        authored_root: canonical_root,
        app_path: resolved,
        relative_path: relative,
        opened,
        app,
    });
    let mut result = summary;
    state.clear_reviews();
    result["appSnapshotId"] = Value::String(handle);
    Ok(result)
}

struct AppInventoryScan {
    summaries: Vec<Value>,
    complete: bool,
    diagnostics: Vec<Value>,
}

fn scan_existing_apps(root: &Path) -> AppInventoryScan {
    let apps_directory = root.join("apps");
    match fs::read_dir(&apps_directory) {
        Ok(entries) => scan_app_directory_entries(root, entries),
        Err(error) => AppInventoryScan {
            summaries: Vec::new(),
            complete: false,
            diagnostics: vec![json!({
                "code": "app_catalog_inventory_incomplete",
                "message": format!("The authored apps directory could not be read: {error}"),
                "file": "apps",
            })],
        },
    }
}

/// Classify one authored apps directory iteration into an inventory scan.
///
/// An iterator item error means part of the directory could not be examined.
/// The scan then reports an incomplete inventory instead of presenting only
/// the readable entries as the complete authored App list.
fn scan_app_directory_entries<I>(root: &Path, entries: I) -> AppInventoryScan
where
    I: IntoIterator<Item = std::io::Result<fs::DirEntry>>,
{
    let mut summaries = Vec::new();
    let mut diagnostics = Vec::new();
    let mut complete = true;
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if is_yaml_catalog_file(&path) && !path.is_dir() {
                    paths.push(path);
                }
            }
            Err(error) => {
                complete = false;
                diagnostics.push(json!({
                    "code": "app_catalog_inventory_incomplete",
                    "message": format!("An authored apps directory entry could not be read: {error}"),
                    "file": "apps",
                }));
            }
        }
    }
    paths.sort();
    for path in paths {
        let relative = relative_path(root, &path);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                complete = false;
                diagnostics.push(json!({
                    "code": "app_catalog_inventory_incomplete",
                    "message": format!("An authored App Definition could not be read: {error}"),
                    "file": relative,
                }));
                continue;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        match authored_models::parse_app_definition_yaml(&text) {
            Ok(app) => {
                let entry_diagnostics = authored_models::validate_app_definition(&app)
                    .into_iter()
                    .map(|diagnostic| {
                        json!({
                            "severity": "error",
                            "code": diagnostic.code,
                            "message": diagnostic.message,
                            "field": diagnostic.field,
                        })
                    })
                    .collect::<Vec<_>>();
                let sets = app.permission_sets.as_ref();
                let count = |set: Option<&authored_models::AppPermissionSet>| {
                    set.map_or(0, |set| set.runtime.len() + set.app_ops.len())
                };
                summaries.push(json!({
                    "appPath": relative,
                    "appId": app.id,
                    "name": app.name,
                    "packageId": app.package_id,
                    "artifactIds": app.artifacts.keys().cloned().collect::<Vec<_>>(),
                    "permissionSetSummary": {
                        "baseline": count(sets.and_then(|sets| sets.baseline.as_ref())),
                        "elevated": count(sets.and_then(|sets| sets.elevated.as_ref())),
                    },
                    "hasLauncher": app.launcher_activity.is_some(),
                    "selectable": entry_diagnostics.is_empty(),
                    "diagnostics": entry_diagnostics,
                }));
            }
            Err(error) => {
                summaries.push(json!({
                    "appPath": relative,
                    "appId": Value::Null,
                    "name": Value::Null,
                    "packageId": Value::Null,
                    "artifactIds": Vec::<String>::new(),
                    "permissionSetSummary": { "baseline": 0, "elevated": 0 },
                    "hasLauncher": false,
                    "selectable": false,
                    "diagnostics": [{
                        "severity": "error",
                        "code": "existing_app_invalid",
                        "message": error.to_string(),
                        "field": "file",
                    }],
                }));
            }
        }
    }
    AppInventoryScan {
        summaries,
        complete,
        diagnostics,
    }
}

/// Refuse existing-App selection when the inventory scan did not read the
/// complete authored apps directory.
fn ensure_inventory_complete(scan: &AppInventoryScan) -> Result<(), ApiError> {
    if !scan.complete {
        return Err(ApiError::new(
            crate::errors::ApiErrorCode::AppCatalogInventoryIncomplete,
            "The authored catalog inventory could not be read completely; existing Apps cannot be selected.",
            json!({ "diagnostics": &scan.diagnostics }),
        ));
    }
    Ok(())
}

pub(crate) fn canonical_authored_root(root: &Path) -> Result<PathBuf, ApiError> {
    if !root.is_absolute() {
        return Err(ApiError::invalid_request_with_details(
            "The authored root must be an absolute path.",
            json!({ "field": "authoredRoot" }),
        ));
    }
    let canonical = fs::canonicalize(root).map_err(|error| {
        ApiError::new(
            crate::errors::ApiErrorCode::ExistingAppUnavailable,
            format!("The authored root is unavailable: {error}"),
            json!({ "authoredRoot": root.to_string_lossy() }),
        )
    })?;
    if canonical != root {
        return Err(ApiError::invalid_request_with_details(
            "The authored root must be its canonical directory path.",
            json!({ "field": "authoredRoot" }),
        ));
    }
    Ok(canonical)
}

fn resolve_catalog_relative_path(root: &Path, app_path: &str) -> Result<PathBuf, ApiError> {
    let candidate = Path::new(app_path);
    let mut components = candidate.components();
    let safe = !candidate.is_absolute()
        && candidate
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    let inside_apps =
        matches!(components.next(), Some(Component::Normal(first)) if first == "apps");
    if !safe || !inside_apps || !is_yaml_catalog_file(candidate) {
        return Err(ApiError::new(
            crate::errors::ApiErrorCode::ExistingAppOutsideRoot,
            "The selected App path must be a catalog-relative apps/<id>.yaml path.",
            json!({ "appPath": app_path }),
        ));
    }
    let resolved = root.join(candidate);
    let canonical = fs::canonicalize(&resolved).map_err(|error| {
        ApiError::new(
            crate::errors::ApiErrorCode::ExistingAppUnavailable,
            format!("The selected App Definition is unavailable: {error}"),
            json!({ "appPath": app_path }),
        )
    })?;
    if canonical != resolved || !canonical.starts_with(root) {
        return Err(ApiError::new(
            crate::errors::ApiErrorCode::ExistingAppOutsideRoot,
            "The selected App path resolves outside the authored root.",
            json!({ "appPath": app_path }),
        ));
    }
    Ok(canonical)
}

fn is_yaml_catalog_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    )
}

pub(crate) fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
/// Intent captured for one review request.
pub(crate) struct ReviewIntent {
    pub(crate) authored_root: PathBuf,
    pub(crate) request_id: i64,
    pub(crate) artifact_name: Option<String>,
    pub(crate) existing: Option<SelectedExistingApp>,
    /// Whether the author enabled permission automation for this review.
    pub(crate) permission_automation_enabled: bool,
}

/// Draft fields shared by local and remote generation used for one review.
pub(crate) struct ReviewCandidate {
    pub(crate) app: AppDefinitionV1,
    pub(crate) apk_inspection: Option<Value>,
    pub(crate) recipe: Value,
    pub(crate) recipe_canonical_yaml: Option<String>,
    pub(crate) app_destination: ProposedDestination,
    pub(crate) recipe_destination: ProposedDestination,
    /// Review diagnostics already serialized for the transport response.
    pub(crate) diagnostics: Vec<Value>,
    pub(crate) blocking: bool,
}

impl From<AppRecipeDraft> for ReviewCandidate {
    fn from(draft: AppRecipeDraft) -> Self {
        Self {
            app: draft.app,
            apk_inspection: draft.apk_inspection,
            recipe: draft.recipe,
            recipe_canonical_yaml: draft.recipe_canonical_yaml,
            app_destination: draft.app_destination,
            recipe_destination: draft.recipe_destination,
            diagnostics: draft
                .diagnostics
                .iter()
                .filter_map(|diagnostic| serde_json::to_value(diagnostic).ok())
                .collect(),
            blocking: draft.blocking,
        }
    }
}

/// Convert one remote-source draft into the shared review candidate fields.
///
/// The conversion is typed so renamed draft fields fail to compile instead of
/// failing during serialization at run time.
impl From<crate::generation::RemoteAppRecipeDraft> for ReviewCandidate {
    fn from(draft: crate::generation::RemoteAppRecipeDraft) -> Self {
        Self {
            app: draft.app,
            apk_inspection: draft.apk_inspection,
            recipe: draft.recipe,
            recipe_canonical_yaml: draft.recipe_canonical_yaml,
            app_destination: ProposedDestination {
                file_name: draft.app_destination.file_name,
                relative_path: draft.app_destination.relative_path,
            },
            recipe_destination: ProposedDestination {
                file_name: draft.recipe_destination.file_name,
                relative_path: draft.recipe_destination.relative_path,
            },
            diagnostics: draft
                .diagnostics
                .iter()
                .filter_map(|diagnostic| serde_json::to_value(diagnostic).ok())
                .collect(),
            blocking: draft.blocking,
        }
    }
}

/// Build one immutable review from a generated local draft.
pub(crate) fn review_from_draft(
    state: &mut AppGeneratorReviewState,
    draft: AppRecipeDraft,
    intent: ReviewIntent,
) -> Result<Value, ApiError> {
    review_from_candidate(state, ReviewCandidate::from(draft), intent)
}

/// Build one immutable review from a generated remote-source draft.
pub(crate) fn review_from_remote_draft(
    state: &mut AppGeneratorReviewState,
    draft: crate::generation::RemoteAppRecipeDraft,
    intent: ReviewIntent,
) -> Result<Value, ApiError> {
    let candidate = ReviewCandidate::from(draft);
    review_from_candidate(state, candidate, intent)
}

/// Build one immutable review from a shared review candidate.
///
/// The review retains the exact candidate App and Recipe capture. The native
/// layer only receives the opaque review handle; publication never re-reads
/// authored intent from the request.
pub(crate) fn review_from_candidate(
    state: &mut AppGeneratorReviewState,
    mut draft: ReviewCandidate,
    intent: ReviewIntent,
) -> Result<Value, ApiError> {
    let root = intent.authored_root.clone();
    let previous_app = intent
        .existing
        .as_ref()
        .map(|snapshot| snapshot.app.clone());
    let artifact_id = draft
        .recipe
        .get("artifacts")
        .and_then(Value::as_object)
        .and_then(|artifacts| artifacts.keys().next())
        .cloned();
    if let (Some(id), Some(name)) = (artifact_id.as_deref(), intent.artifact_name.as_deref()) {
        if let Some(artifact) = draft.app.artifacts.get_mut(id) {
            artifact.name = Some(name.to_string());
        }
    }
    let mut diagnostics: Vec<Value> = draft.diagnostics;
    let canonical_app_yaml = match authored_models::emit_app_definition_yaml(&draft.app) {
        Ok(yaml) => Some(yaml),
        Err(error) => {
            diagnostics.push(simple_diagnostic(
                "app_definition_invalid",
                &error.message(),
            ));
            None
        }
    };
    let app_relative = draft.app_destination.relative_path.clone();
    let recipe_relative = draft.recipe_destination.relative_path.clone();
    let parsed_recipe = match (
        draft.recipe_canonical_yaml.as_deref(),
        recipe_relative.as_deref(),
    ) {
        (Some(yaml), Some(relative)) => match serde_yaml::from_str::<serde_yaml::Mapping>(yaml) {
            Ok(mapping) => match yaml::parse_recipe_mapping(&mapping, Path::new(relative)) {
                Ok(recipe) => Some(recipe),
                Err(error) => {
                    diagnostics.push(simple_diagnostic("recipe_invalid", &error.to_string()));
                    None
                }
            },
            Err(_) => {
                diagnostics.push(simple_diagnostic(
                    "recipe_invalid",
                    "The generated Recipe YAML is invalid.",
                ));
                None
            }
        },
        _ => {
            diagnostics.push(simple_diagnostic(
                "destination_unavailable",
                "Generated App and Recipe destinations are unavailable; choose explicit identities.",
            ));
            None
        }
    };
    let mut pair_snapshot = None;
    let mut warnings: Vec<Value> = Vec::new();
    if let (Some(app_rel), Some(recipe_rel), Some(recipe), Some(_)) = (
        app_relative.as_deref(),
        recipe_relative.as_deref(),
        parsed_recipe,
        canonical_app_yaml.as_deref(),
    ) {
        let mut admission = admit_generated_app_pair_save(
            &draft.app,
            &recipe,
            draft.recipe_canonical_yaml.as_deref().unwrap_or_default(),
            &GeneratedAppSaveContext {
                authored_root: root.clone(),
                app_destination: root.join(app_rel),
                opened_app: intent
                    .existing
                    .as_ref()
                    .map(|snapshot| snapshot.opened.clone()),
                recipe_destination: root.join(recipe_rel),
            },
        );
        warnings = admission
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == "warning")
            .map(|diagnostic| json!({ "code": diagnostic.code, "message": diagnostic.message }))
            .collect();
        diagnostics.extend(admission.diagnostics.iter().map(admission_ui_diagnostic));
        pair_snapshot = admission.snapshot.take();
    }
    let blocking = draft.blocking
        || pair_snapshot.is_none()
        || diagnostics
            .iter()
            .any(|diagnostic| diagnostic["severity"] == "error");
    let change_impact = change_impact_value(
        &draft.app,
        artifact_id.as_deref(),
        pair_snapshot.as_ref(),
        &root,
        previous_app.as_ref(),
        warnings,
    );
    let artifact_value = artifact_id
        .as_deref()
        .and_then(|id| draft.app.artifacts.get(id).map(|artifact| (id, artifact)))
        .map(|(id, artifact)| {
            json!({
                "appArtifactId": id,
                "name": artifact.name.clone().unwrap_or_else(|| id.to_string()),
                "kind": "apk",
                "source": serde_json::to_value(&artifact.source).unwrap_or(Value::Null),
                "change": if previous_app.is_some() { "added" } else { "proposed" },
            })
        })
        .unwrap_or(Value::Null);
    let permission_automation =
        permission_automation_value(&draft.app, intent.permission_automation_enabled);
    let review_id = match pair_snapshot {
        Some(pair) => {
            let handle = state.allocate("review");
            state.reviews.insert(
                handle.clone(),
                StoredReview {
                    request_id: intent.request_id,
                    blocking,
                    app_relative_path: app_relative.clone().unwrap_or_default(),
                    recipe_relative_path: recipe_relative.clone().unwrap_or_default(),
                    pair,
                    authored_root: root.clone(),
                },
            );
            Value::String(handle)
        }
        None => Value::Null,
    };
    let app_destination_value = if app_relative.is_some() {
        json!({
            "fileName": draft.app_destination.file_name,
            "relativePath": draft.app_destination.relative_path,
        })
    } else {
        Value::Null
    };
    let recipe_destination_value = if recipe_relative.is_some() {
        json!({
            "fileName": draft.recipe_destination.file_name,
            "relativePath": draft.recipe_destination.relative_path,
        })
    } else {
        Value::Null
    };
    Ok(json!({
        "reviewId": review_id,
        "requestId": intent.request_id,
        "blocking": blocking,
        "diagnostics": diagnostics,
        "app": serde_json::to_value(&draft.app).unwrap_or(Value::Null),
        "appCanonicalYaml": canonical_app_yaml,
        "recipe": draft.recipe.clone(),
        "recipeCanonicalYaml": draft.recipe_canonical_yaml.clone(),
        "destinations": {
            "app": app_destination_value,
            "recipe": recipe_destination_value,
        },
        "target": {
            "mode": if intent.existing.is_some() { "existing_app" } else { "new_app" },
            "appId": draft.app.id,
            "appPath": intent.existing.as_ref().map(|snapshot| snapshot.relative_path.clone()),
            "packageId": draft.app.package_id,
            "packageLocked": intent.existing.is_some(),
        },
        "artifact": artifact_value,
        "permissionAutomation": permission_automation,
        "changeImpact": change_impact,
        "apkInspection": draft.apk_inspection.clone().unwrap_or(Value::Null),
    }))
}

/// Consume one review handle for a publication attempt.
pub(crate) fn take_review(
    state: &mut AppGeneratorReviewState,
    review_id: &str,
    request_id: i64,
) -> Result<StoredReview, ApiError> {
    let review = state
        .take_review(review_id)
        .ok_or_else(review_invalidated_error)?;
    if review.request_id != request_id {
        return Err(review_invalidated_error());
    }
    Ok(review)
}

/// Inspect one review handle for pre-publication guards without consuming it.
///
/// Publication refusals that depend on external state (an unsaved App
/// Definition document on the target path, or blocking diagnostics) leave the
/// handle intact so the author can resolve the refusal and publish the same
/// reviewed candidate.
pub(crate) fn peek_review<'a>(
    state: &'a AppGeneratorReviewState,
    review_id: &str,
    request_id: i64,
) -> Result<&'a StoredReview, ApiError> {
    let review = state
        .review(review_id)
        .ok_or_else(review_invalidated_error)?;
    if review.request_id != request_id {
        return Err(review_invalidated_error());
    }
    Ok(review)
}

/// Refusal for an unknown or consumed review handle.
pub(crate) fn review_invalidated_error() -> ApiError {
    ApiError::new(
        crate::errors::ApiErrorCode::ReviewInvalidated,
        "The review handle is no longer valid; review the draft again before saving.",
        json!({ "reason": "review_invalidated" }),
    )
}

/// Refusal for a review that retains blocking diagnostics.
pub(crate) fn review_blocking_error(review: &StoredReview) -> ApiError {
    ApiError::new(
        crate::errors::ApiErrorCode::ReviewBlocking,
        "The review contains blocking diagnostics and cannot be saved.",
        json!({ "appPath": review.app_relative_path }),
    )
}

/// Refusal for an unsaved App Definition document on the target path.
pub(crate) fn dirty_app_document_error(review: &StoredReview) -> ApiError {
    ApiError::new(
        crate::errors::ApiErrorCode::DirtyAppDocument,
        "An open App Definition document has unsaved changes for the target catalog path.",
        json!({
            "reason": "dirty_app_document",
            "appPath": review.app_relative_path,
        }),
    )
}

/// Replay a previously recorded publication result for one operation.
pub(crate) fn operation_replay(
    state: &AppGeneratorReviewState,
    operation_id: &str,
) -> Option<Value> {
    state
        .operation(operation_id)
        .map(|record| record.detail.clone())
}

/// Publish one reviewed pair and record its outcome under the operation id.
///
/// The operation record is written before the first filesystem change so a
/// repeated attempt for the same operation never publishes twice. Pre-publication
/// refusals leave no record: nothing was written and the native layer may show
/// the refusal directly.
pub(crate) fn publish_review<Reconcile>(
    state: &mut AppGeneratorReviewState,
    operation_id: &str,
    review: &StoredReview,
    reconcile: Reconcile,
) -> Result<Value, ApiError>
where
    Reconcile: FnOnce(&GeneratedPairOutcome) -> Result<(), String>,
{
    if let Some(record) = state.operation(operation_id) {
        return Ok(record.detail.clone());
    }
    state.record_operation(
        operation_id,
        OperationRecord {
            outcome: "unknown",
            confirmed: false,
            recorded_at: now_iso8601(),
            detail: json!({
                "ok": true,
                "outcome": "unknown",
                "operationId": operation_id,
                "message": "This publication attempt has not confirmed an outcome.",
            }),
        },
    );
    match persist_generated_app_pair_for_operation(
        &review.pair,
        operation_id,
        state.catalog_storage_base.as_deref(),
        reconcile,
    ) {
        Ok(outcome) => {
            // Report the published destinations from the committed outcome so
            // the result reflects the files this publication actually owns.
            let app_path = relative_path(&review.authored_root, &outcome.app.path);
            let recipe_path = relative_path(&review.authored_root, &outcome.recipe_path);
            let created_files = if outcome.app_replaced {
                vec![recipe_path.clone()]
            } else {
                vec![recipe_path.clone(), app_path.clone()]
            };
            let replaced_files = if outcome.app_replaced {
                vec![app_path.clone()]
            } else {
                Vec::new()
            };
            let payload = json!({
                "ok": true,
                "outcome": "saved",
                "operationId": operation_id,
                "app": { "appPath": app_path },
                "recipe": { "recipePath": recipe_path },
                "createdFiles": created_files,
                "replacedFiles": replaced_files,
                "rollback": Value::Null,
                "error": Value::Null,
                "message": Value::Null,
            });
            state.record_operation(
                operation_id,
                OperationRecord {
                    outcome: "saved",
                    confirmed: true,
                    recorded_at: now_iso8601(),
                    detail: payload.clone(),
                },
            );
            Ok(payload)
        }
        Err(GeneratedPairPersistFailure::Refused(error)) => {
            state.remove_operation(operation_id);
            Err(refusal_error(&error))
        }
        Err(GeneratedPairPersistFailure::RecipeCommitted { error, rollback }) => {
            let payload = json!({
                "ok": true,
                "outcome": "failed",
                "operationId": operation_id,
                "app": { "appPath": review.app_relative_path },
                "recipe": { "recipePath": review.recipe_relative_path },
                "createdFiles": Vec::<String>::new(),
                "replacedFiles": Vec::<String>::new(),
                "rollback": {
                    "recipeRemoved": rollback.recipe_removed,
                    "recipeRecoveryRequired": rollback.recipe_recovery_required,
                    "recipePath": relative_path(&review.authored_root, &rollback.recipe_path),
                },
                "error": {
                    "code": error.code,
                    "message": error.message,
                    "details": { "reason": error.code },
                },
                "message": Value::Null,
            });
            state.record_operation(
                operation_id,
                OperationRecord {
                    outcome: "failed",
                    confirmed: true,
                    recorded_at: now_iso8601(),
                    detail: payload.clone(),
                },
            );
            Ok(payload)
        }
        Err(GeneratedPairPersistFailure::PublishedButUnsynchronized { error }) => {
            let app_path = review.app_relative_path.clone();
            let recipe_path = review.recipe_relative_path.clone();
            let app_replaced =
                review.pair.app_snapshot.publish_kind == AdmissionPublishKind::ReplaceOpenedFile;
            let created_files = if app_replaced {
                vec![recipe_path.clone()]
            } else {
                vec![recipe_path.clone(), app_path.clone()]
            };
            let replaced_files = if app_replaced {
                vec![app_path.clone()]
            } else {
                Vec::new()
            };
            let payload = json!({
                "ok": true,
                "outcome": "failed",
                "operationId": operation_id,
                "app": { "appPath": app_path },
                "recipe": { "recipePath": recipe_path },
                "createdFiles": created_files,
                "replacedFiles": replaced_files,
                "rollback": Value::Null,
                "error": {
                    "code": error.code,
                    "message": error.message,
                    "details": { "reason": error.code, "durability": "unresolved" },
                },
                "message": "The files may be present. Verify both destinations before retrying.",
            });
            state.record_operation(
                operation_id,
                OperationRecord {
                    outcome: "failed",
                    confirmed: true,
                    recorded_at: now_iso8601(),
                    detail: payload.clone(),
                },
            );
            Ok(payload)
        }
    }
}

/// Report one publication operation. The durable catalog journal takes
/// precedence over process-local review state so status remains truthful
/// after a sidecar restart.
pub(crate) fn publication_status(
    state: &AppGeneratorReviewState,
    operation_id: &str,
) -> Result<Value, ApiError> {
    let durable = match state.catalog_storage_base.as_deref() {
        Some(storage_base) => {
            CatalogChangeCoordinator::find_operation_at(storage_base, operation_id)
        }
        None => CatalogChangeCoordinator::find_operation(operation_id),
    }
    .map_err(|_| {
        ApiError::command_failed(
            "App Generator publication status could not be read.",
            json!({ "operationId": operation_id }),
        )
    })?;
    if durable.is_some() {
        return Ok(publication_status_from_journal(
            state,
            operation_id,
            durable,
        ));
    }

    Ok(match state.operation(operation_id) {
        Some(record) => json!({
            "operationId": operation_id,
            "outcome": record.outcome,
            "confirmed": record.confirmed,
            "recordedAt": record.recorded_at,
            "detail": record.detail,
        }),
        None => json!({
            "operationId": operation_id,
            "outcome": "unknown",
            "confirmed": false,
            "recordedAt": Value::Null,
            "detail": { "reason": "no_record" },
        }),
    })
}

fn publication_status_from_journal(
    state: &AppGeneratorReviewState,
    operation_id: &str,
    durable: Option<DurableOperationRecord>,
) -> Value {
    let Some(durable) = durable else {
        return json!({
            "operationId": operation_id,
            "outcome": "unknown",
            "confirmed": false,
            "recordedAt": Value::Null,
            "detail": { "reason": "no_record" },
        });
    };

    let (outcome, confirmed, message) = match durable.phase {
        DurableOperationPhase::Completed => (
            "saved",
            true,
            "The App Generator publication completed and was reconciled.",
        ),
        DurableOperationPhase::Rejected => (
            "failed",
            true,
            "The App Generator publication was rejected before publishing.",
        ),
        DurableOperationPhase::Prepared
        | DurableOperationPhase::Staged
        | DurableOperationPhase::Publishing
        | DurableOperationPhase::Published
        | DurableOperationPhase::RecoveryRequired => (
            "unknown",
            false,
            "The App Generator publication is unresolved; inspect catalog recovery status.",
        ),
    };
    let local_record = state
        .operation(operation_id)
        .filter(|record| record.confirmed && record.outcome == outcome);
    let detail = local_record
        .map(|record| record.detail.clone())
        .unwrap_or_else(|| {
            json!({
                "ok": true,
                "outcome": outcome,
                "operationId": operation_id,
                "message": message,
            })
        });
    json!({
        "operationId": operation_id,
        "outcome": outcome,
        "confirmed": confirmed,
        "recordedAt": local_record.map(|record| record.recorded_at.clone()),
        "detail": detail,
    })
}

fn refusal_error(
    error: &crate::app_definition_persistence::AppDefinitionPersistenceError,
) -> ApiError {
    let code = match error.code.as_str() {
        "app_definition_stale_state" | "recipe_destination_exists" => {
            crate::errors::ApiErrorCode::StaleReview
        }
        _ => crate::errors::ApiErrorCode::PublicationDenied,
    };
    ApiError::new(code, error.message.clone(), json!({ "reason": error.code }))
}

fn simple_diagnostic(code: &str, message: &str) -> Value {
    json!({
        "severity": "error",
        "code": code,
        "message": message,
        "field": Value::Null,
    })
}

fn admission_ui_diagnostic(diagnostic: &AppDefinitionSaveDiagnostic) -> Value {
    json!({
        "severity": diagnostic.severity,
        "code": diagnostic.code,
        "message": diagnostic.message,
        "field": diagnostic.field,
        "file": diagnostic.file,
        "objectKind": diagnostic.object_kind,
        "objectId": diagnostic.object_id,
    })
}

/// Project the reviewed permission automation intent and App permission sets.
///
/// The enabled flag reflects the author's request intent rather than the
/// presence of permission sets, because an App Definition may declare sets
/// that a disabled request neither executes nor extends. The action lists
/// describe the declared sets the review shows and are empty when the App
/// Definition declares none.
fn permission_automation_value(app: &AppDefinitionV1, enabled: bool) -> Value {
    let sets = app.permission_sets.as_ref();
    let baseline = permission_actions(sets.and_then(|sets| sets.baseline.as_ref()));
    let elevated = permission_actions(sets.and_then(|sets| sets.elevated.as_ref()));
    json!({
        "enabled": enabled,
        "baseline": { "actions": baseline },
        "elevated": { "actions": elevated, "rootRequired": true, "optional": true },
    })
}

/// Project persisted permission actions for review.
///
/// App-op actions keep their operation name in the permission field because
/// persisted App permission sets carry no separate permission identity for
/// app-ops.
fn permission_actions(set: Option<&authored_models::AppPermissionSet>) -> Vec<Value> {
    let mut actions = Vec::new();
    if let Some(set) = set {
        for action in &set.runtime {
            actions.push(json!({
                "kind": "runtime_permission",
                "permission": action.permission,
                "androidApiMin": action.android_api_min,
                "androidApiMax": action.android_api_max,
            }));
        }
        for action in &set.app_ops {
            actions.push(json!({
                "kind": "app_op",
                "permission": action.op,
                "op": action.op,
                "mode": serde_json::to_value(&action.mode).unwrap_or(Value::Null),
                "androidApiMin": action.android_api_min,
                "androidApiMax": action.android_api_max,
            }));
        }
    }
    actions
}

fn change_impact_value(
    app: &AppDefinitionV1,
    artifact_id: Option<&str>,
    pair_snapshot: Option<&GeneratedAppPairSnapshot>,
    root: &Path,
    previous_app: Option<&AppDefinitionV1>,
    warnings: Vec<Value>,
) -> Value {
    let affected_recipes = pair_snapshot
        .map(|pair| classify_affected_recipes(app, &pair.app_snapshot, root, previous_app))
        .unwrap_or_default();
    let previous_sets = previous_app.and_then(|previous| previous.permission_sets.as_ref());
    let sets = app.permission_sets.as_ref();
    json!({
        "addedArtifactIds": artifact_id.into_iter().collect::<Vec<_>>(),
        "preservedArtifactIds": previous_app
            .map(|previous| {
                previous
                    .artifacts
                    .keys()
                    .filter(|id| artifact_id != Some(id.as_str()))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "addedPermissionActions": {
            "baseline": added_actions(
                sets.and_then(|sets| sets.baseline.as_ref()),
                previous_sets.and_then(|sets| sets.baseline.as_ref()),
            ),
            "elevated": added_actions(
                sets.and_then(|sets| sets.elevated.as_ref()),
                previous_sets.and_then(|sets| sets.elevated.as_ref()),
            ),
        },
        "affectedRecipes": affected_recipes
            .iter()
            .map(|summary| json!({
                "recipeId": summary.recipe_id,
                "recipePath": summary.relative_path,
                "status": summary.status,
                "diagnostics": summary
                    .diagnostics
                    .iter()
                    .map(admission_ui_diagnostic)
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
        "warnings": warnings,
    })
}

fn added_actions(
    set: Option<&authored_models::AppPermissionSet>,
    previous: Option<&authored_models::AppPermissionSet>,
) -> Vec<Value> {
    let mut added = Vec::new();
    if let Some(set) = set {
        for action in &set.runtime {
            let present = previous.is_some_and(|previous| {
                previous.runtime.iter().any(|existing| {
                    existing.permission == action.permission
                        && existing.android_api_min == action.android_api_min
                        && existing.android_api_max == action.android_api_max
                })
            });
            if !present {
                added.push(json!({
                    "kind": "runtime_permission",
                    "permission": action.permission,
                    "androidApiMin": action.android_api_min,
                    "androidApiMax": action.android_api_max,
                }));
            }
        }
        for action in &set.app_ops {
            let present = previous.is_some_and(|previous| {
                previous.app_ops.iter().any(|existing| {
                    existing.op == action.op
                        && existing.mode == action.mode
                        && existing.android_api_min == action.android_api_min
                        && existing.android_api_max == action.android_api_max
                })
            });
            if !present {
                added.push(json!({
                    "kind": "app_op",
                    "permission": action.op,
                    "op": action.op,
                    "mode": serde_json::to_value(&action.mode).unwrap_or(Value::Null),
                    "androidApiMin": action.android_api_min,
                    "androidApiMax": action.android_api_max,
                }));
            }
        }
    }
    added
}

fn now_iso8601() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    format_timestamp(seconds)
}

fn format_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3_600,
        (time % 3_600) / 60,
        time % 60
    )
}

/// Convert days since the Unix epoch to a civil date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_change::{OperationPhase, OperationRecord as DurableOperationRecord};
    use std::io;

    const READABLE_APP: &str = "schema_version: 1\nkind: app_definition\nid: minimal\nname: Minimal\npackage_id: com.example.minimal\nartifacts:\n  apk:\n    kind: apk\n    source:\n      strategy: user_provided\n";

    #[test]
    fn durable_completed_pair_status_replays_after_process_relaunch() {
        let mut durable = DurableOperationRecord::new(
            "app-generator-pair-42",
            "app-generator-pair",
            "request-digest",
        );
        durable.phase = OperationPhase::Completed;

        let status = publication_status_from_journal(
            &AppGeneratorReviewState::default(),
            "app-generator-pair-42",
            Some(durable),
        );

        assert_eq!(status["outcome"], "saved");
        assert_eq!(status["confirmed"], true);
    }

    #[test]
    fn directory_entry_error_marks_inventory_incomplete_and_refuses_selection() {
        let catalog = tempfile::TempDir::new().expect("temporary catalog");
        let root = catalog
            .path()
            .canonicalize()
            .expect("catalog root canonicalizes");
        let apps = root.join("apps");
        fs::create_dir_all(&apps).unwrap();
        fs::write(apps.join("readable.yaml"), READABLE_APP).unwrap();

        let mut entries: Vec<io::Result<fs::DirEntry>> = fs::read_dir(&apps).unwrap().collect();
        assert_eq!(entries.len(), 1, "one readable entry expected");
        entries.push(Err(io::Error::other("injected entry failure")));

        let scan = scan_app_directory_entries(&root, entries);

        assert!(
            !scan.complete,
            "an unreadable directory entry must make the inventory incomplete"
        );
        assert!(
            scan.diagnostics.iter().any(|diagnostic| {
                diagnostic["code"] == "app_catalog_inventory_incomplete"
                    && diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("injected entry failure"))
            }),
            "diagnostics: {:#?}",
            scan.diagnostics
        );
        assert_eq!(
            scan.summaries.len(),
            1,
            "readable entries must still be listed: {:#?}",
            scan.summaries
        );
        assert_eq!(scan.summaries[0]["appPath"], "apps/readable.yaml");

        let refusal = ensure_inventory_complete(&scan)
            .expect_err("incomplete inventory must refuse selection");
        assert_eq!(
            refusal.code,
            crate::errors::ApiErrorCode::AppCatalogInventoryIncomplete
        );
    }
}
