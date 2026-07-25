//! Trusted saved-configuration sessions and the private recent-file index.
//!
//! React receives opaque handles and portable intent only. Configuration file
//! paths, sidecar document identifiers, schema identifiers, canonical YAML,
//! and authored catalog roots remain in this module.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use uuid::Uuid;

use crate::commands::{
    await_picker_selection, catalog, public_configuration_description, safe_error, AppState,
};
use crate::recovery::clear_recovery_after_save;

const RECENT_SCHEMA_VERSION: u64 = 1;
const MAX_RECENTS: usize = 10;

#[derive(Clone, Debug)]
struct OpenDocument {
    sidecar_document_id: String,
    path: PathBuf,
    configuration_id: String,
    name: String,
    revision: u64,
}

#[derive(Clone, Debug)]
struct PreviewDocument {
    path: PathBuf,
    source_digest: String,
    catalog_digest: String,
    runtime_revision: u64,
    preview_revision: u64,
    document: Value,
    repairs: HashMap<String, PreviewRepair>,
}

#[derive(Clone, Debug)]
enum PreviewRepair {
    RemoveRecipe { recipe_id: String },
    RemoveBinding { key: String },
    SelectOption { key: String, value: Value },
    RelinkInput { key: String, directory: bool },
}

struct BindingProjection {
    safe: Map<String, Value>,
    omitted: Vec<String>,
    sensitive_omitted: Vec<String>,
    diagnostics: Vec<Value>,
    description: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecentEntry {
    recent_handle: String,
    configuration_id: String,
    name: String,
    path: PathBuf,
    last_opened_epoch_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecentIndex {
    schema_version: u64,
    entries: Vec<RecentEntry>,
}

/// Process-local document authority plus a private, restart-stable MRU index.
pub struct SavedConfigurationStore {
    documents: HashMap<String, OpenDocument>,
    previews: HashMap<String, PreviewDocument>,
    recent_index_path: PathBuf,
    recents: Vec<RecentEntry>,
    runtime_revision: u64,
    preview_revision: u64,
    recents_revision: u64,
}

impl SavedConfigurationStore {
    pub fn load(recent_index_path: PathBuf) -> Self {
        let recents = fs::read(&recent_index_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RecentIndex>(&bytes).ok())
            .filter(|index| index.schema_version == RECENT_SCHEMA_VERSION)
            .map(|index| sanitize_recents(index.entries))
            .unwrap_or_default();
        Self {
            documents: HashMap::new(),
            previews: HashMap::new(),
            recent_index_path,
            recents,
            runtime_revision: 1,
            preview_revision: 0,
            recents_revision: 1,
        }
    }

    fn document(&self, handle: &str) -> Result<&OpenDocument, String> {
        self.documents.get(handle).ok_or_else(|| {
            safe_error(
                "configuration_session_unavailable",
                "This saved configuration is no longer open. Reopen the portable file to continue.",
            )
        })
    }

    fn recovery_source_identity(&self, handle: &str) -> Result<(String, String), String> {
        let document = self.document(handle)?;
        Ok((document.configuration_id.clone(), safe_name(&document.name)))
    }

    fn recovery_source_path(&self, configuration_id: &str) -> Option<PathBuf> {
        self.recents
            .iter()
            .find(|entry| entry.configuration_id == configuration_id)
            .filter(|entry| entry.path.is_file() && fs::File::open(&entry.path).is_ok())
            .map(|entry| entry.path.clone())
    }

    fn insert_document(&mut self, path: PathBuf, document: &Value) -> Result<String, String> {
        let sidecar_document_id = document_string(document, "documentId")?;
        let configuration = document.get("configuration").ok_or_else(invalid_document)?;
        let configuration_id = document_string(configuration, "id")?;
        let name = document_string(configuration, "name")?;
        let handle = format!("configuration_{}", Uuid::new_v4().simple());
        self.documents.insert(
            handle.clone(),
            OpenDocument {
                sidecar_document_id,
                path,
                configuration_id,
                name,
                revision: 0,
            },
        );
        Ok(handle)
    }

    fn remove_document(&mut self, handle: &str) -> Option<OpenDocument> {
        self.documents.remove(handle)
    }

    fn replace_preview(
        &mut self,
        path: PathBuf,
        source_digest: String,
        catalog_digest: String,
        document: Value,
        repairs: HashMap<String, PreviewRepair>,
    ) -> (String, Vec<String>) {
        let stale_ids = self
            .previews
            .drain()
            .filter_map(|(_, preview)| document_string(&preview.document, "documentId").ok())
            .collect::<Vec<_>>();
        self.preview_revision = self.preview_revision.saturating_add(1);
        let preview_handle = format!("configuration_preview_{}", Uuid::new_v4().simple());
        self.previews.insert(
            preview_handle.clone(),
            PreviewDocument {
                path,
                source_digest,
                catalog_digest,
                runtime_revision: self.runtime_revision,
                preview_revision: self.preview_revision,
                document,
                repairs,
            },
        );
        (preview_handle, stale_ids)
    }

    fn take_current_preview(&mut self, handle: &str) -> Result<PreviewDocument, String> {
        let preview = self.previews.remove(handle).ok_or_else(|| {
            safe_error(
                "configuration_preview_unavailable",
                "This setup preview is no longer available. Choose the file again.",
            )
        })?;
        if preview.runtime_revision != self.runtime_revision
            || preview.preview_revision != self.preview_revision
        {
            return Err(safe_error(
                "configuration_preview_stale",
                "This setup preview is outdated. Choose the file again.",
            ));
        }
        Ok(preview)
    }

    fn current_preview(&self, handle: &str) -> Result<PreviewDocument, String> {
        let preview = self.previews.get(handle).cloned().ok_or_else(|| {
            safe_error(
                "configuration_preview_unavailable",
                "This setup preview is no longer available. Choose the file again.",
            )
        })?;
        if preview.runtime_revision != self.runtime_revision
            || preview.preview_revision != self.preview_revision
        {
            return Err(safe_error(
                "configuration_preview_stale",
                "This setup preview is outdated. Choose the file again.",
            ));
        }
        Ok(preview)
    }

    fn update_preview(
        &mut self,
        handle: &str,
        expected_revision: u64,
        document: Value,
        repairs: HashMap<String, PreviewRepair>,
    ) -> Result<PreviewDocument, String> {
        if expected_revision != self.preview_revision {
            return Err(safe_error(
                "configuration_preview_stale",
                "This setup preview changed before the repair completed. Review it again.",
            ));
        }
        self.preview_revision = self.preview_revision.saturating_add(1);
        let preview = self.previews.get_mut(handle).ok_or_else(|| {
            safe_error(
                "configuration_preview_unavailable",
                "This setup preview is no longer available. Choose the file again.",
            )
        })?;
        preview.preview_revision = self.preview_revision;
        preview.document = document;
        preview.repairs = repairs;
        Ok(preview.clone())
    }

    fn remove_preview(&mut self, handle: &str) -> Option<PreviewDocument> {
        self.previews.remove(handle)
    }

    pub fn drain_document_ids(&mut self) -> Vec<String> {
        let mut ids = self
            .previews
            .drain()
            .filter_map(|(_, preview)| document_string(&preview.document, "documentId").ok())
            .collect::<Vec<_>>();
        ids.extend(
            self.documents
                .drain()
                .map(|(_, document)| document.sidecar_document_id),
        );
        self.runtime_revision = self.runtime_revision.saturating_add(1);
        self.preview_revision = self.preview_revision.saturating_add(1);
        ids
    }

    /// Return aggregate-only support data without names, identifiers, or paths.
    pub fn support_summary(&self) -> Value {
        let available = self
            .recents
            .iter()
            .filter(|entry| entry.path.is_file() && fs::File::open(&entry.path).is_ok())
            .count();
        json!({
            "openCount": self.documents.len(),
            "recentCount": self.recents.len(),
            "availableRecentCount": available,
            "missingRecentCount": self.recents.len().saturating_sub(available),
            "recentsRevision": self.recents_revision,
        })
    }

    #[cfg(test)]
    pub fn recents_revision(&self) -> u64 {
        self.recents_revision
    }

    pub fn recent_count(&self) -> usize {
        self.recents.len()
    }

    /// Clear only the MRU index. Open documents and their files remain intact.
    pub fn reset_recents(&mut self, expected_revision: u64) -> Result<(), String> {
        if self.recents_revision != expected_revision {
            return Err(safe_error(
                "recent_index_stale",
                "Recent setups changed. Review the reset category again.",
            ));
        }
        self.recents.clear();
        self.recents_revision = self.recents_revision.saturating_add(1).max(1);
        self.persist_recents()
    }

    fn touch_recent(&mut self, document: &OpenDocument) -> Result<(), String> {
        let path = canonical_recent_path(&document.path);
        self.recents
            .retain(|entry| canonical_recent_path(&entry.path) != path);
        self.recents.insert(
            0,
            RecentEntry {
                recent_handle: format!("recent_{}", Uuid::new_v4().simple()),
                configuration_id: document.configuration_id.clone(),
                name: safe_name(&document.name),
                path,
                last_opened_epoch_ms: now_epoch_ms(),
            },
        );
        self.recents.truncate(MAX_RECENTS);
        self.recents_revision = self.recents_revision.saturating_add(1).max(1);
        self.persist_recents()
    }

    fn recent(&self, handle: &str) -> Result<&RecentEntry, String> {
        self.recents
            .iter()
            .find(|entry| entry.recent_handle == handle)
            .ok_or_else(|| {
                safe_error(
                    "recent_configuration_unknown",
                    "This recent configuration entry is no longer available.",
                )
            })
    }

    fn remove_recent(&mut self, handle: &str) -> Result<(), String> {
        let previous_len = self.recents.len();
        self.recents.retain(|entry| entry.recent_handle != handle);
        if self.recents.len() == previous_len {
            return Err(safe_error(
                "recent_configuration_unknown",
                "This recent configuration entry is no longer available.",
            ));
        }
        self.recents_revision = self.recents_revision.saturating_add(1).max(1);
        self.persist_recents()
    }

    fn persist_recents(&self) -> Result<(), String> {
        let parent = self.recent_index_path.parent().ok_or_else(|| {
            safe_error(
                "recent_index_unavailable",
                "Recent configurations could not be saved.",
            )
        })?;
        fs::create_dir_all(parent).map_err(|_| {
            safe_error(
                "recent_index_unavailable",
                "Recent configurations could not be saved.",
            )
        })?;
        let bytes = serde_json::to_vec_pretty(&RecentIndex {
            schema_version: RECENT_SCHEMA_VERSION,
            entries: self.recents.clone(),
        })
        .map_err(|_| {
            safe_error(
                "recent_index_unavailable",
                "Recent configurations could not be saved.",
            )
        })?;
        let temporary = self.recent_index_path.with_extension("json.tmp");
        fs::write(&temporary, bytes).map_err(|_| {
            safe_error(
                "recent_index_unavailable",
                "Recent configurations could not be saved.",
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).map_err(|_| {
                safe_error(
                    "recent_index_unavailable",
                    "Recent configurations could not be saved.",
                )
            })?;
        }
        fs::rename(&temporary, &self.recent_index_path).map_err(|_| {
            let _ = fs::remove_file(&temporary);
            safe_error(
                "recent_index_unavailable",
                "Recent configurations could not be saved.",
            )
        })
    }
}

pub type SavedConfigurationState = Mutex<SavedConfigurationStore>;

pub(crate) struct RecentMenuEntry {
    pub recent_handle: String,
    pub label: String,
    pub available: bool,
}

pub(crate) fn recent_menu_entries(state: &AppState) -> Result<Vec<RecentMenuEntry>, String> {
    let store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    Ok(store
        .recents
        .iter()
        .map(|entry| RecentMenuEntry {
            recent_handle: entry.recent_handle.clone(),
            label: format!(
                "{} — {}",
                safe_name(&entry.name),
                safe_file_label(&entry.path)
            ),
            available: entry.path.is_file() && fs::File::open(&entry.path).is_ok(),
        })
        .collect())
}

/// Portable recovery overlay after Tauri has removed sensitive values.
pub struct RecoveryPortableIntent {
    pub dirty: bool,
    pub device_plan: String,
    pub selected_recipes: Vec<String>,
    pub bindings: Map<String, Value>,
    pub omitted_bindings: Vec<String>,
}

pub(crate) fn recovery_source_identity(
    state: &AppState,
    handle: &str,
) -> Result<(String, String), String> {
    state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .recovery_source_identity(handle)
}

/// Reopen a saved source into a fresh sidecar session and apply the recovery
/// overlay without writing the source file. Missing private recent-file state
/// intentionally returns `None` so the caller can restore unsaved intent.
pub(crate) fn restore_recovery_source(
    state: &AppState,
    configuration_id: &str,
    intent: &RecoveryPortableIntent,
) -> Result<Option<Value>, String> {
    let path = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .recovery_source_path(configuration_id);
    let Some(path) = path else {
        return Ok(None);
    };
    let opened = request_open(state, &path)?;
    let document = opened.get("document").ok_or_else(invalid_document)?;
    let actual_id = document
        .pointer("/configuration/id")
        .and_then(Value::as_str)
        .ok_or_else(invalid_document)?;
    if actual_id != configuration_id {
        close_sidecar_document(state, document);
        return Ok(None);
    }
    if !intent.dirty && intent.omitted_bindings.is_empty() {
        let mut store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        let handle = store.insert_document(path, document)?;
        let record = store.document(&handle)?.clone();
        let _ = store.touch_recent(&record);
        let mut projected = project_document(state, &handle, record.revision, document)?;
        projected["outcome"] = json!("opened");
        return Ok(Some(projected));
    }
    let document_id = document_string(document, "documentId")?;
    let current_bindings = document
        .pointer("/configuration/bindings")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(invalid_document)?;
    let apply = (|| {
        state.sidecar.request(
            "setUserConfigurationDevicePlan",
            json!({ "documentId": document_id, "devicePlan": intent.device_plan }),
        )?;
        state.sidecar.request(
            "setUserConfigurationSelectedRecipes",
            json!({ "documentId": document_id, "selectedRecipes": intent.selected_recipes }),
        )?;
        for key in current_bindings.keys() {
            state.sidecar.request(
                "removeUserConfigurationBinding",
                json!({ "documentId": document_id, "key": key }),
            )?;
        }
        for (key, value) in &intent.bindings {
            state.sidecar.request(
                "setUserConfigurationBinding",
                json!({ "documentId": document_id, "key": key, "value": value }),
            )?;
        }
        // Omitted keys are deliberately absent. Reading the final document
        // proves the source remains a dirty in-memory overlay only.
        let _ = &intent.omitted_bindings;
        state.sidecar.request(
            "getUserConfigurationDocument",
            json!({ "documentId": document_id }),
        )
    })();
    let result = match apply {
        Ok(result) => result,
        Err(_) => {
            close_sidecar_document(state, document);
            return Err(configuration_error(
                "recovery_restore_failed",
                "The saved source could not be restored safely. The recovery draft remains available.",
            ));
        }
    };
    let final_document = result.get("document").ok_or_else(invalid_document)?;
    let mut store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    let handle = store.insert_document(path, final_document)?;
    let record = store.document(&handle)?.clone();
    let _ = store.touch_recent(&record);
    let mut projected = project_document(state, &handle, record.revision, final_document)?;
    projected["outcome"] = json!("opened");
    Ok(Some(projected))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSavedConfigurationRequest {
    name: String,
    device_plan: String,
    selected_recipes: Vec<String>,
    bindings: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationHandleRequest {
    configuration_handle: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecentHandleRequest {
    recent_handle: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewHandleRequest {
    preview_handle: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAsRequest {
    configuration_handle: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedConfigurationRequest {
    configuration_handle: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedPreviewRequest {
    preview_handle: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewComparisonRequest {
    preview_handle: String,
    device_plan: Option<String>,
    selected_recipes: Vec<String>,
    bindings: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewRepairRequest {
    preview_handle: String,
    repair_handle: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigurationMutation {
    DevicePlan { value: String },
    SelectedRecipes { value: Vec<String> },
    Binding { key: String, value: Value },
    RemoveBinding { key: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateSavedConfigurationRequest {
    configuration_handle: String,
    expected_revision: u64,
    mutation: ConfigurationMutation,
}

#[tauri::command]
pub fn list_recent_configurations(state: State<'_, AppState>) -> Result<Value, String> {
    let store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    let mut identity_counts = HashMap::<&str, usize>::new();
    for entry in &store.recents {
        *identity_counts.entry(&entry.configuration_id).or_default() += 1;
    }
    Ok(Value::Array(
        store
            .recents
            .iter()
            .map(|entry| {
                json!({
                    "recentHandle": entry.recent_handle,
                    "name": safe_name(&entry.name),
                    "fileLabel": safe_file_label(&entry.path),
                    "lastOpenedEpochMs": entry.last_opened_epoch_ms,
                    "availability": if entry.path.is_file() && fs::File::open(&entry.path).is_ok() { "available" } else { "missing" },
                    "identityConflict": identity_counts.get(entry.configuration_id.as_str()).copied().unwrap_or_default() > 1,
                })
            })
            .collect(),
    ))
}

#[tauri::command]
pub async fn create_saved_configuration(
    app: AppHandle,
    request: CreateSavedConfigurationRequest,
) -> Result<Value, String> {
    let name = validated_name(&request.name)?;
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .set_file_name(format!("{}.yaml", safe_file_stem(&name)))
        .add_filter("EmuChef configuration", &["yaml", "yml"]);
    let selected = await_picker_selection(
        |complete| picker.save_file(complete),
        "configuration_picker_failed",
        "The configuration save dialog could not be opened.",
    )
    .await?;
    let Some(selected) = selected else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    let path = selected_path(selected)?;
    let configuration_id = generated_configuration_id();
    let state = app.state::<AppState>();
    let proposed_bindings = ordered_bindings(request.bindings);
    let proposed_bindings = proposed_bindings
        .as_object()
        .cloned()
        .expect("ordered bindings are an object");
    let projection = classify_bindings(
        &state,
        &request.device_plan,
        &request.selected_recipes,
        &proposed_bindings,
    )?;
    let result = state
        .sidecar
        .request(
            "createUserConfiguration",
            json!({
                "path": path,
                "configurationId": configuration_id,
                "name": name,
                "devicePlan": request.device_plan,
                "selectedRecipes": request.selected_recipes,
                "bindings": projection.safe,
                "authoredRoot": authored_root(&state)?,
            }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_create_failed",
                "The configuration could not be created.",
            )
        })?;
    let projected = open_result(&state, path, &result)?;
    clear_recovery_after_save(&state)?;
    Ok(projected)
}

#[tauri::command]
pub async fn open_saved_configuration(app: AppHandle) -> Result<Value, String> {
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .add_filter("EmuChef configuration", &["yaml", "yml"]);
    let selected = await_picker_selection(
        |complete| picker.pick_file(complete),
        "configuration_picker_failed",
        "The configuration open dialog could not be opened.",
    )
    .await?;
    let Some(selected) = selected else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    let path = selected_path(selected)?;
    let state = app.state::<AppState>();
    open_path(&state, path)
}

#[tauri::command]
pub async fn preview_saved_configuration(app: AppHandle) -> Result<Value, String> {
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .add_filter("EmuChef setup", &["yaml", "yml"]);
    let selected = await_picker_selection(
        |complete| picker.pick_file(complete),
        "configuration_picker_failed",
        "The setup open dialog could not be opened.",
    )
    .await?;
    let Some(selected) = selected else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    let path = selected_path(selected)?;
    begin_preview(&app.state::<AppState>(), path)
}

#[tauri::command]
pub fn preview_recent_configuration(
    request: RecentHandleRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let path = {
        let store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        store.recent(&request.recent_handle)?.path.clone()
    };
    if !path.is_file() {
        return Err(safe_error(
            "recent_configuration_missing",
            "This setup file is missing or inaccessible. Relink it or remove it from Recents.",
        ));
    }
    begin_preview(&state, path)
}

#[tauri::command]
pub fn confirm_saved_configuration_preview(
    request: PreviewHandleRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let preview = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .take_current_preview(&request.preview_handle)?;
    verify_preview(&state, &preview)?;
    open_result(
        &state,
        preview.path,
        &json!({ "document": preview.document }),
    )
}

#[tauri::command]
pub fn cancel_saved_configuration_preview(
    request: PreviewHandleRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let preview = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .remove_preview(&request.preview_handle);
    if let Some(preview) = preview {
        close_sidecar_document(&state, &preview.document);
    }
    Ok(())
}

#[tauri::command]
pub fn compare_saved_configuration_preview(
    request: PreviewComparisonRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let preview = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .current_preview(&request.preview_handle)?;
    verify_preview(&state, &preview)?;
    let projection = classify_document_bindings(&state, &preview.document)?;
    if status_from_diagnostics(&projection.diagnostics) == "requires_attention" {
        return Ok(json!({
            "state": "requires_repair",
            "message": "The saved setup needs repair before it can be compared with current choices.",
        }));
    }
    let Some(device_plan) = request.device_plan else {
        return Ok(json!({
            "state": "no_current_intent",
            "message": "There are no current setup choices to compare.",
        }));
    };
    let configuration = preview
        .document
        .get("configuration")
        .ok_or_else(invalid_document)?;
    let mut saved_recipes = document_string_array(configuration, "selectedRecipes")?;
    saved_recipes.sort();
    let mut current_recipes = request.selected_recipes;
    current_recipes.sort();
    let current_bindings = ordered_bindings(request.bindings)
        .as_object()
        .cloned()
        .expect("ordered bindings are an object");
    let current_projection =
        classify_bindings(&state, &device_plan, &current_recipes, &current_bindings)?;
    let matches = document_string(configuration, "devicePlan")? == device_plan
        && saved_recipes == current_recipes
        && projection.safe == current_projection.safe;
    Ok(json!({
        "state": if matches { "matches" } else { "differs" },
        "message": if matches {
            "This saved setup matches the current portable choices."
        } else {
            "This saved setup differs from the current portable choices."
        },
    }))
}

#[tauri::command]
pub async fn apply_saved_configuration_preview_repair(
    app: AppHandle,
    request: PreviewRepairRequest,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let preview = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .current_preview(&request.preview_handle)?;
    verify_preview(&state, &preview)?;
    let repair = preview
        .repairs
        .get(&request.repair_handle)
        .cloned()
        .ok_or_else(|| {
            safe_error(
                "configuration_repair_unavailable",
                "This repair choice is no longer available. Review the setup again.",
            )
        })?;
    let document_id = document_string(&preview.document, "documentId")?;
    let result = match repair {
        PreviewRepair::RemoveRecipe { recipe_id } => {
            let mut recipes = document_string_array(
                preview
                    .document
                    .get("configuration")
                    .ok_or_else(invalid_document)?,
                "selectedRecipes",
            )?;
            recipes.retain(|candidate| candidate != &recipe_id);
            state.sidecar.request(
                "setUserConfigurationSelectedRecipes",
                json!({ "documentId": document_id, "selectedRecipes": recipes }),
            )
        }
        PreviewRepair::RemoveBinding { key } => state.sidecar.request(
            "removeUserConfigurationBinding",
            json!({ "documentId": document_id, "key": key }),
        ),
        PreviewRepair::SelectOption { key, value } => state.sidecar.request(
            "setUserConfigurationBinding",
            json!({ "documentId": document_id, "key": key, "value": value }),
        ),
        PreviewRepair::RelinkInput { key, directory } => {
            let _dialog_activity = state.update_activity.reserve_native_dialog()?;
            let picker = app.dialog().file();
            let selected = if directory {
                await_picker_selection(
                    |complete| picker.pick_folder(complete),
                    "configuration_repair_picker_failed",
                    "The replacement folder dialog could not be opened.",
                )
                .await?
            } else {
                await_picker_selection(
                    |complete| picker.pick_file(complete),
                    "configuration_repair_picker_failed",
                    "The replacement file dialog could not be opened.",
                )
                .await?
            };
            let Some(selected) = selected else {
                return project_preview(
                    &state,
                    &request.preview_handle,
                    &preview.path,
                    &preview.document,
                );
            };
            let path = selected_path(selected)?;
            state.sidecar.request(
                "setUserConfigurationBinding",
                json!({ "documentId": document_id, "key": key, "value": path }),
            )
        }
    }
    .map_err(|_| {
        configuration_error(
            "configuration_repair_failed",
            "The selected repair could not be applied safely.",
        )
    })?;
    let document = result
        .get("document")
        .cloned()
        .ok_or_else(invalid_document)?;
    let projection = classify_document_bindings(&state, &document)?;
    let repairs = preview_repairs(&document, &projection.description);
    let updated = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .update_preview(
            &request.preview_handle,
            preview.preview_revision,
            document,
            repairs,
        )?;
    project_preview(
        &state,
        &request.preview_handle,
        &updated.path,
        &updated.document,
    )
}

#[tauri::command]
pub fn open_recent_configuration(
    request: RecentHandleRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let path = {
        let store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        store.recent(&request.recent_handle)?.path.clone()
    };
    if !path.is_file() {
        return Err(safe_error(
            "recent_configuration_missing",
            "This configuration file is missing or inaccessible. Relink it or remove it from Recents.",
        ));
    }
    open_path(&state, path)
}

#[tauri::command]
pub async fn relink_recent_configuration(
    app: AppHandle,
    request: RecentHandleRequest,
) -> Result<Value, String> {
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .add_filter("EmuChef configuration", &["yaml", "yml"]);
    let selected = await_picker_selection(
        |complete| picker.pick_file(complete),
        "configuration_picker_failed",
        "The configuration relink dialog could not be opened.",
    )
    .await?;
    let Some(selected) = selected else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    let path = selected_path(selected)?;
    let state = app.state::<AppState>();
    let expected_id = {
        let store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        store
            .recent(&request.recent_handle)?
            .configuration_id
            .clone()
    };
    let result = request_open(&state, &path)?;
    let document = result.get("document").ok_or_else(invalid_document)?;
    let actual_id = document
        .pointer("/configuration/id")
        .and_then(Value::as_str)
        .ok_or_else(invalid_document)?;
    if actual_id != expected_id {
        close_sidecar_document(&state, document);
        return Err(safe_error(
            "recent_configuration_identity_mismatch",
            "The selected file is a different portable configuration and cannot relink this entry.",
        ));
    }
    let name = document
        .pointer("/configuration/name")
        .and_then(Value::as_str)
        .map(safe_name)
        .ok_or_else(invalid_document)?;
    close_sidecar_document(&state, document);
    let mut store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    let canonical_path = canonical_recent_path(&path);
    if store.recents.iter().any(|entry| {
        entry.recent_handle != request.recent_handle
            && canonical_recent_path(&entry.path) == canonical_path
    }) {
        return Err(safe_error(
            "recent_configuration_path_conflict",
            "That setup file already has a Recent entry.",
        ));
    }
    let entry = store
        .recents
        .iter_mut()
        .find(|entry| entry.recent_handle == request.recent_handle)
        .ok_or_else(|| {
            safe_error(
                "recent_configuration_unknown",
                "This recent setup entry is no longer available.",
            )
        })?;
    entry.path = canonical_path;
    entry.name = name;
    entry.last_opened_epoch_ms = now_epoch_ms();
    let updated = json!({
        "recentHandle": entry.recent_handle,
        "name": entry.name,
        "fileLabel": safe_file_label(&entry.path),
        "lastOpenedEpochMs": entry.last_opened_epoch_ms,
        "availability": "available",
        "identityConflict": false,
    });
    store.recents_revision = store.recents_revision.saturating_add(1).max(1);
    store.persist_recents()?;
    Ok(updated)
}

#[tauri::command]
pub fn remove_recent_configuration(
    request: RecentHandleRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .remove_recent(&request.recent_handle)
}

#[tauri::command]
pub fn update_saved_configuration(
    request: UpdateSavedConfigurationRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let (document_id, current_revision) = {
        let store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        let document = store.document(&request.configuration_handle)?;
        (document.sidecar_document_id.clone(), document.revision)
    };
    if current_revision != request.expected_revision {
        return Err(safe_error(
            "configuration_revision_stale",
            "The saved configuration changed before this edit completed. Refresh and try again.",
        ));
    }
    let (operation, payload) = match request.mutation {
        ConfigurationMutation::DevicePlan { value } => (
            "setUserConfigurationDevicePlan",
            json!({ "documentId": document_id, "devicePlan": value }),
        ),
        ConfigurationMutation::SelectedRecipes { value } => (
            "setUserConfigurationSelectedRecipes",
            json!({ "documentId": document_id, "selectedRecipes": value }),
        ),
        ConfigurationMutation::Binding { key, value } => {
            let current = current_document(&state, &document_id)?;
            let configuration = current.get("configuration").ok_or_else(invalid_document)?;
            let mut bindings = configuration
                .get("bindings")
                .and_then(Value::as_object)
                .cloned()
                .ok_or_else(invalid_document)?;
            bindings.insert(key.clone(), value.clone());
            let projection = classify_bindings(
                &state,
                document_string(configuration, "devicePlan")?.as_str(),
                &document_string_array(configuration, "selectedRecipes")?,
                &bindings,
            )?;
            if !projection.safe.contains_key(&key) {
                return project_document(
                    &state,
                    &request.configuration_handle,
                    current_revision,
                    &current,
                );
            }
            (
                "setUserConfigurationBinding",
                json!({ "documentId": document_id, "key": key, "value": value }),
            )
        }
        ConfigurationMutation::RemoveBinding { key } => (
            "removeUserConfigurationBinding",
            json!({ "documentId": document_id, "key": key }),
        ),
    };
    let result = state.sidecar.request(operation, payload).map_err(|_| {
        configuration_error(
            "configuration_update_failed",
            "The configuration edit could not be applied.",
        )
    })?;
    let document = result.get("document").ok_or_else(invalid_document)?;
    let mut store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    let record = store
        .documents
        .get_mut(&request.configuration_handle)
        .ok_or_else(state_error)?;
    record.revision += 1;
    project_document(
        &state,
        &request.configuration_handle,
        record.revision,
        document,
    )
}

#[tauri::command]
pub fn save_saved_configuration(
    request: ConfigurationHandleRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let document_id = {
        let store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        store
            .document(&request.configuration_handle)?
            .sidecar_document_id
            .clone()
    };
    sanitize_before_save(&state, &document_id)?;
    let result = state
        .sidecar
        .request(
            "saveUserConfiguration",
            json!({ "documentId": document_id }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_save_failed",
                "The configuration could not be saved.",
            )
        })?;
    let document = result.get("document").ok_or_else(invalid_document)?;
    let mut store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    let record = store
        .documents
        .get(&request.configuration_handle)
        .cloned()
        .ok_or_else(state_error)?;
    let _ = store.touch_recent(&record);
    let projected = project_document(
        &state,
        &request.configuration_handle,
        record.revision,
        document,
    )?;
    drop(store);
    clear_recovery_after_save(&state)?;
    Ok(projected)
}

#[tauri::command]
pub async fn save_saved_configuration_as(
    app: AppHandle,
    request: SaveAsRequest,
) -> Result<Value, String> {
    let name = validated_name(&request.name)?;
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .set_file_name(format!("{}.yaml", safe_file_stem(&name)))
        .add_filter("EmuChef configuration", &["yaml", "yml"]);
    let selected = await_picker_selection(
        |complete| picker.save_file(complete),
        "configuration_picker_failed",
        "The configuration Save As dialog could not be opened.",
    )
    .await?;
    let Some(selected) = selected else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    let path = selected_path(selected)?;
    let state = app.state::<AppState>();
    let document_id = {
        let store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        store
            .document(&request.configuration_handle)?
            .sidecar_document_id
            .clone()
    };
    let configuration_id = generated_configuration_id();
    sanitize_before_save(&state, &document_id)?;
    let result = state
        .sidecar
        .request(
            "saveUserConfigurationAs",
            json!({
                "documentId": document_id,
                "path": path,
                "configurationId": configuration_id,
                "name": name,
            }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_save_as_failed",
                "The configuration could not be saved as a new portable file.",
            )
        })?;
    let document = result.get("document").ok_or_else(invalid_document)?;
    let mut store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    let record = store
        .documents
        .get_mut(&request.configuration_handle)
        .ok_or_else(state_error)?;
    record.path = path;
    record.configuration_id = configuration_id;
    record.name = name;
    record.revision += 1;
    let record = record.clone();
    let _ = store.touch_recent(&record);
    let mut projected = project_document(
        &state,
        &request.configuration_handle,
        record.revision,
        document,
    )?;
    projected["outcome"] = json!("saved");
    drop(store);
    clear_recovery_after_save(&state)?;
    Ok(projected)
}

#[tauri::command]
pub async fn duplicate_saved_configuration(
    app: AppHandle,
    request: NamedConfigurationRequest,
) -> Result<Value, String> {
    copy_saved_configuration(app, request, true).await
}

#[tauri::command]
pub async fn export_saved_configuration(
    app: AppHandle,
    request: NamedConfigurationRequest,
) -> Result<Value, String> {
    copy_saved_configuration(app, request, false).await
}

async fn copy_saved_configuration(
    app: AppHandle,
    request: NamedConfigurationRequest,
    add_to_recents: bool,
) -> Result<Value, String> {
    let name = validated_name(&request.name)?;
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .set_file_name(format!("{}.yaml", safe_file_stem(&name)))
        .add_filter("EmuChef setup", &["yaml", "yml"]);
    let selected = await_picker_selection(
        |complete| picker.save_file(complete),
        "configuration_picker_failed",
        "The setup destination dialog could not be opened.",
    )
    .await?;
    let Some(selected) = selected else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    let path = selected_path(selected)?;
    let state = app.state::<AppState>();
    let document_id = {
        let store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        store
            .document(&request.configuration_handle)?
            .sidecar_document_id
            .clone()
    };
    let current = current_document(&state, &document_id)?;
    let configuration = current.get("configuration").ok_or_else(invalid_document)?;
    let projection = classify_document_bindings(&state, &current)?;
    let configuration_id = generated_configuration_id();
    let result = state
        .sidecar
        .request(
            "createUserConfiguration",
            json!({
                "path": path,
                "configurationId": configuration_id,
                "name": name,
                "devicePlan": document_string(configuration, "devicePlan")?,
                "selectedRecipes": configuration.get("selectedRecipes").cloned().ok_or_else(invalid_document)?,
                "bindings": projection.safe,
                "authoredRoot": authored_root(&state)?,
            }),
        )
        .map_err(|_| {
            configuration_error(
                if add_to_recents { "configuration_duplicate_failed" } else { "configuration_export_failed" },
                if add_to_recents {
                    "The setup could not be duplicated. The destination may already exist."
                } else {
                    "The setup could not be exported. The destination may already exist."
                },
            )
        })?;
    let document = result.get("document").ok_or_else(invalid_document)?;
    if add_to_recents {
        let record = OpenDocument {
            sidecar_document_id: document_string(document, "documentId")?,
            path: path.clone(),
            configuration_id,
            name: name.clone(),
            revision: 0,
        };
        state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?
            .touch_recent(&record)?;
    }
    close_sidecar_document(&state, document);
    Ok(json!({
        "outcome": "saved",
        "name": name,
        "fileLabel": safe_file_label(&path),
    }))
}

#[tauri::command]
pub async fn import_saved_configuration(
    app: AppHandle,
    request: NamedPreviewRequest,
) -> Result<Value, String> {
    let name = validated_name(&request.name)?;
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .set_file_name(format!("{}.yaml", safe_file_stem(&name)))
        .add_filter("EmuChef setup", &["yaml", "yml"]);
    let selected = await_picker_selection(
        |complete| picker.save_file(complete),
        "configuration_picker_failed",
        "The imported setup destination dialog could not be opened.",
    )
    .await?;
    let Some(selected) = selected else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    let destination = selected_path(selected)?;
    let state = app.state::<AppState>();
    let preview = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .take_current_preview(&request.preview_handle)?;
    verify_preview(&state, &preview)?;
    let document_id = document_string(&preview.document, "documentId")?;
    sanitize_before_save(&state, &document_id)?;
    let configuration_id = generated_configuration_id();
    let result = state
        .sidecar
        .request(
            "saveUserConfigurationAs",
            json!({
                "documentId": document_id,
                "path": destination,
                "configurationId": configuration_id,
                "name": name,
            }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_import_failed",
                "The setup could not be imported. The destination may already exist.",
            )
        })?;
    open_result(&state, destination, &result)
}

#[tauri::command]
pub fn rename_saved_configuration(
    request: NamedConfigurationRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let name = validated_name(&request.name)?;
    let record = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .document(&request.configuration_handle)?
        .clone();
    let extension = record
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("yaml");
    let destination =
        record
            .path
            .with_file_name(format!("{}.{}", safe_file_stem(&name), extension));
    if canonical_recent_path(&destination) == canonical_recent_path(&record.path) {
        return Err(safe_error(
            "configuration_rename_same_path",
            "Choose a name that produces a different setup filename.",
        ));
    }
    sanitize_before_save(&state, &record.sidecar_document_id)?;
    let result = state
        .sidecar
        .request(
            "saveUserConfigurationAs",
            json!({
                "documentId": record.sidecar_document_id,
                "path": destination,
                "configurationId": record.configuration_id,
                "name": name,
            }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_rename_failed",
                "The setup could not be renamed. The destination may already exist.",
            )
        })?;
    let document = result.get("document").ok_or_else(invalid_document)?;
    let remove_result = fs::remove_file(&record.path);
    let mut store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    let updated = store
        .documents
        .get_mut(&request.configuration_handle)
        .ok_or_else(state_error)?;
    updated.path = destination.clone();
    updated.name = name;
    updated.revision = updated.revision.saturating_add(1);
    let updated = updated.clone();
    store
        .recents
        .retain(|entry| canonical_recent_path(&entry.path) != canonical_recent_path(&record.path));
    store.touch_recent(&updated)?;
    let projected = project_document(
        &state,
        &request.configuration_handle,
        updated.revision,
        document,
    )?;
    if remove_result.is_err() {
        return Err(safe_error(
            "configuration_rename_source_remains",
            "The renamed setup is valid, but the original file could not be removed. Both files remain so no setup data was lost.",
        ));
    }
    Ok(projected)
}

#[tauri::command]
pub fn close_saved_configuration(
    request: ConfigurationHandleRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let Some(document) = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .remove_document(&request.configuration_handle)
    else {
        return Ok(());
    };
    state
        .sidecar
        .request(
            "closeUserConfiguration",
            json!({ "documentId": document.sidecar_document_id }),
        )
        .map(|_| ())
        .map_err(|_| {
            configuration_error(
                "configuration_close_failed",
                "The configuration session could not be closed.",
            )
        })
}

fn open_path(state: &AppState, path: PathBuf) -> Result<Value, String> {
    let result = request_open(state, &path)?;
    open_result(state, path, &result)
}

fn begin_preview(state: &AppState, path: PathBuf) -> Result<Value, String> {
    let initial_source_digest = source_digest(&path)?;
    let result = request_open(state, &path)?;
    if source_digest(&path)? != initial_source_digest {
        if let Some(document) = result.get("document") {
            close_sidecar_document(state, document);
        }
        return Err(safe_error(
            "configuration_preview_changed",
            "The setup changed while it was being inspected. Choose it again.",
        ));
    }
    let document = result
        .get("document")
        .cloned()
        .ok_or_else(invalid_document)?;
    let catalog_digest = catalog(state)?.digest().to_string();
    let preview_projection = classify_document_bindings(state, &document)?;
    let repairs = preview_repairs(&document, &preview_projection.description);
    let (preview_handle, stale_document_ids) = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .replace_preview(
            canonical_recent_path(&path),
            initial_source_digest,
            catalog_digest,
            document.clone(),
            repairs,
        );
    for document_id in stale_document_ids {
        let _ = state.sidecar.request(
            "closeUserConfiguration",
            json!({ "documentId": document_id }),
        );
    }
    project_preview(state, &preview_handle, &path, &document)
}

fn verify_preview(state: &AppState, preview: &PreviewDocument) -> Result<(), String> {
    let current_source_digest = source_digest(&preview.path)?;
    let current_catalog_digest = catalog(state)?.digest().to_string();
    if let Err(error) =
        verify_preview_snapshot(preview, &current_source_digest, &current_catalog_digest)
    {
        close_sidecar_document(state, &preview.document);
        return Err(error);
    }
    Ok(())
}

fn verify_preview_snapshot(
    preview: &PreviewDocument,
    current_source_digest: &str,
    current_catalog_digest: &str,
) -> Result<(), String> {
    if current_source_digest != preview.source_digest {
        return Err(safe_error(
            "configuration_preview_changed",
            "The setup changed after it was inspected. Choose it again.",
        ));
    }
    if current_catalog_digest != preview.catalog_digest {
        return Err(safe_error(
            "configuration_preview_stale_catalog",
            "The setup catalog changed after this preview. Inspect the setup again.",
        ));
    }
    Ok(())
}

fn project_preview(
    state: &AppState,
    preview_handle: &str,
    path: &Path,
    document: &Value,
) -> Result<Value, String> {
    let configuration = document.get("configuration").ok_or_else(invalid_document)?;
    let projection = classify_document_bindings(state, document)?;
    let validation_state = status_from_diagnostics(&projection.diagnostics);
    let baseline_state = document
        .pointer("/compatibilityStatus/baselineState")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    let schema_version = configuration
        .get("schemaVersion")
        .and_then(Value::as_i64)
        .ok_or_else(invalid_document)?;
    let setup_label = configuration
        .pointer("/compatibility/devicePlan/label")
        .and_then(Value::as_str)
        .unwrap_or("Saved setup");
    let feature_labels = configuration
        .pointer("/compatibility/recipes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|recipe| recipe.get("selected").and_then(Value::as_bool) == Some(true))
        .filter_map(|recipe| recipe.get("label").and_then(Value::as_str))
        .map(safe_name)
        .collect::<Vec<_>>();
    let modified: Option<u64> = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| duration.as_millis().try_into().ok());
    let compatibility_state = if validation_state == "requires_attention" {
        "repair_required"
    } else if baseline_state == "materially_changed" {
        "materially_changed"
    } else if baseline_state == "pending_first_v2_save" {
        "migrated_baseline_pending"
    } else if validation_state == "valid_with_warnings" {
        "compatible_with_warnings"
    } else {
        "compatible"
    };
    let mut repair_actions = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?
        .previews
        .get(preview_handle)
        .map(|preview| {
            preview
                .repairs
                .iter()
                .map(|(handle, repair)| {
                    let (kind, label) = match repair {
                        PreviewRepair::RemoveRecipe { .. } => (
                            "remove_recipe",
                            "Remove unavailable optional feature".to_string(),
                        ),
                        PreviewRepair::RemoveBinding { .. } => {
                            ("remove_binding", "Remove retired saved input".to_string())
                        }
                        PreviewRepair::SelectOption { value, .. } => {
                            ("select_option", format!("Use {}", safe_value_label(value)))
                        }
                        PreviewRepair::RelinkInput { directory, .. } => (
                            "relink_input",
                            if *directory {
                                "Choose replacement folder…"
                            } else {
                                "Choose replacement file…"
                            }
                            .to_string(),
                        ),
                    };
                    json!({ "repairHandle": handle, "kind": kind, "label": label })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    repair_actions.sort_by(|left, right| {
        left.get("label")
            .and_then(Value::as_str)
            .cmp(&right.get("label").and_then(Value::as_str))
    });
    Ok(json!({
        "outcome": "previewed",
        "previewHandle": preview_handle,
        "name": safe_name(&document_string(configuration, "name")?),
        "fileLabel": safe_file_label(path),
        "schemaVersion": schema_version,
        "lastModifiedEpochMs": modified,
        "setupLabel": safe_name(setup_label),
        "featureLabels": feature_labels,
        "savedInputCount": projection.safe.len(),
        "omittedInputCount": projection.omitted.len(),
        "compatibility": {
            "state": compatibility_state,
            "baselineState": baseline_state,
            "requiresRepair": validation_state == "requires_attention",
            "message": preview_compatibility_message(compatibility_state),
        },
        "repairActions": repair_actions,
    }))
}

fn preview_repairs(document: &Value, description: &Value) -> HashMap<String, PreviewRepair> {
    let mut repairs = HashMap::new();
    let known_recipes = description
        .get("recipeOptions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|recipe| recipe.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    for recipe_id in document
        .pointer("/configuration/selectedRecipes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !known_recipes.contains(recipe_id) {
            repairs.insert(
                format!("preview_repair_{}", Uuid::new_v4().simple()),
                PreviewRepair::RemoveRecipe {
                    recipe_id: recipe_id.to_string(),
                },
            );
        }
    }
    for input in description
        .get("inputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(key) = input.get("key").and_then(Value::as_str) else {
            continue;
        };
        let codes = input
            .get("diagnostics")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|diagnostic| diagnostic.get("code").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        if codes.contains("invalid_enum_value") {
            for value in input
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                repairs.insert(
                    format!("preview_repair_{}", Uuid::new_v4().simple()),
                    PreviewRepair::SelectOption {
                        key: key.to_string(),
                        value: value.clone(),
                    },
                );
            }
        }
        if codes.contains("binding_path_missing") {
            repairs.insert(
                format!("preview_repair_{}", Uuid::new_v4().simple()),
                PreviewRepair::RelinkInput {
                    key: key.to_string(),
                    directory: input.get("pathKind").and_then(Value::as_str) == Some("directory"),
                },
            );
        }
    }
    for diagnostic in description
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let code = diagnostic
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let key = diagnostic.get("key").and_then(Value::as_str);
        if matches!(
            code,
            "unknown_input"
                | "binding_recipe_not_selected"
                | "incompatible_binding_type"
                | "invalid_path_prefix"
        ) {
            if let Some(key) = key {
                repairs.insert(
                    format!("preview_repair_{}", Uuid::new_v4().simple()),
                    PreviewRepair::RemoveBinding {
                        key: key.to_string(),
                    },
                );
            }
        }
    }
    repairs
}

fn safe_value_label(value: &Value) -> String {
    match value {
        Value::String(value) => value.chars().take(80).collect(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        _ => "available option".to_string(),
    }
}

fn preview_compatibility_message(state: &str) -> &'static str {
    match state {
        "repair_required" => "This setup needs repair before it can be used.",
        "materially_changed" => "The authored setup contract changed since this file was saved. Review and repair it before use.",
        "migrated_baseline_pending" => "This older setup is valid against the current catalog. Its first explicit save will establish a compatibility baseline.",
        "compatible_with_warnings" => "This setup can be opened, but some saved choices need attention.",
        _ => "This setup is compatible with the current catalog.",
    }
}

fn source_digest(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|_| {
        safe_error(
            "configuration_source_unavailable",
            "The selected setup file is missing or inaccessible.",
        )
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn request_open(state: &AppState, path: &Path) -> Result<Value, String> {
    state
        .sidecar
        .request(
            "openUserConfiguration",
            json!({ "path": path, "authoredRoot": authored_root(state)? }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_cannot_use",
                "The selected file is not a readable EmuChef portable configuration.",
            )
        })
}

fn open_result(state: &AppState, path: PathBuf, result: &Value) -> Result<Value, String> {
    let document = result.get("document").ok_or_else(invalid_document)?;
    let mut store = state
        .saved_configurations
        .lock()
        .map_err(|_| state_error())?;
    let handle = store.insert_document(path, document)?;
    let record = store.document(&handle)?.clone();
    let _ = store.touch_recent(&record);
    let mut projected = project_document(state, &handle, record.revision, document)?;
    projected["outcome"] = json!("opened");
    Ok(projected)
}

fn project_document(
    state: &AppState,
    handle: &str,
    revision: u64,
    document: &Value,
) -> Result<Value, String> {
    let projection = classify_document_bindings(state, document)?;
    if !projection.sensitive_omitted.is_empty() {
        let mut recovery = state.recovery.lock().map_err(|_| {
            safe_error(
                "recovery_state_unavailable",
                "Recovery state is temporarily unavailable.",
            )
        })?;
        recovery.note_required_reentry(projection.sensitive_omitted.clone());
    }
    project_classified_document(handle, revision, document, projection)
}

/// Build the only React-facing saved-document projection from an authoritative
/// binding classification. Keeping this transformation separate makes the
/// close-without-save contract testable without replacing the real sidecar.
fn project_classified_document(
    handle: &str,
    revision: u64,
    document: &Value,
    projection: BindingProjection,
) -> Result<Value, String> {
    let configuration = document.get("configuration").ok_or_else(invalid_document)?;
    let diagnostics = projection.diagnostics;
    let status = status_from_diagnostics(&diagnostics);
    Ok(json!({
        "configurationHandle": handle,
        "name": document_string(configuration, "name")?,
        "schemaVersion": configuration.get("schemaVersion").cloned().ok_or_else(invalid_document)?,
        "dirty": document.get("dirty").and_then(Value::as_bool).ok_or_else(invalid_document)?,
        "revision": revision,
        "devicePlan": document_string(configuration, "devicePlan")?,
        "selectedRecipes": configuration.get("selectedRecipes").cloned().ok_or_else(invalid_document)?,
        "bindings": projection.safe,
        "pendingSanitationCount": projection.omitted.len(),
        "compatibility": {
            "baselineState": document.pointer("/compatibilityStatus/baselineState").and_then(Value::as_str).unwrap_or("unavailable"),
        },
        "validation": { "state": status, "diagnostics": diagnostics },
    }))
}

fn classify_document_bindings(
    state: &AppState,
    document: &Value,
) -> Result<BindingProjection, String> {
    let configuration = document.get("configuration").ok_or_else(invalid_document)?;
    classify_bindings(
        state,
        document_string(configuration, "devicePlan")?.as_str(),
        &document_string_array(configuration, "selectedRecipes")?,
        configuration
            .get("bindings")
            .and_then(Value::as_object)
            .ok_or_else(invalid_document)?,
    )
}

fn classify_bindings(
    state: &AppState,
    device_plan: &str,
    selected_recipes: &[String],
    bindings: &Map<String, Value>,
) -> Result<BindingProjection, String> {
    let description = state
        .sidecar
        .request(
            "describeConfiguration",
            json!({
                "catalog": catalog(state)?.internal_payload(),
                "devicePlan": device_plan,
                "selectedRecipes": selected_recipes,
                "bindings": bindings,
            }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_classification_failed",
                "Saved input values could not be classified safely.",
            )
        })?;
    Ok(binding_projection_from_description(bindings, &description))
}

/// Apply the backend description's active sensitivity contracts to saved
/// bindings, then reuse the normal public diagnostic projection.
fn binding_projection_from_description(
    bindings: &Map<String, Value>,
    description: &Value,
) -> BindingProjection {
    let (safe, omitted, sensitive_omitted) = filter_binding_values(bindings, description);
    let public_description = public_configuration_description(description, "");
    let diagnostics = public_description
        .get("inputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|input| {
            input
                .get("diagnostics")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .cloned()
        })
        .chain(
            public_description
                .get("diagnostics")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .cloned(),
        )
        .collect();
    BindingProjection {
        safe,
        omitted,
        sensitive_omitted,
        diagnostics,
        description: public_description,
    }
}

fn filter_binding_values(
    bindings: &Map<String, Value>,
    description: &Value,
) -> (Map<String, Value>, Vec<String>, Vec<String>) {
    let contracts = description
        .get("inputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|input| {
            Some((
                input.get("key")?.as_str()?.to_string(),
                input.get("sensitive")?.as_bool()?,
            ))
        })
        .collect::<HashMap<_, _>>();
    let mut safe = Map::new();
    let mut omitted = Vec::new();
    let mut sensitive_omitted = Vec::new();
    for (key, value) in bindings {
        match contracts.get(key) {
            Some(false) => {
                safe.insert(key.clone(), value.clone());
            }
            Some(true) => {
                omitted.push(key.clone());
                sensitive_omitted.push(key.clone());
            }
            None => omitted.push(key.clone()),
        }
    }
    omitted.sort();
    sensitive_omitted.sort();
    (safe, omitted, sensitive_omitted)
}

fn sanitize_before_save(state: &AppState, document_id: &str) -> Result<(), String> {
    let current = current_document(state, document_id)?;
    let projection = classify_document_bindings(state, &current)?;
    for key in projection.omitted {
        state
            .sidecar
            .request(
                "removeUserConfigurationBinding",
                json!({ "documentId": document_id, "key": key }),
            )
            .map_err(|_| {
                configuration_error(
                    "configuration_sanitation_failed",
                    "Saved input values could not be filtered safely.",
                )
            })?;
    }
    Ok(())
}

fn current_document(state: &AppState, document_id: &str) -> Result<Value, String> {
    state
        .sidecar
        .request(
            "getUserConfigurationDocument",
            json!({ "documentId": document_id }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_state_unavailable",
                "The saved configuration could not be refreshed safely.",
            )
        })?
        .get("document")
        .cloned()
        .ok_or_else(invalid_document)
}

fn document_string_array(document: &Value, field: &str) -> Result<Vec<String>, String> {
    document
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(invalid_document)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(invalid_document)
        })
        .collect()
}

fn status_from_diagnostics(diagnostics: &[Value]) -> &'static str {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.get("severity").and_then(Value::as_str) == Some("error"))
    {
        "requires_attention"
    } else if diagnostics.is_empty() {
        "valid"
    } else {
        "valid_with_warnings"
    }
}

fn authored_root(state: &AppState) -> Result<Value, String> {
    catalog(state)?
        .internal_payload()
        .get("root")
        .cloned()
        .ok_or_else(|| {
            safe_error(
                "catalog_resource_invalid",
                "The packaged setup catalog is unavailable.",
            )
        })
}

fn ordered_bindings(bindings: HashMap<String, Value>) -> Value {
    let mut entries = bindings.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Value::Object(entries.into_iter().collect::<Map<_, _>>())
}

fn close_sidecar_document(state: &AppState, document: &Value) {
    if let Some(document_id) = document.get("documentId").and_then(Value::as_str) {
        let _ = state.sidecar.request(
            "closeUserConfiguration",
            json!({ "documentId": document_id }),
        );
    }
}

fn selected_path(path: FilePath) -> Result<PathBuf, String> {
    path.into_path().map_err(|_| {
        safe_error(
            "configuration_path_unavailable",
            "The selected configuration file could not be opened by the application.",
        )
    })
}

fn validated_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(safe_error(
            "configuration_name_invalid",
            "Configuration names must contain between 1 and 120 characters.",
        ));
    }
    Ok(name.to_string())
}

fn generated_configuration_id() -> String {
    format!("saved.{}", Uuid::new_v4().simple())
}

fn safe_name(name: &str) -> String {
    name.trim().chars().take(120).collect()
}

fn safe_file_stem(name: &str) -> String {
    let stem = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(['-', '.'])
        .to_string();
    if stem.is_empty() {
        "emuchef-configuration".to_string()
    } else {
        stem
    }
}

fn safe_file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.chars().take(160).collect())
        .unwrap_or_else(|| "Setup file".to_string())
}

fn sanitize_recents(entries: Vec<RecentEntry>) -> Vec<RecentEntry> {
    let mut handles = HashSet::new();
    let mut paths = HashSet::new();
    let mut entries = entries
        .into_iter()
        .filter_map(|mut entry| {
            entry.path = canonical_recent_path(&entry.path);
            (entry.recent_handle.starts_with("recent_")
                && valid_configuration_id(&entry.configuration_id)
                && !safe_name(&entry.name).is_empty()
                && handles.insert(entry.recent_handle.clone())
                && paths.insert(entry.path.clone()))
            .then(|| {
                entry.name = safe_name(&entry.name);
                entry
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .last_opened_epoch_ms
            .cmp(&left.last_opened_epoch_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    entries.truncate(MAX_RECENTS);
    entries
}

fn canonical_recent_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn document_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(invalid_document)
}

fn valid_configuration_id(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn invalid_document() -> String {
    safe_error(
        "configuration_response_invalid",
        "The Rust runtime returned an invalid saved-configuration response.",
    )
}

fn state_error() -> String {
    safe_error(
        "configuration_state_unavailable",
        "Saved-configuration session state is unavailable.",
    )
}

fn configuration_error(code: &str, message: &str) -> String {
    safe_error(code, message)
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recent(handle: &str, id: &str, name: &str) -> RecentEntry {
        RecentEntry {
            recent_handle: handle.to_string(),
            configuration_id: id.to_string(),
            name: name.to_string(),
            path: PathBuf::from(format!("/private/{handle}.yaml")),
            last_opened_epoch_ms: 1,
        }
    }

    #[test]
    fn recent_sanitization_is_bounded_unique_and_safe() {
        let mut entries = vec![
            recent("bad", "saved.one", "Bad handle"),
            recent("recent_one", "bad/id", "Bad id"),
            recent("recent_two", "saved.two", "  Two  "),
            recent("recent_duplicate", "saved.two", "Duplicate"),
        ];
        for index in 0..20 {
            entries.push(recent(
                &format!("recent_{index}"),
                &format!("saved.{index}"),
                "Valid",
            ));
        }
        entries[2].last_opened_epoch_ms = 2;
        let sanitized = sanitize_recents(entries);
        assert_eq!(sanitized.len(), MAX_RECENTS);
        assert!(sanitized.iter().any(|entry| entry.name == "Two"));
        assert_eq!(
            sanitized
                .iter()
                .filter(|entry| entry.configuration_id == "saved.two")
                .count(),
            1
        );
    }

    #[test]
    fn recent_sanitization_deduplicates_paths_but_retains_identity_conflicts() {
        let mut same_path = recent("recent_same_path", "saved.other", "Other");
        same_path.path = PathBuf::from("/private/recent_one.yaml");
        let sanitized = sanitize_recents(vec![
            recent("recent_one", "saved.shared", "First"),
            recent("recent_two", "saved.shared", "Second"),
            same_path,
        ]);

        assert_eq!(sanitized.len(), 2);
        assert!(sanitized
            .iter()
            .all(|entry| entry.configuration_id == "saved.shared"));
    }

    #[test]
    fn resetting_recents_preserves_open_documents_and_saved_files() {
        let temp = tempfile::tempdir().unwrap();
        let saved_path = temp.path().join("saved.yaml");
        fs::write(&saved_path, b"schemaVersion: 2\n").unwrap();
        let mut store = SavedConfigurationStore::load(temp.path().join("recents.json"));
        store.documents.insert(
            "configuration_open".to_string(),
            OpenDocument {
                sidecar_document_id: "document-one".to_string(),
                path: saved_path.clone(),
                configuration_id: "saved.one".to_string(),
                name: "Saved One".to_string(),
                revision: 0,
            },
        );
        store.recents.push(RecentEntry {
            recent_handle: "recent_one".to_string(),
            configuration_id: "saved.one".to_string(),
            name: "Saved One".to_string(),
            path: saved_path.clone(),
            last_opened_epoch_ms: 1,
        });
        let revision = store.recents_revision();

        store.reset_recents(revision).unwrap();

        assert_eq!(store.recent_count(), 0);
        assert!(store.documents.contains_key("configuration_open"));
        assert_eq!(fs::read(&saved_path).unwrap(), b"schemaVersion: 2\n");
        assert!(store.reset_recents(revision).is_err());
    }

    #[test]
    fn previews_reject_changed_bytes_catalogs_runtime_restarts_and_supersession() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SavedConfigurationStore::load(temp.path().join("recents.json"));
        let document = json!({ "documentId": "preview-document" });
        let (first, _) = store.replace_preview(
            temp.path().join("saved.yaml"),
            "source-one".to_string(),
            "catalog-one".to_string(),
            document.clone(),
            HashMap::new(),
        );
        let (second, _) = store.replace_preview(
            temp.path().join("saved.yaml"),
            "source-two".to_string(),
            "catalog-two".to_string(),
            document,
            HashMap::new(),
        );

        assert!(store.current_preview(&first).is_err());
        let current = store.current_preview(&second).unwrap();
        assert!(verify_preview_snapshot(&current, "changed-source", "catalog-two").is_err());
        assert!(verify_preview_snapshot(&current, "source-two", "changed-catalog").is_err());
        assert!(verify_preview_snapshot(&current, "source-two", "catalog-two").is_ok());

        store.runtime_revision = store.runtime_revision.saturating_add(1);
        assert!(store.current_preview(&second).is_err());
    }

    #[test]
    fn status_mapping_distinguishes_clean_warning_and_attention() {
        assert_eq!(status_from_diagnostics(&[]), "valid");
        assert_eq!(
            status_from_diagnostics(&[json!({ "severity": "warning" })]),
            "valid_with_warnings"
        );
        assert_eq!(
            status_from_diagnostics(&[json!({ "severity": "error" })]),
            "requires_attention"
        );
    }

    #[test]
    fn generated_ids_and_file_stems_are_portable() {
        assert!(generated_configuration_id().starts_with("saved."));
        assert_eq!(safe_file_stem(" Pocket S / ROMs "), "Pocket-S---ROMs");
        assert_eq!(safe_file_stem("///"), "emuchef-configuration");
    }

    #[test]
    fn frontend_requests_reject_paths_and_unknown_authority_fields() {
        assert!(
            serde_json::from_value::<CreateSavedConfigurationRequest>(json!({
                "name": "Saved",
                "devicePlan": "plan.one",
                "selectedRecipes": [],
                "bindings": {},
                "path": "/not/frontend-authority.yaml",
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<SaveAsRequest>(json!({
            "configurationHandle": "configuration_opaque",
            "name": "Copy",
            "planDigest": "forbidden",
        }))
        .is_err());
        assert!(
            serde_json::from_value::<UpdateSavedConfigurationRequest>(json!({
                "configurationHandle": "configuration_opaque",
                "expectedRevision": 1,
                "mutation": { "kind": "device_plan", "value": "plan.one", "serial": "forbidden" },
            }))
            .is_err()
        );
    }

    #[test]
    fn corrupt_or_wrong_version_recent_index_loads_empty() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("recent-configurations.json");
        fs::write(&path, b"not json").unwrap();
        assert!(SavedConfigurationStore::load(path.clone())
            .recents
            .is_empty());
        fs::write(&path, br#"{"schemaVersion":99,"entries":[]}"#).unwrap();
        assert!(SavedConfigurationStore::load(path).recents.is_empty());
    }

    #[test]
    fn recovery_source_lookup_fails_closed_when_private_source_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SavedConfigurationStore::load(temp.path().join("recents.json"));
        store.recents.push(RecentEntry {
            recent_handle: "recent_source".to_string(),
            configuration_id: "saved.source".to_string(),
            name: "Source".to_string(),
            path: temp.path().join("missing.yaml"),
            last_opened_epoch_ms: 1,
        });
        assert_eq!(store.recovery_source_path("saved.source"), None);
    }

    #[test]
    fn saved_binding_projection_keeps_only_active_explicitly_nonsensitive_values() {
        let bindings = Map::from_iter([
            ("recipe/safe".to_string(), json!("portable")),
            ("recipe/secret".to_string(), json!("DO_NOT_PROJECT")),
            ("recipe/inactive".to_string(), json!("DO_NOT_PROJECT")),
        ]);
        let description = json!({
            "inputs": [
                { "key": "recipe/safe", "sensitive": false },
                { "key": "recipe/secret", "sensitive": true }
            ]
        });
        let (safe, omitted, sensitive) = filter_binding_values(&bindings, &description);
        assert_eq!(
            safe,
            Map::from_iter([("recipe/safe".to_string(), json!("portable"))])
        );
        assert_eq!(omitted, vec!["recipe/inactive", "recipe/secret"]);
        assert_eq!(sensitive, vec!["recipe/secret"]);
        let serialized = serde_json::to_string(&safe).unwrap();
        assert!(!serialized.contains("DO_NOT_PROJECT"));
    }

    #[test]
    fn projected_pending_sanitation_can_close_without_rewriting_the_source_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("saved.yaml");
        let original = b"bindings:\n  recipe/safe: portable\n  recipe/secret: legacy-value\n  recipe/inactive: retired-value\n";
        fs::write(&path, original).unwrap();
        let mut store = SavedConfigurationStore::load(temp.path().join("recents.json"));
        let document = json!({
            "documentId": "document-one",
            "dirty": false,
            "configuration": {
                "id": "saved.one",
                "name": "Saved",
                "schemaVersion": 2,
                "devicePlan": "plan.one",
                "selectedRecipes": ["recipe"],
                "bindings": {
                    "recipe/safe": "portable",
                    "recipe/secret": "DO_NOT_PROJECT",
                    "recipe/inactive": "DO_NOT_PROJECT"
                }
            },
            "compatibilityStatus": { "baselineState": "unchanged" }
        });
        let description = json!({
            "devicePlan": "plan.one",
            "selectedRecipes": ["recipe"],
            "expandedRecipes": ["recipe"],
            "recipeOptions": [],
            "inputs": [
                {
                    "key": "recipe/safe", "recipeId": "recipe", "inputId": "safe",
                    "type": "string", "role": "option", "label": "Portable option",
                    "description": "A portable option.", "required": false, "multiple": false,
                    "options": [], "validation": {}, "sensitive": false,
                    "value": "portable", "valueSource": "user_configuration", "diagnostics": []
                },
                {
                    "key": "recipe/secret", "recipeId": "recipe", "inputId": "secret",
                    "type": "string", "role": "credential", "label": "Private token",
                    "description": "Enter the private token.", "required": false, "multiple": false,
                    "options": [], "validation": {}, "sensitive": true,
                    "value": "DO_NOT_PROJECT", "valueSource": "user_configuration", "diagnostics": []
                }
            ],
            "diagnostics": []
        });
        let handle = store.insert_document(path.clone(), &document).unwrap();
        let bindings = document["configuration"]["bindings"].as_object().unwrap();
        let projection = binding_projection_from_description(bindings, &description);
        let public = project_classified_document(&handle, 0, &document, projection).unwrap();

        assert_eq!(public["bindings"], json!({ "recipe/safe": "portable" }));
        assert_eq!(public["pendingSanitationCount"], 2);
        assert!(!public.to_string().contains("DO_NOT_PROJECT"));

        let removed = store.remove_document(&handle).unwrap();
        assert_eq!(removed.path, path);
        assert_eq!(fs::read(&removed.path).unwrap(), original);
    }
}
