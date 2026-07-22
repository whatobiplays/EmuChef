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

struct BindingProjection {
    safe: Map<String, Value>,
    omitted: Vec<String>,
    sensitive_omitted: Vec<String>,
    diagnostics: Vec<Value>,
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
    recent_index_path: PathBuf,
    recents: Vec<RecentEntry>,
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
            recent_index_path,
            recents,
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

    pub fn drain_document_ids(&mut self) -> Vec<String> {
        self.documents
            .drain()
            .map(|(_, document)| document.sidecar_document_id)
            .collect()
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
        })
    }

    fn touch_recent(&mut self, document: &OpenDocument) -> Result<(), String> {
        self.recents.retain(|entry| {
            entry.path != document.path && entry.configuration_id != document.configuration_id
        });
        self.recents.insert(
            0,
            RecentEntry {
                recent_handle: format!("recent_{}", Uuid::new_v4().simple()),
                configuration_id: document.configuration_id.clone(),
                name: safe_name(&document.name),
                path: document.path.clone(),
                last_opened_epoch_ms: now_epoch_ms(),
            },
        );
        self.recents.truncate(MAX_RECENTS);
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
pub struct SaveAsRequest {
    configuration_handle: String,
    name: String,
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
    Ok(Value::Array(
        store
            .recents
            .iter()
            .map(|entry| {
                json!({
                    "recentHandle": entry.recent_handle,
                    "name": safe_name(&entry.name),
                    "lastOpenedEpochMs": entry.last_opened_epoch_ms,
                    "availability": if entry.path.is_file() && fs::File::open(&entry.path).is_ok() { "available" } else { "missing" },
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
    {
        let mut store = state
            .saved_configurations
            .lock()
            .map_err(|_| state_error())?;
        store.remove_recent(&request.recent_handle)?;
    }
    open_result(&state, path, &result)
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
        "dirty": document.get("dirty").and_then(Value::as_bool).ok_or_else(invalid_document)?,
        "revision": revision,
        "devicePlan": document_string(configuration, "devicePlan")?,
        "selectedRecipes": configuration.get("selectedRecipes").cloned().ok_or_else(invalid_document)?,
        "bindings": projection.safe,
        "pendingSanitationCount": projection.omitted.len(),
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
    let public_description = public_configuration_description(&description, "");
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

fn sanitize_recents(entries: Vec<RecentEntry>) -> Vec<RecentEntry> {
    let mut handles = HashSet::new();
    let mut identities = HashSet::new();
    entries
        .into_iter()
        .filter(|entry| {
            entry.recent_handle.starts_with("recent_")
                && valid_configuration_id(&entry.configuration_id)
                && !safe_name(&entry.name).is_empty()
                && handles.insert(entry.recent_handle.clone())
                && identities.insert(entry.configuration_id.clone())
        })
        .map(|mut entry| {
            entry.name = safe_name(&entry.name);
            entry
        })
        .take(MAX_RECENTS)
        .collect()
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
            path: PathBuf::from("/private/config.yaml"),
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
        let sanitized = sanitize_recents(entries);
        assert_eq!(sanitized.len(), MAX_RECENTS);
        assert_eq!(sanitized[0].name, "Two");
        assert_eq!(
            sanitized
                .iter()
                .filter(|entry| entry.configuration_id == "saved.two")
                .count(),
            1
        );
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
                "devicePlan": "plan.one",
                "selectedRecipes": ["recipe"],
                "bindings": {
                    "recipe/safe": "portable",
                    "recipe/secret": "DO_NOT_PROJECT",
                    "recipe/inactive": "DO_NOT_PROJECT"
                }
            }
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
