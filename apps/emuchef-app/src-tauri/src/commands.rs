//! React-facing commands and DTO projection.
//!
//! Sidecar DTOs are never returned directly. This module removes trusted paths,
//! exact serials, raw command output, and internal plan snapshots at the IPC
//! boundary.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::adb::{AdbManager, AdbSetupStatusDto, PLATFORM_TOOLS_URL};
use crate::catalog::CatalogDescriptor;
use crate::execution::ExecutionHandleStore;
use crate::handles::{ReviewedPlanSnapshot, SessionHandles};
use crate::recovery::RecoveryState;
use crate::saved_configurations::SavedConfigurationState;
use crate::sidecar::{RuntimeStatusDto, SidecarState};
use crate::support::SupportStore;
use crate::updates::{ActivityGate, UpdateService};

pub struct AppState {
    pub sidecar: SidecarState,
    pub catalog: Result<CatalogDescriptor, String>,
    pub adb: Mutex<AdbManager>,
    pub platform_tools_selections: Mutex<PlatformToolsSelectionStore>,
    pub handles: Mutex<SessionHandles>,
    pub executions: Mutex<ExecutionHandleStore>,
    pub saved_configurations: SavedConfigurationState,
    pub recovery: RecoveryState,
    pub support: Mutex<SupportStore>,
    pub updates: UpdateService,
    pub update_activity: ActivityGate,
}

/// Holds at most one user-selected archive path behind an opaque, one-shot
/// handle. Paths never cross the React IPC boundary.
#[derive(Default)]
pub struct PlatformToolsSelectionStore {
    selected: Option<(String, PathBuf)>,
}

impl PlatformToolsSelectionStore {
    fn replace(&mut self, path: PathBuf) -> String {
        let handle = format!("platform_tools_selection_{}", Uuid::new_v4().simple());
        self.selected = Some((handle.clone(), path));
        handle
    }

    fn take(&mut self, handle: &str) -> Result<PathBuf, String> {
        let Some((expected, _)) = self.selected.as_ref() else {
            return Err(safe_error(
                "platform_tools_selection_stale",
                "Choose the Platform-Tools ZIP again before continuing.",
            ));
        };
        if expected != handle {
            return Err(safe_error(
                "platform_tools_selection_stale",
                "Choose the Platform-Tools ZIP again before continuing.",
            ));
        }
        let (_, path) = self.selected.take().expect("selection was checked above");
        Ok(path)
    }

    fn clear(&mut self) {
        self.selected = None;
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PlatformToolsPickerResult {
    Cancelled,
    Selected {
        #[serde(rename = "selectionHandle")]
        selection_handle: String,
    },
}

#[tauri::command]
pub fn get_runtime_status(state: State<'_, AppState>) -> RuntimeStatusDto {
    state.sidecar.status()
}

/// Begin a new frontend session without retaining process-local authority.
#[tauri::command]
pub fn begin_app_session(state: State<'_, AppState>) -> Result<Value, String> {
    reset_app_session(&state, true)?;
    state
        .recovery
        .lock()
        .map_err(|_| {
            safe_error(
                "recovery_state_unavailable",
                "Recovery state is temporarily unavailable.",
            )
        })?
        .begin_session()
}

/// Restart the Rust sidecar after proving no execution is in flight.
#[tauri::command]
pub fn restart_runtime(state: State<'_, AppState>) -> Result<RuntimeStatusDto, String> {
    if state
        .executions
        .lock()
        .map_err(|_| {
            safe_error(
                "execution_state_unavailable",
                "Execution state is unavailable.",
            )
        })?
        .has_in_flight()
    {
        return Err(safe_error(
            "execution_active",
            "The Rust runtime cannot restart while an execution is starting or active.",
        ));
    }
    state.sidecar.initialize();
    reset_app_session(&state, false)?;
    Ok(state.sidecar.status())
}

fn reset_app_session(state: &AppState, close_documents: bool) -> Result<(), String> {
    state
        .platform_tools_selections
        .lock()
        .map_err(|_| {
            safe_error(
                "selection_state_unavailable",
                "File selection state is unavailable.",
            )
        })?
        .clear();
    let document_ids = state
        .saved_configurations
        .lock()
        .map_err(|_| {
            safe_error(
                "configuration_state_unavailable",
                "Saved-configuration session state is unavailable.",
            )
        })?
        .drain_document_ids();
    if close_documents {
        for document_id in document_ids {
            let _ = state.sidecar.request(
                "closeUserConfiguration",
                json!({ "documentId": document_id }),
            );
        }
    }
    state
        .handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?
        .invalidate_all();
    state
        .executions
        .lock()
        .map_err(|_| {
            safe_error(
                "execution_state_unavailable",
                "Execution state is unavailable.",
            )
        })?
        .reset();
    state
        .support
        .lock()
        .map_err(|_| safe_error("support_state_unavailable", "Support state is unavailable."))?
        .invalidate();
    Ok(())
}

#[tauri::command]
pub fn get_catalog(state: State<'_, AppState>) -> Result<Value, String> {
    let catalog = catalog(&state)?;
    let result = state
        .sidecar
        .request(
            "describeCatalog",
            json!({ "catalog": catalog.internal_payload() }),
        )
        .map_err(|_| {
            safe_error(
                "catalog_unavailable",
                "The bundled setup catalog could not be loaded.",
            )
        })?;
    let recipes = result
        .get("recipes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            safe_error(
                "catalog_invalid",
                "The bundled setup catalog response is invalid.",
            )
        })?
        .iter()
        .map(|recipe| {
            json!({
                "id": recipe.get("id"),
                "name": recipe.get("name"),
                "description": recipe.get("description"),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "catalog": catalog.public_identity(), "recipes": recipes }))
}

#[tauri::command]
pub fn get_adb_setup_status(state: State<'_, AppState>) -> Result<AdbSetupStatusDto, String> {
    Ok(state
        .adb
        .lock()
        .map_err(|_| {
            safe_error(
                "adb_state_unavailable",
                "Platform-Tools setup state is unavailable.",
            )
        })?
        .status())
}

#[tauri::command]
pub fn open_platform_tools_download_page(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(PLATFORM_TOOLS_URL, None::<&str>)
        .map_err(|_| {
            safe_error(
                "platform_tools_page_open_failed",
                "The Android Platform-Tools page could not be opened in the default browser.",
            )
        })
}

#[tauri::command]
pub async fn pick_platform_tools_zip(app: AppHandle) -> Result<PlatformToolsPickerResult, String> {
    app.state::<AppState>()
        .platform_tools_selections
        .lock()
        .map_err(|_| {
            safe_error(
                "selection_state_unavailable",
                "File selection state is unavailable.",
            )
        })?
        .clear();
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .add_filter("Android Platform-Tools ZIP", &["zip"]);
    let selected = await_picker_selection(
        |complete| picker.pick_file(complete),
        "platform_tools_picker_failed",
        "The Platform-Tools ZIP picker could not be opened.",
    )
    .await?;
    let Some(selected) = selected else {
        return Ok(PlatformToolsPickerResult::Cancelled);
    };
    let path = selected.into_path().map_err(|_| {
        safe_error(
            "archive_path_unavailable",
            "The selected ZIP could not be opened by the application.",
        )
    })?;

    let selection_handle = app
        .state::<AppState>()
        .platform_tools_selections
        .lock()
        .map_err(|_| {
            safe_error(
                "selection_state_unavailable",
                "File selection state is unavailable.",
            )
        })?
        .replace(path);
    Ok(PlatformToolsPickerResult::Selected { selection_handle })
}

#[tauri::command]
pub async fn install_platform_tools_selection(
    selection_handle: String,
    app: AppHandle,
) -> Result<AdbSetupStatusDto, String> {
    let path = app
        .state::<AppState>()
        .platform_tools_selections
        .lock()
        .map_err(|_| {
            safe_error(
                "selection_state_unavailable",
                "File selection state is unavailable.",
            )
        })?
        .take(&selection_handle)?;

    let import_app = app.clone();
    let result = run_import_task(move || {
        let state = import_app.state::<AppState>();
        let mut adb = state.adb.lock().map_err(|_| {
            safe_error(
                "adb_state_unavailable",
                "Platform-Tools setup state is unavailable.",
            )
        })?;
        let result = adb.import_zip(&path);
        drop(adb);
        result
    })
    .await?;

    let state = app.state::<AppState>();
    state
        .handles
        .lock()
        .map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "The device session could not be reset.",
            )
        })?
        .invalidate_runtime_authority_preserving_identities();
    Ok(result)
}

