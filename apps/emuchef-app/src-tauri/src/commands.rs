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
use tauri_plugin_dialog::DialogExt;
#[cfg(test)]
use tauri_plugin_dialog::FilePath;
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::adb::{AdbManager, AdbSetupStatusDto, PLATFORM_TOOLS_URL};
use crate::catalog::CatalogDescriptor;
use crate::device_qualification::{reconcile_inventory_with_context, RootQualificationStore};
use crate::device_qualification::{
    RootQualificationInvalidation, RootQualificationKey, RootQualificationState,
};
use crate::execution::ExecutionHandleStore;
use crate::handles::{DeviceDto, ReviewedPlanSnapshot, SessionHandles};
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
    pub input_contracts: Mutex<InputContractSnapshot>,
    pub handles: Mutex<SessionHandles>,
    pub root_qualification: Mutex<RootQualificationStore>,
    pub executions: Mutex<ExecutionHandleStore>,
    pub saved_configurations: SavedConfigurationState,
    pub recovery: RecoveryState,
    pub support: Mutex<SupportStore>,
    pub updates: UpdateService,
    pub update_activity: ActivityGate,
}

/// Request one fresh ADB inventory and pass it through the shared native
/// continuity reconciliation before any caller resolves a device target.
pub(crate) fn list_and_reconcile_inventory<F>(
    state: &AppState,
    request: &mut F,
) -> Result<Vec<DeviceDto>, String>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let adb_path = current_adb_path(state)?;
    let runtime_generation = state.sidecar.try_generation().map_err(|_| {
        safe_error(
            "runtime_generation_unavailable",
            "Device qualification state is temporarily unavailable.",
        )
    })?;
    let platform_tools_revision = state
        .adb
        .lock()
        .map_err(|_| {
            safe_error(
                "adb_state_unavailable",
                "Platform-Tools setup state is unavailable.",
            )
        })?
        .revision();
    list_and_reconcile_inventory_with_authority(
        &state.handles,
        &state.root_qualification,
        &adb_path,
        runtime_generation,
        platform_tools_revision,
        request,
    )
}

/// Request and reconcile one inventory using explicit native authority inputs.
/// Polling, qualification, and final execution all use this same path.
pub(crate) fn list_and_reconcile_inventory_with_authority<F>(
    handles: &Mutex<SessionHandles>,
    root_qualification: &Mutex<RootQualificationStore>,
    adb_path: &str,
    runtime_generation: u64,
    platform_tools_revision: u64,
    request: &mut F,
) -> Result<Vec<DeviceDto>, String>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let inventory = request("listAdbDevices", json!({ "adbPath": adb_path })).map_err(|_| {
        safe_error(
            "adb_inventory_failed",
            "Connected Android devices could not be listed.",
        )
    })?;
    reconcile_inventory_with_context(
        handles,
        root_qualification,
        &inventory,
        runtime_generation,
        platform_tools_revision,
    )
}

/// Latest backend-authored input contract set accepted for the current runtime
/// session. The frontend request sequence rejects out-of-order descriptions;
/// this snapshot gives native pickers the same contract without trusting React
/// to restate path kind, multiplicity, or accepted extensions.
#[derive(Clone, Debug, Default)]
pub struct InputContractSnapshot {
    request_generation: u64,
    contracts: HashMap<String, InputContract>,
}

#[derive(Clone, Debug)]
struct InputContract {
    type_name: String,
    path_kind: String,
    multiple: bool,
    allowed_extensions: Vec<String>,
}

impl InputContractSnapshot {
    fn replace(&mut self, request_generation: u64, description: &Value) -> bool {
        if request_generation < self.request_generation {
            return false;
        }
        let contracts = description
            .get("inputs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|input| {
                let key = input.get("key")?.as_str()?.to_string();
                let type_name = input.get("type")?.as_str()?.to_string();
                if !matches!(
                    type_name.as_str(),
                    "file" | "directory" | "path" | "path_list"
                ) {
                    return None;
                }
                let validation = input.get("validation")?;
                let path_kind = validation.get("pathKind")?.as_str()?.to_string();
                let allowed_extensions = validation
                    .get("allowedExtensions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                Some((
                    key,
                    InputContract {
                        type_name,
                        path_kind,
                        multiple: input
                            .get("multiple")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        allowed_extensions,
                    },
                ))
            })
            .collect();
        self.request_generation = request_generation;
        self.contracts = contracts;
        true
    }

    fn require(&self, request_generation: u64, key: &str) -> Result<InputContract, String> {
        self.require_generation(request_generation)?;
        self.contracts.get(key).cloned().ok_or_else(|| {
            safe_error(
                "input_contract_unavailable",
                "This input is no longer part of the selected setup.",
            )
        })
    }

    fn require_generation(&self, request_generation: u64) -> Result<(), String> {
        if request_generation == self.request_generation {
            Ok(())
        } else {
            Err(safe_error(
                "input_request_stale",
                "The input requirements changed. Wait for validation, then try again.",
            ))
        }
    }

    fn clear(&mut self) {
        self.request_generation = 0;
        self.contracts.clear();
    }
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
pub fn get_runtime_status(state: State<'_, AppState>) -> Value {
    public_runtime_status(state.sidecar.status())
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
pub fn restart_runtime(
    expected_generation: Option<u64>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if expected_generation.is_some_and(|expected| expected != state.sidecar.generation()) {
        return Err(safe_error(
            "runtime_generation_stale",
            "App service status changed. Review troubleshooting status before retrying.",
        ));
    }
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
            "The app service cannot restart while an execution is starting or active.",
        ));
    }
    state.sidecar.initialize();
    reset_app_session(&state, false)?;
    Ok(public_runtime_status(state.sidecar.status()))
}

