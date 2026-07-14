//! Trusted adaptation of retained reviews to simulated and guarded real executions.
//!
//! React supplies only opaque, session-scoped handles. This module owns target
//! and digest revalidation, selects the execution mode inside Tauri, and projects
//! sidecar reports into serial-free, path-safe DTOs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use uuid::Uuid;

use crate::adb::AdbRevalidationError;
use crate::commands::{
    catalog, current_adb_path, redact_absolute_paths, redact_exact_serial, safe_error, AppState,
};
use crate::handles::ReviewedPlanSnapshot;
use crate::sidecar::SidecarState;

/// One opaque app handle mapped to the sidecar execution that implements it.
#[derive(Clone, Debug)]
struct ExecutionMapping {
    kind: ExecutionKind,
    public_handle: String,
    sidecar_id: String,
    review_handle: String,
    review: ReviewedPlanSnapshot,
}

#[derive(Clone, Debug)]
struct LaunchActionRecord {
    action_handle: String,
    label: String,
    mapping: ExecutionMapping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionKind {
    Simulated,
    Real,
}

/// Bounded, restart-volatile execution handle state.
///
/// A start reservation prevents concurrent preflight races. At most one active
/// mapping and the latest terminal mapping are retained; terminal replacement
/// drops the older handle permanently.
#[derive(Default)]
pub struct ExecutionHandleStore {
    start_reserved: Option<ExecutionKind>,
    active: Option<ExecutionMapping>,
    latest_terminal: Option<ExecutionMapping>,
    launch_actions: HashMap<String, LaunchActionRecord>,
    successful_launches: HashSet<String>,
}

impl ExecutionHandleStore {
    fn reserve_start(&mut self, kind: ExecutionKind) -> Result<(), ()> {
        if self.start_reserved.is_some() || self.active.is_some() {
            return Err(());
        }
        self.start_reserved = Some(kind);
        Ok(())
    }

    fn release_start(&mut self) {
        self.start_reserved = None;
    }

    fn bind_started(
        &mut self,
        kind: ExecutionKind,
        sidecar_id: String,
        review_handle: String,
        review: ReviewedPlanSnapshot,
    ) -> ExecutionMapping {
        debug_assert_eq!(self.start_reserved, Some(kind));
        let mapping = ExecutionMapping {
            kind,
            public_handle: format!("execution_{}", Uuid::new_v4().simple()),
            sidecar_id,
            review_handle,
            review,
        };
        self.start_reserved = None;
        self.active = Some(mapping.clone());
        mapping
    }

    fn mapping(
        &self,
        kind: ExecutionKind,
        public_handle: &str,
        unavailable_message: &str,
    ) -> Result<ExecutionMapping, String> {
        self.active
            .as_ref()
            .filter(|mapping| mapping.kind == kind && mapping.public_handle == public_handle)
            .or_else(|| {
                self.latest_terminal.as_ref().filter(|mapping| {
                    mapping.kind == kind && mapping.public_handle == public_handle
                })
            })
            .cloned()
            .ok_or_else(|| safe_error("execution_unavailable", unavailable_message))
    }

    fn mapping_any(&self, public_handle: &str) -> Result<ExecutionMapping, String> {
        self.active
            .as_ref()
            .filter(|mapping| mapping.public_handle == public_handle)
            .or_else(|| {
                self.latest_terminal
                    .as_ref()
                    .filter(|mapping| mapping.public_handle == public_handle)
            })
            .cloned()
            .ok_or_else(|| {
                safe_error(
                    "execution_unavailable",
                    "This execution report is no longer available in this app session.",
                )
            })
    }

    fn mark_terminal(&mut self, kind: ExecutionKind, public_handle: &str) {
        if self
            .active
            .as_ref()
            .is_some_and(|mapping| mapping.kind == kind && mapping.public_handle == public_handle)
        {
            if let Some(previous) = self.latest_terminal.as_ref() {
                let previous_handle = previous.public_handle.clone();
                self.discard_launch_actions_for_execution(&previous_handle);
                self.successful_launches.remove(&previous_handle);
            }
            self.latest_terminal = self.active.take();
        }
    }

    fn launch_action(&mut self, mapping: &ExecutionMapping, report: &Value) -> Option<Value> {
        if self.successful_launches.contains(&mapping.public_handle) {
            return None;
        }
        if let Some(existing) = self
            .launch_actions
            .values()
            .find(|action| action.mapping.public_handle == mapping.public_handle)
        {
            return Some(json!({
                "handle": existing.action_handle,
                "label": existing.label,
            }));
        }
        let label = eligible_launch_label(mapping, report)?;
        let action_handle = format!("launch_{}", Uuid::new_v4().simple());
        self.launch_actions.insert(
            action_handle.clone(),
            LaunchActionRecord {
                action_handle: action_handle.clone(),
                label: label.clone(),
                mapping: mapping.clone(),
            },
        );
        Some(json!({ "handle": action_handle, "label": label }))
    }

    /// Atomically remove one opaque action before any external revalidation or ADB work.
    fn consume_launch_action(&mut self, action_handle: &str) -> Result<LaunchActionRecord, String> {
        self.launch_actions.remove(action_handle).ok_or_else(|| {
            safe_error(
                "launch_unavailable",
                "This launch action is unavailable. Refresh the completed execution before trying again.",
            )
        })
    }

    fn mark_launch_succeeded(&mut self, public_handle: &str) {
        self.successful_launches.insert(public_handle.to_string());
        self.discard_launch_actions_for_execution(public_handle);
    }

    fn discard_launch_actions_for_execution(&mut self, public_handle: &str) {
        self.launch_actions
            .retain(|_, action| action.mapping.public_handle != public_handle);
    }

    fn forget_mapping(
        &mut self,
        kind: ExecutionKind,
        public_handle: &str,
    ) -> Option<ExecutionMapping> {
        if self
            .active
            .as_ref()
            .is_some_and(|mapping| mapping.kind == kind && mapping.public_handle == public_handle)
        {
            self.discard_launch_actions_for_execution(public_handle);
            self.successful_launches.remove(public_handle);
            return self.active.take();
        }
        if self
            .latest_terminal
            .as_ref()
            .is_some_and(|mapping| mapping.kind == kind && mapping.public_handle == public_handle)
        {
            self.discard_launch_actions_for_execution(public_handle);
            self.successful_launches.remove(public_handle);
            return self.latest_terminal.take();
        }
        None
    }

    fn forget_active(&mut self, kind: ExecutionKind, public_handle: &str) {
        if self
            .active
            .as_ref()
            .is_some_and(|mapping| mapping.kind == kind && mapping.public_handle == public_handle)
        {
            self.discard_launch_actions_for_execution(public_handle);
            self.successful_launches.remove(public_handle);
            self.active = None;
        }
    }
}

trait RuntimeRequester {
    fn request(&self, request_type: &str, payload: Value) -> Result<Value, String>;
}

impl RuntimeRequester for SidecarState {
    fn request(&self, request_type: &str, payload: Value) -> Result<Value, String> {
        SidecarState::request(self, request_type, payload)
    }
}

fn runtime_request(
    runtime: &impl RuntimeRequester,
    request_type: &str,
    payload: Value,
) -> Result<Value, String> {
    runtime.request(request_type, payload)
}

#[tauri::command]
pub fn start_simulated_execution(
    review_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut executions = state.executions.lock().map_err(|_| {
        safe_error(
            "execution_state_unavailable",
            "Simulated execution state is unavailable.",
        )
    })?;
    executions
        .reserve_start(ExecutionKind::Simulated)
        .map_err(|()| {
            safe_error(
                "execution_in_progress",
                "A simulated run is already starting or active.",
            )
        })?;

    let outcome = start_simulated_execution_inner(&review_handle, &state, &mut executions);
    if outcome.is_err() {
        executions.release_start();
    }
    outcome
}

fn start_simulated_execution_inner(
    review_handle: &str,
    state: &AppState,
    executions: &mut ExecutionHandleStore,
) -> Result<Value, String> {
    let review = state
        .handles
        .lock()
        .map_err(|_| session_error())?
        .review(review_handle)?
        .clone();

    validate_catalog(&review, state)?;
    let adb_path = current_adb_path(state)?;
    let inventory = runtime_request(
        &state.sidecar,
        "listAdbDevices",
        json!({ "adbPath": &adb_path }),
    )
    .map_err(|_| stale_review("The reviewed device could not be found."))?;

    let (serial, refreshed_review) = {
        let mut handles = state.handles.lock().map_err(|_| session_error())?;
        handles
            .update_devices(&inventory)
            .map_err(|_| stale_review("The reviewed device inventory changed."))?;
        let refreshed = handles.review(review_handle)?.clone();
        let device = handles
            .device(&refreshed.device_handle)
            .map_err(|_| stale_review("The reviewed device disconnected."))?;
        if device.state != "available" {
            return Err(stale_review(
                "The reviewed device is not currently available.",
            ));
        }
        (device.serial.clone(), refreshed)
    };

    let facts = runtime_request(
        &state.sidecar,
        "probeDevice",
        json!({ "adbPath": adb_path, "serial": &serial }),
    )
    .map_err(|_| stale_review("The reviewed device facts could not be refreshed."))?;
    validate_target(&refreshed_review.target, &serial, &facts)?;
    validate_plan_digest(&refreshed_review)?;

    let start_result = request_dry_run_start(&state.sidecar, &refreshed_review)?;
    bind_start_result(executions, review_handle, refreshed_review, &start_result)
}

fn request_dry_run_start(
    runtime: &impl RuntimeRequester,
    review: &ReviewedPlanSnapshot,
) -> Result<Value, String> {
    runtime_request(
        runtime,
        "startExecution",
        json!({
            "plan": review.response.get("plan"),
            "planDigest": review.plan_digest,
            "mode": "dry_run",
            "targetDevice": review.target,
        }),
    )
    .map_err(|error| execution_start_error(&error))
}

fn bind_start_result(
    executions: &mut ExecutionHandleStore,
    review_handle: &str,
    review: ReviewedPlanSnapshot,
    start_result: &Value,
) -> Result<Value, String> {
    let report = start_result.get("execution").ok_or_else(|| {
        safe_error(
            "simulation_start_failed",
            "The simulated run returned an invalid initial report.",
        )
    })?;
    let sidecar_id = report
        .get("executionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            safe_error(
                "simulation_start_failed",
                "The simulated run did not provide an execution identifier.",
            )
        })?
        .to_string();
    let mapping = executions.bind_started(
        ExecutionKind::Simulated,
        sidecar_id,
        review_handle.to_string(),
        review,
    );
    Ok(project_snapshot(&mapping, report))
}