pub(crate) type PickerCompletion<T> = Box<dyn FnOnce(Option<T>) + Send>;

/// Bridges the dialog plugin's non-blocking callback into the Tauri async runtime.
///
/// The callback is dropped if the native dialog cannot be opened, which closes
/// the channel and produces a stable actionable error instead of hanging IPC.
pub(crate) async fn await_picker_selection<T, F>(
    open_picker: F,
    error_code: &'static str,
    error_message: &'static str,
) -> Result<Option<T>, String>
where
    T: Send + 'static,
    F: FnOnce(PickerCompletion<T>),
{
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    open_picker(Box::new(move |selection| {
        let _ = sender.try_send(selection);
    }));
    receiver
        .recv()
        .await
        .ok_or_else(|| safe_error(error_code, error_message))
}

/// Runs archive validation and installation away from the async IPC executor.
async fn run_import_task<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| {
            safe_error(
                "platform_tools_import_worker_failed",
                "The Platform-Tools import worker stopped unexpectedly.",
            )
        })?
}

#[tauri::command]
pub fn remove_platform_tools(state: State<'_, AppState>) -> Result<AdbSetupStatusDto, String> {
    if state
        .executions
        .lock()
        .map_err(|_| {
            safe_error(
                "execution_state_unavailable",
                "Execution state is unavailable.",
            )
        })?
        .has_in_flight()
    {
        return Err(safe_error(
            "execution_active",
            "Platform-Tools cannot be removed while an execution is starting or active.",
        ));
    }
    state
        .platform_tools_selections
        .lock()
        .map_err(|_| {
            safe_error(
                "selection_state_unavailable",
                "File selection state is unavailable.",
            )
        })?
        .clear();
    let result = state
        .adb
        .lock()
        .map_err(|_| {
            safe_error(
                "adb_state_unavailable",
                "Platform-Tools setup state is unavailable.",
            )
        })?
        .remove()?;
    state
        .handles
        .lock()
        .map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "The device session could not be reset.",
            )
        })?
        .invalidate_all();
    state
        .executions
        .lock()
        .map_err(|_| {
            safe_error(
                "execution_state_unavailable",
                "Execution state is unavailable.",
            )
        })?
        .reset();
    Ok(result)
}

#[tauri::command]
pub fn poll_devices(state: State<'_, AppState>) -> Result<Value, String> {
    let adb_path = current_adb_path(&state)?;
    let inventory = state
        .sidecar
        .request("listAdbDevices", json!({ "adbPath": adb_path }))
        .map_err(|_| {
            safe_error(
                "adb_inventory_failed",
                "Connected Android devices could not be listed.",
            )
        })?;
    let devices = state
        .handles
        .lock()
        .map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "Device session state is unavailable.",
            )
        })?
        .update_devices(&inventory)?;
    serde_json::to_value(devices).map_err(|_| {
        safe_error(
            "device_projection_failed",
            "Device results could not be prepared.",
        )
    })
}

#[tauri::command]
pub fn probe_device(device_handle: String, state: State<'_, AppState>) -> Result<Value, String> {
    let adb_path = current_adb_path(&state)?;
    let serial = state
        .handles
        .lock()
        .map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "Device session state is unavailable.",
            )
        })?
        .device(&device_handle)?
        .serial
        .clone();
    let facts = state
        .sidecar
        .request(
            "probeDevice",
            json!({ "adbPath": adb_path, "serial": &serial }),
        )
        .map_err(|_| {
            safe_error(
                "adb_probe_failed",
                "The selected device information could not be read.",
            )
        })?;
    state
        .handles
        .lock()
        .map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "Device session state is unavailable.",
            )
        })?
        .set_facts(&device_handle, facts.clone())?;
    Ok(public_device_facts(&device_handle, &facts, &serial))
}

