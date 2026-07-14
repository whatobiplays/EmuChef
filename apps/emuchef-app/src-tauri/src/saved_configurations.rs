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

use crate::commands::{await_picker_selection, catalog, safe_error, AppState};

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
                "bindings": ordered_bindings(request.bindings),
                "authoredRoot": authored_root(&state)?,
            }),
        )
        .map_err(|_| {
            configuration_error(
                "configuration_create_failed",
                "The configuration could not be created.",
            )
        })?;
    open_result(&state, path, &result)
}

#[tauri::command]
pub async fn open_saved_configuration(app: AppHandle) -> Result<Value, String> {
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
        ConfigurationMutation::Binding { key, value } => (
            "setUserConfigurationBinding",
            json!({ "documentId": document_id, "key": key, "value": value }),
        ),
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
    project_document(&request.configuration_handle, record.revision, document)
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
    project_document(&request.configuration_handle, record.revision, document)
}

#[tauri::command]
pub async fn save_saved_configuration_as(
    app: AppHandle,
    request: SaveAsRequest,
) -> Result<Value, String> {
    let name = validated_name(&request.name)?;
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
    let mut projected = project_document(&request.configuration_handle, record.revision, document)?;
    projected["outcome"] = json!("saved");
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
    let mut projected = project_document(&handle, record.revision, document)?;
    projected["outcome"] = json!("opened");
    Ok(projected)
}

fn project_document(handle: &str, revision: u64, document: &Value) -> Result<Value, String> {
    let configuration = document.get("configuration").ok_or_else(invalid_document)?;
    let diagnostics = public_diagnostics(document.get("diagnostics"));
    let status = status_from_diagnostics(&diagnostics);
    Ok(json!({
        "configurationHandle": handle,
        "name": document_string(configuration, "name")?,
        "dirty": document.get("dirty").and_then(Value::as_bool).ok_or_else(invalid_document)?,
        "revision": revision,
        "devicePlan": document_string(configuration, "devicePlan")?,
        "selectedRecipes": configuration.get("selectedRecipes").cloned().ok_or_else(invalid_document)?,
        "bindings": configuration.get("bindings").cloned().ok_or_else(invalid_document)?,
        "validation": { "state": status, "diagnostics": diagnostics },
    }))
}

fn public_diagnostics(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|diagnostic| {
            json!({
                "key": diagnostic.get("key"),
                "code": diagnostic.get("code"),
                "message": diagnostic.get("message"),
                "severity": diagnostic.get("severity"),
            })
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
}