#[tauri::command]
pub fn get_simulated_execution(
    execution_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mapping = state
        .executions
        .lock()
        .map_err(|_| execution_state_error())?
        .mapping(
            ExecutionKind::Simulated,
            &execution_handle,
            "This simulated run is unavailable. Return to Review or generate a new review.",
        )?;
    let response = match runtime_request(
        &state.sidecar,
        "getExecution",
        json!({ "executionId": mapping.sidecar_id }),
    ) {
        Ok(response) => response,
        Err(error) => {
            if sidecar_error_code(&error).as_deref() == Some("unknown_execution") {
                state
                    .executions
                    .lock()
                    .map_err(|_| execution_state_error())?
                    .forget_active(ExecutionKind::Simulated, &execution_handle);
                return Err(safe_error(
                    "execution_unavailable",
                    "The in-memory simulated run was lost. Return to Review or generate a new review.",
                ));
            }
            return Err(safe_error(
                "execution_status_failed",
                "The simulated run status could not be refreshed.",
            ));
        }
    };
    let report = response.get("execution").ok_or_else(|| {
        safe_error(
            "execution_status_failed",
            "The simulated run returned an invalid status report.",
        )
    })?;
    let public = project_snapshot(&mapping, report);
    if is_terminal_status(report.get("status").and_then(Value::as_str)) {
        state
            .executions
            .lock()
            .map_err(|_| execution_state_error())?
            .mark_terminal(ExecutionKind::Simulated, &execution_handle);
    }
    Ok(public)
}

#[tauri::command]
pub fn get_simulated_execution_events(
    execution_handle: String,
    after_sequence: u64,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut executions = state
        .executions
        .lock()
        .map_err(|_| execution_state_error())?;
    request_simulated_execution_events(
        &state.sidecar,
        &mut executions,
        &execution_handle,
        after_sequence,
    )
}

fn request_simulated_execution_events(
    runtime: &impl RuntimeRequester,
    executions: &mut ExecutionHandleStore,
    execution_handle: &str,
    after_sequence: u64,
) -> Result<Value, String> {
    let mapping = executions.mapping(
        ExecutionKind::Simulated,
        execution_handle,
        "This simulated run is unavailable. Return to Review or generate a new review.",
    )?;
    let response = runtime_request(
        runtime,
        "getExecutionEvents",
        json!({
            "executionId": mapping.sidecar_id,
            "afterSequence": after_sequence,
        }),
    );
    let response = match response {
        Ok(response) => response,
        Err(error) if sidecar_error_code(&error).as_deref() == Some("unknown_execution") => {
            executions.forget_active(ExecutionKind::Simulated, execution_handle);
            return Err(safe_error(
                "execution_unavailable",
                "The in-memory simulated run was lost. Return to Review or generate a new review.",
            ));
        }
        Err(_) => {
            return Err(safe_error(
                "execution_status_failed",
                "Incremental simulated progress could not be refreshed.",
            ));
        }
    };
    Ok(project_event_batch(&mapping, &response))
}

#[tauri::command]
pub fn cancel_simulated_execution(
    execution_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mapping = state
        .executions
        .lock()
        .map_err(|_| execution_state_error())?
        .mapping(
            ExecutionKind::Simulated,
            &execution_handle,
            "This simulated run is unavailable. Return to Review or generate a new review.",
        )?;
    let response = runtime_request(
        &state.sidecar,
        "cancelExecution",
        json!({ "executionId": mapping.sidecar_id }),
    )
    .map_err(|_| {
        safe_error(
            "execution_cancel_failed",
            "Cancellation could not be requested for this simulated run.",
        )
    })?;
    Ok(json!({
        "executionHandle": execution_handle,
        "accepted": response.get("accepted").and_then(Value::as_bool).unwrap_or(false),
        "status": allowlisted_real_string(
            response.get("status"),
            &[
                "queued",
                "running",
                "succeeded",
                "succeeded_with_warnings",
                "failed",
                "cancelled",
            ],
            "running",
        ),
    }))
}