fn public_runtime_status(status: RuntimeStatusDto) -> Value {
    match status {
        RuntimeStatusDto::Starting => json!({ "status": "starting" }),
        RuntimeStatusDto::Ready {
            protocol_version,
            catalog_version,
        } => json!({
            "status": "ready",
            "protocolVersion": protocol_version,
            "catalogVersion": catalog_version,
        }),
        RuntimeStatusDto::Unsupported { .. } => json!({
            "status": "unsupported",
            "error": { "message": "This EmuChef build cannot run its local app service on this system." },
        }),
        RuntimeStatusDto::Failed { .. } => json!({
            "status": "failed",
            "error": { "message": "The local app service could not start. Open Troubleshooting for safe recovery options." },
        }),
    }
}

fn reset_app_session(state: &AppState, close_documents: bool) -> Result<(), String> {
    state
        .root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .invalidate();
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
    state
        .input_contracts
        .lock()
        .map_err(|_| {
            safe_error(
                "input_state_unavailable",
                "Input requirements are unavailable.",
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

/// Stale only reviews that were planned with root evidence removed by the
/// current device qualification context. The invalidation handle is opaque;
/// no serial lookup is needed for this native-to-native transition.
fn invalidate_reviews_for_root_invalidation(
    handles: &mut SessionHandles,
    invalidation: &RootQualificationInvalidation,
) {
    if let Some(device_handle) = invalidation.device_handle.as_deref() {
        handles.invalidate_reviews_for_device(device_handle, "root_qualification_changed");
    }
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
pub fn get_adb_setup_status(state: State<'_, AppState>) -> Result<Value, String> {
    let status = state
        .adb
        .lock()
        .map_err(|_| {
            safe_error(
                "adb_state_unavailable",
                "Platform-Tools setup state is unavailable.",
            )
        })?
        .status();
    Ok(public_adb_status(&status))
}

fn public_adb_status(status: &AdbSetupStatusDto) -> Value {
    json!({
        "status": status.status,
        "version": status.version,
        "warning": status.warning,
        "error": (status.status == "invalid").then(|| json!({
            "message": "The managed Platform-Tools installation did not pass validation. Open Troubleshooting to repair it."
        })),
        "canImport": status.can_import,
        "canReplace": status.can_replace,
        "canRemove": status.can_remove,
    })
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
    expected_revision: Option<u64>,
    app: AppHandle,
) -> Result<Value, String> {
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
        if expected_revision.is_some_and(|expected| expected != adb.revision()) {
            return Err(safe_error(
                "platform_tools_revision_stale",
                "Platform-Tools status changed. Review troubleshooting status before retrying.",
            ));
        }
        let result = adb.import_zip(&path);
        drop(adb);
        result
    })
    .await?;

    let state = app.state::<AppState>();
    state
        .root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .invalidate();
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
    Ok(public_adb_status(&result))
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
pub fn remove_platform_tools(
    expected_revision: Option<u64>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
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
    let mut adb = state.adb.lock().map_err(|_| {
        safe_error(
            "adb_state_unavailable",
            "Platform-Tools setup state is unavailable.",
        )
    })?;
    if expected_revision.is_some_and(|expected| expected != adb.revision()) {
        return Err(safe_error(
            "platform_tools_revision_stale",
            "Platform-Tools status changed. Review troubleshooting status before retrying.",
        ));
    }
    if !adb.is_app_managed() {
        return Err(safe_error(
            "platform_tools_not_managed",
            "Only an app-managed Platform-Tools installation can be removed here.",
        ));
    }
    let result = adb.remove()?;
    drop(adb);
    state
        .root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .invalidate();
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
    Ok(public_adb_status(&result))
}

#[tauri::command]
pub fn poll_devices(
    expected_generation: Option<u64>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if expected_generation.is_some_and(|expected| {
        state
            .handles
            .lock()
            .map(|handles| handles.device_generation() != expected)
            .unwrap_or(true)
    }) {
        return Err(safe_error(
            "device_generation_stale",
            "Device status changed. Refresh troubleshooting status before retrying.",
        ));
    }
    let mut request =
        |request_type: &str, payload: Value| state.sidecar.request(request_type, payload);
    let devices = list_and_reconcile_inventory(&state, &mut request)?;
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
    request_generation: u64,
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
    let accepted = state
        .input_contracts
        .lock()
        .map_err(|_| {
            safe_error(
                "input_state_unavailable",
                "Input requirements are unavailable.",
            )
        })?
        .replace(request_generation, &result);
    let required_reentry = {
        let mut recovery = state.recovery.lock().map_err(|_| {
            safe_error(
                "recovery_state_unavailable",
                "Recovery state is temporarily unavailable.",
            )
        })?;
        if accepted {
            recovery.record_schema(&result);
            recovery.note_current_binding_keys(&binding_keys);
        }
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
    request_generation: u64,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    state
        .input_contracts
        .lock()
        .map_err(|_| {
            safe_error(
                "input_state_unavailable",
                "Input requirements are unavailable.",
            )
        })?
        .require_generation(request_generation)?;
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
    let review = result
        .get("review")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| {
            safe_error(
                "review_projection_missing",
                "The reviewed plan omitted its safe presentation.",
            )
        })?;
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
        device_handle: device_handle.clone(),
        qualification_context: state
            .handles
            .lock()
            .map_err(|_| {
                safe_error(
                    "session_state_unavailable",
                    "Review session state is unavailable.",
                )
            })?
            .qualification_context(&device_handle),
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
    let exact_serial = plan
        .pointer("/target_device/serial")
        .and_then(Value::as_str);
    Ok(public_review(&review_handle, &review, exact_serial))
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
/// Cancellation is represented as `Ok(None)`. Picker policy comes only from the
/// latest backend-authored input contract; React supplies portable intent and an
/// edit operation but cannot choose path kind, multiplicity, or file filters.
pub async fn pick_input_path(
    app: AppHandle,
    input_key: String,
    request_generation: u64,
    mode: String,
    current_value: Value,
    entry_index: Option<usize>,
) -> Result<Option<Value>, String> {
    let contract = app
        .state::<AppState>()
        .input_contracts
        .lock()
        .map_err(|_| {
            safe_error(
                "input_state_unavailable",
                "Input requirements are unavailable.",
            )
        })?
        .require(request_generation, &input_key)?;
    if !matches!(mode.as_str(), "replace_all" | "append" | "replace_entry") {
        return Err(safe_error(
            "input_selection_mode_invalid",
            "The requested input edit is unsupported.",
        ));
    }
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let mut picker = app.dialog().file();
    if contract.path_kind == "file" && !contract.allowed_extensions.is_empty() {
        let extensions = contract
            .allowed_extensions
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        picker = picker.add_filter("Supported files", &extensions);
    }
    let choose_many = contract.multiple && mode != "replace_entry";
    let selected = match (contract.path_kind.as_str(), choose_many) {
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
    let Some(selected) = selected else {
        return Ok(None);
    };
    let paths = selected
        .into_iter()
        .map(|path| {
            path.into_path().map_err(|_| {
                safe_error(
                    "input_path_unavailable",
                    "The selected item could not be opened by the application.",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let value = compose_input_selection(&contract, &mode, current_value, entry_index, paths)?;
    validate_input_selection(&contract, &value)?;
    Ok(Some(value))
}

fn compose_input_selection(
    contract: &InputContract,
    mode: &str,
    current_value: Value,
    entry_index: Option<usize>,
    selected: Vec<PathBuf>,
) -> Result<Value, String> {
    let selected = selected
        .into_iter()
        .map(|path| Value::String(path.to_string_lossy().into_owned()))
        .collect::<Vec<_>>();
    if !contract.multiple {
        if mode != "replace_all" || selected.len() != 1 {
            return Err(safe_error(
                "input_selection_invalid",
                "Choose one item for this input.",
            ));
        }
        return Ok(selected
            .into_iter()
            .next()
            .expect("one selection was required"));
    }
    let mut values = match mode {
        "replace_all" => Vec::new(),
        "append" | "replace_entry" => current_value.as_array().cloned().unwrap_or_default(),
        _ => unreachable!("selection mode was checked above"),
    };
    match mode {
        "replace_all" | "append" => values.extend(selected),
        "replace_entry" => {
            let index = entry_index
                .filter(|index| *index < values.len())
                .ok_or_else(|| {
                    safe_error(
                        "input_entry_stale",
                        "That selected entry changed. Wait for validation, then try again.",
                    )
                })?;
            if selected.len() != 1 {
                return Err(safe_error(
                    "input_selection_invalid",
                    "Choose one replacement item.",
                ));
            }
            values[index] = selected
                .into_iter()
                .next()
                .expect("one selection was required");
        }
        _ => unreachable!("selection mode was checked above"),
    }
    Ok(Value::Array(values))
}

fn validate_input_selection(contract: &InputContract, value: &Value) -> Result<(), String> {
    let values = if contract.multiple {
        value.as_array().cloned().ok_or_else(|| {
            safe_error(
                "input_selection_invalid",
                "Choose files again for this input.",
            )
        })?
    } else {
        vec![value.clone()]
    };
    if values.is_empty() {
        return Err(safe_error(
            "input_selection_invalid",
            "Choose at least one item for this input.",
        ));
    }
    let mut identities = HashSet::new();
    for value in values {
        let path = value.as_str().map(PathBuf::from).ok_or_else(|| {
            safe_error(
                "input_selection_invalid",
                "Choose files again for this input.",
            )
        })?;
        let readable = match contract.path_kind.as_str() {
            "file" => path.is_file() && std::fs::File::open(&path).is_ok(),
            "directory" => path.is_dir() && std::fs::read_dir(&path).is_ok(),
            _ => false,
        };
        if !readable {
            return Err(safe_error(
                "input_path_inaccessible",
                "The selected item is missing or cannot be read. Choose it again.",
            ));
        }
        if contract.path_kind == "file"
            && !contract.allowed_extensions.is_empty()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| {
                    !contract
                        .allowed_extensions
                        .iter()
                        .any(|allowed| allowed.eq_ignore_ascii_case(extension))
                })
        {
            return Err(safe_error(
                "input_extension_unsupported",
                "The selected file type is unsupported for this input.",
            ));
        }
        let identity = std::fs::canonicalize(&path).map_err(|_| {
            safe_error(
                "input_path_inaccessible",
                "The selected item is missing or cannot be read. Choose it again.",
            )
        })?;
        if !identities.insert(identity) {
            return Err(safe_error(
                "input_duplicate_path",
                "The same item was selected more than once. Remove the duplicate and try again.",
            ));
        }
    }
    let _ = &contract.type_name;
    Ok(())
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
    let (serial, facts) = {
        let handles = state.handles.lock().map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "Device session state is unavailable.",
            )
        })?;
        let device = handles.device(device_handle)?;
        (device.serial.clone(), handles.facts(device_handle)?.clone())
    };
    let mut payload = configuration_payload_from_parts(
        catalog(state)?.internal_payload(),
        device_plan,
        selected_recipes,
        bindings,
        &serial,
        &facts,
    );
    let qualification_context = state
        .handles
        .lock()
        .map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "Device session state is unavailable.",
            )
        })?
        .qualification_context(device_handle);
    let root_granted = state
        .root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .get(
            &qualification_context
                .as_ref()
                .map(RootQualificationKey::from_context)
                .unwrap_or_else(|| RootQualificationKey::new(device_handle, 0, 0)),
        )
        .is_some_and(|result| result == RootQualificationState::Granted);
    payload["runtimeCapabilityAvailability"] = json!({
        "rootShell": root_granted,
        "appDataWrite": root_granted,
    });
    Ok(payload)
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
                    "selectionMode": candidate.get("selectionMode"),
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
        "blankSetupPlans": candidates("blankSetupPlans"),
        "blocked": result.get("blocked"),
        "blockReason": result.get("blockReason"),
    });
    if let Some(serial) = exact_serial.filter(|serial| !serial.is_empty()) {
        redact_exact_serial(&mut public, serial);
    }
    public
}

fn input_presentation_category(input: &Value) -> &'static str {
    match input
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "apk" => "Applications",
        "bios" => "Firmware",
        "rom" | "rom_library" | "content" => "Games and content",
        "rom_destination" => "Destination",
        "copy_policy" => "Options",
        _ => "Other",
    }
}