#[tauri::command]
pub fn match_device(device_handle: String, state: State<'_, AppState>) -> Result<Value, String> {
    let facts = state
        .handles
        .lock()
        .map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "Device session state is unavailable.",
            )
        })?
        .facts(&device_handle)?
        .clone();
    let catalog = catalog(&state)?;
    let exact_serial = facts
        .get("serial")
        .and_then(Value::as_str)
        .map(str::to_string);
    let result = state
        .sidecar
        .request(
            "matchDevice",
            json!({ "catalog": catalog.internal_payload(), "facts": facts }),
        )
        .map_err(|_| {
            safe_error(
                "device_match_failed",
                "The device could not be matched to the setup catalog.",
            )
        })?;
    Ok(public_match(&result, exact_serial.as_deref()))
}

#[tauri::command]
pub fn describe_configuration(
    device_handle: String,
    device_plan: String,
    selected_recipes: Option<Vec<String>>,
    bindings: HashMap<String, Value>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let binding_keys = bindings.keys().cloned().collect::<HashSet<_>>();
    let payload = configuration_payload(
        &state,
        &device_handle,
        &device_plan,
        selected_recipes,
        bindings,
    )?;
    let exact_serial = payload
        .pointer("/targetDevice/serial")
        .and_then(Value::as_str)
        .filter(|serial| !serial.is_empty())
        .ok_or_else(|| {
            safe_error(
                "target_binding_unavailable",
                "The selected device binding is unavailable.",
            )
        })?
        .to_string();
    let result = state
        .sidecar
        .request("describeConfiguration", payload)
        .map_err(|error| configuration_sidecar_error(&error, &exact_serial))?;
    let required_reentry = {
        let mut recovery = state.recovery.lock().map_err(|_| {
            safe_error(
                "recovery_state_unavailable",
                "Recovery state is temporarily unavailable.",
            )
        })?;
        recovery.record_schema(&result);
        recovery.note_current_binding_keys(&binding_keys);
        recovery.required_reentry()
    };
    let mut public = public_configuration_description(&result, &exact_serial);
    add_recovery_reentry_diagnostics(&mut public, &required_reentry, &binding_keys);
    Ok(public)
}

#[tauri::command]
pub fn create_review(
    device_handle: String,
    device_plan: String,
    selected_recipes: Option<Vec<String>>,
    bindings: HashMap<String, Value>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let missing_reentry = state
        .recovery
        .lock()
        .map_err(|_| {
            safe_error(
                "recovery_state_unavailable",
                "Recovery state is temporarily unavailable.",
            )
        })?
        .required_reentry()
        .into_iter()
        .any(|key| !bindings.contains_key(&key));
    if missing_reentry {
        return Err(safe_error(
            "recovery_sensitive_input_required",
            "Re-enter the omitted sensitive configuration values before creating a new review.",
        ));
    }
    let payload = configuration_payload(
        &state,
        &device_handle,
        &device_plan,
        selected_recipes,
        bindings,
    )?;
    let target = payload
        .get("targetDevice")
        .cloned()
        .expect("configuration payload always binds a target");
    let catalog_identity = payload
        .get("catalog")
        .cloned()
        .expect("configuration payload always binds a catalog");
    let result = state
        .sidecar
        .request("planConfiguration", payload)
        .map_err(|_| {
            safe_error(
                "review_generation_failed",
                "The reviewed plan could not be generated.",
            )
        })?;
    let plan = result
        .get("plan")
        .filter(|value| !value.is_null())
        .cloned()
        .ok_or_else(|| {
            safe_error(
                "review_not_ready",
                "Resolve the setup validation errors before reviewing the plan.",
            )
        })?;
    let digest = result
        .get("planDigest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            safe_error(
                "review_digest_missing",
                "The reviewed plan omitted its canonical digest.",
            )
        })?
        .to_string();
    let catalog_digest = catalog(&state)?.digest().to_string();
    let platform_tools_identity = state
        .adb
        .lock()
        .map_err(|_| {
            safe_error(
                "adb_state_unavailable",
                "Platform-Tools setup state is unavailable.",
            )
        })?
        .installation_identity()?;
    let snapshot = ReviewedPlanSnapshot {
        response: result.clone(),
        target,
        catalog_identity,
        catalog_digest,
        plan_digest: digest.clone(),
        device_handle,
        platform_tools_identity: Some(platform_tools_identity),
        created: Instant::now(),
        last_access: Instant::now(),
    };
    let mut handles = state.handles.lock().map_err(|_| {
        safe_error(
            "session_state_unavailable",
            "Review session state is unavailable.",
        )
    })?;
    handles.invalidate_catalog(catalog(&state)?.digest());
    let review_handle = handles.insert_review(snapshot);
    Ok(public_review(&review_handle, &digest, &plan, &result))
}

#[tauri::command]
pub fn discard_review(review_handle: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .handles
        .lock()
        .map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "Review session state is unavailable.",
            )
        })?
        .discard_review(&review_handle)
}

#[tauri::command]
pub fn get_review_status(
    review_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut handles = state.handles.lock().map_err(|_| {
        safe_error(
            "session_state_unavailable",
            "Review session state is unavailable.",
        )
    })?;
    let review = handles.review(&review_handle)?;
    let catalog = &review.catalog_identity;
    let exact_serial = review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut public = json!({
        "reviewHandle": review_handle,
        "planDigest": review.plan_digest,
        "target": {
            "manufacturer": review.target.get("manufacturer"),
            "model": review.target.get("model"),
            "androidApiLevel": review.target.get("androidApiLevel"),
        },
        "catalog": {
            "sourceKind": catalog.get("sourceKind"),
            "sourceId": catalog.get("sourceId"),
            "version": catalog.get("version"),
            "contentDigest": catalog.get("contentDigest"),
        },
        "snapshotRetained": review.response.get("plan").is_some(),
    });
    if let Some(serial) = exact_serial.as_deref().filter(|serial| !serial.is_empty()) {
        redact_exact_serial(&mut public, serial);
    }
    Ok(public)
}