const REAL_EXECUTION_UNAVAILABLE: &str = "This real-device execution is unavailable. Its outcome is unknown and the device may have been partially changed. Reconnect and generate a new review.";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RealExecutionStartRequest {
    review_handle: String,
    confirmation: RealExecutionConfirmation,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RealExecutionConfirmation {
    phrase: String,
    irreversible_changes_acknowledged: bool,
    no_rollback_acknowledged: bool,
    keep_device_connected_acknowledged: bool,
}

#[tauri::command]
pub fn get_real_execution_availability() -> Value {
    json!({ "enabled": cfg!(feature = "real-execution") })
}

#[tauri::command]
pub fn start_real_execution(request: Value, state: State<'_, AppState>) -> Result<Value, String> {
    if !cfg!(feature = "real-execution") {
        return Err(safe_error(
            "real_execution_disabled",
            "Real-device execution is not enabled for this build.",
        ));
    }
    let request = parse_real_start_request(request)?;
    let mut executions = state.executions.lock().map_err(|_| {
        safe_error(
            "execution_state_unavailable",
            "Real-device execution state is unavailable.",
        )
    })?;
    executions
        .reserve_start(ExecutionKind::Real)
        .map_err(|()| {
            safe_error(
                "execution_in_progress",
                "Another execution is already starting or active.",
            )
        })?;
    let result = start_real_execution_inner(&request.review_handle, &state, &mut executions);
    if result.is_err() {
        executions.release_start();
    }
    result
}

fn parse_real_start_request(request: Value) -> Result<RealExecutionStartRequest, String> {
    let request: RealExecutionStartRequest = serde_json::from_value(request).map_err(|_| {
        safe_error(
            "real_execution_confirmation_invalid",
            "Confirm irreversible changes, no rollback, and a stable device connection before continuing.",
        )
    })?;
    let valid = request.confirmation.phrase.trim() == "APPLY TO DEVICE"
        && request.confirmation.irreversible_changes_acknowledged
        && request.confirmation.no_rollback_acknowledged
        && request.confirmation.keep_device_connected_acknowledged;
    if !valid {
        return Err(safe_error(
            "real_execution_confirmation_invalid",
            "Confirm irreversible changes, no rollback, and a stable device connection before continuing.",
        ));
    }
    Ok(request)
}

fn start_real_execution_inner(
    review_handle: &str,
    state: &AppState,
    executions: &mut ExecutionHandleStore,
) -> Result<Value, String> {
    let review = state
        .handles
        .lock()
        .map_err(|_| session_error())?
        .review(review_handle)?
        .clone();
    validate_catalog(&review, state)?;

    let expected_adb = review.platform_tools_identity.as_ref().ok_or_else(|| {
        stale_review(
            "The reviewed Platform-Tools installation is no longer associated with this review.",
        )
    })?;
    let adb_path = state
        .adb
        .lock()
        .map_err(|_| {
            safe_error(
                "platform_tools_unavailable",
                "The reviewed Platform-Tools installation is unavailable. Repair it and generate a new review.",
            )
        })?
        .revalidate_for_execution(expected_adb)
        .map_err(|error| match error {
            AdbRevalidationError::Unavailable => safe_error(
                "platform_tools_unavailable",
                "The reviewed Platform-Tools installation is unavailable. Repair it and generate a new review.",
            ),
            AdbRevalidationError::Changed => {
                stale_review("The Platform-Tools installation changed after review.")
            }
        })?;
    let adb_path = adb_path.to_string_lossy().into_owned();

    let inventory = runtime_request(
        &state.sidecar,
        "listAdbDevices",
        json!({ "adbPath": &adb_path }),
    )
    .map_err(|_| device_disconnected())?;
    let (serial, refreshed_review) = {
        let mut handles = state.handles.lock().map_err(|_| session_error())?;
        handles
            .update_devices(&inventory)
            .map_err(|_| device_disconnected())?;
        let refreshed = handles.review(review_handle)?.clone();
        let device = handles
            .device(&refreshed.device_handle)
            .map_err(|_| device_disconnected())?;
        if device.state != "available" {
            return Err(device_disconnected());
        }
        (device.serial.clone(), refreshed)
    };
    let facts = runtime_request(
        &state.sidecar,
        "probeDevice",
        json!({ "adbPath": adb_path, "serial": &serial }),
    )
    .map_err(|_| device_disconnected())?;
    validate_target(&refreshed_review.target, &serial, &facts)?;
    validate_plan_digest(&refreshed_review)?;
    validate_retained_byo_inputs(&refreshed_review, &SystemInputReadability)?;

    let start_result = request_real_start(&state.sidecar, &refreshed_review)?;
    bind_real_start_result(executions, review_handle, refreshed_review, &start_result)
}

fn request_real_start(
    runtime: &impl RuntimeRequester,
    review: &ReviewedPlanSnapshot,
) -> Result<Value, String> {
    runtime_request(
        runtime,
        "startExecution",
        json!({
            "plan": review.response.get("plan"),
            "planDigest": review.plan_digest,
            "mode": "real",
            "targetDevice": review.target,
        }),
    )
    .map_err(|error| real_start_error(&error))
}

fn bind_real_start_result(
    executions: &mut ExecutionHandleStore,
    review_handle: &str,
    review: ReviewedPlanSnapshot,
    start_result: &Value,
) -> Result<Value, String> {
    let report = start_result
        .get("execution")
        .ok_or_else(real_start_failed)?;
    let sidecar_id = report
        .get("executionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(real_start_failed)?
        .to_string();
    let mapping = executions.bind_started(
        ExecutionKind::Real,
        sidecar_id,
        review_handle.to_string(),
        review,
    );
    Ok(project_real_snapshot(&mapping, report))
}

#[tauri::command]
pub fn get_real_execution(
    execution_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mapping = state
        .executions
        .lock()
        .map_err(|_| real_execution_state_error())?
        .mapping(
            ExecutionKind::Real,
            &execution_handle,
            REAL_EXECUTION_UNAVAILABLE,
        )?;
    let response = match runtime_request(
        &state.sidecar,
        "getExecution",
        json!({ "executionId": mapping.sidecar_id }),
    ) {
        Ok(response) => response,
        Err(error) if sidecar_error_code(&error).as_deref() == Some("unknown_execution") => {
            forget_lost_real_mapping(&state, &execution_handle)?;
            return Err(safe_error(
                "execution_unavailable",
                REAL_EXECUTION_UNAVAILABLE,
            ));
        }
        Err(_) => {
            return Err(safe_error(
                "execution_status_failed",
                "Real-device execution status could not be refreshed.",
            ));
        }
    };
    let report = response.get("execution").ok_or_else(|| {
        safe_error(
            "execution_status_failed",
            "Real-device execution returned an invalid status report.",
        )
    })?;
    let mut public = project_real_snapshot(&mapping, report);
    if is_terminal_status(report.get("status").and_then(Value::as_str)) {
        let mut executions = state
            .executions
            .lock()
            .map_err(|_| real_execution_state_error())?;
        executions.mark_terminal(ExecutionKind::Real, &execution_handle);
        public["launchAction"] = executions
            .launch_action(&mapping, report)
            .unwrap_or(Value::Null);
    } else {
        public["launchAction"] = Value::Null;
    }
    Ok(public)
}

#[tauri::command]
pub fn get_real_execution_events(
    execution_handle: String,
    after_sequence: u64,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mapping = state
        .executions
        .lock()
        .map_err(|_| real_execution_state_error())?
        .mapping(
            ExecutionKind::Real,
            &execution_handle,
            REAL_EXECUTION_UNAVAILABLE,
        )?;
    let response = match runtime_request(
        &state.sidecar,
        "getExecutionEvents",
        json!({
            "executionId": mapping.sidecar_id,
            "afterSequence": after_sequence,
        }),
    ) {
        Ok(response) => response,
        Err(error) if sidecar_error_code(&error).as_deref() == Some("unknown_execution") => {
            forget_lost_real_mapping(&state, &execution_handle)?;
            return Err(safe_error(
                "execution_unavailable",
                REAL_EXECUTION_UNAVAILABLE,
            ));
        }
        Err(_) => {
            return Err(safe_error(
                "execution_status_failed",
                "Incremental real-device progress could not be refreshed.",
            ));
        }
    };
    Ok(project_real_event_batch(&mapping, &response))
}

#[tauri::command]
pub fn cancel_real_execution(
    execution_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mapping = state
        .executions
        .lock()
        .map_err(|_| real_execution_state_error())?
        .mapping(
            ExecutionKind::Real,
            &execution_handle,
            REAL_EXECUTION_UNAVAILABLE,
        )?;
    let response = match runtime_request(
        &state.sidecar,
        "cancelExecution",
        json!({ "executionId": mapping.sidecar_id }),
    ) {
        Ok(response) => response,
        Err(error) if sidecar_error_code(&error).as_deref() == Some("unknown_execution") => {
            forget_lost_real_mapping(&state, &execution_handle)?;
            return Err(safe_error(
                "execution_unavailable",
                REAL_EXECUTION_UNAVAILABLE,
            ));
        }
        Err(_) => {
            return Err(safe_error(
                "execution_cancel_failed",
                "Cancellation could not be requested. The current operation may still be running.",
            ));
        }
    };
    Ok(json!({
        "executionHandle": execution_handle,
        "accepted": response.get("accepted").and_then(Value::as_bool).unwrap_or(false),
        "status": response.get("status").and_then(Value::as_str).unwrap_or("running"),
    }))
}

/// Consume one opaque launch action before revalidating any external state.
///
/// A failed invocation leaves the retained execution eligible, so a subsequent
/// authoritative snapshot refresh may mint a new action handle. The consumed
/// handle itself is never reusable.
#[tauri::command]
pub fn launch_configured_app(
    launch_action_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if !cfg!(feature = "real-execution") {
        return Err(launch_unavailable());
    }
    let action = state
        .executions
        .lock()
        .map_err(|_| real_execution_state_error())?
        .consume_launch_action(&launch_action_handle)?;
    let mapping = action.mapping;

    let review = state
        .handles
        .lock()
        .map_err(|_| session_error())?
        .review(&mapping.review_handle)
        .map_err(|_| launch_stale_target())?
        .clone();
    validate_catalog(&review, &state).map_err(|_| launch_stale_target())?;
    let expected_adb = review
        .platform_tools_identity
        .as_ref()
        .ok_or_else(platform_tools_unavailable)?;
    let adb_path = state
        .adb
        .lock()
        .map_err(|_| platform_tools_unavailable())?
        .revalidate_for_execution(expected_adb)
        .map_err(|error| match error {
            AdbRevalidationError::Unavailable => platform_tools_unavailable(),
            AdbRevalidationError::Changed => launch_stale_target(),
        })?
        .to_string_lossy()
        .into_owned();

    let inventory = runtime_request(
        &state.sidecar,
        "listAdbDevices",
        json!({ "adbPath": &adb_path }),
    )
    .map_err(|_| device_disconnected())?;
    let (serial, refreshed_review) = {
        let mut handles = state.handles.lock().map_err(|_| session_error())?;
        handles
            .update_devices(&inventory)
            .map_err(|_| device_disconnected())?;
        let refreshed = handles
            .review(&mapping.review_handle)
            .map_err(|_| launch_stale_target())?
            .clone();
        let device = handles
            .device(&refreshed.device_handle)
            .map_err(|_| device_disconnected())?;
        if device.state != "available" {
            return Err(device_disconnected());
        }
        (device.serial.clone(), refreshed)
    };
    let facts = runtime_request(
        &state.sidecar,
        "probeDevice",
        json!({ "adbPath": adb_path, "serial": &serial }),
    )
    .map_err(|_| device_disconnected())?;
    validate_target(&refreshed_review.target, &serial, &facts)
        .map_err(|_| launch_stale_target())?;
    validate_plan_digest(&refreshed_review).map_err(|_| launch_stale_target())?;

    let report_response = runtime_request(
        &state.sidecar,
        "getExecution",
        json!({ "executionId": mapping.sidecar_id }),
    )
    .map_err(|_| launch_unavailable())?;
    let report = report_response
        .get("execution")
        .ok_or_else(launch_unavailable)?;
    eligible_launch_label(&mapping, report).ok_or_else(launch_unavailable)?;

    runtime_request(
        &state.sidecar,
        "launchExecutionApp",
        json!({ "executionId": mapping.sidecar_id }),
    )
    .map_err(|_| {
        safe_error(
            "launch_failed",
            "The configured app could not be launched. Refresh the completed execution to create a new launch action.",
        )
    })?;
    state
        .executions
        .lock()
        .map_err(|_| real_execution_state_error())?
        .mark_launch_succeeded(&mapping.public_handle);
    Ok(json!({
        "launched": true,
        "message": "The configured app was launched.",
    }))
}

#[tauri::command]
pub async fn export_execution_report(
    app: AppHandle,
    execution_handle: String,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let mapping = state
        .executions
        .lock()
        .map_err(|_| real_execution_state_error())?
        .mapping_any(&execution_handle)?;
    let response = runtime_request(
        &state.sidecar,
        "getExecution",
        json!({ "executionId": mapping.sidecar_id }),
    )
    .map_err(|_| {
        safe_error(
            "report_unavailable",
            "This execution report is no longer available in this app session.",
        )
    })?;
    let report = response.get("execution").ok_or_else(|| {
        safe_error(
            "report_unavailable",
            "This execution report could not be prepared.",
        )
    })?;
    if !is_terminal_status(report.get("status").and_then(Value::as_str)) {
        return Err(safe_error(
            "report_not_terminal",
            "Wait for execution to finish before exporting its report.",
        ));
    }
    let public = match mapping.kind {
        ExecutionKind::Simulated => project_snapshot(&mapping, report),
        ExecutionKind::Real => project_real_snapshot(&mapping, report),
    };
    let runtime = serde_json::to_value(state.sidecar.status()).map_err(|_| {
        safe_error(
            "report_serialization_failed",
            "Runtime metadata could not be prepared for the report.",
        )
    })?;
    let document = execution_report_document(&mapping, report, &public, runtime);
    let mut serialized = serde_json::to_string_pretty(&document).map_err(|_| {
        safe_error(
            "report_serialization_failed",
            "The sanitized execution report could not be serialized.",
        )
    })?;
    serialized.push('\n');

    let picker = app
        .dialog()
        .file()
        .set_file_name("emuchef-execution-report.json")
        .add_filter("EmuChef execution report", &["json"]);
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    picker.save_file(move |selection| {
        let _ = sender.try_send(selection);
    });
    let selected: Option<FilePath> = receiver.recv().await.ok_or_else(|| {
        safe_error(
            "report_picker_failed",
            "The report save dialog could not be opened.",
        )
    })?;
    let Some(selected) = selected else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    let path = selected.into_path().map_err(|_| {
        safe_error(
            "report_destination_unavailable",
            "The selected report destination is unavailable.",
        )
    })?;
    tauri::async_runtime::spawn_blocking(move || fs::write(path, serialized))
        .await
        .map_err(|_| {
            safe_error(
                "report_write_failed",
                "The execution report could not be written.",
            )
        })?
        .map_err(|_| {
            safe_error(
                "report_write_failed",
                "The execution report could not be written.",
            )
        })?;
    Ok(json!({ "outcome": "saved" }))
}

fn execution_report_document(
    mapping: &ExecutionMapping,
    report: &Value,
    public: &Value,
    runtime: Value,
) -> Value {
    let mut document = json!({
        "schema": "emuchef.execution-report",
        "schemaVersion": 1,
        "app": {
            "name": "EmuChef",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "runtime": runtime,
        "catalog": mapping.review.catalog_identity,
        "plan": {
            "planId": report.get("planId"),
            "planDigest": mapping.review.plan_digest,
        },
        "execution": {
            "simulated": public.get("simulated"),
            "verificationScope": public.get("verificationScope"),
            "status": public.get("status"),
            "startedAt": public.get("startedAt"),
            "finishedAt": public.get("finishedAt"),
            "completion": public.get("completion"),
            "recipes": public.get("recipes"),
            "warnings": public.get("warnings"),
            "errors": public.get("errors"),
            "target": public.get("target"),
        },
    });
    let exact_serial = mapping
        .review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    sanitize_real_projection(&mut document, exact_serial);
    document
}

fn launch_unavailable() -> String {
    safe_error(
        "launch_unavailable",
        "This launch action is unavailable. Refresh the completed execution before trying again.",
    )
}

fn launch_stale_target() -> String {
    safe_error(
        "launch_stale_target",
        "The reviewed device or configuration changed. Generate and review a fresh plan.",
    )
}

fn platform_tools_unavailable() -> String {
    safe_error(
        "platform_tools_unavailable",
        "The reviewed Platform-Tools installation is unavailable. Repair it before trying again.",
    )
}

fn forget_lost_real_mapping(state: &AppState, public_handle: &str) -> Result<(), String> {
    let removed = state
        .executions
        .lock()
        .map_err(|_| real_execution_state_error())?
        .forget_mapping(ExecutionKind::Real, public_handle);
    if let Some(mapping) = removed {
        state
            .handles
            .lock()
            .map_err(|_| session_error())?
            .invalidate_review(&mapping.review_handle, "review_stale");
    }
    Ok(())
}

trait InputReadability {
    fn file_readable(&self, path: &Path) -> bool;
    fn directory_readable(&self, path: &Path) -> bool;
}

struct SystemInputReadability;

impl InputReadability for SystemInputReadability {
    fn file_readable(&self, path: &Path) -> bool {
        fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) && File::open(path).is_ok()
    }

    fn directory_readable(&self, path: &Path) -> bool {
        if !fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
            return false;
        }
        fs::read_dir(path)
            .and_then(|mut entries| entries.next().transpose().map(|_| ()))
            .is_ok()
    }
}

fn validate_retained_byo_inputs(
    review: &ReviewedPlanSnapshot,
    readability: &impl InputReadability,
) -> Result<(), String> {
    for input in review
        .response
        .get("resolvedInputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !matches!(
            input.get("source").and_then(Value::as_str),
            Some("explicit" | "user_configuration")
        ) {
            continue;
        }
        let Some(kind @ ("file" | "directory")) = input.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = input.get("value").filter(|value| !value.is_null()) else {
            continue;
        };
        let paths = if let Some(path) = value.as_str() {
            vec![path]
        } else if let Some(values) = value.as_array() {
            if values.iter().any(|value| !value.is_string()) {
                return Err(artifact_not_ready());
            }
            values
                .iter()
                .map(|value| value.as_str().expect("array values were checked above"))
                .collect::<Vec<_>>()
        } else {
            return Err(artifact_not_ready());
        };
        if paths.is_empty()
            || paths.iter().any(|path| {
                let path = Path::new(path);
                if kind == "file" {
                    !readability.file_readable(path)
                } else {
                    !readability.directory_readable(path)
                }
            })
        {
            return Err(artifact_not_ready());
        }
    }
    Ok(())
}