fn input_presentation_kind(input: &Value, options: &[Value]) -> &'static str {
    if input.get("type").and_then(Value::as_str) == Some("device_path") {
        return "Device folder";
    }
    let validation = input.get("validation").unwrap_or(&Value::Null);
    match validation.get("pathKind").and_then(Value::as_str) {
        Some("file")
            if input
                .get("multiple")
                .and_then(Value::as_bool)
                .unwrap_or(false) =>
        {
            "Multiple files"
        }
        Some("file") => "Single file",
        Some("directory") => "Folder",
        _ => match input
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "boolean" => "On or off",
            _ if !options.is_empty() => "Choose one",
            _ => "Text",
        },
    }
}

fn input_expected_description(input: &Value) -> String {
    if let Some(description) = input
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|description| !description.is_empty())
    {
        return description.to_string();
    }
    let role = input
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let type_name = input
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let path_kind = input
        .pointer("/validation/pathKind")
        .and_then(Value::as_str);
    let multiple = input
        .get("multiple")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let host_shape = match path_kind {
        Some("file") => Some(if multiple {
            "multiple_files"
        } else {
            "single_file"
        }),
        Some("directory") => Some("directory"),
        _ => match type_name {
            "file" => Some(if multiple {
                "multiple_files"
            } else {
                "single_file"
            }),
            "directory" => Some("directory"),
            _ => None,
        },
    };

    match (role, host_shape) {
        ("apk", Some("single_file")) => {
            "Choose the Android application package required by this setup.".to_string()
        }
        ("apk", Some("multiple_files")) => {
            "Choose the Android application package files required by this setup.".to_string()
        }
        ("bios", Some("single_file")) => "Choose the BIOS file required by this setup.".to_string(),
        ("bios", Some("multiple_files")) => {
            "Choose the BIOS files required by this setup.".to_string()
        }
        ("bios", Some("directory")) => {
            "Choose the folder containing the BIOS files required by this setup.".to_string()
        }
        ("rom" | "rom_library" | "content", Some("single_file")) => {
            "Choose the game or content file required by this setup.".to_string()
        }
        ("rom" | "rom_library" | "content", Some("multiple_files")) => {
            "Choose the game or content files required by this setup.".to_string()
        }
        ("rom" | "rom_library" | "content", Some("directory")) => {
            "Choose the folder containing the game or content files required by this setup."
                .to_string()
        }
        ("rom_destination", _) => {
            "Enter the folder on the Android device where the content will be stored.".to_string()
        }
        ("copy_policy", _) => "Choose how files should be combined at the destination.".to_string(),
        (_, Some("single_file")) => "Choose the file used by this setup.".to_string(),
        (_, Some("multiple_files")) => "Choose the files used by this setup.".to_string(),
        (_, Some("directory")) => "Choose the folder used by this setup.".to_string(),
        _ => match type_name {
            "device_path" => "Enter a folder path on the connected Android device.".to_string(),
            "boolean" => "Choose whether this option should be enabled.".to_string(),
            _ => "Provide the value requested by this setup.".to_string(),
        },
    }
}