#[tauri::command]
/// Opens the requested native input dialog without blocking the Tauri IPC executor.
///
/// Cancellation is represented as `Ok(None)`. The selected paths remain within
/// the existing trusted path-input DTO and are not inspected on the async task.
pub async fn pick_input_path(
    app: AppHandle,
    path_kind: String,
    multiple: bool,
) -> Result<Option<Vec<String>>, String> {
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app.dialog().file();
    let selected = match (path_kind.as_str(), multiple) {
        ("file", true) => {
            await_picker_selection(
                |complete| picker.pick_files(complete),
                "input_path_picker_failed",
                "The input file picker could not be opened.",
            )
            .await?
        }
        ("file", false) => await_picker_selection(
            |complete| picker.pick_file(complete),
            "input_path_picker_failed",
            "The input file picker could not be opened.",
        )
        .await?
        .map(|path| vec![path]),
        ("directory", _) => await_picker_selection(
            |complete| picker.pick_folder(complete),
            "input_path_picker_failed",
            "The input folder picker could not be opened.",
        )
        .await?
        .map(|path| vec![path]),
        _ => {
            return Err(safe_error(
                "path_kind_invalid",
                "The requested path picker type is unsupported.",
            ))
        }
    };
    Ok(selected.map(|paths: Vec<FilePath>| {
        paths
            .into_iter()
            .filter_map(|path| path.into_path().ok())
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }))
}

pub(crate) fn catalog(state: &AppState) -> Result<&CatalogDescriptor, String> {
    state.catalog.as_ref().map_err(|_| {
        safe_error(
            "catalog_resource_invalid",
            "The packaged setup catalog is missing or failed integrity verification.",
        )
    })
}

pub(crate) fn current_adb_path(state: &AppState) -> Result<String, String> {
    Ok(state
        .adb
        .lock()
        .map_err(|_| {
            safe_error(
                "adb_state_unavailable",
                "Platform-Tools setup state is unavailable.",
            )
        })?
        .adb_path()?
        .to_string_lossy()
        .into_owned())
}

fn configuration_payload(
    state: &AppState,
    device_handle: &str,
    device_plan: &str,
    selected_recipes: Option<Vec<String>>,
    bindings: HashMap<String, Value>,
) -> Result<Value, String> {
    let handles = state.handles.lock().map_err(|_| {
        safe_error(
            "session_state_unavailable",
            "Device session state is unavailable.",
        )
    })?;
    let device = handles.device(device_handle)?;
    let facts = handles.facts(device_handle)?;
    Ok(configuration_payload_from_parts(
        catalog(state)?.internal_payload(),
        device_plan,
        selected_recipes,
        bindings,
        &device.serial,
        facts,
    ))
}

/// Converts trusted snake_case probe facts into the sidecar's camelCase request.
fn configuration_payload_from_parts(
    catalog: Value,
    device_plan: &str,
    selected_recipes: Option<Vec<String>>,
    bindings: HashMap<String, Value>,
    serial: &str,
    facts: &Value,
) -> Value {
    let manufacturer = facts.get("manufacturer").cloned().unwrap_or(Value::Null);
    let model = facts.get("model").cloned().unwrap_or(Value::Null);
    let android_version = facts.get("android_version").cloned().unwrap_or(Value::Null);
    let android_api_level = facts
        .get("android_api_level")
        .cloned()
        .unwrap_or(Value::Null);
    let mut binding_map = Map::new();
    let mut bindings = bindings.into_iter().collect::<Vec<_>>();
    bindings.sort_by(|left, right| left.0.cmp(&right.0));
    for (key, value) in bindings {
        binding_map.insert(key, value);
    }
    json!({
        "catalog": catalog,
        "devicePlan": device_plan,
        "selectedRecipes": selected_recipes,
        "bindings": binding_map,
        "deviceContext": {
            "manufacturer": manufacturer,
            "model": model,
            "androidVersion": android_version,
            "androidApiLevel": android_api_level,
        },
        "targetDevice": {
            "serial": serial,
            "manufacturer": manufacturer,
            "model": model,
            "androidApiLevel": android_api_level,
        },
    })
}

fn public_device_facts(device_handle: &str, facts: &Value, exact_serial: &str) -> Value {
    let mut public = json!({
        "deviceHandle": device_handle,
        "manufacturer": facts.get("manufacturer"),
        "brand": facts.get("brand"),
        "model": facts.get("model"),
        "androidVersion": facts.get("android_version"),
        "androidApiLevel": facts.get("android_api_level"),
    });
    if !exact_serial.is_empty() {
        redact_exact_serial(&mut public, exact_serial);
    }
    public
}

fn public_match(result: &Value, exact_serial: Option<&str>) -> Value {
    let candidates = |field: &str| {
        result
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|candidate| {
                json!({
                    "planId": candidate.get("planId"),
                    "name": candidate.get("name"),
                    "description": candidate.get("description"),
                    "profileId": candidate.get("profileId"),
                    "profileName": candidate.get("profileName"),
                    "confidence": candidate.get("confidence"),
                    "reasons": candidate.get("reasons"),
                    "requiresExplicitChoice": candidate.get("requiresExplicitChoice"),
                })
            })
            .collect::<Vec<_>>()
    };
    let mut public = json!({
        "confidence": result.get("confidence"),
        "recommendedPlanId": result.get("recommendedPlanId"),
        "requiresExplicitChoice": result.get("requiresExplicitChoice"),
        "candidates": candidates("candidates"),
        "safeGenericPlans": candidates("safeGenericPlans"),
        "blocked": result.get("blocked"),
        "blockReason": result.get("blockReason"),
    });
    if let Some(serial) = exact_serial.filter(|serial| !serial.is_empty()) {
        redact_exact_serial(&mut public, serial);
    }
    public
}

fn public_configuration_description(result: &Value, exact_serial: &str) -> Value {
    let recipe_options = result
        .get("recipeOptions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|recipe| {
            json!({
                "id": recipe.get("id"),
                "name": recipe.get("name"),
                "description": recipe.get("description"),
                "selected": recipe.get("selected"),
                "recommended": recipe.get("recommended"),
                "dependencyRequired": recipe.get("dependencyRequired"),
                "available": recipe.get("available"),
                "unavailableCapabilities": recipe.get("unavailableCapabilities"),
            })
        })
        .collect::<Vec<_>>();
    let inputs = result
        .get("inputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|input| {
            let validation = input.get("validation").unwrap_or(&Value::Null);
            let options = input
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|option| option.get("value").cloned())
                .collect::<Vec<_>>();
            json!({
                "key": input.get("key"),
                "recipeId": input.get("recipeId"),
                "inputId": input.get("inputId"),
                "type": input.get("type"),
                "label": input.get("label"),
                "description": input.get("description"),
                "required": input.get("required"),
                "multiple": input.get("multiple"),
                "sensitive": input.get("sensitive"),
                "options": options,
                "pathKind": validation.get("pathKind"),
                "acceptedExtensions": validation.get("allowedExtensions"),
                "value": input.get("value"),
                "valueSource": input.get("valueSource"),
                "diagnostics": public_diagnostics(input.get("diagnostics")),
            })
        })
        .collect::<Vec<_>>();
    let mut public = json!({
        "devicePlan": result.get("devicePlan"),
        "selectedRecipes": result.get("selectedRecipes"),
        "expandedRecipes": result.get("expandedRecipes"),
        "recipeOptions": recipe_options,
        "inputs": inputs,
        "diagnostics": public_diagnostics(result.get("diagnostics")),
    });
    if !exact_serial.is_empty() {
        redact_exact_serial(&mut public, exact_serial);
    }
    public
}