fn real_start_error(error: &str) -> String {
    let parsed = serde_json::from_str::<Value>(error).ok();
    let code = parsed
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str);
    let detail_code = parsed
        .as_ref()
        .and_then(|value| value.pointer("/details/code"))
        .and_then(Value::as_str);
    match (code, detail_code) {
        (Some("execution_start_failed"), Some("artifact_not_ready")) => artifact_not_ready(),
        (Some("execution_in_progress"), _) => safe_error(
            "execution_in_progress",
            "Another execution is already starting or active.",
        ),
        (Some("plan_digest_mismatch" | "target_device_mismatch"), _) => {
            stale_review("The reviewed plan or target changed before execution.")
        }
        _ => real_start_failed(),
    }
}

fn device_disconnected() -> String {
    safe_error(
        "device_disconnected",
        "The reviewed device is not connected and available. Reconnect it and generate a new review.",
    )
}

fn artifact_not_ready() -> String {
    safe_error(
        "artifact_not_ready",
        "Required execution inputs or artifacts are not ready. Generate a fresh review after correcting them.",
    )
}

fn real_start_failed() -> String {
    safe_error(
        "real_execution_start_failed",
        "The real-device execution could not be started. Reopen confirmation before trying again.",
    )
}

fn real_execution_state_error() -> String {
    safe_error(
        "execution_state_unavailable",
        "Real-device execution state is unavailable.",
    )
}

fn validate_catalog(review: &ReviewedPlanSnapshot, state: &AppState) -> Result<(), String> {
    let current = catalog(state)?;
    let identity = serde_json::to_value(current.public_identity()).map_err(|_| {
        safe_error(
            "catalog_resource_invalid",
            "The packaged setup catalog identity could not be verified.",
        )
    })?;
    let fields_match = ["sourceKind", "sourceId", "version", "contentDigest"]
        .iter()
        .all(|field| review.catalog_identity.get(field) == identity.get(field));
    if review.catalog_digest != current.digest() || !fields_match {
        return Err(stale_review("The setup catalog changed after review."));
    }
    Ok(())
}

fn validate_target(target: &Value, serial: &str, facts: &Value) -> Result<(), String> {
    let reviewed_serial = target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reviewed_serial.trim() != serial.trim() {
        return Err(stale_review("The target device changed after review."));
    }
    for (reviewed_field, actual_field) in [("manufacturer", "manufacturer"), ("model", "model")] {
        if let Some(expected) = target.get(reviewed_field).and_then(Value::as_str) {
            let actual = facts.get(actual_field).and_then(Value::as_str);
            if actual
                .is_none_or(|value| normalize_target_text(expected) != normalize_target_text(value))
            {
                return Err(stale_review(
                    "The target device facts changed after review.",
                ));
            }
        }
    }
    if let Some(expected) = target.get("androidApiLevel").and_then(Value::as_i64) {
        if facts.get("android_api_level").and_then(Value::as_i64) != Some(expected) {
            return Err(stale_review(
                "The target Android API level changed after review.",
            ));
        }
    }
    Ok(())
}

fn normalize_target_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn validate_plan_digest(review: &ReviewedPlanSnapshot) -> Result<(), String> {
    let plan = review
        .response
        .get("plan")
        .ok_or_else(|| stale_review("The retained reviewed plan is no longer available."))?;
    let actual = canonical_json_digest(plan)
        .map_err(|_| stale_review("The retained reviewed plan could not be verified."))?;
    if actual != review.plan_digest.to_lowercase() {
        return Err(stale_review("The reviewed plan changed after review."));
    }
    Ok(())
}

fn canonical_json_digest(value: &Value) -> Result<String, serde_json::Error> {
    let canonical = canonicalize(value.clone());
    let bytes = serde_json::to_vec(&canonical)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        scalar => scalar,
    }
}

fn project_real_snapshot(mapping: &ExecutionMapping, report: &Value) -> Value {
    let mut public = project_snapshot(mapping, report);
    let exact_serial = mapping
        .review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    public["simulated"] = Value::Bool(false);
    public["verificationScope"] = Value::String("real_device".to_string());
    let status = allowlisted_real_string(
        report.get("status"),
        &[
            "queued",
            "running",
            "succeeded",
            "succeeded_with_warnings",
            "failed",
            "cancelled",
        ],
        "running",
    );
    public["status"] = Value::String(status.to_string());
    public["terminal"] = Value::Bool(is_terminal_status(Some(status)));
    public["warnings"] = Value::Array(project_real_issues(report.get("warnings")));
    public["errors"] = Value::Array(project_real_issues(report.get("errors")));

    if let Some(recipes) = public.get_mut("recipes").and_then(Value::as_array_mut) {
        for recipe in recipes {
            recipe["status"] = Value::String(
                allowlisted_real_string(
                    recipe.get("status"),
                    &[
                        "pending",
                        "running",
                        "succeeded",
                        "succeeded_with_warnings",
                        "failed",
                        "blocked",
                        "cancelled",
                    ],
                    "pending",
                )
                .to_string(),
            );
            sanitize_real_field(recipe, "name", exact_serial, "Setup recipe");
            sanitize_real_field(recipe, "description", exact_serial, "");
            if let Some(steps) = recipe.get_mut("steps").and_then(Value::as_array_mut) {
                for step in steps {
                    step["status"] = Value::String(
                        allowlisted_real_string(
                            step.get("status"),
                            &[
                                "pending",
                                "running",
                                "succeeded",
                                "skipped",
                                "failed",
                                "blocked",
                                "cancelled",
                            ],
                            "pending",
                        )
                        .to_string(),
                    );
                    sanitize_real_field(step, "name", exact_serial, "Setup step");
                    sanitize_real_field(step, "note", exact_serial, "");
                    step["message"] = match step.get("status").and_then(Value::as_str) {
                        Some("failed" | "blocked") => {
                            Value::String("This device operation did not complete.".to_string())
                        }
                        _ => Value::Null,
                    };
                }
            }
        }
    }
    public["target"] = project_real_target(mapping, exact_serial);
    public["completion"] = completion_summary(&public, true);
    sanitize_real_projection(&mut public, exact_serial);
    public
}