pub(crate) fn public_configuration_description(result: &Value, exact_serial: &str) -> Value {
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
                "recipeDependencies": recipe.get("recipeDependencies"),
                "contentRequirements": recipe.get("contentRequirements"),
                "requiredCapabilities": recipe.get("requiredCapabilities"),
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
            let presentation_category = input_presentation_category(input);
            let presentation_kind = input_presentation_kind(input, &options);
            let type_name = input
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let path_kind = matches!(type_name, "file" | "directory" | "path" | "path_list")
                .then(|| validation.get("pathKind").cloned().unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            let diagnostics = public_input_diagnostics(input);
            let entries = public_input_entries(input, &diagnostics);
            json!({
                "key": input.get("key"),
                "recipeId": input.get("recipeId"),
                "inputId": input.get("inputId"),
                "type": input.get("type"),
                "label": input.get("label"),
                "description": input_expected_description(input),
                "required": input.get("required"),
                "multiple": input.get("multiple"),
                "sensitive": input.get("sensitive"),
                "options": options,
                "pathKind": path_kind,
                "acceptedExtensions": validation.get("allowedExtensions"),
                "presentationCategory": presentation_category,
                "presentationKind": presentation_kind,
                "value": input.get("value"),
                "valueSource": input.get("valueSource"),
                "entries": entries,
                "diagnostics": diagnostics,
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

fn public_input_diagnostics(input: &Value) -> Vec<Value> {
    let label = input
        .get("label")
        .and_then(Value::as_str)
        .filter(|label| !label.trim().is_empty())
        .unwrap_or("This input");
    let required = input
        .get("required")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    input
        .get("diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|diagnostic| {
            let mut public = public_diagnostic(diagnostic);
            public["message"] = Value::String(match diagnostic
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "binding_missing" => format!("Select {label} before continuing."),
                "binding_path_missing" => if required {
                    format!("{label} could not be found. Select it again.")
                } else {
                    format!("{label} could not be found. Select it again or clear this optional input.")
                },
                "binding_path_kind_mismatch" => format!(
                    "{label} is not the expected file or folder type. Select it again."
                ),
                "binding_path_inaccessible" => format!(
                    "{label} cannot be read. Check access or select it again."
                ),
                "binding_extension_unsupported" => format!(
                    "{label} uses an unsupported file type. Choose one of the accepted file types."
                ),
                "binding_duplicate_path" => format!(
                    "{label} contains the same item more than once. Remove the duplicate."
                ),
                "binding_path_reused" => diagnostic
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("The same file is used by another input. Confirm that this is intentional.")
                    .to_string(),
                "recovery_sensitive_input_required" => format!(
                    "Re-enter {label}. This value was not stored."
                ),
                _ => format!("{label} does not meet its current requirements. Review and correct it."),
            });
            public
        })
        .collect()
}

fn public_input_entries(input: &Value, diagnostics: &[Value]) -> Vec<Value> {
    let values = match input.get("value") {
        Some(Value::String(value)) => vec![value.as_str()],
        Some(Value::Array(values)) => values.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    values
        .into_iter()
        .enumerate()
        .map(|(index, path)| {
            let entry_diagnostics = diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.get("entryIndex").and_then(Value::as_u64) == Some(index as u64)
                })
                .cloned()
                .collect::<Vec<_>>();
            let state = if entry_diagnostics.iter().any(|diagnostic| {
                diagnostic.get("severity").and_then(Value::as_str) == Some("error")
            }) {
                "error"
            } else if entry_diagnostics.is_empty() {
                "valid"
            } else {
                "warning"
            };
            json!({
                "index": index,
                "displayName": PathBuf::from(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Selected item"),
                "displayPath": path,
                "state": state,
                "diagnostics": entry_diagnostics,
            })
        })
        .collect()
}

fn public_diagnostics(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(public_diagnostic)
        .collect()
}

fn public_diagnostic(diagnostic: &Value) -> Value {
    let message = diagnostic
        .get("message")
        .and_then(Value::as_str)
        .map(redact_absolute_paths);
    json!({
        "key": diagnostic.get("key"),
        "code": diagnostic.get("code"),
        "message": message,
        "severity": diagnostic.get("severity"),
        "entryIndex": diagnostic.pointer("/details/entry_index"),
    })
}

/// Attaches only the opaque handle to the backend-authored review projection.
/// Tauri deliberately does not recreate action meaning from retained plan data.
fn public_review(review_handle: &str, review: &Value, exact_serial: Option<&str>) -> Value {
    let mut public = review.clone();
    if let Some(object) = public.as_object_mut() {
        object.insert(
            "reviewHandle".to_string(),
            Value::String(review_handle.to_string()),
        );
    }
    if let Some(serial) = exact_serial.filter(|serial| !serial.is_empty()) {
        redact_exact_serial(&mut public, serial);
    }
    public
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

    fn review_snapshot(device_handle: &str) -> ReviewedPlanSnapshot {
        ReviewedPlanSnapshot {
            response: json!({ "plan": { "id": "plan" } }),
            target: json!({ "deviceHandle": device_handle }),
            catalog_identity: json!({ "sourceId": "catalog" }),
            catalog_digest: "sha256:catalog".to_string(),
            plan_digest: "sha256:plan".to_string(),
            device_handle: device_handle.to_string(),
            qualification_context: None,
            platform_tools_identity: None,
            created: Instant::now(),
            last_access: Instant::now(),
        }
    }

    #[test]
    fn root_poll_invalidation_stales_only_reviews_for_removed_evidence() {
        let mut root_store = RootQualificationStore::default();
        let old_key = RootQualificationKey::new("device-a", 4, 9);
        let attempt = root_store.begin(old_key).unwrap();
        assert!(root_store.complete(attempt, RootQualificationState::Granted));

        let mut handles = SessionHandles::default();
        let removed = handles.insert_review(review_snapshot("device-a"));
        let retained = handles.insert_review(review_snapshot("device-b"));
        let invalidation =
            root_store.invalidate_if_not_key(Some(&RootQualificationKey::new("device-b", 4, 9)));
        invalidate_reviews_for_root_invalidation(&mut handles, &invalidation);

        assert!(handles
            .review(&removed)
            .unwrap_err()
            .contains("root_qualification_changed"));
        assert_eq!(handles.review(&retained).unwrap().device_handle, "device-b");
    }

    fn file_contract(multiple: bool) -> InputContract {
        InputContract {
            type_name: "file".to_string(),
            path_kind: "file".to_string(),
            multiple,
            allowed_extensions: vec!["txt".to_string()],
        }
    }

    #[test]
    fn input_contract_snapshot_replaces_current_generation_and_rejects_stale_requests() {
        let mut snapshot = InputContractSnapshot::default();
        snapshot.replace(
            4,
            &json!({
                "inputs": [{
                    "key": "recipe/files", "type": "file", "multiple": true,
                    "validation": { "pathKind": "file", "allowedExtensions": ["txt"] }
                }]
            }),
        );
        assert!(snapshot.require(4, "recipe/files").unwrap().multiple);
        snapshot.replace(3, &json!({ "inputs": [] }));
        assert!(snapshot.require(4, "recipe/files").is_ok());
        assert!(snapshot
            .require(3, "recipe/files")
            .unwrap_err()
            .contains("input_request_stale"));
    }

    #[test]
    fn multi_file_selection_composes_replacements_and_rejects_canonical_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.txt");
        let second = temp.path().join("second.txt");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        let contract = file_contract(true);
        let current = json!([first.to_string_lossy()]);
        let appended =
            compose_input_selection(&contract, "append", current, None, vec![second.clone()])
                .unwrap();
        validate_input_selection(&contract, &appended).unwrap();
        assert_eq!(appended.as_array().unwrap().len(), 2);

        let duplicate = compose_input_selection(
            &contract,
            "replace_all",
            Value::Null,
            None,
            vec![first.clone(), first],
        )
        .unwrap();
        assert!(validate_input_selection(&contract, &duplicate)
            .unwrap_err()
            .contains("input_duplicate_path"));

        let replaced =
            compose_input_selection(&contract, "replace_entry", appended, Some(0), vec![second])
                .unwrap();
        assert!(validate_input_selection(&contract, &replaced)
            .unwrap_err()
            .contains("input_duplicate_path"));
    }

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
        let public = public_review(
            "review_opaque",
            &json!({
                "setup": { "name": "Setup" },
                "target": { "label": "Connected device", "model": "secret" },
                "features": [],
                "inputs": [{ "label": "Credential", "summary": "Provided", "required": true }],
                "notices": [],
                "work": { "actionCount": 1 },
                "canExecute": true
            }),
            Some("secret"),
        );
        let serialized = public.to_string();
        assert!(!serialized.contains("secret"));
        assert!(serialized.contains("review_opaque"));
        assert!(serialized.contains("[device]"));
        assert!(!serialized.contains("planDigest"));
        assert!(!serialized.contains("recipeId"));
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
                    "recipeDependencies": ["dependency"],
                    "contentRequirements": ["bios_files"],
                    "requiredCapabilities": ["shared_storage_write"],
                    "available": true, "unavailableCapabilities": [], "internal": "secret-serial"
                }],
                "inputs": [{
                    "key": "recipe/file", "recipeId": "recipe", "inputId": "file",
                    "type": "file", "role": "bios", "label": "File", "description": null, "required": true,
                    "multiple": false, "options": [{ "value": "one", "label": "One" }],
                    "validation": { "pathKind": "file", "allowedExtensions": ["apk"] },
                    "sensitive": false,
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
        assert_eq!(public["inputs"][0]["presentationCategory"], "Firmware");
        assert_eq!(public["inputs"][0]["presentationKind"], "Single file");
        assert_eq!(public["inputs"][0]["options"], json!(["one"]));
        assert_eq!(
            public["recipeOptions"][0]["contentRequirements"],
            json!(["bios_files"])
        );
        assert_eq!(
            public["recipeOptions"][0]["recipeDependencies"],
            json!(["dependency"])
        );
        assert_eq!(public["inputs"][0]["diagnostics"][0]["key"], "recipe/file");
        assert_eq!(public["diagnostics"][0]["key"], "recipe/file");
        let serialized = public.to_string();
        assert!(!serialized.contains("/private"));
        assert!(!serialized.contains("secret-serial"));
        assert!(!serialized.contains("targetDevice"));
        assert!(!serialized.contains("internal"));
    }

    #[test]
    fn missing_host_paths_use_input_labels_and_actionable_repair_guidance() {
        let public = public_configuration_description(
            &json!({
                "devicePlan": "plan",
                "selectedRecipes": ["recipe"],
                "expandedRecipes": ["recipe"],
                "recipeOptions": [],
                "inputs": [
                    {
                        "key": "recipe/optional_cfg",
                        "recipeId": "recipe",
                        "inputId": "optional_cfg",
                        "type": "file",
                        "role": "content",
                        "label": "RetroArch config",
                        "description": null,
                        "required": false,
                        "multiple": false,
                        "options": [],
                        "validation": { "pathKind": "file", "allowedExtensions": ["cfg"] },
                        "sensitive": false,
                        "value": "/private/deleted.cfg",
                        "valueSource": "explicit",
                        "diagnostics": [{
                            "key": "recipe/optional_cfg",
                            "code": "binding_path_missing",
                            "message": "Input 'optional_cfg' must reference an existing file.",
                            "severity": "error",
                            "details": { "entry_index": 0 }
                        }]
                    },
                    {
                        "key": "recipe/required_dir",
                        "recipeId": "recipe",
                        "inputId": "required_dir",
                        "type": "directory",
                        "role": "bios",
                        "label": "BIOS folder",
                        "description": null,
                        "required": true,
                        "multiple": false,
                        "options": [],
                        "validation": { "pathKind": "directory", "allowedExtensions": [] },
                        "sensitive": false,
                        "value": "/private/deleted",
                        "valueSource": "explicit",
                        "diagnostics": [{
                            "key": "recipe/required_dir",
                            "code": "binding_path_missing",
                            "message": "Input 'required_dir' must reference an existing directory.",
                            "severity": "error",
                            "details": { "entry_index": 0 }
                        }]
                    }
                ],
                "diagnostics": [{
                    "key": null,
                    "code": "configuration_warning",
                    "message": "Could not inspect /private/deleted.cfg",
                    "severity": "warning"
                }]
            }),
            "",
        );

        assert_eq!(
            public["inputs"][0]["diagnostics"][0]["message"],
            "RetroArch config could not be found. Select it again or clear this optional input."
        );
        assert_eq!(
            public["inputs"][1]["diagnostics"][0]["message"],
            "BIOS folder could not be found. Select it again."
        );
        assert!(!public.to_string().contains("optional_cfg' must reference"));
        assert_eq!(public["inputs"][0]["entries"][0]["state"], "error");
        assert_eq!(
            public["inputs"][0]["entries"][0]["displayName"],
            "deleted.cfg"
        );
        assert_eq!(public["inputs"][0]["diagnostics"][0]["entryIndex"], 0);
        assert_eq!(
            public["diagnostics"][0]["message"],
            "Could not inspect [path]"
        );
    }

    #[test]
    fn fallback_input_descriptions_follow_authoritative_role_and_host_shape() {
        let cases = [
            ("bios-file", "bios", "file", false, "file"),
            ("bios-files", "bios", "file", true, "file"),
            ("bios-directory", "bios", "directory", false, "directory"),
            ("content-file", "content", "file", false, "file"),
            ("content-files", "content", "file", true, "file"),
            (
                "content-directory",
                "content",
                "directory",
                false,
                "directory",
            ),
            ("apk-directory", "apk", "directory", false, "directory"),
        ];
        let inputs = cases
            .into_iter()
            .map(|(key, role, type_name, multiple, path_kind)| {
                json!({
                    "key": key,
                    "recipeId": "recipe",
                    "inputId": key,
                    "type": type_name,
                    "role": role,
                    "label": "Authoritative label",
                    "description": null,
                    "required": true,
                    "multiple": multiple,
                    "options": [],
                    "validation": { "pathKind": path_kind, "allowedExtensions": [] },
                    "sensitive": false,
                    "value": null,
                    "valueSource": null,
                    "diagnostics": []
                })
            })
            .collect::<Vec<_>>();
        let public = public_configuration_description(
            &json!({
                "devicePlan": "plan",
                "selectedRecipes": ["recipe"],
                "expandedRecipes": ["recipe"],
                "recipeOptions": [],
                "inputs": inputs,
                "diagnostics": []
            }),
            "",
        );
        let descriptions = public["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| input["description"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            descriptions,
            vec![
                "Choose the BIOS file required by this setup.",
                "Choose the BIOS files required by this setup.",
                "Choose the folder containing the BIOS files required by this setup.",
                "Choose the game or content file required by this setup.",
                "Choose the game or content files required by this setup.",
                "Choose the folder containing the game or content files required by this setup.",
                "Choose the folder used by this setup.",
            ]
        );
    }

    #[test]
    fn authored_input_description_takes_precedence_over_shape_fallback() {
        let public = public_configuration_description(
            &json!({
                "devicePlan": "plan",
                "selectedRecipes": ["recipe"],
                "expandedRecipes": ["recipe"],
                "recipeOptions": [],
                "inputs": [{
                    "key": "recipe/content", "recipeId": "recipe", "inputId": "content",
                    "type": "file", "role": "content", "label": "Content",
                    "description": "  Choose the verified content bundle supplied by the publisher.  ",
                    "required": true, "multiple": true, "options": [],
                    "validation": { "pathKind": "file", "allowedExtensions": [] },
                    "sensitive": false, "value": null, "valueSource": null,
                    "diagnostics": []
                }],
                "diagnostics": []
            }),
            "",
        );
        assert_eq!(
            public["inputs"][0]["description"],
            "Choose the verified content bundle supplied by the publisher."
        );
    }

    #[test]
    fn device_paths_never_project_host_picker_authority() {
        let public = public_configuration_description(
            &json!({
                "devicePlan": "plan",
                "selectedRecipes": ["recipe"],
                "expandedRecipes": ["recipe"],
                "recipeOptions": [],
                "inputs": [{
                    "key": "recipe/destination", "recipeId": "recipe", "inputId": "destination",
                    "type": "device_path", "role": "rom_destination", "label": "Device ROM folder",
                    "description": null, "required": true, "multiple": false, "options": [],
                    "validation": { "pathKind": "directory", "allowedExtensions": [] },
                    "sensitive": false, "value": "/sdcard/ROMs", "valueSource": "recipe_default",
                    "diagnostics": []
                }],
                "diagnostics": []
            }),
            "",
        );
        assert!(public["inputs"][0]["pathKind"].is_null());
        assert_eq!(public["inputs"][0]["presentationKind"], "Device folder");
        assert!(public["inputs"][0]["description"]
            .as_str()
            .unwrap()
            .contains("Android device"));
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