fn add_recovery_reentry_diagnostics(
    description: &mut Value,
    required_reentry: &[String],
    supplied_bindings: &HashSet<String>,
) {
    for key in required_reentry {
        if supplied_bindings.contains(key) {
            continue;
        }
        let diagnostic = json!({
            "key": key,
            "code": "recovery_sensitive_input_required",
            "message": "This sensitive value was not stored in recovery. Re-enter it before review.",
            "severity": "error",
        });
        let mut attached = false;
        if let Some(inputs) = description.get_mut("inputs").and_then(Value::as_array_mut) {
            if let Some(input) = inputs
                .iter_mut()
                .find(|input| input.get("key").and_then(Value::as_str) == Some(key))
            {
                if let Some(diagnostics) =
                    input.get_mut("diagnostics").and_then(Value::as_array_mut)
                {
                    diagnostics.push(diagnostic.clone());
                    attached = true;
                }
            }
        }
        if !attached {
            if let Some(diagnostics) = description
                .get_mut("diagnostics")
                .and_then(Value::as_array_mut)
            {
                diagnostics.push(diagnostic);
            }
        }
    }
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

fn public_review(review_handle: &str, digest: &str, plan: &Value, result: &Value) -> Value {
    let exact_serial = plan
        .get("target_device")
        .and_then(|target| target.get("serial"))
        .and_then(Value::as_str);
    let recipes = plan
        .get("recipes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let steps = plan
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let groups = recipes
        .iter()
        .map(|recipe| {
            let recipe_id = recipe
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let steps = steps
                .iter()
                .filter(|step| step.get("recipe_ref").and_then(Value::as_str) == Some(recipe_id))
                .map(|step| {
                    let capabilities = step
                        .get("constraints")
                        .and_then(|value| value.get("capabilities"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let elevated = capabilities.iter().any(|capability| {
                        matches!(
                            capability.as_str(),
                            Some("root_shell" | "app_data_write" | "package_remove_for_user")
                        )
                    });
                    json!({
                        "name": step.get("name"),
                        "note": step.get("note"),
                        "elevated": elevated,
                        "kindLabel": public_step_kind(
                            step.get("type").and_then(Value::as_str).unwrap_or("unknown")
                        ),
                        "requirements": capabilities.iter().filter_map(Value::as_str).map(public_capability).collect::<Vec<_>>(),
                        "technicalId": step.get("id"),
                        "technicalType": step.get("type"),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "recipeId": recipe_id,
                "recipeName": recipe.get("name"),
                "recipeDescription": recipe.get("description"),
                "steps": steps,
            })
        })
        .collect::<Vec<_>>();
    let target = plan.get("target_device").cloned().unwrap_or(Value::Null);
    let context = plan.get("device_context").cloned().unwrap_or(Value::Null);
    let warnings = result
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|diagnostic| diagnostic.get("severity").and_then(Value::as_str) == Some("warning"))
        .map(|diagnostic| json!({ "code": diagnostic.get("code"), "message": diagnostic.get("message") }))
        .collect::<Vec<_>>();
    let selected_inputs = result
        .get("resolvedInputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|input| {
            !input
                .get("inputId")
                .and_then(Value::as_str)
                .is_some_and(|id| id.to_ascii_lowercase().contains("serial"))
        })
        .filter_map(|input| {
            let value = input.get("value").filter(|value| !value.is_null())?;
            Some(json!({
                "key": input.get("key"),
                "value": public_input_value(value),
                "source": input.get("source"),
            }))
        })
        .collect::<Vec<_>>();
    let mut public = json!({
        "reviewHandle": review_handle,
        "planDigest": digest,
        "target": {
            "manufacturer": target.get("manufacturer").or_else(|| context.get("manufacturer")),
            "model": target.get("model").or_else(|| context.get("model")),
            "androidVersion": context.get("android_version"),
            "androidApiLevel": target.get("android_api_level").or_else(|| context.get("android_api_level")),
        },
        "groups": groups,
        "selectedInputs": selected_inputs,
        "warnings": warnings,
    });
    if let Some(serial) = exact_serial.filter(|serial| !serial.is_empty()) {
        redact_exact_serial(&mut public, serial);
    }
    public
}

fn public_step_kind(step_type: &str) -> &'static str {
    match step_type {
        "install_apk" => "Install app",
        "launch_app" => "Launch app",
        "copy_files" => "Copy files",
        "resolve_artifacts" => "Prepare required files",
        "shell" | "shell_command" => "Run device action",
        "uninstall_package" => "Remove app for user",
        "wait" => "Wait",
        _ => "Device setup action",
    }
}

fn public_capability(capability: &str) -> String {
    capability.replace('_', " ")
}

fn public_input_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => if *value { "Yes" } else { "No" }.to_string(),
        Value::Array(values) => values
            .iter()
            .map(public_input_value)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Number(value) => value.to_string(),
        _ => "Configured".to_string(),
    }
}

pub(crate) fn redact_exact_serial(value: &mut Value, exact_serial: &str) {
    match value {
        Value::String(text) => {
            if text.contains(exact_serial) {
                *text = text.replace(exact_serial, "[device]");
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_exact_serial(value, exact_serial);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                redact_exact_serial(value, exact_serial);
            }
        }
        _ => {}
    }
}

pub(crate) fn safe_error(code: &str, message: &str) -> String {
    json!({ "code": code, "message": message }).to_string()
}

/// Maps trusted sidecar failures to stable, path- and serial-free React errors.
fn configuration_sidecar_error(internal_error: &str, exact_serial: &str) -> String {
    #[cfg(debug_assertions)]
    eprintln!(
        "describeConfiguration sidecar error: {}",
        redact_internal_sidecar_error(internal_error, exact_serial)
    );
    #[cfg(not(debug_assertions))]
    let _ = exact_serial;

    let sidecar_code = serde_json::from_str::<Value>(internal_error)
        .ok()
        .and_then(|error| {
            error
                .get("code")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    match sidecar_code.as_deref() {
        Some("invalid_request") => safe_error(
            "configuration_request_invalid",
            "The setup request was not accepted by the runtime. Reconnect the device and try again.",
        ),
        Some("load_failed") => safe_error(
            "configuration_catalog_invalid",
            "The selected device plan or one of its recipes could not be loaded from the bundled catalog.",
        ),
        Some("validation_failed") => safe_error(
            "configuration_validation_failed",
            "The selected setup contains invalid values. Review the highlighted inputs and try again.",
        ),
        _ => safe_error(
            "configuration_description_failed",
            "The selected setup could not be validated because the runtime request failed. Try again.",
        ),
    }
}

#[cfg(debug_assertions)]
fn redact_internal_sidecar_error(internal_error: &str, exact_serial: &str) -> String {
    let without_serial = if exact_serial.is_empty() {
        internal_error.to_string()
    } else {
        internal_error.replace(exact_serial, "[device]")
    };
    redact_absolute_paths(&without_serial)
}

pub(crate) fn redact_absolute_paths(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut redacted = String::with_capacity(value.len());
    let mut index = 0;
    let mut active_quote = None;
    while index < characters.len() {
        if characters[index] != '/' {
            if matches!(characters[index], '"' | '\'')
                && index
                    .checked_sub(1)
                    .is_none_or(|position| characters[position] != '\\')
            {
                active_quote = if active_quote == Some(characters[index]) {
                    None
                } else {
                    active_quote.or(Some(characters[index]))
                };
            }
            redacted.push(characters[index]);
            index += 1;
            continue;
        }
        let previous = index.checked_sub(1).map(|position| characters[position]);
        let token_start = (0..index)
            .rev()
            .find(|position| {
                characters[*position].is_whitespace()
                    || matches!(characters[*position], '"' | '\'' | '(' | '=')
            })
            .map_or(0, |position| position + 1);
        let inside_url = characters[token_start..index]
            .windows(3)
            .any(|window| window == [':', '/', '/']);
        let path_start = !inside_url
            && previous != Some(':')
            && previous != Some('/')
            && (index == 0
                || active_quote.is_some()
                || previous.is_some_and(char::is_whitespace)
                || previous.is_some_and(|character| matches!(character, '(' | '=')));
        if !path_start {
            redacted.push('/');
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < characters.len() {
            let character = characters[end];
            let finished = active_quote.map_or_else(
                || {
                    character.is_whitespace()
                        || matches!(character, '"' | '\'' | ',' | ';' | ')' | ']' | '}')
                },
                |quote| character == quote || matches!(character, ',' | ';' | ')' | ']' | '}'),
            );
            if finished {
                break;
            }
            end += 1;
        }
        redacted.push_str("[path]");
        index = end;
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_tools_selections_are_single_use_opaque_and_replace_prior_state() {
        let mut store = PlatformToolsSelectionStore::default();
        let first_path = PathBuf::from("/private/first.zip");
        let first = store.replace(first_path);
        assert!(first.starts_with("platform_tools_selection_"));
        assert!(!first.contains("private"));

        let second_path = PathBuf::from("/private/second.zip");
        let second = store.replace(second_path.clone());
        assert!(store.take(&first).is_err());
        assert_eq!(store.take(&second).unwrap(), second_path);
        assert!(store.take(&second).is_err());

        let serialized = serde_json::to_string(&PlatformToolsPickerResult::Selected {
            selection_handle: second,
        })
        .unwrap();
        assert!(serialized.contains("selectionHandle"));
        assert!(!serialized.contains("/private"));
    }

    #[test]
    fn platform_tools_picker_cancellation_is_a_successful_empty_selection() {
        let selected = tauri::async_runtime::block_on(await_picker_selection::<FilePath, _>(
            |complete| complete(None),
            "platform_tools_picker_failed",
            "The Platform-Tools ZIP picker could not be opened.",
        ))
        .expect("picker cancellation should resolve successfully");
        assert!(selected.is_none());
    }

    #[test]
    fn single_file_picker_cancellation_is_a_successful_empty_selection() {
        let selected = tauri::async_runtime::block_on(await_picker_selection::<FilePath, _>(
            |complete| complete(None),
            "input_path_picker_failed",
            "The input file picker could not be opened.",
        ))
        .expect("single-file cancellation should resolve successfully");
        assert!(selected.is_none());
    }

    #[test]
    fn multiple_file_picker_cancellation_is_a_successful_empty_selection() {
        let selected = tauri::async_runtime::block_on(await_picker_selection::<Vec<FilePath>, _>(
            |complete| complete(None),
            "input_path_picker_failed",
            "The input file picker could not be opened.",
        ))
        .expect("multi-file cancellation should resolve successfully");
        assert!(selected.is_none());
    }

    #[test]
    fn folder_picker_cancellation_is_a_successful_empty_selection() {
        let selected = tauri::async_runtime::block_on(await_picker_selection::<FilePath, _>(
            |complete| complete(None),
            "input_path_picker_failed",
            "The input folder picker could not be opened.",
        ))
        .expect("folder cancellation should resolve successfully");
        assert!(selected.is_none());
    }

    #[test]
    fn picker_success_callback_returns_its_selection() {
        let selected = tauri::async_runtime::block_on(await_picker_selection(
            |complete| complete(Some("selected".to_string())),
            "input_path_picker_failed",
            "The input file picker could not be opened.",
        ))
        .expect("a completed picker should resolve successfully");
        assert_eq!(selected.as_deref(), Some("selected"));
    }

    #[test]
    fn dropped_picker_callback_returns_an_actionable_error() {
        let error = tauri::async_runtime::block_on(await_picker_selection::<FilePath, _>(
            drop,
            "input_path_picker_failed",
            "The input file picker could not be opened.",
        ))
        .expect_err("a picker that cannot open must resolve as an error");
        assert!(error.contains("input_path_picker_failed"));
        assert!(error.contains("The input file picker could not be opened."));
    }

    #[test]
    fn dropped_platform_tools_picker_callback_returns_an_actionable_error() {
        let error = tauri::async_runtime::block_on(await_picker_selection::<FilePath, _>(
            drop,
            "platform_tools_picker_failed",
            "The Platform-Tools ZIP picker could not be opened.",
        ))
        .expect_err("a picker that cannot open must resolve as an error");
        assert!(error.contains("platform_tools_picker_failed"));
    }

    #[test]
    fn blocking_import_task_propagates_import_errors() {
        let error = tauri::async_runtime::block_on(run_import_task(|| {
            Err::<(), _>("expected import failure".to_string())
        }))
        .expect_err("an import error must settle the blocking task");
        assert_eq!(error, "expected import failure");
    }

    #[test]
    fn blocking_import_task_converts_worker_panics_to_safe_errors() {
        let error = tauri::async_runtime::block_on(run_import_task(|| -> Result<(), String> {
            panic!("private worker panic details");
        }))
        .expect_err("a worker panic must settle the blocking task");
        assert!(error.contains("platform_tools_import_worker_failed"));
        assert!(!error.contains("private worker panic details"));
    }

    #[test]
    fn configuration_payload_maps_probe_fields_and_retains_trusted_target_binding() {
        let payload = configuration_payload_from_parts(
            json!({
                "root": "/private/catalog",
                "sourceKind": "bundled",
                "sourceId": "emuchef.phase1.bundled",
                "version": "phase1-bundled-1",
                "contentDigest": { "algorithm": "sha256", "value": "a".repeat(64) },
            }),
            "ayaneo.pocket_s_mini.base",
            None,
            HashMap::new(),
            "trusted-exact-serial",
            &json!({
                "manufacturer": "AYANEO",
                "model": "Pocket S mini",
                "android_version": 13,
                "android_api_level": 33,
            }),
        );

        assert_eq!(payload["selectedRecipes"], Value::Null);
        assert_eq!(payload["deviceContext"]["androidVersion"], 13);
        assert_eq!(payload["deviceContext"]["androidApiLevel"], 33);
        assert_eq!(payload["targetDevice"]["serial"], "trusted-exact-serial");
        assert_eq!(payload["targetDevice"]["androidApiLevel"], 33);
        assert_eq!(payload["catalog"]["contentDigest"]["algorithm"], "sha256");
        assert!(payload["deviceContext"].get("android_version").is_none());
        assert!(payload["targetDevice"].get("android_api_level").is_none());
    }

    #[test]
    fn configuration_sidecar_errors_map_to_stable_sanitized_react_errors() {
        let invalid = configuration_sidecar_error(
            r#"{"code":"invalid_request","message":"serial-123 at /Users/test/private/catalog/device_plans/plan.yaml"}"#,
            "serial-123",
        );
        assert!(invalid.contains("configuration_request_invalid"));
        assert!(invalid.contains("Reconnect the device"));
        assert!(!invalid.contains("serial-123"));
        assert!(!invalid.contains("/Users/test"));
        assert!(!invalid.contains("device_plans"));

        let load = configuration_sidecar_error(
            r#"{"code":"load_failed","message":"private catalog details"}"#,
            "serial-123",
        );
        assert!(load.contains("configuration_catalog_invalid"));
        assert!(!load.contains("private catalog details"));

        let unknown = configuration_sidecar_error("raw transport failure", "serial-123");
        assert!(unknown.contains("configuration_description_failed"));
        assert!(!unknown.contains("raw transport failure"));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_sidecar_diagnostic_retains_context_but_redacts_serials_and_paths() {
        let raw = r#"{"code":"load_failed","message":"serial-123 failed at /Users/test/My Catalog/device_plans/plan.yaml; retry https://example.com/help"}"#;
        let redacted = redact_internal_sidecar_error(raw, "serial-123");
        assert!(redacted.contains("load_failed"));
        assert!(redacted.contains("failed at"));
        assert!(redacted.contains("[device]"));
        assert!(redacted.contains("[path]"));
        assert!(redacted.contains("https://example.com/help"));
        assert!(!redacted.contains("serial-123"));
        assert!(!redacted.contains("/Users/test"));
        assert!(!redacted.contains("device_plans"));
    }

    #[test]
    fn phase_one_real_catalog_description_preserves_trusted_data_and_projects_privately() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../authored")
            .canonicalize()
            .expect("real authored catalog should exist");
        let digest = crate::catalog::verify_and_digest(&root)
            .expect("Tauri should verify the real authored catalog");
        assert_eq!(
            digest,
            emuchef_rust_backend::catalog_source::compute_catalog_sha256(&root)
                .expect("sidecar should hash the real authored catalog")
        );
        let exact_serial = "trusted-integration-serial";
        let describe = |selected_recipes: Value, request_id: &str| {
            let request = json!({
                "id": request_id,
                "type": "describeConfiguration",
                "payload": {
                    "catalog": {
                        "root": root,
                        "sourceKind": "bundled",
                        "sourceId": "emuchef.phase1.bundled",
                        "version": "phase1-bundled-1",
                        "contentDigest": { "algorithm": "sha256", "value": digest },
                    },
                    "devicePlan": "ayaneo.pocket_s_mini.base",
                    "selectedRecipes": selected_recipes,
                    "bindings": {},
                    "deviceContext": {
                        "manufacturer": "AYANEO",
                        "model": "Pocket S mini",
                        "androidVersion": 13,
                        "androidApiLevel": 33,
                    },
                    "targetDevice": {
                        "serial": exact_serial,
                        "manufacturer": "AYANEO",
                        "model": "Pocket S mini",
                        "androidApiLevel": 33,
                    },
                },
            });
            let output = emuchef_rust_backend::jsonl::process_jsonl(&format!("{request}\n"));
            serde_json::from_str::<Value>(output.trim())
                .expect("sidecar description should be valid JSON")
        };

        let envelope = describe(Value::Null, "phase-one-defaults");
        assert_eq!(envelope["ok"], true, "{envelope:#}");
        let internal = &envelope["result"];
        assert_eq!(internal["targetDevice"]["serial"], exact_serial);
        assert_eq!(internal["targetDevice"]["androidApiLevel"], 33);
        assert_eq!(
            internal["selectedRecipes"],
            json!(["app.retroarch.provision"])
        );
        assert!(internal["recipeOptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|recipe| recipe["id"] == "app.retroarch.provision"));
        assert!(internal["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|input| input["key"] == "app.retroarch.provision/retroarch_cfg"));

        let public = public_configuration_description(internal, exact_serial);
        let public_serialized = public.to_string();
        assert!(!public_serialized.contains(exact_serial));
        assert!(!public_serialized.contains(root.to_string_lossy().as_ref()));
        assert!(!public_serialized.contains("targetDevice"));
        assert!(!public_serialized.contains("catalog"));
        assert!(public["recipeOptions"].as_array().unwrap().len() > 1);
        assert!(!public["inputs"].as_array().unwrap().is_empty());

        let missing = describe(
            json!(["app.retroarch.provision", "feature.copy_roms"]),
            "phase-one-missing-input",
        );
        assert_eq!(missing["ok"], true, "{missing:#}");
        let missing_public = public_configuration_description(&missing["result"], exact_serial);
        let source = missing_public["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["key"] == "feature.copy_roms/source")
            .expect("missing required input should remain visible");
        assert!(source["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "binding_missing"));
        assert!(!missing_public.to_string().contains(exact_serial));
    }

    #[test]
    fn react_device_projection_never_contains_serial() {
        let public = public_device_facts(
            "device_opaque",
            &json!({
                "serial": "exact-sensitive-serial",
                "manufacturer": "AYANEO",
                "model": "Pocket",
                "android_version": 13,
                "android_api_level": 33,
            }),
            "exact-sensitive-serial",
        );
        let serialized = public.to_string();
        assert!(!serialized.contains("exact-sensitive-serial"));
        assert!(!serialized.contains("serial"));
    }

    #[test]
    fn review_projection_omits_exact_target_serial_and_full_plan() {
        let plan = json!({
            "target_device": { "serial": "secret", "manufacturer": "AYANEO", "model": "Pocket", "android_api_level": 33 },
            "device_context": { "android_version": 13 },
            "recipes": [{ "id": "recipe.one", "name": "Recipe", "description": null }],
            "steps": [{ "id": "step.one", "recipe_ref": "recipe.one", "type": "wait", "name": "Wait", "note": "Pause", "constraints": { "capabilities": [] } }],
            "inputs": [{ "value": "private-input" }]
        });
        let public = public_review(
            "review_opaque",
            "digest",
            &plan,
            &json!({
                "diagnostics": [{ "severity": "warning", "code": "warning", "message": "Do not show secret" }],
                "resolvedInputs": [
                    { "key": "recipe/path", "inputId": "path", "value": "/tmp/secret/file", "source": "explicit" },
                    { "key": "recipe/serial", "inputId": "serial", "value": "secret", "source": "explicit" }
                ]
            }),
        );
        let serialized = public.to_string();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("private-input"));
        assert!(serialized.contains("review_opaque"));
        assert!(serialized.contains("[device]"));
        assert!(!serialized.contains("recipe/serial"));
    }

    #[test]
    fn configuration_projection_flattens_picker_metadata_and_drops_internal_fields() {
        let public = public_configuration_description(
            &json!({
                "catalog": { "root": "/private/catalog" },
                "devicePlan": "plan",
                "selectedRecipes": ["recipe"],
                "expandedRecipes": ["recipe"],
                "recipeOptions": [{
                    "id": "recipe", "name": "Recipe", "description": null,
                    "selected": true, "recommended": true, "dependencyRequired": false,
                    "available": true, "unavailableCapabilities": [], "internal": "secret-serial"
                }],
                "inputs": [{
                    "key": "recipe/file", "recipeId": "recipe", "inputId": "file",
                    "type": "file", "label": "File", "description": null, "required": true,
                    "multiple": false, "options": [{ "value": "one", "label": "One" }],
                    "validation": { "pathKind": "file", "allowedExtensions": ["apk"] },
                    "value": null, "valueSource": null,
                    "diagnostics": [{
                        "key": "recipe/file", "code": "binding_missing",
                        "message": "A file is required.", "severity": "error",
                        "internal": "drop-me"
                    }],
                    "internalPath": "/private/input"
                }],
                "diagnostics": [{
                    "key": "recipe/file", "code": "binding_missing",
                    "message": "A file is required.", "severity": "error",
                    "internal": "drop-me"
                }],
                "targetDevice": {
                    "serial": "secret-serial",
                    "manufacturer": "AYANEO",
                    "model": "Pocket S mini",
                    "androidApiLevel": 33
                }
            }),
            "secret-serial",
        );
        assert_eq!(public["inputs"][0]["pathKind"], "file");
        assert_eq!(public["inputs"][0]["acceptedExtensions"], json!(["apk"]));
        assert_eq!(public["inputs"][0]["options"], json!(["one"]));
        assert_eq!(public["inputs"][0]["diagnostics"][0]["key"], "recipe/file");
        assert_eq!(public["diagnostics"][0]["key"], "recipe/file");
        let serialized = public.to_string();
        assert!(!serialized.contains("/private"));
        assert!(!serialized.contains("secret-serial"));
        assert!(!serialized.contains("targetDevice"));
        assert!(!serialized.contains("internal"));
    }

    #[test]
    fn match_projection_drops_internal_fields_and_redacts_serial_text() {
        let public = public_match(
            &json!({
                "confidence": "high", "recommendedPlanId": "plan", "requiresExplicitChoice": false,
                "candidates": [{
                    "planId": "plan", "name": "For exact-serial", "description": null,
                    "profileId": "profile", "profileName": "Profile", "confidence": "high",
                    "reasons": ["Matched exact-serial"], "internalRoot": "/private/catalog"
                }],
                "safeGenericPlans": [], "blocked": false, "blockReason": null,
                "serial": "exact-serial"
            }),
            Some("exact-serial"),
        );
        let serialized = public.to_string();
        assert!(!serialized.contains("exact-serial"));
        assert!(!serialized.contains("internalRoot"));
        assert!(!serialized.contains("/private"));
    }
}