fn eligible_launch_label(mapping: &ExecutionMapping, report: &Value) -> Option<String> {
    if report.get("simulated").and_then(Value::as_bool) != Some(false)
        || !matches!(
            report.get("status").and_then(Value::as_str),
            Some("succeeded" | "succeeded_with_warnings")
        )
    {
        return None;
    }
    let succeeded = report
        .get("recipes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|recipe| {
            recipe
                .get("steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|step| step.get("status").and_then(Value::as_str) == Some("succeeded"))
        .filter_map(|step| step.get("stepId").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let mut candidates = HashMap::<(String, Option<String>), String>::new();
    for step in mapping
        .review
        .response
        .pointer("/plan/steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let step_id = step.get("id").and_then(Value::as_str)?;
        if step.get("type").and_then(Value::as_str) != Some("launch_app")
            || !succeeded.contains(step_id)
        {
            continue;
        }
        let params = step.get("params")?;
        let package_name = params
            .pointer("/package_name/value")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())?;
        let activity = match params.get("activity") {
            None => None,
            Some(value) if value.get("value").is_some_and(Value::is_null) => None,
            Some(value) => Some(
                value
                    .get("value")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())?
                    .to_string(),
            ),
        };
        let label = step
            .get("note")
            .or_else(|| step.get("name"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Launch configured app")
            .to_string();
        candidates
            .entry((package_name.to_string(), activity))
            .or_insert(label);
    }
    if candidates.len() != 1 {
        return None;
    }
    candidates.into_values().next()
}

fn project_real_target(mapping: &ExecutionMapping, exact_serial: &str) -> Value {
    let plan = mapping.review.response.get("plan").unwrap_or(&Value::Null);
    let target = plan.get("target_device").unwrap_or(&mapping.review.target);
    let context = plan.get("device_context").unwrap_or(&Value::Null);
    let mut projected = Map::new();
    projected.insert(
        "label".to_string(),
        Value::String("Connected Android device".to_string()),
    );
    for field in ["manufacturer", "model"] {
        if let Some(value) = target
            .get(field)
            .or_else(|| context.get(field))
            .and_then(Value::as_str)
            .and_then(|value| safe_real_text(value, exact_serial))
        {
            projected.insert(field.to_string(), Value::String(value));
        }
    }
    if let Some(api) = target
        .get("android_api_level")
        .or_else(|| mapping.review.target.get("androidApiLevel"))
        .or_else(|| context.get("android_api_level"))
        .and_then(Value::as_i64)
    {
        projected.insert("androidApiLevel".to_string(), Value::from(api));
    }
    if let Some(version) = context
        .get("android_version")
        .or_else(|| target.get("android_version"))
        .and_then(Value::as_str)
        .and_then(|value| safe_real_text(value, exact_serial))
    {
        projected.insert("androidVersion".to_string(), Value::String(version));
    }
    Value::Object(projected)
}

fn project_real_issues(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(project_real_issue)
        .collect()
}

fn project_real_issue(issue: &Value) -> Value {
    let internal = issue
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("execution_issue");
    let (code, message) = match internal {
        "dependency_blocked" => (
            internal,
            "Required work was blocked because a dependency did not complete.",
        ),
        "artifact_tls_verification_failed" => {
            (internal, "An artifact could not be verified securely.")
        }
        "artifact_http_status" => (
            internal,
            "An artifact server returned an unsuccessful response.",
        ),
        "artifact_transport_failed" => (internal, "An artifact could not be transferred."),
        "artifact_digest_mismatch" => (internal, "An artifact failed integrity verification."),
        "artifact_size_mismatch" => (internal, "An artifact size did not match its definition."),
        "unknown_artifact_ref" => (internal, "A required reviewed artifact was unavailable."),
        "verification_failed" => (internal, "Completed work could not be verified."),
        "missing_capability" => (internal, "The device lacks a required capability."),
        "step_conflict" => (internal, "Conflicting work prevented this operation."),
        "step_execution_failed" => (internal, "A device operation failed."),
        "optional_permission_failed" => (internal, "An optional permission action failed."),
        "execution_worker_panicked" => (internal, "The execution worker stopped unexpectedly."),
        _ => ("execution_issue", "Execution reported an issue."),
    };
    json!({
        "code": code,
        "message": message,
        "recipeId": issue.get("recipeId"),
        "stepId": issue.get("stepId"),
        "remediation": remediation_for_code(code),
    })
}

fn remediation_for_code(code: &str) -> Value {
    let (kind, title, message) = match code {
        "artifact_tls_verification_failed"
        | "artifact_digest_mismatch"
        | "artifact_size_mismatch"
        | "unknown_artifact_ref"
        | "missing_capability"
        | "step_conflict" => (
            "review_inputs",
            "Review this configuration",
            "Refresh the configuration and review its inputs before creating a new plan.",
        ),
        "artifact_http_status" | "artifact_transport_failed" => (
            "generate_fresh_plan",
            "Try again with a fresh plan",
            "After resolving connectivity or artifact availability, generate and review a fresh plan.",
        ),
        "dependency_blocked"
        | "verification_failed"
        | "step_execution_failed"
        | "optional_permission_failed" => (
            "generate_fresh_plan",
            "Repair and retry",
            "Resolve the reported feature problem, then generate and review a fresh plan.",
        ),
        _ => (
            "view_report",
            "Review the execution report",
            "Export the sanitized report for support, then start a fresh planning flow.",
        ),
    };
    json!({ "kind": kind, "title": title, "message": message })
}

fn project_real_event_batch(mapping: &ExecutionMapping, response: &Value) -> Value {
    let mut public = project_event_batch(mapping, response);
    let exact_serial = mapping
        .review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(events) = public.get_mut("events").and_then(Value::as_array_mut) {
        for event in events {
            event["eventType"] = Value::String(
                allowlisted_real_string(
                    event.get("eventType"),
                    &[
                        "execution_started",
                        "step_progress",
                        "cancel_requested",
                        "execution_completed",
                        "execution_worker_panicked",
                    ],
                    "execution_progress",
                )
                .to_string(),
            );
            event["phase"] = allowlisted_real_optional_string(
                event.get("phase"),
                &[
                    "checking_skip_conditions",
                    "executing",
                    "verifying",
                    "finished",
                ],
            );
            event["status"] = allowlisted_real_optional_string(
                event.get("status"),
                &["skipped", "blocked", "succeeded", "failed", "cancelled"],
            );
            sanitize_real_field(event, "note", exact_serial, "");
            event["message"] = match event.get("eventType").and_then(Value::as_str) {
                Some("execution_started") => {
                    Value::String("Real-device execution started.".to_string())
                }
                Some("execution_completed") => {
                    Value::String("Real-device execution completed.".to_string())
                }
                Some("cancel_requested") => Value::String(
                    "Cancellation was requested and will be observed between atomic operations."
                        .to_string(),
                ),
                Some("execution_worker_panicked") => {
                    Value::String("The execution worker stopped unexpectedly.".to_string())
                }
                _ => Value::Null,
            };
            if let Some(issue) = event.get("issue").filter(|value| !value.is_null()).cloned() {
                event["issue"] = project_real_issue(&issue);
            }
        }
    }
    sanitize_real_projection(&mut public, exact_serial);
    public
}

fn allowlisted_real_string<'a>(
    value: Option<&Value>,
    allowed: &[&'a str],
    fallback: &'a str,
) -> &'a str {
    value
        .and_then(Value::as_str)
        .and_then(|candidate| {
            allowed
                .iter()
                .copied()
                .find(|allowed| candidate == *allowed)
        })
        .unwrap_or(fallback)
}

fn allowlisted_real_optional_string(value: Option<&Value>, allowed: &[&str]) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(|candidate| {
            allowed
                .iter()
                .copied()
                .find(|allowed| candidate == *allowed)
        })
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null)
}

fn sanitize_real_projection(value: &mut Value, exact_serial: &str) {
    match value {
        Value::String(text) => {
            *text = safe_real_text(text, exact_serial).unwrap_or_else(|| "[redacted]".to_string());
        }
        Value::Array(values) => {
            for value in values {
                sanitize_real_projection(value, exact_serial);
            }
        }
        Value::Object(values) => {
            for (key, value) in values.iter_mut() {
                // These values are generated locally from fixed protocol enums,
                // counters, timestamps, booleans, or opaque public handles. Do
                // not let an arbitrary serial corrupt that protocol merely by
                // matching a fixed value such as `running`.
                if matches!(
                    key.as_str(),
                    "executionHandle"
                        | "reviewHandle"
                        | "simulated"
                        | "verificationScope"
                        | "status"
                        | "latestSequence"
                        | "terminal"
                        | "sequence"
                        | "eventType"
                        | "phase"
                        | "accepted"
                        | "code"
                        | "classification"
                        | "kind"
                        | "schema"
                        | "schemaVersion"
                        | "protocolVersion"
                ) {
                    continue;
                }
                sanitize_real_projection(value, exact_serial);
            }
        }
        _ => {}
    }
}

fn sanitize_real_field(value: &mut Value, field: &str, serial: &str, fallback: &str) {
    let replacement = value
        .get(field)
        .and_then(Value::as_str)
        .and_then(|text| safe_real_text(text, serial))
        .filter(|text| !text.is_empty())
        .map(Value::String)
        .unwrap_or_else(|| {
            if fallback.is_empty() {
                Value::Null
            } else {
                Value::String(fallback.to_string())
            }
        });
    value[field] = replacement;
}

fn safe_real_text(value: &str, exact_serial: &str) -> Option<String> {
    let lower = value.to_lowercase();
    if lower.contains("://") || contains_windows_absolute_path(value) {
        return None;
    }
    let serial = exact_serial.to_lowercase();
    if !serial.is_empty() && lower.contains(&serial) {
        return None;
    }
    let characters = serial.chars().collect::<Vec<_>>();
    if characters.len() >= 4 {
        for width in 4..=characters.len() {
            for start in 0..=characters.len() - width {
                let fragment = characters[start..start + width].iter().collect::<String>();
                if lower.contains(&fragment) {
                    return None;
                }
            }
        }
    }
    Some(redact_absolute_paths(value))
}

fn contains_windows_absolute_path(value: &str) -> bool {
    if value.contains("\\\\") {
        return true;
    }
    let characters = value.chars().collect::<Vec<_>>();
    characters.windows(3).any(|window| {
        window[0].is_ascii_alphabetic() && window[1] == ':' && matches!(window[2], '\\' | '/')
    })
}

fn project_snapshot(mapping: &ExecutionMapping, report: &Value) -> Value {
    let exact_serial = mapping
        .review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let report_recipes = report
        .get("recipes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let retained_recipes = mapping
        .review
        .response
        .pointer("/plan/recipes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut by_id = report_recipes
        .iter()
        .filter_map(|recipe| Some((recipe.get("recipeId")?.as_str()?.to_string(), recipe)))
        .collect::<HashMap<_, _>>();
    let mut recipes = Vec::new();
    for retained in retained_recipes {
        let Some(recipe_id) = retained.get("id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(report_recipe) = by_id.remove(recipe_id) {
            recipes.push(project_recipe(report_recipe, Some(&retained), exact_serial));
        }
    }
    for report_recipe in &report_recipes {
        let recipe_id = report_recipe
            .get("recipeId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if by_id.remove(recipe_id).is_some() {
            recipes.push(project_recipe(report_recipe, None, exact_serial));
        }
    }
    let warnings = project_issues(report.get("warnings"), exact_serial);
    let errors = project_issues(report.get("errors"), exact_serial);
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("running");
    let mut public = json!({
        "executionHandle": mapping.public_handle,
        "reviewHandle": mapping.review_handle,
        "simulated": true,
        "verificationScope": "simulation_only",
        "status": status,
        "startedAt": report.get("startedAt"),
        "finishedAt": report.get("finishedAt"),
        "latestSequence": report.get("latestSequence").and_then(Value::as_u64).unwrap_or(0),
        "terminal": is_terminal_status(Some(status)),
        "recipes": recipes,
        "warnings": warnings,
        "errors": errors,
    });
    public["completion"] = completion_summary(&public, false);
    if !exact_serial.is_empty() {
        redact_exact_serial(&mut public, exact_serial);
    }
    public
}

fn project_recipe(recipe: &Value, retained: Option<&Value>, exact_serial: &str) -> Value {
    let recipe_id = recipe
        .get("recipeId")
        .and_then(Value::as_str)
        .unwrap_or("unknown_recipe");
    let name = recipe
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            retained
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| humanize_identifier(recipe_id));
    let steps = recipe
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|step| project_step(step, exact_serial))
        .collect::<Vec<_>>();
    json!({
        "recipeId": recipe_id,
        "name": sanitize_text(&name, exact_serial),
        "description": retained
            .and_then(|value| value.get("description"))
            .and_then(Value::as_str)
            .map(|value| sanitize_text(value, exact_serial)),
        "status": recipe.get("status").and_then(Value::as_str).unwrap_or("pending"),
        "steps": steps,
    })
}

fn project_step(step: &Value, exact_serial: &str) -> Value {
    let step_id = step
        .get("stepId")
        .and_then(Value::as_str)
        .unwrap_or("unknown_step");
    let name = step
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| humanize_identifier(step_id));
    json!({
        "stepId": step_id,
        "name": sanitize_text(&name, exact_serial),
        "note": step.get("note").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
        "status": step.get("status").and_then(Value::as_str).unwrap_or("pending"),
        "message": step.get("message").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
    })
}

fn project_issues(value: Option<&Value>, exact_serial: &str) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(project_real_issue)
        .map(|mut issue| {
            redact_exact_serial(&mut issue, exact_serial);
            issue
        })
        .collect()
}

fn completion_summary(snapshot: &Value, real: bool) -> Value {
    let status = snapshot
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("running");
    let mut counts = BTreeMap::<&'static str, u64>::from([
        ("total", 0),
        ("completed", 0),
        ("skipped", 0),
        ("blocked", 0),
        ("failed", 0),
        ("cancelled", 0),
        ("pending", 0),
    ]);
    let mut features = Vec::new();
    for recipe in snapshot
        .get("recipes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let mut feature_counts = BTreeMap::<&'static str, u64>::new();
        for step in recipe
            .get("steps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            *counts.get_mut("total").expect("count exists") += 1;
            let key = match step.get("status").and_then(Value::as_str) {
                Some("succeeded") => "completed",
                Some("skipped") => "skipped",
                Some("blocked") => "blocked",
                Some("failed") => "failed",
                Some("cancelled") => "cancelled",
                _ => "pending",
            };
            *counts.get_mut(key).expect("count exists") += 1;
            *feature_counts.entry(key).or_default() += 1;
        }
        features.push(json!({
            "recipeId": recipe.get("recipeId"),
            "name": recipe.get("name"),
            "status": recipe.get("status"),
            "counts": feature_counts,
        }));
    }
    let classification = match status {
        "succeeded" => "success",
        "succeeded_with_warnings" => "success_with_warnings",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "in_progress",
    };
    json!({
        "classification": classification,
        "counts": counts,
        "warningCount": snapshot.get("warnings").and_then(Value::as_array).map_or(0, Vec::len),
        "features": features,
        "partialChangesPossible": real
            && matches!(status, "failed" | "cancelled")
            && counts.get("completed").copied().unwrap_or(0) > 0,
    })
}

fn project_event_batch(mapping: &ExecutionMapping, response: &Value) -> Value {
    let exact_serial = mapping
        .review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut events = response
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            let sequence = event.get("sequence").and_then(Value::as_u64)?;
            if !seen.insert(sequence) {
                return None;
            }
            Some(json!({
                "sequence": sequence,
                "timestamp": event.get("timestamp"),
                "eventType": event.get("eventType"),
                "recipeId": event.get("recipeId"),
                "stepId": event.get("stepId"),
                "phase": event.get("phase"),
                "status": event.get("status"),
                "note": event.get("note").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
                "message": event.get("message").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
                "issue": event.get("issue").map(|issue| project_issues(Some(&Value::Array(vec![issue.clone()])), exact_serial).into_iter().next().unwrap_or(Value::Null)),
            }))
        })
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.get("sequence").and_then(Value::as_u64).unwrap_or(0));
    let mut public = json!({
        "executionHandle": mapping.public_handle,
        "events": events,
        "latestSequence": response.get("latestSequence").and_then(Value::as_u64).unwrap_or(0),
        "terminal": response.get("terminal").and_then(Value::as_bool).unwrap_or(false),
    });
    if !exact_serial.is_empty() {
        redact_exact_serial(&mut public, exact_serial);
    }
    public
}

fn sanitize_text(value: &str, exact_serial: &str) -> String {
    let without_serial = if exact_serial.is_empty() {
        value.to_string()
    } else {
        value.replace(exact_serial, "[device]")
    };
    redact_absolute_paths(&without_serial)
}

fn humanize_identifier(value: &str) -> String {
    value
        .split(['.', '_', '-', '/'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_terminal_status(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("succeeded" | "succeeded_with_warnings" | "failed" | "cancelled")
    )
}

fn sidecar_error_code(error: &str) -> Option<String> {
    serde_json::from_str::<Value>(error)
        .ok()?
        .get("code")?
        .as_str()
        .map(str::to_string)
}

fn execution_start_error(error: &str) -> String {
    match sidecar_error_code(error).as_deref() {
        Some("execution_in_progress") => safe_error(
            "execution_in_progress",
            "Another simulated run is already active.",
        ),
        Some("plan_digest_mismatch" | "target_device_mismatch") => {
            stale_review("The reviewed plan or target changed before simulation.")
        }
        _ => safe_error(
            "simulation_start_failed",
            "The simulated run could not be started.",
        ),
    }
}

fn stale_review(message: &str) -> String {
    safe_error("review_stale", message)
}

fn session_error() -> String {
    safe_error(
        "session_state_unavailable",
        "Review session state is unavailable.",
    )
}

fn execution_state_error() -> String {
    safe_error(
        "execution_state_unavailable",
        "Simulated execution state is unavailable.",
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Instant;

    use super::*;

    #[test]
    fn real_execution_availability_reflects_only_the_cargo_feature() {
        assert_eq!(
            get_real_execution_availability(),
            json!({ "enabled": cfg!(feature = "real-execution") })
        );
    }

    fn review() -> ReviewedPlanSnapshot {
        let plan = json!({
            "kind": "execution_plan",
            "id": "plan.one",
            "recipes": [{ "id": "recipe.one", "name": "Recipe One", "description": "Safe description" }],
            "steps": [],
            "target_device": { "serial": "sensitive-serial", "manufacturer": "AYANEO", "model": "Pocket S", "android_api_level": 33 },
        });
        ReviewedPlanSnapshot {
            response: json!({ "plan": plan }),
            target: json!({ "serial": "sensitive-serial", "manufacturer": "AYANEO", "model": "Pocket S", "androidApiLevel": 33 }),
            catalog_identity: json!({
                "sourceKind": "bundled", "sourceId": "catalog", "version": "1",
                "contentDigest": { "algorithm": "sha256", "value": "catalog" }
            }),
            catalog_digest: "catalog".to_string(),
            plan_digest: canonical_json_digest(&plan).unwrap(),
            device_handle: "device_one".to_string(),
            platform_tools_identity: None,
            created: Instant::now(),
            last_access: Instant::now(),
        }
    }

    #[test]
    fn store_is_bounded_and_handles_are_never_reused() {
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Simulated).unwrap();
        let first = store.bind_started(
            ExecutionKind::Simulated,
            "sidecar-1".into(),
            "review-1".into(),
            review(),
        );
        assert!(store.reserve_start(ExecutionKind::Real).is_err());
        store.mark_terminal(ExecutionKind::Simulated, &first.public_handle);
        store.reserve_start(ExecutionKind::Real).unwrap();
        let second = store.bind_started(
            ExecutionKind::Real,
            "sidecar-2".into(),
            "review-2".into(),
            review(),
        );
        assert_ne!(first.public_handle, second.public_handle);
        store.mark_terminal(ExecutionKind::Real, &second.public_handle);
        assert!(store
            .mapping(
                ExecutionKind::Simulated,
                &first.public_handle,
                "unavailable"
            )
            .unwrap_err()
            .contains("execution_unavailable"));
        assert_eq!(
            store
                .mapping(ExecutionKind::Real, &second.public_handle, "unavailable")
                .unwrap()
                .sidecar_id,
            "sidecar-2"
        );
    }

    #[test]
    fn failed_start_reservation_can_always_be_released() {
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Simulated).unwrap();
        store.release_start();
        store.reserve_start(ExecutionKind::Real).unwrap();
    }

    fn launch_review() -> ReviewedPlanSnapshot {
        let mut reviewed = review();
        reviewed.response["plan"]["steps"] = json!([{
            "id": "recipe.one/launch",
            "recipe_ref": "recipe.one",
            "type": "launch_app",
            "name": "Launch app",
            "note": "Launch configured app",
            "params": {
                "package_name": { "value": "com.example.app" },
                "activity": { "value": ".MainActivity" }
            }
        }]);
        reviewed
    }

    fn eligible_launch_report(status: &str) -> Value {
        json!({
            "simulated": false,
            "status": status,
            "recipes": [{
                "recipeId": "recipe.one",
                "name": "Recipe One",
                "status": status,
                "steps": [{
                    "stepId": "recipe.one/launch",
                    "status": "succeeded"
                }]
            }]
        })
    }

    #[test]
    fn tauri_consumes_each_launch_handle_once_and_can_regenerate_after_failure() {
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Real).unwrap();
        let mapping = store.bind_started(
            ExecutionKind::Real,
            "sidecar-real".into(),
            "review-real".into(),
            launch_review(),
        );
        let report = eligible_launch_report("succeeded_with_warnings");
        store.mark_terminal(ExecutionKind::Real, &mapping.public_handle);
        let first = store.launch_action(&mapping, &report).unwrap();
        let first_handle = first.get("handle").and_then(Value::as_str).unwrap();
        let consumed = store.consume_launch_action(first_handle).unwrap();
        assert_eq!(consumed.mapping.public_handle, mapping.public_handle);
        assert!(store.consume_launch_action(first_handle).is_err());

        let replacement = store.launch_action(&mapping, &report).unwrap();
        assert_ne!(replacement.get("handle"), first.get("handle"));
        store.mark_launch_succeeded(&mapping.public_handle);
        assert!(store.launch_action(&mapping, &report).is_none());
    }

    #[test]
    fn concurrent_duplicate_launch_consumption_has_one_winner() {
        use std::sync::{Arc, Barrier};

        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Real).unwrap();
        let mapping = store.bind_started(
            ExecutionKind::Real,
            "sidecar-real".into(),
            "review-real".into(),
            launch_review(),
        );
        let report = eligible_launch_report("succeeded");
        store.mark_terminal(ExecutionKind::Real, &mapping.public_handle);
        let action = store.launch_action(&mapping, &report).unwrap();
        let handle = action
            .get("handle")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let store = Arc::new(Mutex::new(store));
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let handle = handle.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                store.lock().unwrap().consume_launch_action(&handle).is_ok()
            }));
        }
        barrier.wait();
        let successes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|succeeded| *succeeded)
            .count();
        assert_eq!(successes, 1);
    }

    #[test]
    fn failed_completion_keeps_failure_primary_and_reports_partial_changes() {
        let snapshot = json!({
            "status": "failed",
            "warnings": [{ "code": "optional_permission_failed" }],
            "recipes": [{
                "recipeId": "recipe.one",
                "name": "Recipe One",
                "status": "failed",
                "steps": [
                    { "status": "succeeded" },
                    { "status": "blocked" },
                    { "status": "failed" }
                ]
            }]
        });
        let summary = completion_summary(&snapshot, true);
        assert_eq!(summary["classification"], "failed");
        assert_eq!(summary["counts"]["completed"], 1);
        assert_eq!(summary["counts"]["blocked"], 1);
        assert_eq!(summary["counts"]["failed"], 1);
        assert_eq!(summary["warningCount"], 1);
        assert_eq!(summary["partialChangesPossible"], true);
        assert_eq!(remediation_for_code("unrecognized")["kind"], "view_report");
    }

    #[test]
    fn report_document_is_deterministic_and_excludes_private_authority() {
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Real).unwrap();
        let mapping = store.bind_started(
            ExecutionKind::Real,
            "sidecar-private-id".into(),
            "review-private-id".into(),
            review(),
        );
        let report = json!({ "planId": "/private/plan", "status": "failed" });
        let public = json!({
            "executionHandle": mapping.public_handle,
            "reviewHandle": mapping.review_handle,
            "simulated": false,
            "verificationScope": "real_device",
            "status": "failed",
            "startedAt": "2026-07-14T00:00:00Z",
            "finishedAt": "2026-07-14T00:00:01Z",
            "completion": { "classification": "failed" },
            "recipes": [{ "name": "/Users/private/input.apk", "status": "failed" }],
            "warnings": [],
            "errors": [{ "message": "https://user:secret@example.invalid/file" }],
            "target": { "model": "sensitive-serial" }
        });
        let runtime = json!({ "status": "ready", "protocolVersion": 1 });
        let first = execution_report_document(&mapping, &report, &public, runtime.clone());
        let second = execution_report_document(&mapping, &report, &public, runtime);
        assert_eq!(first, second);
        let serialized = serde_json::to_string_pretty(&first).unwrap();
        for forbidden in [
            "sidecar-private-id",
            "review-private-id",
            "sensitive-serial",
            "/Users/private",
            "user:secret",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        assert_eq!(first["schemaVersion"], 1);
        assert_eq!(first["execution"]["verificationScope"], "real_device");
    }

    #[test]
    fn target_comparison_matches_phase_zero_normalization_only() {
        let target = json!({
            "serial": " serial ", "manufacturer": "AyaNeo", "model": "Pocket   S", "androidApiLevel": 33
        });
        let facts = json!({
            "manufacturer": " ayaneo ", "model": "pocket s", "android_api_level": 33,
            "brand": "irrelevant changed brand"
        });
        validate_target(&target, "serial", &facts).unwrap();
        assert!(validate_target(&target, "different", &facts)
            .unwrap_err()
            .contains("review_stale"));
        assert!(validate_target(
            &target,
            "serial",
            &json!({ "manufacturer": "AYANEO", "model": "Other", "android_api_level": 33 })
        )
        .unwrap_err()
        .contains("review_stale"));
    }

    #[test]
    fn canonical_digest_detects_retained_plan_mutation() {
        let mut retained = review();
        validate_plan_digest(&retained).unwrap();
        retained.response["plan"]["id"] = json!("changed");
        assert!(validate_plan_digest(&retained)
            .unwrap_err()
            .contains("review_stale"));
    }

    #[test]
    fn projections_are_ordered_and_remove_sensitive_runtime_fields() {
        let retained = review();
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Simulated,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: retained,
        };
        let report = json!({
            "executionId": "execution-private",
            "reviewedPlan": { "secret": true },
            "targetDevice": { "serial": "sensitive-serial" },
            "status": "failed", "startedAt": "2026-01-01T00:00:00Z", "finishedAt": "2026-01-01T00:00:01Z", "latestSequence": 4,
            "recipes": [{
                "recipeId": "recipe.one", "name": "Recipe One", "status": "blocked",
                "steps": [{ "stepId": "step.one", "name": "", "note": "Read /Users/private/file", "status": "blocked", "message": "sensitive-serial failed", "outputs": { "path": "/secret" } }]
            }],
            "warnings": [], "errors": [{ "code": "blocked", "message": "At /private/path", "recipeId": "recipe.one", "stepId": "step.one" }]
        });
        let public = project_snapshot(&mapping, &report);
        let serialized = public.to_string();
        assert_eq!(public["recipes"][0]["name"], "Recipe One");
        assert_eq!(public["recipes"][0]["steps"][0]["name"], "Step One");
        assert!(!serialized.contains("execution-private"));
        assert!(!serialized.contains("reviewedPlan"));
        assert!(!serialized.contains("targetDevice"));
        assert!(!serialized.contains("outputs"));
        assert!(!serialized.contains("sensitive-serial"));
        assert!(!serialized.contains("/Users/private"));
        assert!(!serialized.contains("/private/path"));
    }

    struct FakeRuntime {
        requests: Mutex<Vec<(String, Value)>>,
        result: Result<Value, String>,
    }

    impl RuntimeRequester for FakeRuntime {
        fn request(&self, request_type: &str, payload: Value) -> Result<Value, String> {
            self.requests
                .lock()
                .unwrap()
                .push((request_type.into(), payload));
            self.result.clone()
        }
    }

    #[test]
    fn deterministic_runtime_records_existing_phase_zero_request_shape() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Ok(json!({})),
        };
        runtime
            .request(
                "getExecutionEvents",
                json!({ "executionId": "private", "afterSequence": 7 }),
            )
            .unwrap();
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(requests[0].0, "getExecutionEvents");
        assert_eq!(requests[0].1["afterSequence"], 7);
    }

    #[test]
    fn deterministic_runtime_proves_start_is_forced_dry_run_with_retained_data() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Ok(json!({ "execution": { "executionId": "sidecar-private" } })),
        };
        let retained = review();
        request_dry_run_start(&runtime, &retained).unwrap();
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "startExecution");
        assert_eq!(requests[0].1["mode"], "dry_run");
        assert_eq!(requests[0].1["plan"], retained.response["plan"]);
        assert_eq!(requests[0].1["planDigest"], retained.plan_digest);
        assert_eq!(requests[0].1["targetDevice"], retained.target);
        assert!(requests[0].1.get("adbPath").is_none());
        assert!(requests[0].1.get("runtimeRoot").is_none());
        assert!(requests[0].1.get("cacheRoot").is_none());
    }

    #[test]
    fn unknown_event_session_releases_only_the_matching_active_mapping() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err(json!({ "code": "unknown_execution", "message": "private" }).to_string()),
        };
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Simulated).unwrap();
        let active = store.bind_started(
            ExecutionKind::Simulated,
            "sidecar-active".into(),
            "review".into(),
            review(),
        );

        let error =
            request_simulated_execution_events(&runtime, &mut store, &active.public_handle, 3)
                .unwrap_err();
        assert!(error.contains("execution_unavailable"));
        assert!(!error.contains("sidecar-active"));
        assert!(!error.contains("private"));
        assert!(store
            .mapping(
                ExecutionKind::Simulated,
                &active.public_handle,
                "unavailable"
            )
            .unwrap_err()
            .contains("execution_unavailable"));
        store
            .reserve_start(ExecutionKind::Real)
            .expect("the lost active slot should be reusable");
    }

    #[test]
    fn ordinary_event_failure_keeps_the_active_mapping_reserved() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err(json!({ "code": "runtime_request_failed" }).to_string()),
        };
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Simulated).unwrap();
        let active = store.bind_started(
            ExecutionKind::Simulated,
            "sidecar-active".into(),
            "review".into(),
            review(),
        );

        let error =
            request_simulated_execution_events(&runtime, &mut store, &active.public_handle, 0)
                .unwrap_err();
        assert!(error.contains("execution_status_failed"));
        assert_eq!(
            store
                .mapping(
                    ExecutionKind::Simulated,
                    &active.public_handle,
                    "unavailable"
                )
                .unwrap()
                .sidecar_id,
            "sidecar-active"
        );
        assert!(store.reserve_start(ExecutionKind::Real).is_err());
    }

    #[test]
    fn unknown_terminal_event_session_does_not_remove_another_active_mapping() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err(json!({ "code": "unknown_execution" }).to_string()),
        };
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Simulated).unwrap();
        let terminal = store.bind_started(
            ExecutionKind::Simulated,
            "sidecar-terminal".into(),
            "review-old".into(),
            review(),
        );
        store.mark_terminal(ExecutionKind::Simulated, &terminal.public_handle);
        store.reserve_start(ExecutionKind::Simulated).unwrap();
        let active = store.bind_started(
            ExecutionKind::Simulated,
            "sidecar-active".into(),
            "review-new".into(),
            review(),
        );

        let error =
            request_simulated_execution_events(&runtime, &mut store, &terminal.public_handle, 0)
                .unwrap_err();
        assert!(error.contains("execution_unavailable"));
        assert_eq!(
            store
                .mapping(
                    ExecutionKind::Simulated,
                    &terminal.public_handle,
                    "unavailable"
                )
                .unwrap()
                .sidecar_id,
            "sidecar-terminal"
        );
        assert_eq!(
            store
                .mapping(
                    ExecutionKind::Simulated,
                    &active.public_handle,
                    "unavailable"
                )
                .unwrap()
                .sidecar_id,
            "sidecar-active"
        );
        assert!(store.reserve_start(ExecutionKind::Real).is_err());
    }

    #[test]
    fn real_confirmation_is_strict_complete_and_request_local() {
        let valid = json!({
            "reviewHandle": "review_public",
            "confirmation": {
                "phrase": " APPLY TO DEVICE ",
                "irreversibleChangesAcknowledged": true,
                "noRollbackAcknowledged": true,
                "keepDeviceConnectedAcknowledged": true
            }
        });
        assert_eq!(
            parse_real_start_request(valid.clone())
                .unwrap()
                .review_handle,
            "review_public"
        );
        assert!(parse_real_start_request(valid).is_ok());

        for phrase in ["apply to device", "Apply To Device", "APPLY  TO DEVICE"] {
            let invalid = json!({
                "reviewHandle": "review_public",
                "confirmation": {
                    "phrase": phrase,
                    "irreversibleChangesAcknowledged": true,
                    "noRollbackAcknowledged": true,
                    "keepDeviceConnectedAcknowledged": true
                }
            });
            assert!(parse_real_start_request(invalid)
                .unwrap_err()
                .contains("real_execution_confirmation_invalid"));
        }

        for acknowledgment in [
            "irreversibleChangesAcknowledged",
            "noRollbackAcknowledged",
            "keepDeviceConnectedAcknowledged",
        ] {
            let mut invalid = json!({
                "reviewHandle": "review_public",
                "confirmation": {
                    "phrase": "APPLY TO DEVICE",
                    "irreversibleChangesAcknowledged": true,
                    "noRollbackAcknowledged": true,
                    "keepDeviceConnectedAcknowledged": true
                }
            });
            invalid["confirmation"][acknowledgment] = Value::Bool(false);
            assert!(parse_real_start_request(invalid)
                .unwrap_err()
                .contains("real_execution_confirmation_invalid"));
        }

        let mut missing_acknowledgment = json!({
            "reviewHandle": "review_public",
            "confirmation": {
                "phrase": "APPLY TO DEVICE",
                "irreversibleChangesAcknowledged": true,
                "noRollbackAcknowledged": true,
                "keepDeviceConnectedAcknowledged": true
            }
        });
        missing_acknowledgment["confirmation"]
            .as_object_mut()
            .unwrap()
            .remove("noRollbackAcknowledged");
        assert!(parse_real_start_request(missing_acknowledgment)
            .unwrap_err()
            .contains("real_execution_confirmation_invalid"));

        let unexpected = json!({
            "reviewHandle": "review_public",
            "confirmation": {
                "phrase": "APPLY TO DEVICE",
                "irreversibleChangesAcknowledged": true,
                "noRollbackAcknowledged": true,
                "keepDeviceConnectedAcknowledged": true
            },
            "plan": { "forbidden": true }
        });
        assert!(parse_real_start_request(unexpected)
            .unwrap_err()
            .contains("real_execution_confirmation_invalid"));
    }

    #[test]
    fn deterministic_runtime_proves_real_start_uses_only_retained_data() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Ok(json!({ "execution": { "executionId": "private" } })),
        };
        let retained = review();
        request_real_start(&runtime, &retained).unwrap();
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(requests[0].0, "startExecution");
        assert_eq!(requests[0].1["mode"], "real");
        assert_eq!(requests[0].1["plan"], retained.response["plan"]);
        assert_eq!(requests[0].1["planDigest"], retained.plan_digest);
        assert_eq!(requests[0].1["targetDevice"], retained.target);
        for forbidden in ["adbPath", "runtimeRoot", "cacheRoot", "artifactPath"] {
            assert!(requests[0].1.get(forbidden).is_none());
        }
    }

    #[derive(Default)]
    struct FakeReadability {
        files: HashSet<String>,
        directories: HashSet<String>,
    }

    impl InputReadability for FakeReadability {
        fn file_readable(&self, path: &Path) -> bool {
            self.files.contains(&path.to_string_lossy().into_owned())
        }

        fn directory_readable(&self, path: &Path) -> bool {
            self.directories
                .contains(&path.to_string_lossy().into_owned())
        }
    }

    #[test]
    fn retained_byo_checks_are_kind_aware_and_ignore_ambiguous_paths() {
        let mut retained = review();
        retained.response["resolvedInputs"] = json!([
            { "key": "recipe/file", "type": "file", "value": "/chosen/file", "source": "explicit" },
            { "key": "recipe/dir", "type": "directory", "value": ["/chosen/dir"], "source": "user_configuration" },
            { "key": "recipe/path", "type": "path", "value": "/ambiguous", "source": "explicit" },
            { "key": "recipe/path-list", "type": "path_list", "value": [42], "source": "explicit" },
            { "key": "recipe/default", "type": "file", "value": "/not-user-supplied", "source": "recipe_default" }
        ]);
        let mut readable = FakeReadability::default();
        readable.files.insert("/chosen/file".to_string());
        readable.directories.insert("/chosen/dir".to_string());
        validate_retained_byo_inputs(&retained, &readable).unwrap();
        readable.files.clear();
        assert!(validate_retained_byo_inputs(&retained, &readable)
            .unwrap_err()
            .contains("artifact_not_ready"));
    }

    #[test]
    fn retained_byo_value_shapes_reject_malformed_arrays_without_partial_acceptance() {
        let mut retained = review();
        let mut readable = FakeReadability::default();
        readable.files.extend([
            "/chosen/scalar".to_string(),
            "/chosen/array-one".to_string(),
            "/chosen/array-two".to_string(),
        ]);

        for value in [
            json!("/chosen/scalar"),
            json!(["/chosen/array-one", "/chosen/array-two"]),
        ] {
            retained.response["resolvedInputs"] = json!([{
                "key": "recipe/file",
                "type": "file",
                "value": value,
                "source": "explicit"
            }]);
            validate_retained_byo_inputs(&retained, &readable).unwrap();
        }

        for value in [
            json!(["/chosen/array-one", 42]),
            json!([42, false]),
            json!([]),
            json!({ "path": "/chosen/scalar" }),
            json!(42),
            json!(true),
        ] {
            retained.response["resolvedInputs"] = json!([{
                "key": "recipe/file",
                "type": "file",
                "value": value,
                "source": "explicit"
            }]);
            assert!(validate_retained_byo_inputs(&retained, &readable)
                .unwrap_err()
                .contains("artifact_not_ready"));
        }
    }

    #[test]
    fn system_readability_checks_open_files_and_enumerate_directories_without_mutation() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("input.bin");
        fs::write(&file, b"input").unwrap();
        let checker = SystemInputReadability;
        assert!(checker.file_readable(&file));
        assert!(checker.directory_readable(temporary.path()));
        assert!(!checker.file_readable(temporary.path()));
        assert!(!checker.directory_readable(&file));
    }

    #[test]
    fn real_projection_is_allowlisted_serial_free_and_does_not_invent_android_version() {
        let retained = review();
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: retained,
        };
        let report = json!({
            "executionId": "execution-private",
            "targetDevice": { "serial": "sensitive-serial" },
            "status": "sensitive-serial",
            "startedAt": "2026-01-01T00:00:00Z",
            "finishedAt": "2026-01-01T00:00:01Z",
            "latestSequence": 4,
            "recipes": [{
                "recipeId": "recipe.one", "name": "Recipe One", "status": "failed",
                "steps": [{
                    "stepId": "step.one", "name": "Step One", "note": "Open https://user:secret@example.test/file?token=x",
                    "status": "failed", "message": "sensitive-serial /Users/private", "outputs": { "private": true }
                }]
            }],
            "warnings": [],
            "errors": [{ "code": "unfamiliar_private_code", "message": "raw sensitive-serial", "stepId": "step.one" }]
        });
        let public = project_real_snapshot(&mapping, &report);
        let serialized = public.to_string();
        assert_eq!(public["simulated"], false);
        assert_eq!(public["verificationScope"], "real_device");
        assert_eq!(public["status"], "running");
        assert_eq!(public["terminal"], false);
        assert_eq!(public["target"]["manufacturer"], "AYANEO");
        assert_eq!(public["target"]["model"], "Pocket S");
        assert_eq!(public["target"]["androidApiLevel"], 33);
        assert!(public["target"].get("androidVersion").is_none());
        assert_eq!(public["errors"][0]["code"], "execution_issue");
        for forbidden in [
            "execution-private",
            "sensitive-serial",
            "serial",
            "/Users/private",
            "user:secret",
            "token=x",
            "outputs",
            "raw",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn real_event_projection_allowlists_protocol_values_and_sanitizes_payload_text() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: review(),
        };
        let response = json!({
            "events": [{
                "sequence": 1,
                "timestamp": "HTTPS://user:secret@example.test/sensitive-serial",
                "eventType": "sensitive-serial",
                "recipeId": "recipe.sensitive-serial",
                "stepId": "step.sensitive-serial",
                "phase": "sensitive-serial",
                "status": "sensitive-serial",
                "note": "private C:\\Users\\operator sensitive-serial",
                "message": "raw sensitive-serial"
            }],
            "latestSequence": 1,
            "terminal": false
        });

        let public = project_real_event_batch(&mapping, &response);
        assert_eq!(public["events"][0]["eventType"], "execution_progress");
        assert!(public["events"][0]["phase"].is_null());
        assert!(public["events"][0]["status"].is_null());
        let serialized = public.to_string();
        for forbidden in [
            "sensitive-serial",
            "execution-private",
            "C:\\Users\\operator",
            "user:secret",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn real_store_wrong_kind_is_private_and_terminal_loss_preserves_active_mapping() {
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Real).unwrap();
        let terminal = store.bind_started(
            ExecutionKind::Real,
            "real-sidecar".into(),
            "real-review".into(),
            review(),
        );
        store.mark_terminal(ExecutionKind::Real, &terminal.public_handle);
        assert!(store
            .mapping(
                ExecutionKind::Simulated,
                &terminal.public_handle,
                "simulation unavailable"
            )
            .unwrap_err()
            .contains("simulation unavailable"));
        store.reserve_start(ExecutionKind::Simulated).unwrap();
        let active = store.bind_started(
            ExecutionKind::Simulated,
            "sim-sidecar".into(),
            "sim-review".into(),
            review(),
        );
        let removed = store
            .forget_mapping(ExecutionKind::Real, &terminal.public_handle)
            .unwrap();
        assert_eq!(removed.review_handle, "real-review");
        assert_eq!(
            store
                .mapping(
                    ExecutionKind::Simulated,
                    &active.public_handle,
                    "unavailable"
                )
                .unwrap()
                .sidecar_id,
            "sim-sidecar"
        );
    }

    #[test]
    fn artifact_admission_errors_map_without_private_cause_details() {
        let error = json!({
            "code": "execution_start_failed",
            "message": "private path and URL",
            "details": { "code": "artifact_not_ready", "artifactCode": "private_cause" }
        })
        .to_string();
        let public = real_start_error(&error);
        assert!(public.contains("artifact_not_ready"));
        assert!(!public.contains("private_cause"));
        assert!(!public.contains("private path"));
    }
}
