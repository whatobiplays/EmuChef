//! Trusted adaptation of retained reviews to simulated and guarded real executions.
//!
//! React supplies only opaque, session-scoped handles. This module owns target
//! and digest revalidation, selects the execution mode inside Tauri, and projects
//! sidecar reports into serial-free, path-safe DTOs.

/// The sidecar owns typed backend execution, while this native boundary retains
/// serialized reviews. Compile the dependency-free classifier from the same
/// source so invalidation cannot reinterpret root requirements independently or
/// expand the public review contract.
#[path = "../../../../crates/emuchef-rust-backend/src/executor/root_requirements.rs"]
mod root_requirements;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use uuid::Uuid;

use crate::adb::{AdbRevalidationError, PlatformToolsReadiness};
use crate::commands::{
    catalog, current_adb_path, redact_absolute_paths, redact_exact_serial, safe_error, AppState,
};
use crate::device_observation::{
    qualify_reconciled_current_with_runtime, CurrentQualification, DeviceQualificationState,
};
use crate::device_qualification::{
    RootQualificationKey, RootQualificationState, RootQualificationStore,
};
use crate::handles::{ReviewedPlanSnapshot, SessionHandles};
use crate::sidecar::SidecarState;

const ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER: &str =
    "Root authority could not be confirmed after earlier device changes may have occurred.";
const CANCELLED_SAFE_BOUNDARY_TEXT: &str = "This action was cancelled at a safe boundary.";

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

#[derive(Clone, Debug)]
struct StoredExecutionReport {
    report: Value,
    runtime: Value,
}

/// Sanitized projection of one retained launch action record.
fn retained_launch_action_value(action: &LaunchActionRecord) -> Value {
    json!({
        "handle": action.action_handle,
        "label": action.label,
    })
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
    latest_terminal_report: Option<StoredExecutionReport>,
    latest_lost: Option<(String, Option<ExecutionMapping>)>,
    launch_actions: HashMap<String, LaunchActionRecord>,
    successful_launches: HashSet<String>,
}

impl ExecutionHandleStore {
    pub fn has_in_flight(&self) -> bool {
        self.start_reserved.is_some() || self.active.is_some()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Return aggregate-only execution retention state for support diagnostics.
    pub fn support_summary(&self, sidecar: &SidecarState) -> Value {
        let latest_terminal = self.latest_terminal.as_ref().and_then(|mapping| {
            let response = runtime_request(
                sidecar,
                "getExecution",
                json!({ "executionId": mapping.sidecar_id }),
            )
            .ok()?;
            let report = response.get("execution")?;
            Some(json!({
                "mode": match mapping.kind {
                    ExecutionKind::Simulated => "simulated",
                    ExecutionKind::Real => "real",
                },
                "status": allowlisted_real_string(
                    report.get("status"),
                    &["succeeded", "succeeded_with_warnings", "failed", "cancelled"],
                    "failed",
                ),
                "completion": if mapping.kind == ExecutionKind::Real {
                    let (
                        identity_failure_count,
                        post_identity_marker,
                        root_failure_count,
                        root_marker,
                    ) = real_projection_facts(report);
                    completion_summary_with_identity_state(
                        report,
                        true,
                        identity_failure_count,
                        post_identity_marker,
                        root_failure_count,
                        root_marker,
                    )
                } else {
                    completion_summary(report, false)
                },
            }))
        });
        json!({
            "startingOrActive": self.has_in_flight(),
            "retainedTerminalCount": usize::from(latest_terminal.is_some()),
            "latestTerminal": latest_terminal,
        })
    }

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

    #[cfg(test)]
    fn mark_terminal(&mut self, kind: ExecutionKind, public_handle: &str) -> bool {
        self.promote_active_to_terminal(kind, public_handle, None)
    }

    fn mark_terminal_with_report(
        &mut self,
        kind: ExecutionKind,
        public_handle: &str,
        report: Value,
        runtime: Value,
    ) -> bool {
        self.promote_active_to_terminal(
            kind,
            public_handle,
            Some(StoredExecutionReport { report, runtime }),
        )
    }

    fn promote_active_to_terminal(
        &mut self,
        kind: ExecutionKind,
        public_handle: &str,
        report: Option<StoredExecutionReport>,
    ) -> bool {
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
            self.latest_terminal_report = report;
            return true;
        }
        false
    }

    fn set_terminal_report_runtime(
        &mut self,
        public_handle: &str,
        runtime: Value,
    ) -> Result<(), String> {
        let is_matching_terminal = self
            .latest_terminal
            .as_ref()
            .is_some_and(|mapping| mapping.public_handle == public_handle);
        if !is_matching_terminal {
            return Err(safe_error(
                "report_unavailable",
                "This execution report is no longer available in this app session.",
            ));
        }
        let Some(report) = self.latest_terminal_report.as_mut() else {
            return Err(safe_error(
                "report_unavailable",
                "This execution report could not be prepared.",
            ));
        };
        report.runtime = runtime;
        Ok(())
    }

    fn terminal_report(&self, public_handle: &str) -> Result<StoredExecutionReport, String> {
        let is_matching_terminal = self
            .latest_terminal
            .as_ref()
            .is_some_and(|mapping| mapping.public_handle == public_handle);
        if !is_matching_terminal {
            return Err(safe_error(
                "report_unavailable",
                "This execution report is no longer available in this app session.",
            ));
        }
        self.latest_terminal_report.clone().ok_or_else(|| {
            safe_error(
                "report_unavailable",
                "This execution report could not be prepared.",
            )
        })
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
            return Some(retained_launch_action_value(existing));
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

    /// Read the retained launch action for one execution without minting a new
    /// one. UI reads are pure projections and never create product state.
    fn retained_launch_action(&self, public_handle: &str) -> Option<Value> {
        if self.successful_launches.contains(public_handle) {
            return None;
        }
        self.launch_actions
            .values()
            .find(|action| action.mapping.public_handle == public_handle)
            .map(retained_launch_action_value)
    }

    /// Whether one terminal transition is already retained for this execution.
    fn terminal_retained(&self, kind: ExecutionKind, public_handle: &str) -> bool {
        self.is_lost(public_handle)
            || self.latest_terminal.as_ref().is_some_and(|mapping| {
                mapping.kind == kind
                    && mapping.public_handle == public_handle
                    && self.latest_terminal_report.is_some()
            })
    }

    fn is_lost(&self, public_handle: &str) -> bool {
        self.latest_lost
            .as_ref()
            .is_some_and(|(handle, _)| handle == public_handle)
    }

    fn lost_mapping(&self, public_handle: &str) -> Option<ExecutionMapping> {
        self.latest_lost
            .as_ref()
            .filter(|(handle, _)| handle == public_handle)
            .and_then(|(_, mapping)| mapping.clone())
    }

    fn mark_lost(&mut self, public_handle: &str, mapping: Option<ExecutionMapping>) {
        self.forget_mapping(ExecutionKind::Real, public_handle);
        self.latest_lost = Some((public_handle.to_string(), mapping));
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
            self.latest_terminal_report = None;
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
    // ActivityGate is always acquired before the execution store. The short
    // lease closes the race with an already-reserved browser handoff.
    let _activity = state.update_activity.reserve_execution_start()?;
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

    validate_review_executable(&review)?;
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
            match execution_session_loss(&error) {
                Some(ExecutionSessionLoss::UnknownExecution) => {
                    state
                        .executions
                        .lock()
                        .map_err(|_| execution_state_error())?
                        .forget_active(ExecutionKind::Simulated, &execution_handle);
                }
                Some(ExecutionSessionLoss::RuntimeSessionLost) => {
                    invalidate_lost_runtime_authority(&state)?;
                }
                None => {
                    return Err(safe_error(
                        "execution_status_failed",
                        "The simulated run status could not be refreshed.",
                    ));
                }
            }
            return Err(safe_error(
                "execution_unavailable",
                "The in-memory simulated run was lost. Return to Review or generate a new review.",
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
        let runtime = serde_json::to_value(state.sidecar.status()).map_err(|_| {
            safe_error(
                "report_serialization_failed",
                "Runtime metadata could not be prepared for the report.",
            )
        })?;
        state
            .executions
            .lock()
            .map_err(|_| execution_state_error())?
            .mark_terminal_with_report(
                ExecutionKind::Simulated,
                &execution_handle,
                report.clone(),
                runtime,
            );
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
    let result = request_simulated_execution_events(
        &state.sidecar,
        &mut executions,
        &execution_handle,
        after_sequence,
    );
    drop(executions);
    if result.is_err() && state.sidecar.runtime_session_was_lost() {
        invalidate_lost_runtime_authority(&state)?;
    }
    result
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
        Err(error) if execution_session_loss(&error).is_some() => {
            match execution_session_loss(&error) {
                Some(ExecutionSessionLoss::RuntimeSessionLost) => executions.reset(),
                Some(ExecutionSessionLoss::UnknownExecution) => {
                    executions.forget_active(ExecutionKind::Simulated, execution_handle);
                }
                None => unreachable!("guard requires a recognized execution session loss"),
            }
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
    );
    let response = match response {
        Ok(response) => response,
        Err(error) if execution_session_loss(&error).is_some() => {
            match execution_session_loss(&error) {
                Some(ExecutionSessionLoss::RuntimeSessionLost) => {
                    invalidate_lost_runtime_authority(&state)?;
                }
                Some(ExecutionSessionLoss::UnknownExecution) => {
                    state
                        .executions
                        .lock()
                        .map_err(|_| execution_state_error())?
                        .forget_active(ExecutionKind::Simulated, &execution_handle);
                }
                None => unreachable!("guard requires a recognized execution session loss"),
            }
            return Err(safe_error(
                "execution_unavailable",
                "The in-memory simulated run was lost. Return to Review or generate a new review.",
            ));
        }
        Err(_) => {
            return Err(safe_error(
                "execution_cancel_failed",
                "Cancellation could not be requested for this simulated run.",
            ));
        }
    };
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

/// Sanitized Platform-Tools readiness authored by the trusted backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlatformToolsStatus {
    NotApplicable,
    Ready,
    NotFound,
    Invalid,
    CheckFailed,
}

/// Informational host-side readiness for attempting guarded real execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutorReadiness {
    NotCompiled,
    Ready,
    Blocked,
    Unknown,
}

/// Immutable, session-local execution capabilities authored by Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCapabilities {
    pub real_execution_compiled: bool,
    pub platform_tools_status: PlatformToolsStatus,
    pub executor_readiness: ExecutorReadiness,
}

impl ExecutionCapabilities {
    fn from_readiness(real_execution_compiled: bool, readiness: PlatformToolsReadiness) -> Self {
        if !real_execution_compiled {
            return Self {
                real_execution_compiled: false,
                platform_tools_status: PlatformToolsStatus::NotApplicable,
                executor_readiness: ExecutorReadiness::NotCompiled,
            };
        }
        let (platform_tools_status, executor_readiness) = match readiness {
            PlatformToolsReadiness::Ready => (PlatformToolsStatus::Ready, ExecutorReadiness::Ready),
            PlatformToolsReadiness::NotFound => {
                (PlatformToolsStatus::NotFound, ExecutorReadiness::Blocked)
            }
            PlatformToolsReadiness::Invalid => {
                (PlatformToolsStatus::Invalid, ExecutorReadiness::Blocked)
            }
            PlatformToolsReadiness::CheckFailed => {
                (PlatformToolsStatus::CheckFailed, ExecutorReadiness::Unknown)
            }
        };
        Self {
            real_execution_compiled: true,
            platform_tools_status,
            executor_readiness,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReadinessGenerations {
    adb_revision: u64,
    runtime_generation: u64,
}

impl ReadinessGenerations {
    fn matches(self, adb_revision: u64, runtime_generation: u64) -> bool {
        self.adb_revision == adb_revision && self.runtime_generation == runtime_generation
    }
}

#[tauri::command]
pub async fn get_execution_capabilities(
    state: State<'_, AppState>,
) -> Result<ExecutionCapabilities, String> {
    if !cfg!(feature = "real-execution") {
        return Ok(ExecutionCapabilities::from_readiness(
            false,
            PlatformToolsReadiness::NotFound,
        ));
    }
    let runtime_generation = state
        .sidecar
        .try_generation()
        .map_err(|_| execution_capabilities_unavailable())?;
    let snapshot = state
        .adb
        .lock()
        .map_err(|_| execution_capabilities_unavailable())?
        .readiness_snapshot();
    let generations = ReadinessGenerations {
        adb_revision: snapshot.adb_revision(),
        runtime_generation,
    };
    let readiness = tauri::async_runtime::spawn_blocking(move || snapshot.evaluate())
        .await
        .map_err(|_| execution_capabilities_unavailable())?;
    let current_runtime_generation = state
        .sidecar
        .try_generation()
        .map_err(|_| execution_capabilities_unavailable())?;
    let current_adb_revision = state
        .adb
        .lock()
        .map_err(|_| execution_capabilities_unavailable())?
        .revision();
    if !generations.matches(current_adb_revision, current_runtime_generation) {
        return Err(execution_capabilities_unavailable());
    }
    Ok(ExecutionCapabilities::from_readiness(true, readiness))
}

fn execution_capabilities_unavailable() -> String {
    safe_error(
        "execution_capabilities_unavailable",
        "Execution capability status is temporarily unavailable.",
    )
}

#[tauri::command]
pub fn start_real_execution(
    app: AppHandle,
    request: Value,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if !cfg!(feature = "real-execution") {
        return Err(safe_error(
            "real_execution_disabled",
            "Real-device execution is not enabled for this build.",
        ));
    }
    let request = parse_real_start_request(request)?;
    let _activity = state.update_activity.reserve_execution_start()?;
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
    let admission_fence = crate::qualification_session::capture_execution_admission_fence(
        &state,
        &request.review_handle,
    );
    match start_real_execution_inner(
        &request.review_handle,
        &state,
        &mut executions,
        admission_fence,
    ) {
        Ok(public) => {
            // The product owns terminal retention for every real execution, so
            // the monitor starts before the start result reaches React.
            if let Some(execution_handle) = public
                .get("executionHandle")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                spawn_real_execution_terminal_monitor(app, execution_handle);
            }
            Ok(public)
        }
        Err(error) => {
            executions.release_start();
            Err(error)
        }
    }
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
    admission_fence: Option<crate::qualification_session::QualificationAdmissionFence>,
) -> Result<Value, String> {
    let review = state
        .handles
        .lock()
        .map_err(|_| session_error())?
        .review(review_handle)?
        .clone();
    validate_review_executable(&review)?;
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

    // The shared final helper requests "listAdbDevices" only after this
    // reviewed Platform-Tools identity has been revalidated.
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
    let platform_tools = PlatformToolsSnapshot {
        adb_path: &adb_path,
        runtime_generation,
        platform_tools_revision,
    };
    start_real_execution_inner_with_admission_fence(
        review_handle,
        Some(state),
        &state.handles,
        &state.root_qualification,
        executions,
        &state.sidecar,
        &platform_tools,
        admission_fence,
    )
}

/// Immutable snapshot of the revalidated Platform-Tools state consumed by the
/// integrated final execution seam. The adb path remains a borrowed reference
/// owned by the caller, so no copy or ownership transfer is introduced.
struct PlatformToolsSnapshot<'a> {
    adb_path: &'a str,
    runtime_generation: u64,
    platform_tools_revision: u64,
}

/// Integrated final execution seam. Inventory reconciliation, target probes,
/// qualification, root evidence, and the one start request all share one
/// runtime requester so request-count tests exercise the actual authority
/// boundary instead of testing the validator in isolation.
fn start_real_execution_inner_with_runtime<R: RuntimeRequester>(
    review_handle: &str,
    qualification_state: Option<&AppState>,
    handles: &Mutex<SessionHandles>,
    root_qualification: &Mutex<RootQualificationStore>,
    executions: &mut ExecutionHandleStore,
    runtime: &R,
    platform_tools: &PlatformToolsSnapshot<'_>,
) -> Result<Value, String> {
    let admission_fence = qualification_state.and_then(|state| {
        crate::qualification_session::capture_execution_admission_fence(state, review_handle)
    });
    start_real_execution_inner_with_admission_fence(
        review_handle,
        qualification_state,
        handles,
        root_qualification,
        executions,
        runtime,
        platform_tools,
        admission_fence,
    )
}

fn start_real_execution_inner_with_admission_fence<R: RuntimeRequester>(
    review_handle: &str,
    qualification_state: Option<&AppState>,
    handles: &Mutex<SessionHandles>,
    root_qualification: &Mutex<RootQualificationStore>,
    executions: &mut ExecutionHandleStore,
    runtime: &R,
    platform_tools: &PlatformToolsSnapshot<'_>,
    admission_fence: Option<crate::qualification_session::QualificationAdmissionFence>,
) -> Result<Value, String> {
    let review = handles
        .lock()
        .map_err(|_| session_error())?
        .review(review_handle)?
        .clone();
    validate_review_executable(&review)?;

    let inventory_request =
        |request_type: &str, payload: Value| runtime_request(runtime, request_type, payload);
    let inventory = inventory_request(
        "listAdbDevices",
        json!({ "adbPath": platform_tools.adb_path }),
    )
    .map_err(|_| device_disconnected())?;
    if let Some(state) = qualification_state {
        crate::commands::reconcile_inventory_snapshot_with_state_and_hook(
            state,
            &inventory,
            platform_tools.runtime_generation,
            platform_tools.platform_tools_revision,
            || {},
        )
        .map_err(|_| device_disconnected())?;
    } else {
        crate::device_qualification::reconcile_inventory_with_context(
            handles,
            root_qualification,
            &inventory,
            platform_tools.runtime_generation,
            platform_tools.platform_tools_revision,
        )
        .map_err(|_| device_disconnected())?;
    }

    // Resolve the retained handle only after fresh reconciliation. A changed
    // transport, cardinality transition, or disappeared record therefore
    // produces a stable stale/disconnected error before any start request.
    let (serial, session_epoch, refreshed_review) = {
        let mut handles = handles.lock().map_err(|_| session_error())?;
        let refreshed = handles.review(review_handle)?.clone();
        let device = handles
            .device(&refreshed.device_handle)
            .map_err(|_| device_disconnected())?;
        if device.state != "available" {
            return Err(device_disconnected());
        }
        (device.serial.clone(), device.session_epoch, refreshed)
    };
    // Keep the active session's immutable product intent available for the
    // remaining final-gate projections. The authoritative probe below may
    // invalidate and close the session before catalog profile matching runs.
    let qualification_device_plan =
        qualification_state.and_then(crate::qualification_session::active_device_plan);
    let observation_failure_target = qualification_state.and_then(|state| {
        crate::qualification_session::capture_device_observation_failure_target(
            state,
            &refreshed_review.device_handle,
            session_epoch,
        )
    });
    let facts = match runtime_request(
        runtime,
        "probeDevice",
        json!({ "adbPath": platform_tools.adb_path, "serial": &serial }),
    ) {
        Ok(facts) => facts,
        Err(_) => {
            if let Some(state) = qualification_state {
                let transition = crate::commands::qualification_transition_lock(state);
                let current = state
                    .handles
                    .lock()
                    .map_err(|_| session_error())?
                    .device(&refreshed_review.device_handle)
                    .is_ok_and(|device| {
                        device.state == "available"
                            && device.serial == serial
                            && device.session_epoch == session_epoch
                    });
                if current {
                    crate::qualification_session::observe_device_observation_failure_in_transition(
                        state,
                        observation_failure_target.clone(),
                    );
                }
                transition.release_and_retry_best_effort();
            }
            return Err(device_disconnected());
        }
    };
    let typed_probe_facts = crate::device_observation::DeviceProbeFacts::decode(&facts);
    let mut final_probe_observation = typed_probe_facts.as_ref().map(|probe_facts| {
        crate::device_observation::SelectedDeviceObservation::new(&refreshed_review.device_handle)
            .with_probe_facts(probe_facts)
            .with_session_epoch(session_epoch)
    });
    {
        let transition = qualification_state.map(crate::commands::qualification_transition_lock);
        let mut handles = handles.lock().map_err(|_| session_error())?;
        let current = handles
            .device(&refreshed_review.device_handle)
            .is_ok_and(|device| {
                device.state == "available"
                    && device.serial == serial
                    && device.session_epoch == session_epoch
            });
        if !current {
            return Err(device_disconnected());
        }
        handles
            .set_facts_for_epoch(
                &refreshed_review.device_handle,
                session_epoch,
                facts.clone(),
            )
            .map_err(|_| device_disconnected())?;
        drop(handles);
        let mut qualification_result = Ok(());
        if let Some(state) = qualification_state {
            match final_probe_observation.as_ref() {
                Some(observation) => {
                    qualification_result =
                        crate::device_observation::commit_selected_observation_in_transition(
                            state,
                            observation.clone(),
                        );
                }
                None => {
                    crate::qualification_session::observe_device_observation_failure_in_transition(
                        state,
                        observation_failure_target.clone(),
                    )
                }
            }
        }
        if let Some(transition) = transition {
            transition.release_and_retry_best_effort();
        }
        qualification_result?;
    }
    validate_target(&refreshed_review.target, &serial, &facts)?;
    validate_plan_digest(&refreshed_review)?;
    validate_retained_byo_inputs(&refreshed_review, &SystemInputReadability)?;

    let mut qualification_request =
        |request_type: &str, payload: Value| runtime_request(runtime, request_type, payload);
    let current = match qualification_state {
        Some(state) => crate::device_observation::qualify_reconciled_current_for_state(
            state,
            platform_tools.adb_path,
            platform_tools.runtime_generation,
            platform_tools.platform_tools_revision,
            Some(&refreshed_review.device_handle),
            observation_failure_target.clone(),
            &mut qualification_request,
        )?,
        None => qualify_reconciled_current_with_runtime(
            handles,
            root_qualification,
            platform_tools.adb_path,
            platform_tools.runtime_generation,
            platform_tools.platform_tools_revision,
            Some(&refreshed_review.device_handle),
            &mut qualification_request,
        )?,
    };
    if current
        .context
        .as_ref()
        .is_none_or(|context| context.session_epoch != session_epoch)
    {
        return Err(device_disconnected());
    }
    let root_granted = if review_requires_root(&refreshed_review) {
        let current_context = current.context.as_ref().ok_or_else(|| {
            safe_error(
                "device_qualification_incomplete",
                "The current device qualification context is incomplete.",
            )
        })?;
        let root_key = RootQualificationKey::from_context(current_context);
        root_qualification
            .lock()
            .map_err(|_| {
                safe_error(
                    "qualification_state_unavailable",
                    "Device qualification state is unavailable.",
                )
            })?
            .get(&root_key)
            .is_some_and(|result| result == RootQualificationState::Granted)
    } else {
        false
    };
    validate_final_qualification(&refreshed_review, &current, root_granted)?;

    let current_profile_id = qualification_device_plan.as_ref().and_then(|device_plan| {
        let state = qualification_state?;
        let catalog = crate::commands::catalog(state).ok()?.internal_payload();
        let match_result = runtime_request(
            runtime,
            "matchDevice",
            json!({ "catalog": catalog, "facts": facts }),
        )
        .ok()?;
        let public = crate::commands::public_match(&match_result, Some(&serial));
        let projection = crate::device_observation::DeviceMatchProjection::decode(&public)?;
        crate::device_observation::matched_profile_id(&projection, device_plan)
    });

    {
        let transition = qualification_state.map(crate::commands::qualification_transition_lock);
        let handles = handles.lock().map_err(|_| session_error())?;
        let device = handles
            .device(&refreshed_review.device_handle)
            .map_err(|_| device_disconnected())?;
        if device.state != "available"
            || device.serial != serial
            || device.session_epoch != session_epoch
            || device.facts_session_epoch != Some(session_epoch)
        {
            return Err(device_disconnected());
        }
        drop(handles);

        let mut qualification_result = Ok(());
        if let (Some(state), Some(observation)) =
            (qualification_state, final_probe_observation.take())
        {
            let mut observation = observation.with_snapshot(&current.snapshot);
            if qualification_device_plan.is_some() {
                if let Some(profile_id) = current_profile_id {
                    observation = observation.with_profile_id(profile_id);
                    qualification_result =
                        crate::device_observation::commit_selected_observation_in_transition(
                            state,
                            observation,
                        );
                } else {
                    // Catalog matching is qualification-only at this point. Its
                    // failure invalidates the attempt but never changes the
                    // already validated product execution decision.
                    crate::qualification_session::observe_device_observation_failure_in_transition(
                        state,
                        observation_failure_target,
                    );
                }
            } else {
                qualification_result =
                    crate::device_observation::commit_selected_observation_in_transition(
                        state,
                        observation,
                    );
            }
        }
        if let Some(transition) = transition {
            transition.release_and_retry_best_effort();
        }
        qualification_result?;
    }

    let start_result = request_real_start(runtime, &refreshed_review)?;
    if let Some(state) = qualification_state {
        // The sidecar start request is complete. Retain product admission and
        // notify qualification as one ordered transition so inventory or a
        // terminal report cannot cross the commit-to-observation boundary.
        let transition = crate::commands::qualification_transition_lock(state);
        let result = (|| {
            let public =
                bind_real_start_result(executions, review_handle, refreshed_review, &start_result)?;
            if let Some(execution_handle) = public.get("executionHandle").and_then(Value::as_str) {
                let mapping = executions.mapping(
                    ExecutionKind::Real,
                    execution_handle,
                    REAL_EXECUTION_UNAVAILABLE,
                )?;
                observe_real_admission(state, admission_fence.as_ref(), &mapping, &mapping.review);
            }
            Ok(public)
        })();
        transition.release_and_retry_best_effort();
        result
    } else {
        bind_real_start_result(executions, review_handle, refreshed_review, &start_result)
    }
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

/// Feed one committed real-execution admission to the active attempt.
///
/// The admission is observed after the product mapping exists and before the
/// start result is published, so a qualification attempt can never bind an
/// execution the product has not started.
fn observe_real_admission(
    state: &AppState,
    admission_fence: Option<&crate::qualification_session::QualificationAdmissionFence>,
    mapping: &ExecutionMapping,
    review: &ReviewedPlanSnapshot,
) {
    if !cfg!(feature = "real-execution") {
        return;
    }
    crate::qualification_session::observe_reserved_real_execution_admission_in_transition(
        state,
        admission_fence,
        crate::qualification_session::ExecutionAdmissionObservation {
            execution_handle: mapping.public_handle.clone(),
            review: crate::qualification_session::review_observation(
                &mapping.review_handle,
                review,
            ),
            device_handle: review.device_handle.clone(),
        },
    );
}

/// One authoritative resolution produced by the product terminal monitor.
#[derive(Clone, Debug)]
enum RealExecutionMonitorEvent {
    /// The monitor retained one terminal execution transition.
    Terminal {
        observation: crate::qualification_session::TerminalExecutionObservation,
    },
    /// The runtime session that owned the execution is gone, so the execution
    /// was resolved through the existing authoritative loss semantics.
    Lost { execution_handle: String },
}

/// Operator-facing description of one monitor resolution. It never contains a
/// device serial, filesystem path, or report content.
fn monitor_resolution_message(event: &RealExecutionMonitorEvent) -> String {
    match event {
        RealExecutionMonitorEvent::Terminal { observation } => format!(
            "real execution {} retained an authoritative terminal transition",
            observation.execution_handle
        ),
        RealExecutionMonitorEvent::Lost { execution_handle } => format!(
            "real execution {execution_handle} was resolved through its runtime session loss"
        ),
    }
}

/// Interval between authoritative status polls while an execution runs.
const REAL_EXECUTION_MONITOR_INTERVAL: Duration = Duration::from_millis(1_000);
/// Longest backoff applied after repeated transient status failures.
const REAL_EXECUTION_MONITOR_MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Start the product-owned terminal monitor for one real execution.
///
/// Every guarded real execution gets exactly one monitor, whether or not a
/// qualification attempt is active or qualification mode is enabled: the
/// product must retain its own terminal transition (report bytes, device and
/// root authority invalidation, launch action) before an execution can be
/// considered finished. The monitor never abandons an execution after a
/// transient failure. When the runtime session that owned the execution is
/// gone it resolves the execution through the same authoritative loss
/// semantics every other execution path uses, so an execution is never left
/// active merely because observation stopped.
fn spawn_real_execution_terminal_monitor(app: AppHandle, execution_handle: String) {
    if !cfg!(feature = "real-execution") {
        return;
    }
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let resolution =
            monitor_real_terminal(&execution_handle, &state, &state.sidecar, &mut |interval| {
                std::thread::sleep(interval)
            })
            .map(|event| monitor_resolution_message(&event));
        #[cfg(debug_assertions)]
        if let Some(message) = resolution {
            eprintln!("{message}");
        }
        #[cfg(not(debug_assertions))]
        let _ = resolution;
    });
}

/// Run one terminal monitor until the execution is resolved.
///
/// Returns the authoritative resolution, or `None` when the execution was
/// already terminal or its mapping was dropped, so the monitor only ever
/// reports a transition it actually committed. Transient runtime failures back
/// off and retry forever: giving up would leave the execution store active
/// without any authority tracking it.
fn monitor_real_terminal<R, W>(
    execution_handle: &str,
    state: &AppState,
    runtime: &R,
    wait: &mut W,
) -> Option<RealExecutionMonitorEvent>
where
    R: RuntimeRequester,
    W: FnMut(Duration),
{
    monitor_real_terminal_with_loss_recovery(
        execution_handle,
        state,
        runtime,
        wait,
        &mut recover_from_real_execution_loss,
    )
}

/// Injectable loss-recovery boundary so retry behavior remains deterministic
/// under test. Production always supplies the authoritative product recovery.
fn monitor_real_terminal_with_loss_recovery<R, W, F>(
    execution_handle: &str,
    state: &AppState,
    runtime: &R,
    wait: &mut W,
    recover_loss: &mut F,
) -> Option<RealExecutionMonitorEvent>
where
    R: RuntimeRequester,
    W: FnMut(Duration),
    F: FnMut(&AppState, &str, &str) -> Result<(), String>,
{
    let mut interval = REAL_EXECUTION_MONITOR_INTERVAL;
    let mut pending_loss: Option<String> = None;
    let mut pending_terminal: Option<(ExecutionMapping, Value, String)> = None;
    loop {
        if let Some(error) = pending_loss.as_deref() {
            match recover_loss(state, execution_handle, error) {
                Ok(()) => {
                    return Some(RealExecutionMonitorEvent::Lost {
                        execution_handle: execution_handle.to_string(),
                    });
                }
                Err(_) => {
                    interval = (interval * 2).min(REAL_EXECUTION_MONITOR_MAX_BACKOFF);
                    wait(interval);
                    continue;
                }
            }
        }
        if let Some((mapping, report, observed_at)) = pending_terminal.as_ref() {
            match retain_terminal_real_execution(state, mapping, report, observed_at) {
                Ok(Some(event)) => return Some(event),
                Ok(None) => return None,
                Err(_) if state.executions.is_poisoned() => {
                    pending_terminal = None;
                    pending_loss = Some(runtime_session_lost_error());
                    continue;
                }
                Err(_) => {
                    interval = (interval * 2).min(REAL_EXECUTION_MONITOR_MAX_BACKOFF);
                    wait(interval);
                    continue;
                }
            }
        }
        let mapping = match state.executions.lock() {
            Ok(store) => {
                if store.terminal_retained(ExecutionKind::Real, execution_handle) {
                    return None;
                }
                match store.mapping(
                    ExecutionKind::Real,
                    execution_handle,
                    REAL_EXECUTION_UNAVAILABLE,
                ) {
                    Ok(mapping) => mapping,
                    Err(_) => {
                        pending_loss = Some(unknown_execution_error());
                        continue;
                    }
                }
            }
            Err(_) => {
                pending_loss = Some(runtime_session_lost_error());
                continue;
            }
        };
        match runtime_request(
            runtime,
            "getExecution",
            json!({ "executionId": mapping.sidecar_id }),
        ) {
            Ok(response) => {
                let report = response.get("execution").cloned().unwrap_or(Value::Null);
                if is_terminal_status(report.get("status").and_then(Value::as_str)) {
                    match crate::qualification_session::current_timestamp() {
                        Ok(observed_at) => {
                            pending_terminal = Some((mapping, report, observed_at));
                            interval = REAL_EXECUTION_MONITOR_INTERVAL;
                            continue;
                        }
                        Err(_) => {
                            interval = (interval * 2).min(REAL_EXECUTION_MONITOR_MAX_BACKOFF);
                        }
                    }
                } else {
                    interval = REAL_EXECUTION_MONITOR_INTERVAL;
                }
            }
            Err(error) if execution_session_loss(&error).is_some() => {
                pending_loss = Some(error);
            }
            Err(_) => {
                interval = (interval * 2).min(REAL_EXECUTION_MONITOR_MAX_BACKOFF);
            }
        }
        wait(interval);
    }
}

/// Retain one authoritative terminal transition exactly once, then feed the
/// committed typed result to the active qualification attempt.
///
/// The qualification attempt receives the transition only after the product
/// terminal state, authority invalidation, launch action, and report bytes are
/// retained, so it can never observe a terminal state the product has not
/// committed.
fn retain_terminal_real_execution(
    state: &AppState,
    mapping: &ExecutionMapping,
    report: &Value,
    observed_at: &str,
) -> Result<Option<RealExecutionMonitorEvent>, String> {
    let report_runtime = serde_json::to_value(state.sidecar.status()).map_err(|_| {
        safe_error(
            "report_serialization_failed",
            "The sanitized execution runtime metadata could not be serialized.",
        )
    })?;
    let report_bytes =
        production_execution_report_bytes_for(mapping, report, report_runtime.clone())?;
    let identity_failed = report_has_identity_failure(report);
    let root_failed = !identity_failed && report_has_root_authority_failure(report);
    // Execution state precedes the transition gate everywhere it is needed.
    // The status/report sidecar work above is complete, so the gate covers
    // only product retention and the matching qualification notification.
    let mut executions = state.executions.lock().map_err(|_| {
        safe_error(
            "execution_state_unavailable",
            "Real-device execution state is unavailable.",
        )
    })?;
    let transition = crate::commands::qualification_transition_lock(state);
    if executions.terminal_retained(ExecutionKind::Real, &mapping.public_handle) {
        return Ok(None);
    }
    if identity_failed {
        invalidate_identity_terminal_authority(&state.handles, &state.root_qualification, mapping)?;
    } else if root_failed {
        invalidate_root_terminal_authority(&state.handles, &state.root_qualification, mapping)?;
    }
    let launch_expected = eligible_launch_label(mapping, report).is_some();
    let launch_action = executions.launch_action(mapping, report);
    if launch_expected && launch_action.is_none() {
        return Err(safe_error(
            "launch_action_retention_failed",
            "The completed execution action could not be retained.",
        ));
    }
    if !executions.mark_terminal_with_report(
        ExecutionKind::Real,
        &mapping.public_handle,
        report.clone(),
        report_runtime,
    ) {
        return Err(safe_error(
            "execution_state_unavailable",
            "Real-device execution state is unavailable.",
        ));
    }
    let observation = crate::qualification_session::TerminalExecutionObservation {
        execution_handle: mapping.public_handle.clone(),
        status: report
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_string),
        observed_at: observed_at.to_string(),
        report_available: true,
        report_bytes: Some(report_bytes),
        authority_invalidated: identity_failed || root_failed,
    };
    crate::qualification_session::observe_in_transition(
        state,
        crate::qualification_session::QualificationLifecycleObservation::RealExecutionTerminal(
            Box::new(observation.clone()),
        ),
    );
    drop(executions);
    if transition.release_and_retry().is_err() {
        eprintln!(
            "The retained execution terminal is safe; qualification candidate materialization remains pending for status recovery."
        );
    }
    Ok(Some(RealExecutionMonitorEvent::Terminal { observation }))
}

/// Project one real execution for the client.
///
/// Terminal retention, device and root authority invalidation, launch-action
/// creation, report retention, and qualification notification are owned by the
/// product terminal monitor. This command only publishes retained product
/// state, so polling it can never advance product or qualification lifecycle
/// state. A runtime session that is already gone is reported as unavailable
/// and is resolved by the monitor through the authoritative loss semantics.
#[tauri::command]
pub fn get_real_execution(
    execution_handle: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    get_real_execution_inner_with_runtime(&execution_handle, &state.executions, &state.sidecar)
}

/// Retrieve one real execution report as a pure projection. The runtime
/// requester is injectable so deterministic tests exercise the same mapping and
/// projection path as the Tauri command without starting a native process.
fn get_real_execution_inner_with_runtime<R>(
    execution_handle: &str,
    executions: &Mutex<ExecutionHandleStore>,
    runtime: &R,
) -> Result<Value, String>
where
    R: RuntimeRequester,
{
    let mapping = {
        let executions = executions
            .lock()
            .map_err(|_| real_execution_state_error())?;
        if executions.is_lost(execution_handle) {
            return Ok(lost_real_execution_snapshot(
                execution_handle,
                executions.lost_mapping(execution_handle).as_ref(),
            ));
        }
        if let Some(snapshot) = retained_real_execution_snapshot(&executions, execution_handle) {
            return Ok(snapshot);
        }
        executions.mapping(
            ExecutionKind::Real,
            execution_handle,
            REAL_EXECUTION_UNAVAILABLE,
        )?
    };
    let response = match runtime_request(
        runtime,
        "getExecution",
        json!({ "executionId": mapping.sidecar_id }),
    ) {
        Ok(response) => response,
        Err(error) if execution_session_loss(&error).is_some() => {
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
    let executions = executions
        .lock()
        .map_err(|_| real_execution_state_error())?;
    if executions.is_lost(execution_handle) {
        return Ok(lost_real_execution_snapshot(
            execution_handle,
            executions.lost_mapping(execution_handle).as_ref(),
        ));
    }
    if let Some(snapshot) = retained_real_execution_snapshot(&executions, execution_handle) {
        return Ok(snapshot);
    }
    let mut visible_report = report.clone();
    if is_terminal_status(report.get("status").and_then(Value::as_str)) {
        visible_report["status"] = Value::String("running".to_string());
        visible_report["finishedAt"] = Value::Null;
    }
    let mut public = project_real_snapshot(&mapping, &visible_report);
    public["launchAction"] = executions
        .retained_launch_action(execution_handle)
        .unwrap_or(Value::Null);
    let launch_action_present = public
        .get("launchAction")
        .and_then(Value::as_object)
        .is_some();
    attach_terminal_policy(&mut public, launch_action_present);
    Ok(public)
}

fn retained_real_execution_snapshot(
    executions: &ExecutionHandleStore,
    execution_handle: &str,
) -> Option<Value> {
    if !executions.terminal_retained(ExecutionKind::Real, execution_handle) {
        return None;
    }
    let mapping = executions
        .mapping(
            ExecutionKind::Real,
            execution_handle,
            REAL_EXECUTION_UNAVAILABLE,
        )
        .ok()?;
    let stored = executions.terminal_report(execution_handle).ok()?;
    let mut public = project_real_snapshot(&mapping, &stored.report);
    public["launchAction"] = executions
        .retained_launch_action(execution_handle)
        .unwrap_or(Value::Null);
    let launch_action_present = public
        .get("launchAction")
        .and_then(Value::as_object)
        .is_some();
    attach_terminal_policy(&mut public, launch_action_present);
    Some(public)
}

fn lost_real_execution_snapshot(
    execution_handle: &str,
    mapping: Option<&ExecutionMapping>,
) -> Value {
    let report = json!({
        "status": "failed",
        "errors": [{
            "code": "execution_unavailable",
            "message": REAL_EXECUTION_UNAVAILABLE,
        }],
        "warnings": [],
        "recipes": [],
    });
    if let Some(mapping) = mapping {
        let mut public = project_real_snapshot(mapping, &report);
        public["executionHandle"] = Value::String(execution_handle.to_string());
        public["launchAction"] = Value::Null;
        return public;
    }
    json!({
        "executionHandle": execution_handle,
        "reviewHandle": "",
        "simulated": false,
        "verificationScope": "real_device",
        "target": { "label": "Connected Android device" },
        "status": "failed",
        "startedAt": null,
        "finishedAt": null,
        "latestSequence": 0,
        "terminal": true,
        "recipes": [],
        "warnings": [],
        "errors": [{
            "message": REAL_EXECUTION_UNAVAILABLE,
            "remediation": {
                "kind": "reconnect_device",
                "title": "Reconnect the device",
                "message": "Reconnect the device, then create and review a fresh plan before another execution.",
            },
        }],
        "completion": {
            "classification": "failed",
            "counts": { "total": 0, "completed": 0, "skipped": 0, "blocked": 0, "failed": 0, "cancelled": 0, "pending": 0 },
            "warningCount": 0,
            "partialChangesPossible": true,
            "features": [],
        },
        "progress": { "currentFeature": null, "currentAction": null },
        "launchAction": null,
        "terminalPolicy": {
            "authorityInvalidated": true,
            "recoveryState": "fresh_review_required",
            "partialChangePresentation": "indeterminate",
        "availableControls": ["fresh_workflow"],
        },
    })
}

#[tauri::command]
pub fn get_real_execution_events(
    execution_handle: String,
    after_sequence: u64,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    get_real_execution_events_inner_with_runtime(
        &execution_handle,
        after_sequence,
        &state.executions,
        &state.sidecar,
    )
}

/// Poll one real execution's events, using terminal state already retained by
/// the product monitor as authoritative fallback. If the runtime remains
/// available, unread events are still projected before that terminal state is
/// applied to the batch.
fn get_real_execution_events_inner_with_runtime<R>(
    execution_handle: &str,
    after_sequence: u64,
    executions: &Mutex<ExecutionHandleStore>,
    runtime: &R,
) -> Result<Value, String>
where
    R: RuntimeRequester,
{
    let retained_before_request = retained_real_event_batch(executions, execution_handle)?;
    let mapping = match executions
        .lock()
        .map_err(|_| real_execution_state_error())
        .and_then(|executions| {
            executions.mapping(
                ExecutionKind::Real,
                execution_handle,
                REAL_EXECUTION_UNAVAILABLE,
            )
        }) {
        Ok(mapping) => mapping,
        Err(_) if retained_before_request.is_some() => {
            return Ok(retained_before_request.expect("retained state was checked above"));
        }
        Err(error) => return Err(error),
    };
    let response = runtime_request(
        runtime,
        "getExecutionEvents",
        json!({
            "executionId": mapping.sidecar_id,
            "afterSequence": after_sequence,
        }),
    );
    let retained_after_request = match retained_real_event_batch(executions, execution_handle) {
        Ok(Some(batch)) => Some(batch),
        Ok(None) => retained_before_request,
        Err(_) if retained_before_request.is_some() => retained_before_request,
        Err(error) => return Err(error),
    };
    if let Some(retained_batch) = retained_after_request {
        return match response {
            Ok(response) => Ok(retained_terminal_event_batch_with_runtime_events(
                &mapping,
                &response,
                &retained_batch,
            )),
            Err(_) => Ok(retained_batch),
        };
    }
    match response {
        Ok(response) => Ok(project_real_event_batch(&mapping, &response)),
        Err(error) if execution_session_loss(&error).is_some() => Err(safe_error(
            "execution_unavailable",
            REAL_EXECUTION_UNAVAILABLE,
        )),
        Err(_) => Err(safe_error(
            "execution_status_failed",
            "Incremental real-device progress could not be refreshed.",
        )),
    }
}

fn retained_real_event_batch(
    executions: &Mutex<ExecutionHandleStore>,
    execution_handle: &str,
) -> Result<Option<Value>, String> {
    let executions = executions
        .lock()
        .map_err(|_| real_execution_state_error())?;
    if executions.is_lost(execution_handle) {
        let response = json!({
            "events": [],
            "latestSequence": 0,
            "terminal": true,
        });
        let batch = executions
            .lost_mapping(execution_handle)
            .map(|mapping| project_real_event_batch(&mapping, &response))
            .unwrap_or_else(|| {
                json!({
                    "executionHandle": execution_handle,
                    "events": [],
                    "latestSequence": 0,
                    "terminal": true,
                })
            });
        return Ok(Some(batch));
    }
    if !executions.terminal_retained(ExecutionKind::Real, execution_handle) {
        return Ok(None);
    }
    let mapping = executions.mapping(
        ExecutionKind::Real,
        execution_handle,
        REAL_EXECUTION_UNAVAILABLE,
    )?;
    let stored = executions.terminal_report(execution_handle)?;
    let response = json!({
        "events": [],
        "latestSequence": stored.report.get("latestSequence").and_then(Value::as_u64).unwrap_or(0),
        "terminal": true,
    });
    Ok(Some(project_real_event_batch(&mapping, &response)))
}

fn retained_terminal_event_batch_with_runtime_events(
    mapping: &ExecutionMapping,
    response: &Value,
    retained_batch: &Value,
) -> Value {
    let mut batch = project_real_event_batch(mapping, response);
    batch["terminal"] = Value::Bool(true);
    let latest_sequence = batch["latestSequence"].as_u64().unwrap_or_default().max(
        retained_batch["latestSequence"]
            .as_u64()
            .unwrap_or_default(),
    );
    batch["latestSequence"] = json!(latest_sequence);
    batch
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
        Err(error) if execution_session_loss(&error).is_some() => {
            recover_from_real_execution_loss(&state, &execution_handle, &error)?;
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

/// Serialize the exact sanitized report retained for one terminal production
/// execution. Normal export and qualification capture call this helper so the
/// report projection, redaction, and byte formatting remain one authority.
pub(crate) fn production_execution_report_bytes(
    store: &ExecutionHandleStore,
    execution_handle: &str,
) -> Result<Vec<u8>, String> {
    let mapping = store.mapping_any(execution_handle)?;
    let stored = store.terminal_report(execution_handle)?;
    production_execution_report_bytes_for(&mapping, &stored.report, stored.runtime)
}

fn production_execution_report_bytes_for(
    mapping: &ExecutionMapping,
    report: &Value,
    runtime: Value,
) -> Result<Vec<u8>, String> {
    if !is_terminal_status(report.get("status").and_then(Value::as_str)) {
        return Err(safe_error(
            "report_not_terminal",
            "Wait for execution to finish before exporting its report.",
        ));
    }
    let public = match mapping.kind {
        ExecutionKind::Simulated => project_snapshot(mapping, report),
        ExecutionKind::Real => project_real_snapshot(mapping, report),
    };
    let document = execution_report_document(mapping, report, &public, runtime);
    let mut serialized = serde_json::to_string_pretty(&document).map_err(|_| {
        safe_error(
            "report_serialization_failed",
            "The sanitized execution report could not be serialized.",
        )
    })?;
    serialized.push('\n');
    Ok(serialized.into_bytes())
}

#[tauri::command]
pub async fn export_execution_report(
    app: AppHandle,
    execution_handle: String,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let serialized = {
        let executions = state
            .executions
            .lock()
            .map_err(|_| real_execution_state_error())?;
        production_execution_report_bytes(&executions, &execution_handle)?
    };

    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
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

fn recover_from_real_execution_loss(
    state: &AppState,
    public_handle: &str,
    error: &str,
) -> Result<(), String> {
    let Some(loss) = execution_session_loss(error) else {
        return Ok(());
    };
    let mut executions = recover_poisoned_lock(&state.executions);
    let transition = crate::commands::qualification_transition_lock(state);
    let mapping = executions
        .mapping(
            ExecutionKind::Real,
            public_handle,
            REAL_EXECUTION_UNAVAILABLE,
        )
        .ok();
    let retained_mapping = mapping.clone();
    match loss {
        ExecutionSessionLoss::RuntimeSessionLost => {
            executions.reset();
            recover_poisoned_lock(&state.handles)
                .invalidate_runtime_authority_preserving_identities();
            recover_poisoned_lock(&state.root_qualification).invalidate();
        }
        ExecutionSessionLoss::UnknownExecution => {
            let removed = executions.forget_mapping(ExecutionKind::Real, public_handle);
            if let Some(mapping) = removed {
                recover_poisoned_lock(&state.handles)
                    .invalidate_review(&mapping.review_handle, "review_stale");
            }
        }
    }
    executions.mark_lost(public_handle, retained_mapping);
    drop(executions);
    // The product no longer holds any authority for this execution, so an
    // active attempt fails closed instead of waiting for evidence that can
    // never arrive.
    crate::qualification_session::observe_in_transition(
        state,
        crate::qualification_session::QualificationLifecycleObservation::RealExecutionLost {
            execution_handle: public_handle.to_string(),
        },
    );
    transition.release_and_retry_best_effort();
    Ok(())
}

/// Discard native authority after the shared runtime session is lost. Real
/// execution loss uses the ordered qualification-aware path above; simulated
/// execution callers use this product-only cleanup.
fn invalidate_lost_runtime_authority(state: &AppState) -> Result<(), String> {
    recover_poisoned_lock(&state.executions).reset();
    recover_poisoned_lock(&state.handles).invalidate_runtime_authority_preserving_identities();
    recover_poisoned_lock(&state.root_qualification).invalidate();
    Ok(())
}

/// Discard all native authority derived from a sidecar process generation that
/// can no longer answer requests. Portable user intent is owned elsewhere and
/// remains intact; executions, launch actions, reviews, device facts, and root
/// qualification evidence cannot survive the lost in-memory runtime session.
fn recover_poisoned_lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            mutex.clear_poison();
            guard
        }
    }
}

fn runtime_session_lost_error() -> String {
    safe_error(
        "runtime_session_lost",
        "The execution runtime session is no longer available.",
    )
}

fn unknown_execution_error() -> String {
    safe_error(
        "unknown_execution",
        "The execution runtime no longer retains this execution.",
    )
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

/// Rejects retained plans whose backend-authored review cannot faithfully
/// describe every action. The UI cannot override this trusted decision.
fn validate_review_executable(review: &ReviewedPlanSnapshot) -> Result<(), String> {
    if review.response.pointer("/review/canExecute") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    Err(safe_error(
        "review_not_executable",
        "This plan cannot be executed safely. Generate a new review after updating EmuChef or the setup catalog.",
    ))
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

fn review_requires_root(review: &ReviewedPlanSnapshot) -> bool {
    review
        .response
        .get("plan")
        .is_some_and(root_requirements::reviewed_plan_requires_root_json)
}

fn validate_final_qualification(
    review: &ReviewedPlanSnapshot,
    current: &CurrentQualification,
    root_granted: bool,
) -> Result<(), String> {
    match current.snapshot.state {
        DeviceQualificationState::Supported => {}
        DeviceQualificationState::Unsupported => {
            return Err(safe_error(
                "device_qualification_unsupported",
                "The current device does not meet the supported qualification requirements.",
            ));
        }
        _ => {
            return Err(safe_error(
                "device_qualification_incomplete",
                "The current device could not be fully qualified for real execution.",
            ));
        }
    }
    let current_context = current.context.as_ref().ok_or_else(|| {
        safe_error(
            "device_qualification_incomplete",
            "The current device qualification context is incomplete.",
        )
    })?;
    if review.qualification_context.as_ref() != Some(current_context) {
        return Err(stale_review(
            "The device qualification changed after review. Generate a fresh review.",
        ));
    }
    if review_requires_root(review) && !root_granted {
        return Err(safe_error(
            "root_qualification_required",
            "Check root access again for this current device session before applying the plan.",
        ));
    }
    Ok(())
}

pub(crate) fn canonical_json_digest(value: &Value) -> Result<String, serde_json::Error> {
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
    let (identity_failure_count, post_identity_marker, root_failure_count, root_marker) =
        real_projection_facts(report);
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
    public["warnings"] = Value::Array(project_real_issues(mapping, report.get("warnings")));
    public["errors"] = Value::Array(project_real_issues(mapping, report.get("errors")));

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
    public["completion"] = completion_summary_with_identity_state(
        &public,
        true,
        identity_failure_count,
        post_identity_marker,
        root_failure_count,
        root_marker,
    );
    attach_terminal_policy(&mut public, false);
    sanitize_real_projection(&mut public, exact_serial);
    public
}

/// Project one development Phase 6D.6 UI-smoke terminal report through the
/// same production real-execution projection used for retained executions.
///
/// The mapping is deliberately authority-free: it owns no sidecar execution,
/// device target, review, or session authority. All recipes, step states,
/// statuses, and issues come from the caller-provided fixed terminal report,
/// and the returned DTO carries the same production terminal policy that a
/// normal retained real execution would present.
pub(crate) fn project_phase6d6_terminal(public_handle: &str, report: Value) -> Value {
    let mapping = ExecutionMapping {
        kind: ExecutionKind::Real,
        public_handle: public_handle.to_string(),
        sidecar_id: "phase6d6-ui-smoke".to_string(),
        review_handle: "phase6d6-ui-smoke-review".to_string(),
        review: ReviewedPlanSnapshot {
            response: json!({ "plan": { "recipes": [], "steps": [] } }),
            target: json!({}),
            catalog_identity: Value::Null,
            catalog_digest: String::new(),
            plan_digest: String::new(),
            device_handle: String::new(),
            qualification_context: None,
            platform_tools_identity: None,
            created: std::time::Instant::now(),
            last_access: std::time::Instant::now(),
        },
    };
    project_real_snapshot(&mapping, &report)
}

/// Attach the production-authored terminal presentation policy to one public
/// real-execution snapshot.
///
/// This is the single production source for authority-invalidation state,
/// recovery-state identity, partial-change presentation classification, and
/// available terminal controls. Development UI-smoke capture copies these
/// values from the projected DTO instead of reconstructing a second catalog.
pub(crate) fn attach_terminal_policy(public: &mut Value, launch_action_present: bool) {
    if !is_terminal_status(public.get("status").and_then(Value::as_str)) {
        return;
    }
    let mut policy = terminal_policy(public);
    if launch_action_present {
        if let Some(controls) = policy
            .get_mut("availableControls")
            .and_then(Value::as_array_mut)
        {
            controls.push(Value::String("launch_configured_app".to_string()));
        }
    }
    public["terminalPolicy"] = policy;
    if public.get("status").and_then(Value::as_str) == Some("cancelled") {
        public["cancellation"] = json!({
            "title": "Execution cancelled",
            "message": CANCELLED_SAFE_BOUNDARY_TEXT,
            "remediation": {
                "kind": "generate_fresh_plan",
                "title": "Execution cancelled",
                "message": "Review the retained results, then create and review a fresh plan before another execution. The old execution cannot resume.",
            },
        });
    }
}

/// Derive the terminal presentation policy from one already-projected public
/// snapshot. Values are authored here, once, for both the normal real-terminal
/// UI and the Phase 6D.6 qualification bridge.
fn terminal_policy(snapshot: &Value) -> Value {
    let status = snapshot
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("running");
    let completion = snapshot.get("completion");
    let completed = completion
        .and_then(|value| value.get("counts"))
        .and_then(|value| value.get("completed"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let partial_possible = completion
        .and_then(|value| value.get("partialChangesPossible"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let partial_presentation = if !partial_possible {
        "none"
    } else if completed > 0 {
        "possible_partial_change"
    } else {
        "indeterminate"
    };
    let (recovery_state, authority_invalidated) = if status == "cancelled" {
        ("fresh_review_required", false)
    } else if status == "failed" {
        let remediation_kind = snapshot
            .get("errors")
            .and_then(Value::as_array)
            .and_then(|issues| issues.first())
            .and_then(|issue| issue.get("remediation"))
            .and_then(|remediation| remediation.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("view_report");
        match remediation_kind {
            "reconnect_device" => ("requalification_required", true),
            "requalify_root" => ("root_requalification_required", true),
            "generate_fresh_plan" | "repair_platform_tools" => {
                ("fresh_qualification_required", true)
            }
            _ => ("fresh_review_required", false),
        }
    } else {
        ("none", false)
    };
    let mut controls = vec!["export_report", "fresh_workflow"];
    if matches!(status, "failed" | "cancelled" | "succeeded_with_warnings") {
        controls.push("repair_setup");
    }
    json!({
        "authorityInvalidated": authority_invalidated,
        "recoveryState": recovery_state,
        "partialChangePresentation": partial_presentation,
        "availableControls": controls,
    })
}

fn real_projection_facts(report: &Value) -> (u64, bool, u64, bool) {
    let mut failures = 0;
    let mut post_operation_marker = false;
    let mut root_failures = 0;
    let mut root_marker = false;
    for field in ["errors", "warnings"] {
        for issue in report
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match issue.get("code").and_then(Value::as_str) {
                Some("device_identity_changed" | "device_identity_unverified") => {
                    failures += 1;
                    if issue.get("message").and_then(Value::as_str)
                        == Some(POST_OPERATION_IDENTITY_FAILURE_MARKER)
                    {
                        post_operation_marker = true;
                    }
                }
                Some("root_authority_revoked" | "root_authority_unverified") => {
                    root_failures += 1;
                    if issue.get("message").and_then(Value::as_str)
                        == Some(ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER)
                    {
                        root_marker = true;
                    }
                }
                _ => {}
            }
        }
    }
    (failures, post_operation_marker, root_failures, root_marker)
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

fn project_real_issues(mapping: &ExecutionMapping, value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|issue| project_real_issue(mapping, issue))
        .collect()
}

fn project_real_issue(mapping: &ExecutionMapping, issue: &Value) -> Value {
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
        "operation_timed_out" => (internal, "A device operation timed out."),
        "device_storage_exhausted" => (internal, "The device ran out of storage during execution."),
        "device_offline" => (
            internal,
            "The reviewed device went offline during execution.",
        ),
        "device_unauthorized" => (
            internal,
            "The intended reviewed device needs USB debugging authorization.",
        ),
        "device_disconnected" => (
            internal,
            "The intended reviewed device disconnected or could not be found.",
        ),
        "adb_server_unavailable" => (
            internal,
            "The local ADB/Platform-Tools service was unavailable.",
        ),
        "device_identity_changed" => (
            internal,
            "The reviewed device identity changed during execution.",
        ),
        "device_identity_unverified" => (
            internal,
            "The reviewed device identity could not be verified safely.",
        ),
        "root_authority_revoked" => (internal, "Root access was revoked during execution."),
        "root_authority_unverified" => (
            internal,
            "EmuChef could not safely confirm continued root access.",
        ),
        "device_transport_lost" => (internal, "The device connection was lost during execution."),
        "step_execution_failed" => (internal, "A device operation failed."),
        "optional_permission_failed" => (internal, "An optional permission action failed."),
        "execution_worker_panicked" => (internal, "The execution worker stopped unexpectedly."),
        _ => ("execution_issue", "Execution reported an issue."),
    };
    let message = issue_action_context(mapping, issue).map_or_else(
        || message.to_string(),
        |(feature, action)| format!("{action} in {feature} did not complete. {message}"),
    );
    json!({
        "message": message,
        "remediation": remediation_for_code(code),
    })
}

/// Resolves opaque executor identity only inside trusted code and returns
/// authored presentation text. Technical identifiers never enter the DTO.
fn issue_action_context(mapping: &ExecutionMapping, issue: &Value) -> Option<(String, String)> {
    let step_id = issue.get("stepId").and_then(Value::as_str)?;
    action_context_for_step_id(mapping, step_id)
}

fn action_context_for_step_id(
    mapping: &ExecutionMapping,
    step_id: &str,
) -> Option<(String, String)> {
    let plan = mapping.review.response.get("plan")?;
    let step = plan
        .get("steps")
        .and_then(Value::as_array)?
        .iter()
        .find(|step| step.get("id").and_then(Value::as_str) == Some(step_id))?;
    let recipe_id = step.get("recipe_ref").and_then(Value::as_str)?;
    let feature = plan
        .get("recipes")
        .and_then(Value::as_array)?
        .iter()
        .find(|recipe| recipe.get("id").and_then(Value::as_str) == Some(recipe_id))?
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    let action = step
        .get("note")
        .or_else(|| step.get("name"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    let exact_serial = mapping
        .review
        .target
        .get("serial")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some((
        sanitize_text(feature, exact_serial),
        sanitize_text(action, exact_serial),
    ))
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
        | "operation_timed_out"
        | "device_storage_exhausted"
        | "step_execution_failed"
        | "optional_permission_failed" => (
            "generate_fresh_plan",
            if code == "device_storage_exhausted" {
                "Free storage and start again"
            } else {
                "Repair and retry"
            },
            if code == "device_storage_exhausted" {
                "Free device storage, complete fresh qualification, then generate and review a fresh plan before starting a new execution. The old execution cannot resume."
            } else {
                "Resolve the reported feature problem, then generate and review a fresh plan."
            },
        ),
        "device_offline" | "device_unauthorized" | "device_disconnected" | "device_transport_lost" => (
            "reconnect_device",
            "Reconnect and requalify",
            "Reconnect or authorize the intended reviewed device, then complete fresh qualification and generate and review a fresh plan before another real run. Reconnecting does not resume the old execution.",
        ),
        "device_identity_changed" | "device_identity_unverified" => (
            "reconnect_device",
            "Reconnect and requalify",
            "Reconnect the intended device, complete a fresh identity probe and qualification, then generate and review a fresh plan before another real run. The old execution cannot be resumed.",
        ),
        "root_authority_revoked" | "root_authority_unverified" => (
            "requalify_root",
            "Requalify root access",
            "Complete fresh root qualification, generate a fresh plan, review it, and start a new execution. The old execution cannot be resumed.",
        ),
        "adb_server_unavailable" => (
            "repair_platform_tools",
            "Repair local ADB",
            "Restore the local ADB/Platform-Tools service, then complete fresh qualification and generate and review a fresh plan before another real run. Repairing the service does not resume the old execution.",
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
    let warnings = project_issues(mapping, report.get("warnings"), exact_serial);
    let errors = project_issues(mapping, report.get("errors"), exact_serial);
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
    public["progress"] = execution_progress(&public);
    if !exact_serial.is_empty() {
        redact_exact_serial(&mut public, exact_serial);
    }
    public
}

fn project_recipe(recipe: &Value, retained: Option<&Value>, exact_serial: &str) -> Value {
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
        .unwrap_or_else(|| "Setup feature".to_string());
    let steps = recipe
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|step| project_step(step, exact_serial))
        .collect::<Vec<_>>();
    json!({
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
    let name = step
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Setup action".to_string());
    let status = step
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("pending");
    let message = match status {
        "failed" => Some("This action did not complete."),
        "blocked" => Some("This action was blocked by required earlier work."),
        "cancelled" => Some(CANCELLED_SAFE_BOUNDARY_TEXT),
        _ => None,
    };
    json!({
        "name": sanitize_text(&name, exact_serial),
        "note": step.get("note").and_then(Value::as_str).map(|value| sanitize_text(value, exact_serial)),
        "status": status,
        "message": message,
    })
}

fn project_issues(
    mapping: &ExecutionMapping,
    value: Option<&Value>,
    exact_serial: &str,
) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|issue| project_real_issue(mapping, issue))
        .map(|mut issue| {
            redact_exact_serial(&mut issue, exact_serial);
            issue
        })
        .collect()
}

fn completion_summary(snapshot: &Value, real: bool) -> Value {
    completion_summary_with_identity_state(snapshot, real, 0, false, 0, false)
}

fn completion_summary_with_identity_state(
    snapshot: &Value,
    real: bool,
    identity_failure_count: u64,
    post_identity_marker: bool,
    root_failure_count: u64,
    root_marker: bool,
) -> Value {
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
            && (counts.get("completed").copied().unwrap_or(0) > 0
                || counts
                    .get("failed")
                    .copied()
                    .unwrap_or(0)
                    > identity_failure_count.saturating_add(root_failure_count)
                || post_identity_marker
                || root_marker),
    })
}

fn execution_progress(snapshot: &Value) -> Value {
    for recipe in snapshot
        .get("recipes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let current = recipe
            .get("steps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|step| step.get("status").and_then(Value::as_str) == Some("running"));
        if let Some(step) = current {
            return json!({
                "currentFeature": recipe.get("name"),
                "currentAction": step.get("note").or_else(|| step.get("name")),
            });
        }
    }
    json!({ "currentFeature": null, "currentAction": null })
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
            let label = event_presentation_label(mapping, event, exact_serial);
            Some(json!({
                "sequence": sequence,
                "timestamp": event.get("timestamp"),
                "label": label,
                "status": allowlisted_real_optional_string(
                    event.get("status"),
                    &["pending", "running", "skipped", "blocked", "succeeded", "failed", "cancelled"],
                ),
                "issue": event.get("issue").map(|issue| project_issues(mapping, Some(&Value::Array(vec![issue.clone()])), exact_serial).into_iter().next().unwrap_or(Value::Null)),
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

fn event_presentation_label(
    mapping: &ExecutionMapping,
    event: &Value,
    exact_serial: &str,
) -> String {
    if let Some(note) = event
        .get("note")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return sanitize_text(note, exact_serial);
    }
    if let Some((feature, action)) = event
        .get("stepId")
        .and_then(Value::as_str)
        .and_then(|step_id| action_context_for_step_id(mapping, step_id))
    {
        return sanitize_text(&format!("{action} in {feature}"), exact_serial);
    }
    let real = mapping.kind == ExecutionKind::Real;
    match event.get("eventType").and_then(Value::as_str) {
        Some("execution_started") => {
            if real {
                "Real-device execution started"
            } else {
                "Simulation started"
            }
        }
        Some("cancel_requested") => {
            "Cancellation requested; the current atomic operation may finish"
        }
        Some("execution_completed") => {
            if real {
                "Real-device execution completed"
            } else {
                "Simulation completed"
            }
        }
        Some("execution_worker_panicked") => "Execution stopped unexpectedly",
        _ => "Execution updated",
    }
    .to_string()
}

fn sanitize_text(value: &str, exact_serial: &str) -> String {
    let without_serial = if exact_serial.is_empty() {
        value.to_string()
    } else {
        value.replace(exact_serial, "[device]")
    };
    redact_absolute_paths(&without_serial)
}

fn is_terminal_status(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("succeeded" | "succeeded_with_warnings" | "failed" | "cancelled")
    )
}

const POST_OPERATION_IDENTITY_FAILURE_MARKER: &str =
    "The device identity could not be verified after the operation may have run.";

fn report_has_identity_failure(report: &Value) -> bool {
    ["errors", "warnings"].into_iter().any(|field| {
        report
            .get(field)
            .and_then(Value::as_array)
            .is_some_and(|issues| {
                issues.iter().any(|issue| {
                    matches!(
                        issue.get("code").and_then(Value::as_str),
                        Some("device_identity_changed" | "device_identity_unverified")
                    )
                })
            })
    })
}

fn report_has_root_authority_failure(report: &Value) -> bool {
    ["errors", "warnings"].into_iter().any(|field| {
        report
            .get(field)
            .and_then(Value::as_array)
            .is_some_and(|issues| {
                issues.iter().any(|issue| {
                    matches!(
                        issue.get("code").and_then(Value::as_str),
                        Some("root_authority_revoked" | "root_authority_unverified")
                    )
                })
            })
    })
}

fn invalidate_identity_terminal_authority(
    handles: &Mutex<SessionHandles>,
    root_qualification: &Mutex<RootQualificationStore>,
    mapping: &ExecutionMapping,
) -> Result<(), String> {
    let device_handle = mapping.review.device_handle.clone();
    recover_poisoned_lock(handles).invalidate_identity_authority(&device_handle);
    recover_poisoned_lock(root_qualification).invalidate_for_device(&device_handle);
    Ok(())
}

fn invalidate_root_terminal_authority(
    handles: &Mutex<SessionHandles>,
    root_qualification: &Mutex<RootQualificationStore>,
    mapping: &ExecutionMapping,
) -> Result<(), String> {
    let device_handle = mapping.review.device_handle.clone();
    let mut handles = recover_poisoned_lock(handles);
    handles.invalidate_review(&mapping.review_handle, "root_authority_changed");
    handles.invalidate_reviews_for_device_if(&device_handle, "root_authority_changed", |review| {
        review_requires_root(review)
    });
    drop(handles);
    recover_poisoned_lock(root_qualification).invalidate_for_device(&device_handle);
    Ok(())
}

fn sidecar_error_code(error: &str) -> Option<String> {
    serde_json::from_str::<Value>(error)
        .ok()?
        .get("code")?
        .as_str()
        .map(str::to_string)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionSessionLoss {
    UnknownExecution,
    RuntimeSessionLost,
}

/// Classify only failures that prove execution authority no longer exists.
/// An unknown execution invalidates one mapping; a lost runtime process
/// invalidates every authority object derived from that process generation.
fn execution_session_loss(error: &str) -> Option<ExecutionSessionLoss> {
    match sidecar_error_code(error).as_deref() {
        Some("unknown_execution") => Some(ExecutionSessionLoss::UnknownExecution),
        Some("runtime_session_lost") => Some(ExecutionSessionLoss::RuntimeSessionLost),
        _ => None,
    }
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

    #[cfg(not(feature = "real-execution"))]
    #[test]
    fn execution_capabilities_report_real_execution_not_compiled_without_feature() {
        let capabilities =
            ExecutionCapabilities::from_readiness(false, PlatformToolsReadiness::Ready);

        assert_eq!(
            serde_json::to_value(capabilities).unwrap(),
            json!({
                "realExecutionCompiled": false,
                "platformToolsStatus": "notApplicable",
                "executorReadiness": "notCompiled",
            })
        );
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn execution_capabilities_report_real_execution_compiled_with_feature() {
        let capabilities =
            ExecutionCapabilities::from_readiness(true, PlatformToolsReadiness::Ready);

        assert_eq!(
            serde_json::to_value(capabilities).unwrap(),
            json!({
                "realExecutionCompiled": true,
                "platformToolsStatus": "ready",
                "executorReadiness": "ready",
            })
        );
    }

    #[test]
    fn execution_capabilities_serialize_as_the_exact_frontend_contract() {
        let capabilities =
            ExecutionCapabilities::from_readiness(true, PlatformToolsReadiness::Ready);
        let serialized = serde_json::to_value(capabilities).unwrap();

        assert_eq!(
            serialized,
            json!({
                "realExecutionCompiled": true,
                "platformToolsStatus": "ready",
                "executorReadiness": "ready",
            })
        );
        assert!(serialized.get("enabled").is_none());
        let serialized = serialized.to_string();
        for protected in [
            "/Users/fixture/platform-tools/adb",
            "sensitive fixture process details",
            "Android Debug Bridge version",
        ] {
            assert!(!serialized.contains(protected));
        }
    }

    #[test]
    fn execution_capabilities_derive_blocked_and_unknown_only_from_rust_readiness() {
        assert_eq!(
            ExecutionCapabilities::from_readiness(true, PlatformToolsReadiness::NotFound)
                .executor_readiness,
            ExecutorReadiness::Blocked
        );
        assert_eq!(
            ExecutionCapabilities::from_readiness(true, PlatformToolsReadiness::Invalid)
                .executor_readiness,
            ExecutorReadiness::Blocked
        );
        assert_eq!(
            ExecutionCapabilities::from_readiness(true, PlatformToolsReadiness::CheckFailed)
                .executor_readiness,
            ExecutorReadiness::Unknown
        );
    }

    #[test]
    fn readiness_snapshot_requires_matching_adb_and_runtime_generations() {
        let generations = ReadinessGenerations {
            adb_revision: 12,
            runtime_generation: 8,
        };

        assert!(generations.matches(12, 8));
        assert!(!generations.matches(13, 8));
        assert!(!generations.matches(12, 9));
    }

    #[test]
    fn review_requires_root_uses_shared_classifier_for_root_dependent_work() {
        let mut review = review();
        assert!(!review_requires_root(&review));
        review.response["plan"]["runtime_capabilities"] = json!({ "root_shell": true });
        assert!(
            !review_requires_root(&review),
            "capability availability alone must not retain root authority"
        );

        review.response["plan"]["steps"] = json!([{
            "id": "recipe.one/extract",
            "recipe_ref": "recipe.one",
            "type": "extract_archive",
            "name": "Extract",
            "note": "Extract",
            "dependencies": [],
            "constraints": { "capabilities": [], "conflicts_with": [] },
            "params": {
                "extract_on": { "value": "device" },
                "dest": { "value": "/data/data/com.example.app/extracted" }
            },
            "skip_if": [],
            "verify": []
        }]);
        assert!(review_requires_root(&review));

        let context = crate::device_observation::QualificationContextKey::new(
            "device_one",
            1,
            1,
            2,
            2,
            "input-bound-root",
        );
        review.qualification_context = Some(context.clone());
        let current = crate::device_observation::test_current_qualification(
            DeviceQualificationState::Supported,
            Some(context),
        );
        assert!(validate_final_qualification(&review, &current, false)
            .unwrap_err()
            .contains("root_qualification_required"));

        review.response["plan"]["steps"][0]["params"]["dest"] =
            json!({ "value": "/sdcard/EmuChef/extracted" });
        assert!(!review_requires_root(&review));
        assert!(validate_final_qualification(&review, &current, false).is_ok());

        review.response["plan"]["steps"] = json!([{
            "id": "recipe.one/copy",
            "recipe_ref": "recipe.one",
            "type": "copy_files",
            "name": "Copy",
            "note": "Copy",
            "dependencies": [],
            "constraints": { "capabilities": [], "conflicts_with": [] },
            "params": {
                "source": { "value": "/sdcard/EmuChef/source" },
                "dest": { "value": "/data/data/com.example.app/files" }
            },
            "skip_if": [],
            "verify": []
        }]);
        assert!(review_requires_root(&review));
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
            response: json!({
                "plan": plan,
                "review": { "canExecute": true }
            }),
            target: json!({ "serial": "sensitive-serial", "manufacturer": "AYANEO", "model": "Pocket S", "androidApiLevel": 33 }),
            catalog_identity: json!({
                "sourceKind": "bundled", "sourceId": "catalog", "version": "1",
                "contentDigest": { "algorithm": "sha256", "value": "catalog" }
            }),
            catalog_digest: "catalog".to_string(),
            plan_digest: canonical_json_digest(&plan).unwrap(),
            device_handle: "device_one".to_string(),
            qualification_context: None,
            platform_tools_identity: None,
            created: Instant::now(),
            last_access: Instant::now(),
        }
    }

    #[test]
    fn final_qualification_gate_rejects_unsupported_and_incomplete_profiles() {
        let review = review();
        let unsupported = crate::device_observation::test_current_qualification(
            DeviceQualificationState::Unsupported,
            None,
        );
        assert!(validate_final_qualification(&review, &unsupported, false)
            .unwrap_err()
            .contains("device_qualification_unsupported"));
        let incomplete = crate::device_observation::test_current_qualification(
            DeviceQualificationState::InsufficientlyQualified,
            None,
        );
        assert!(validate_final_qualification(&review, &incomplete, false)
            .unwrap_err()
            .contains("device_qualification_incomplete"));
    }

    #[test]
    fn final_qualification_gate_allows_only_matching_supported_context() {
        let context = crate::device_observation::QualificationContextKey::new(
            "device_one",
            1,
            1,
            2,
            2,
            "fingerprint",
        );
        let mut review = review();
        review.qualification_context = Some(context.clone());
        let current = crate::device_observation::test_current_qualification(
            DeviceQualificationState::Supported,
            Some(context),
        );
        assert!(validate_final_qualification(&review, &current, false).is_ok());

        review.response["plan"]["runtime_capabilities"] = json!({
            "root_shell": true,
            "app_data_write": true,
        });
        review.response["plan"]["steps"] = json!([{
            "id": "recipe.one/root-check",
            "recipe_ref": "recipe.one",
            "type": "wait",
            "name": "Root check",
            "note": "Root check",
            "dependencies": [],
            "constraints": { "capabilities": [], "conflicts_with": [] },
            "params": { "duration_ms": { "value": 1 } },
            "skip_if": [{
                "type": "path_exists",
                "params": { "path": "/data/data/com.example/root" }
            }],
            "verify": []
        }]);
        assert!(validate_final_qualification(&review, &current, false)
            .unwrap_err()
            .contains("root_qualification_required"));
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
    fn timeout_issue_is_projected_without_backend_details() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let projected = project_real_issue(
            &mapping,
            &json!({
                "code": "operation_timed_out",
                "recipeId": "recipe.one",
                "stepId": "recipe.one/launch",
                "message": "private timeout detail",
            }),
        );
        assert_eq!(
            projected["message"],
            "Launch configured app in Recipe One did not complete. A device operation timed out."
        );
        assert_eq!(projected["remediation"]["kind"], "generate_fresh_plan");
        assert!(!projected.to_string().contains("private timeout detail"));
    }

    #[test]
    fn storage_issue_is_projected_with_authored_recovery_without_backend_details() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let projected = project_real_issue(
            &mapping,
            &json!({
                "code": "device_storage_exhausted",
                "recipeId": "recipe.one",
                "stepId": "recipe.one/copy",
                "message": "private ADB output /data/private/path",
            }),
        );
        assert!(projected["message"]
            .as_str()
            .unwrap()
            .contains("The device ran out of storage during execution."));
        assert_eq!(projected["remediation"]["kind"], "generate_fresh_plan");
        let remediation = projected["remediation"]["message"].as_str().unwrap();
        assert!(remediation.contains("Free device storage"));
        assert!(remediation.contains("fresh qualification"));
        assert!(remediation.contains("old execution cannot resume"));
        assert!(!projected.to_string().contains("private ADB output"));
        assert!(!projected.to_string().contains("/data/private/path"));
    }

    #[test]
    fn transport_issue_codes_project_to_authored_guidance_without_backend_details() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let cases = [
            (
                "device_offline",
                "The reviewed device went offline during execution.",
                "reconnect_device",
            ),
            (
                "device_unauthorized",
                "The intended reviewed device needs USB debugging authorization.",
                "reconnect_device",
            ),
            (
                "device_disconnected",
                "The intended reviewed device disconnected or could not be found.",
                "reconnect_device",
            ),
            (
                "adb_server_unavailable",
                "The local ADB/Platform-Tools service was unavailable.",
                "repair_platform_tools",
            ),
            (
                "device_transport_lost",
                "The device connection was lost during execution.",
                "reconnect_device",
            ),
        ];
        for (code, authored_message, remediation_kind) in cases {
            let projected = project_real_issue(
                &mapping,
                &json!({
                    "code": code,
                    "recipeId": "recipe.one",
                    "stepId": "recipe.one/launch",
                    "message": "error: device 'private-serial' not found; /private/path; stderr detail",
                }),
            );
            assert!(projected["message"]
                .as_str()
                .unwrap()
                .contains(authored_message));
            assert_eq!(projected["remediation"]["kind"], remediation_kind);
            let remediation = projected["remediation"]["message"].as_str().unwrap();
            assert!(remediation.contains("fresh qualification"));
            assert!(remediation.contains("fresh plan"));
            assert!(remediation.contains("does not resume"));
            let text = projected.to_string();
            assert!(!text.contains("private-serial"));
            assert!(!text.contains("/private/path"));
            assert!(!text.contains("stderr detail"));
        }
    }

    #[test]
    fn identity_issue_codes_project_to_distinct_authored_guidance_without_backend_details() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let changed = project_real_issue(
            &mapping,
            &json!({
                "code": "device_identity_changed",
                "recipeId": "recipe.one",
                "stepId": "recipe.one/launch",
                "message": "private serial, android id, build fingerprint, and command"
            }),
        );
        let unverified = project_real_issue(
            &mapping,
            &json!({
                "code": "device_identity_unverified",
                "recipeId": "recipe.one",
                "stepId": "recipe.one/launch",
                "message": "private serial, android id, build fingerprint, and command"
            }),
        );
        assert_ne!(changed["message"], unverified["message"]);
        assert!(changed["message"]
            .as_str()
            .unwrap()
            .contains("identity changed"));
        assert!(unverified["message"]
            .as_str()
            .unwrap()
            .contains("could not be verified"));
        for projected in [changed, unverified] {
            assert_eq!(projected["remediation"]["kind"], "reconnect_device");
            let text = projected.to_string();
            assert!(!text.contains("private serial"));
            assert!(!text.contains("android id"));
            assert!(!text.contains("build fingerprint"));
            assert!(!text.contains("command"));
        }
    }

    #[test]
    fn root_issue_codes_project_to_authored_requalification_guidance() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        for code in ["root_authority_revoked", "root_authority_unverified"] {
            let projected = project_real_issue(
                &mapping,
                &json!({
                    "code": code,
                    "recipeId": "recipe.one",
                    "stepId": "recipe.one/launch",
                    "message": "private root stderr and serial detail",
                }),
            );
            let authored = if code == "root_authority_revoked" {
                "Root access was revoked during execution."
            } else {
                "EmuChef could not safely confirm continued root access."
            };
            assert!(projected["message"].as_str().unwrap().contains(authored));
            assert_eq!(projected["remediation"]["kind"], "requalify_root");
            assert!(!projected.to_string().contains("private root stderr"));
            assert!(!projected.to_string().contains("serial detail"));
        }
    }

    #[test]
    fn only_the_exact_post_identity_marker_allows_real_partial_warning_without_prior_evidence() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let report = |message: &str| {
            json!({
                "status": "failed",
                "errors": [{
                    "code": "device_identity_changed",
                    "message": message
                }],
                "recipes": [{
                    "recipeId": "recipe.one",
                    "name": "Recipe One",
                    "status": "failed",
                    "steps": [{ "stepId": "recipe.one/launch", "status": "failed" }]
                }]
            })
        };
        let marked =
            project_real_snapshot(&mapping, &report(POST_OPERATION_IDENTITY_FAILURE_MARKER));
        let arbitrary = project_real_snapshot(
            &mapping,
            &report("identity changed; private serial and command output"),
        );
        assert_eq!(marked["completion"]["partialChangesPossible"], true);
        assert_eq!(arbitrary["completion"]["partialChangesPossible"], false);
        assert!(!marked
            .to_string()
            .contains(POST_OPERATION_IDENTITY_FAILURE_MARKER));
        assert!(!arbitrary
            .to_string()
            .contains("private serial and command output"));
    }

    #[test]
    fn root_marker_requires_exact_root_issue_pair_for_partial_warning() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let report = |code: &str, message: &str| {
            json!({
                "status": "failed",
                "errors": [{ "code": code, "message": message }],
                "recipes": [{
                    "recipeId": "recipe.one",
                    "name": "Recipe One",
                    "status": "failed",
                    "steps": [{ "stepId": "recipe.one/launch", "status": "failed" }]
                }]
            })
        };
        let unmarked = project_real_snapshot(
            &mapping,
            &report("root_authority_revoked", "Root access was revoked."),
        );
        let marked = project_real_snapshot(
            &mapping,
            &report(
                "root_authority_unverified",
                ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER,
            ),
        );
        let identity_with_root_marker = project_real_snapshot(
            &mapping,
            &report(
                "device_identity_changed",
                ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER,
            ),
        );
        let root_with_identity_marker = project_real_snapshot(
            &mapping,
            &report(
                "root_authority_revoked",
                POST_OPERATION_IDENTITY_FAILURE_MARKER,
            ),
        );
        assert_eq!(unmarked["completion"]["partialChangesPossible"], false);
        assert_eq!(marked["completion"]["partialChangesPossible"], true);
        assert_eq!(
            identity_with_root_marker["completion"]["partialChangesPossible"],
            false
        );
        assert_eq!(
            root_with_identity_marker["completion"]["partialChangesPossible"],
            false
        );
    }

    #[test]
    fn real_identity_event_projection_is_authored_and_sanitized() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let projected = project_real_event_batch(
            &mapping,
            &json!({
                "events": [{
                    "sequence": 4,
                    "timestamp": "2026-08-02T13:44:00Z",
                    "executionId": "execution-private",
                    "eventType": "step_failed",
                    "stepId": "recipe.one/launch",
                    "status": "failed",
                    "issue": {
                        "code": "device_identity_changed",
                        "message": "serial=private; android_id=secret; build.fingerprint=raw; command=adb shell getprop"
                    }
                }],
                "latestSequence": 4,
                "terminal": true,
            }),
        );
        assert_eq!(
            projected["events"][0]["issue"]["message"],
            "The reviewed device identity changed during execution."
        );
        let serialized = projected.to_string();
        for forbidden in [
            "execution-private",
            "device_identity_changed",
            "private",
            "android_id",
            "build.fingerprint",
            "adb shell getprop",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn real_identity_report_projection_is_sanitized_and_side_effect_free() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let report = json!({
            "executionId": "execution-private",
            "planId": "plan.one",
            "status": "failed",
            "recipes": [{
                "recipeId": "recipe.one",
                "name": "Recipe One",
                "status": "failed",
                "steps": [{
                    "stepId": "recipe.one/launch",
                    "name": "Launch app",
                    "status": "failed"
                }]
            }],
            "errors": [{
                "code": "device_identity_unverified",
                "recipeId": "recipe.one",
                "stepId": "recipe.one/launch",
                "message": "serial=private; android_id=secret; build.fingerprint=raw; path=/private"
            }],
            "warnings": []
        });
        let public = project_real_snapshot(&mapping, &report);
        let document = execution_report_document(
            &mapping,
            &report,
            &public,
            json!({ "status": "ready", "protocolVersion": 1 }),
        );
        assert_eq!(
            document["execution"]["errors"][0]["message"],
            "Launch configured app in Recipe One did not complete. The reviewed device identity could not be verified safely."
        );
        let serialized = document.to_string();
        for forbidden in [
            "execution-private",
            "device_identity_unverified",
            "private",
            "android_id",
            "build.fingerprint",
            "/private",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        assert_eq!(public["completion"]["partialChangesPossible"], false);
    }

    #[test]
    fn real_storage_report_projection_exports_authored_recovery_without_private_details() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Real,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let report = json!({
            "executionId": "execution-private",
            "planId": "plan.private",
            "status": "failed",
            "recipes": [{
                "recipeId": "recipe.one",
                "name": "Recipe One",
                "status": "failed",
                "steps": [{ "stepId": "recipe.one/copy", "status": "failed" }]
            }],
            "errors": [{
                "code": "device_storage_exhausted",
                "recipeId": "recipe.one",
                "stepId": "recipe.one/copy",
                "message": "adb stderr /sdcard/private/raw-path secret-serial"
            }],
            "warnings": []
        });
        let public = project_real_snapshot(&mapping, &report);
        let document = execution_report_document(
            &mapping,
            &report,
            &public,
            json!({ "status": "ready", "protocolVersion": 1 }),
        );
        assert!(document["execution"]["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("The device ran out of storage during execution."));
        assert!(document["execution"]["errors"][0]["remediation"]["message"]
            .as_str()
            .unwrap()
            .contains("old execution cannot resume"));
        let serialized = document.to_string();
        for forbidden in [
            "execution-private",
            "device_storage_exhausted",
            "/sdcard/private/raw-path",
            "secret-serial",
            "adb stderr",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn terminal_pending_work_remains_derivable_without_a_new_completion_field() {
        let snapshot = json!({
            "status": "cancelled",
            "warnings": [],
            "recipes": [{
                "name": "Recipe One",
                "status": "cancelled",
                "steps": [
                    { "status": "succeeded" },
                    { "status": "pending" }
                ]
            }]
        });

        let summary = completion_summary(&snapshot, true);

        assert_eq!(summary["counts"]["completed"], 1);
        assert_eq!(summary["counts"]["pending"], 1);
        assert!(summary["counts"].get("unattempted").is_none());
        assert_eq!(summary["partialChangesPossible"], true);
    }

    #[test]
    fn failed_atomic_work_warns_about_possible_partial_changes() {
        let snapshot = json!({
            "status": "failed",
            "warnings": [],
            "recipes": [{
                "name": "Recipe One",
                "status": "failed",
                "steps": [{ "status": "failed" }]
            }]
        });

        let summary = completion_summary(&snapshot, true);

        assert_eq!(summary["counts"]["failed"], 1);
        assert_eq!(summary["partialChangesPossible"], true);
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
    fn production_report_bytes_match_the_sanitized_export_document() {
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Real).unwrap();
        let mapping = store.bind_started(
            ExecutionKind::Real,
            "sidecar-private-id".into(),
            "review-private-id".into(),
            review(),
        );
        let report = json!({ "planId": "/private/plan", "status": "failed" });
        let runtime = json!({ "status": "ready", "protocolVersion": 1 });
        let public = project_real_snapshot(&mapping, &report);
        let expected_document =
            execution_report_document(&mapping, &report, &public, runtime.clone());
        let mut expected_bytes = serde_json::to_string_pretty(&expected_document).unwrap();
        expected_bytes.push('\n');

        assert!(store.mark_terminal_with_report(
            ExecutionKind::Real,
            &mapping.public_handle,
            report,
            Value::Null,
        ));
        store
            .set_terminal_report_runtime(&mapping.public_handle, runtime)
            .unwrap();
        let actual = production_execution_report_bytes(&store, &mapping.public_handle).unwrap();

        assert_eq!(actual, expected_bytes.as_bytes());
        assert_eq!(
            serde_json::from_slice::<Value>(&actual).unwrap(),
            expected_document
        );
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
        assert_eq!(public["recipes"][0]["steps"][0]["name"], "Setup action");
        assert!(!serialized.contains("execution-private"));
        assert!(!serialized.contains("reviewedPlan"));
        assert!(!serialized.contains("targetDevice"));
        assert!(!serialized.contains("outputs"));
        assert!(!serialized.contains("sensitive-serial"));
        assert!(!serialized.contains("/Users/private"));
        assert!(!serialized.contains("/private/path"));
    }

    #[test]
    fn failures_and_events_use_authored_action_context_without_exposing_identity() {
        let mapping = ExecutionMapping {
            kind: ExecutionKind::Simulated,
            public_handle: "execution_public".into(),
            sidecar_id: "execution-private".into(),
            review_handle: "review_public".into(),
            review: launch_review(),
        };
        let issue = json!({
            "code": "verification_failed",
            "recipeId": "recipe.one",
            "stepId": "recipe.one/launch",
            "message": "raw verifier text",
        });
        let projected = project_real_issue(&mapping, &issue);
        assert_eq!(
            projected["message"],
            "Launch configured app in Recipe One did not complete. Completed work could not be verified."
        );
        let event = project_event_batch(
            &mapping,
            &json!({
                "events": [{
                    "sequence": 1,
                    "timestamp": "2026-01-01T00:00:00Z",
                    "eventType": "step_progress",
                    "stepId": "recipe.one/launch",
                    "status": "running",
                }],
                "latestSequence": 1,
                "terminal": false,
            }),
        );
        assert_eq!(
            event["events"][0]["label"],
            "Launch configured app in Recipe One"
        );
        let serialized = json!({ "issue": projected, "event": event }).to_string();
        for forbidden in [
            "recipe.one",
            "stepId",
            "recipeId",
            "verification_failed",
            "raw verifier",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn unsafe_backend_review_cannot_start() {
        let mut retained = review();
        retained.response["review"]["canExecute"] = Value::Bool(false);
        assert!(validate_review_executable(&retained)
            .unwrap_err()
            .contains("review_not_executable"));
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

    struct ScriptedRuntime {
        requests: Mutex<Vec<(String, Value)>>,
        responses: Mutex<Vec<Result<Value, String>>>,
    }

    impl RuntimeRequester for ScriptedRuntime {
        fn request(&self, request_type: &str, payload: Value) -> Result<Value, String> {
            self.requests
                .lock()
                .unwrap()
                .push((request_type.to_string(), payload));
            self.responses.lock().unwrap().remove(0)
        }
    }

    struct TerminalDuringPollRuntime<'a> {
        executions: &'a Mutex<ExecutionHandleStore>,
        execution_handle: &'a str,
        response: Value,
    }

    impl RuntimeRequester for TerminalDuringPollRuntime<'_> {
        fn request(&self, request_type: &str, _payload: Value) -> Result<Value, String> {
            assert_eq!(request_type, "getExecutionEvents");
            let mut executions = self.executions.lock().unwrap();
            assert!(executions.mark_terminal_with_report(
                ExecutionKind::Real,
                self.execution_handle,
                json!({
                    "status": "failed",
                    "finishedAt": "2026-10-03T10:02:00Z",
                    "latestSequence": 8,
                    "recipes": [],
                    "errors": [],
                    "warnings": []
                }),
                json!({ "status": "ready" }),
            ));
            Ok(self.response.clone())
        }
    }

    /// Build one application state around the exact authoritative stores a
    /// test prepared, so the product terminal monitor and the qualification
    /// session can be driven through the same process-wide seams production
    /// uses.
    fn test_app(
        executions: Mutex<ExecutionHandleStore>,
        handles: Mutex<SessionHandles>,
        root_qualification: Mutex<RootQualificationStore>,
    ) -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
        test_app_with_qualification(
            executions,
            handles,
            root_qualification,
            crate::qualification_repository::QualificationRepositoryProvider::default(),
        )
    }

    fn test_app_with_qualification(
        executions: Mutex<ExecutionHandleStore>,
        handles: Mutex<SessionHandles>,
        root_qualification: Mutex<RootQualificationStore>,
        qualification_repository: crate::qualification_repository::QualificationRepositoryProvider,
    ) -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
        let temp = tempfile::tempdir().expect("test app directory should be created");
        let app_root = temp.path();
        let app = tauri::test::mock_app();
        let catalog = crate::catalog::CatalogDescriptor::for_test()
            .expect("the repository catalog should be available to execution tests");
        let app_state = AppState {
            sidecar: SidecarState::new(app_root.join("sidecar-cache")),
            catalog: Ok(catalog),
            qualification_repository,
            qualification_transition_gate: Mutex::new(()),
            adb: Mutex::new(crate::adb::AdbManager::new(app_root.join("platform-tools"))),
            platform_tools_selections: Mutex::new(
                crate::commands::PlatformToolsSelectionStore::default(),
            ),
            input_contracts: Mutex::new(crate::commands::InputContractSnapshot::default()),
            handles,
            root_qualification,
            executions,
            qualification_sessions: Mutex::new(
                crate::qualification_session::QualificationSessionStore::default(),
            ),
            saved_configurations: Mutex::new(
                crate::saved_configurations::SavedConfigurationStore::load(
                    app_root.join("recent-configurations.json"),
                ),
            ),
            recovery: Mutex::new(crate::recovery::RecoveryStore::load(
                app_root.join("recovery-draft.json"),
                app_root.join("session-active.marker"),
            )),
            support: Mutex::new(crate::support::SupportStore::new(
                app_root.join("support-cache"),
            )),
            updates: crate::updates::UpdateService::from_production_document()
                .expect("test update trust should be available"),
            update_activity: crate::updates::ActivityGate::default(),
        };
        assert!(app.manage(app_state));
        (temp, app)
    }

    fn supported_inventory(transport_id: &str) -> Value {
        json!({
            "devices": [{
                "serial": "sensitive-serial",
                "state": "available",
                "model": "Pocket S",
                "transportId": transport_id,
            }]
        })
    }

    fn supported_qualification() -> Value {
        json!({
            "state": "online",
            "androidMajor": 14,
            "androidApiLevel": 33,
            "abi": "arm64-v8a",
            "storage": "available",
            "packageManager": "available",
            "activityManager": "available",
        })
    }

    fn match_result(profile_id: &str) -> Value {
        json!({
            "confidence": "high",
            "recommendedPlanId": "test-plan",
            "requiresExplicitChoice": false,
            "candidates": [{
                "planId": "test-plan",
                "profileId": profile_id,
                "name": "Test plan",
                "description": null,
                "profileName": "Test profile",
                "confidence": "high",
                "reasons": [],
                "requiresExplicitChoice": false,
                "selectionMode": "automatic",
            }],
            "safeGenericPlans": [],
            "blankSetupPlans": [],
            "blocked": false,
            "blockReason": null,
        })
    }

    fn target_facts() -> Value {
        json!({
            "manufacturer": "AYANEO",
            "model": "Pocket S",
            "android_api_level": 33,
        })
    }

    fn prepared_real_review(
        root_required: bool,
    ) -> (Mutex<SessionHandles>, Mutex<RootQualificationStore>, String) {
        prepared_real_review_with_epoch(root_required, None)
    }

    fn prepared_real_review_with_epoch(
        root_required: bool,
        session_epoch: Option<u64>,
    ) -> (Mutex<SessionHandles>, Mutex<RootQualificationStore>, String) {
        let handles = Mutex::new(SessionHandles::default());
        let root = Mutex::new(RootQualificationStore::default());
        let handle = {
            let mut handles = handles.lock().unwrap();
            handles
                .update_devices(&supported_inventory("transport-1"))
                .unwrap();
            handles.single_available_device_handle().unwrap()
        };
        let setup_runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Ok(supported_qualification()),
        };
        let mut setup_request =
            |request_type: &str, payload: Value| setup_runtime.request(request_type, payload);
        let context = crate::device_observation::qualify_reconciled_current_with_runtime(
            &handles,
            &root,
            "/trusted/adb",
            1,
            2,
            Some(&handle),
            &mut setup_request,
        )
        .unwrap()
        .context
        .unwrap();
        let mut retained = review();
        retained.device_handle = handle;
        let mut context = context;
        if let Some(session_epoch) = session_epoch {
            context.session_epoch = session_epoch;
        }
        retained.qualification_context = Some(context);
        if root_required {
            retained.response["plan"]["runtime_capabilities"] = json!({
                "root_shell": true,
                "app_data_write": true,
            });
            retained.response["plan"]["steps"] = json!([{
                "id": "recipe.one/extract",
                "recipe_ref": "recipe.one",
                "type": "extract_archive",
                "name": "Extract",
                "note": "Extract",
                "dependencies": [],
                "constraints": { "capabilities": [], "conflicts_with": [] },
                "params": {
                    "extract_on": { "value": "device" },
                    "dest": { "value": "/data/data/com.example.app/extracted" }
                },
                "skip_if": [],
                "verify": []
            }]);
            retained.plan_digest = canonical_json_digest(&retained.response["plan"]).unwrap();
        }
        let review_handle = handles.lock().unwrap().insert_review(retained);
        (handles, root, review_handle)
    }

    fn run_integrated_case(
        review_handle: &str,
        handles: &Mutex<SessionHandles>,
        root: &Mutex<RootQualificationStore>,
        inventory: Value,
        qualification: Option<Value>,
        runtime_generation: u64,
        platform_tools_revision: u64,
    ) -> (Result<Value, String>, usize) {
        let mut responses = vec![Ok(inventory)];
        if let Some(qualification) = qualification {
            responses.push(Ok(target_facts()));
            responses.push(Ok(qualification));
            responses.push(Ok(
                json!({ "execution": { "executionId": "sidecar-real" } }),
            ));
        }
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses),
        };
        let mut executions = ExecutionHandleStore::default();
        executions
            .reserve_start(ExecutionKind::Real)
            .expect("integrated preflight owns the real start reservation");
        let platform_tools = PlatformToolsSnapshot {
            adb_path: "/trusted/adb",
            runtime_generation,
            platform_tools_revision,
        };
        let result = start_real_execution_inner_with_runtime(
            review_handle,
            None,
            handles,
            root,
            &mut executions,
            &runtime,
            &platform_tools,
        );
        let starts = runtime
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(request_type, _)| request_type == "startExecution")
            .count();
        (result, starts)
    }

    #[test]
    fn terminal_real_retrieval_invalidates_only_the_affected_identity_authority_once() {
        let inventory = json!({
            "devices": [
                {
                    "serial": "sensitive-serial",
                    "state": "available",
                    "model": "Pocket S",
                    "transportId": "transport-affected"
                },
                {
                    "serial": "unrelated-serial",
                    "state": "available",
                    "model": "Pocket Other",
                    "transportId": "transport-unrelated"
                }
            ]
        });
        let handles = Mutex::new(SessionHandles::default());
        let root = Mutex::new(RootQualificationStore::default());
        let (affected_handle, unrelated_handle) = {
            let mut handles = handles.lock().unwrap();
            handles.update_devices(&inventory).unwrap();
            let devices = handles.qualification_devices();
            let affected = devices
                .iter()
                .find(|device| device.serial == "sensitive-serial")
                .unwrap()
                .handle
                .clone();
            let unrelated = devices
                .iter()
                .find(|device| device.serial == "unrelated-serial")
                .unwrap()
                .handle
                .clone();
            handles
                .set_facts(&affected, target_facts())
                .expect("affected facts should be retained");
            handles
                .set_facts(
                    &unrelated,
                    json!({
                        "manufacturer": "Other",
                        "model": "Pocket Other",
                        "android_api_level": 33,
                    }),
                )
                .expect("unrelated facts should be retained");
            (affected, unrelated)
        };
        let affected_context = crate::device_observation::QualificationContextKey::new(
            &affected_handle,
            1,
            2,
            3,
            4,
            "affected-capabilities",
        );
        let unrelated_context = crate::device_observation::QualificationContextKey::new(
            &unrelated_handle,
            1,
            2,
            3,
            4,
            "unrelated-capabilities",
        );
        let (
            affected_review_handle,
            unrelated_review_handle,
            affected_review,
            affected_key,
            unrelated_key,
        ) = {
            let mut handles = handles.lock().unwrap();
            handles.set_qualification_context(affected_context.clone());
            handles.set_qualification_context(unrelated_context.clone());

            let mut affected_review = launch_review();
            affected_review.device_handle = affected_handle.clone();
            affected_review.qualification_context = Some(affected_context.clone());
            let affected_review_handle = handles.insert_review(affected_review.clone());

            let mut unrelated_review = review();
            unrelated_review.device_handle = unrelated_handle.clone();
            unrelated_review.qualification_context = Some(unrelated_context.clone());
            let unrelated_review_handle = handles.insert_review(unrelated_review);

            (
                affected_review_handle,
                unrelated_review_handle,
                affected_review,
                RootQualificationKey::from_context(&affected_context),
                RootQualificationKey::from_context(&unrelated_context),
            )
        };
        let late_attempt = {
            let mut root = root.lock().unwrap();
            let completed_affected = root.begin(affected_key.clone()).unwrap();
            assert!(root.complete(completed_affected, RootQualificationState::Granted));
            let completed_unrelated = root.begin(unrelated_key.clone()).unwrap();
            assert!(root.complete(completed_unrelated, RootQualificationState::Granted));
            assert_eq!(
                root.get(&affected_key),
                Some(RootQualificationState::Granted)
            );
            assert_eq!(
                root.get(&unrelated_key),
                Some(RootQualificationState::Granted)
            );
            root.begin(affected_key.clone()).unwrap()
        };

        let executions = Mutex::new(ExecutionHandleStore::default());
        let execution_handle = {
            let mut executions = executions.lock().unwrap();
            executions.reserve_start(ExecutionKind::Real).unwrap();
            executions
                .bind_started(
                    ExecutionKind::Real,
                    "sidecar-terminal".to_string(),
                    affected_review_handle.clone(),
                    affected_review,
                )
                .public_handle
                .clone()
        };
        let terminal_response = || {
            Ok(json!({
                "execution": {
                    "executionId": "sidecar-terminal",
                    "status": "failed",
                    "errors": [{
                        "code": "device_identity_changed",
                        "message": POST_OPERATION_IDENTITY_FAILURE_MARKER
                    }],
                    "recipes": []
                }
            }))
        };
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![terminal_response(), terminal_response()]),
        };
        let generation_before = handles.lock().unwrap().device_generation();
        let affected_epoch_before = handles
            .lock()
            .unwrap()
            .session_epoch_for_test(&affected_handle)
            .unwrap();

        let (_temp, app) = test_app(executions, handles, root);
        let state = app.state::<AppState>();
        let handles = &state.handles;
        let root = &state.root_qualification;
        let executions = &state.executions;
        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        assert!(
            matches!(event, Some(RealExecutionMonitorEvent::Terminal { .. })),
            "the monitor must retain the authoritative terminal transition"
        );
        let first = get_real_execution_inner_with_runtime(&execution_handle, executions, &runtime)
            .expect("first terminal retrieval should succeed");
        assert_eq!(first["status"], "failed");
        assert_eq!(first["terminal"], true);
        assert!(!first.to_string().contains("sensitive-serial"));

        let generation_after_first = handles.lock().unwrap().device_generation();
        assert!(generation_after_first > generation_before);
        let mut handles_after_first = handles.lock().unwrap();
        assert!(handles_after_first.device(&affected_handle).is_err());
        assert!(handles_after_first.facts(&affected_handle).is_err());
        assert!(handles_after_first
            .qualification_context(&affected_handle)
            .is_none());
        assert!(handles_after_first
            .review(&affected_review_handle)
            .unwrap_err()
            .contains("review_stale"));
        assert!(handles_after_first.device(&unrelated_handle).is_ok());
        assert!(handles_after_first.facts(&unrelated_handle).is_ok());
        assert_eq!(
            handles_after_first.qualification_context(&unrelated_handle),
            Some(unrelated_context.clone())
        );
        assert!(handles_after_first.review(&unrelated_review_handle).is_ok());
        drop(handles_after_first);
        assert!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&affected_handle)
                .unwrap()
                > affected_epoch_before
        );
        let affected_epoch_after_first = handles
            .lock()
            .unwrap()
            .session_epoch_for_test(&affected_handle)
            .unwrap();
        let root_after_first = root.lock().unwrap();
        assert_eq!(root_after_first.get(&affected_key), None);
        assert_eq!(
            root_after_first.get(&unrelated_key),
            Some(RootQualificationState::Granted)
        );
        drop(root_after_first);
        assert!(!root
            .lock()
            .unwrap()
            .complete(late_attempt, RootQualificationState::Granted));
        assert!(executions
            .lock()
            .unwrap()
            .mapping(ExecutionKind::Real, &execution_handle, "missing")
            .is_ok());

        let repeated = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        assert!(
            repeated.is_none(),
            "an authoritative terminal transition must never be retained twice"
        );
        assert_eq!(
            handles.lock().unwrap().device_generation(),
            generation_after_first
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&affected_handle),
            Some(affected_epoch_after_first)
        );
        assert!(handles.lock().unwrap().device(&unrelated_handle).is_ok());
        assert_eq!(
            root.lock().unwrap().get(&unrelated_key),
            Some(RootQualificationState::Granted)
        );

        let mapping = executions
            .lock()
            .unwrap()
            .mapping(ExecutionKind::Real, &execution_handle, "missing")
            .unwrap();
        let report = terminal_response().unwrap()["execution"].clone();
        let public_before_export = project_real_snapshot(&mapping, &report);
        let generation_before_export = handles.lock().unwrap().device_generation();
        let epoch_before_export = handles
            .lock()
            .unwrap()
            .session_epoch_for_test(&affected_handle);
        let document = execution_report_document(
            &mapping,
            &report,
            &public_before_export,
            json!({ "status": "ready" }),
        );
        let repeated_document = execution_report_document(
            &mapping,
            &report,
            &project_real_snapshot(&mapping, &report),
            json!({ "status": "ready" }),
        );
        assert_eq!(document, repeated_document);
        assert!(!document.to_string().contains("sensitive-serial"));
        assert!(!document
            .to_string()
            .contains(POST_OPERATION_IDENTITY_FAILURE_MARKER));
        assert_eq!(
            handles.lock().unwrap().device_generation(),
            generation_before_export
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&affected_handle),
            epoch_before_export
        );
        assert!(executions
            .lock()
            .unwrap()
            .mapping(ExecutionKind::Real, &execution_handle, "missing")
            .is_ok());
    }

    #[test]
    fn terminal_root_failure_invalidates_only_root_reviews_for_single_device_once() {
        let (handles, root, root_review_handle) = prepared_real_review(true);
        let (device_handle, context, root_review) = {
            let mut handles = handles.lock().unwrap();
            let root_review = handles.review(&root_review_handle).unwrap().clone();
            let device_handle = root_review.device_handle.clone();
            let context = root_review
                .qualification_context
                .clone()
                .expect("prepared root review should retain qualification context");
            let mut non_root_review = review();
            non_root_review.device_handle = device_handle.clone();
            non_root_review.qualification_context = Some(context.clone());
            let non_root_handle = handles.insert_review(non_root_review);
            (device_handle, context, (root_review, non_root_handle))
        };
        let non_root_handle = root_review.1;
        let root_review = root_review.0;
        let root_key = RootQualificationKey::from_context(&context);
        handles
            .lock()
            .unwrap()
            .set_facts(&device_handle, target_facts())
            .expect("prepared device facts should be retained");
        let late_attempt = {
            let mut root = root.lock().unwrap();
            let attempt = root.begin(root_key.clone()).unwrap();
            assert!(root.complete(attempt, RootQualificationState::Granted));
            root.begin(root_key.clone()).unwrap()
        };

        let executions = Mutex::new(ExecutionHandleStore::default());
        let execution_handle = {
            let mut executions = executions.lock().unwrap();
            executions.reserve_start(ExecutionKind::Real).unwrap();
            executions
                .bind_started(
                    ExecutionKind::Real,
                    "sidecar-root-terminal".to_string(),
                    root_review_handle.clone(),
                    root_review,
                )
                .public_handle
                .clone()
        };
        let terminal_response = || {
            Ok(json!({
                "execution": {
                    "executionId": "sidecar-root-terminal",
                    "status": "failed",
                    "errors": [{
                        "code": "root_authority_revoked",
                        "message": "private root detail"
                    }],
                    "recipes": []
                }
            }))
        };
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![terminal_response(), terminal_response()]),
        };
        let generation_before = handles.lock().unwrap().device_generation();
        let (_temp, app) = test_app(executions, handles, root);
        let state = app.state::<AppState>();
        let handles = &state.handles;
        let root = &state.root_qualification;
        let executions = &state.executions;
        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        assert!(
            matches!(event, Some(RealExecutionMonitorEvent::Terminal { .. })),
            "the monitor must retain the authoritative terminal transition"
        );
        let first = get_real_execution_inner_with_runtime(&execution_handle, executions, &runtime)
            .expect("first root terminal retrieval should succeed");
        assert_eq!(first["status"], "failed");
        assert_eq!(first["errors"][0]["remediation"]["kind"], "requalify_root");
        assert!(!first.to_string().contains("private root detail"));
        assert_eq!(
            handles.lock().unwrap().device_generation(),
            generation_before
        );
        assert!(handles.lock().unwrap().device(&device_handle).is_ok());
        assert!(handles.lock().unwrap().facts(&device_handle).is_ok());
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .qualification_context(&device_handle),
            Some(context.clone())
        );
        assert!(handles
            .lock()
            .unwrap()
            .review(&root_review_handle)
            .unwrap_err()
            .contains("root_authority_changed"));
        assert!(handles.lock().unwrap().review(&non_root_handle).is_ok());
        assert_eq!(root.lock().unwrap().get(&root_key), None);
        assert!(!root
            .lock()
            .unwrap()
            .complete(late_attempt, RootQualificationState::Granted));

        let repeated = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        assert!(
            repeated.is_none(),
            "an authoritative terminal transition must never be retained twice"
        );
        assert_eq!(
            handles.lock().unwrap().device_generation(),
            generation_before
        );
        assert!(handles.lock().unwrap().review(&non_root_handle).is_ok());
    }

    #[test]
    fn terminal_root_failure_invalidates_only_root_reviews_once() {
        let inventory = json!({
            "devices": [
                {
                    "serial": "sensitive-serial",
                    "state": "available",
                    "model": "Pocket S",
                    "transportId": "transport-affected"
                },
                {
                    "serial": "unrelated-serial",
                    "state": "available",
                    "model": "Pocket Other",
                    "transportId": "transport-unrelated"
                }
            ]
        });
        let handles = Mutex::new(SessionHandles::default());
        let root = Mutex::new(RootQualificationStore::default());
        let (affected_handle, unrelated_handle) = {
            let mut handles = handles.lock().unwrap();
            handles.update_devices(&inventory).unwrap();
            let devices = handles.qualification_devices();
            let affected = devices
                .iter()
                .find(|device| device.serial == "sensitive-serial")
                .unwrap()
                .handle
                .clone();
            let unrelated = devices
                .iter()
                .find(|device| device.serial == "unrelated-serial")
                .unwrap()
                .handle
                .clone();
            handles
                .set_facts(&affected, target_facts())
                .expect("affected facts should be retained");
            handles
                .set_facts(
                    &unrelated,
                    json!({
                        "manufacturer": "Other",
                        "model": "Pocket Other",
                        "android_api_level": 33,
                    }),
                )
                .expect("unrelated facts should be retained");
            (affected, unrelated)
        };
        let affected_context = crate::device_observation::QualificationContextKey::new(
            &affected_handle,
            1,
            2,
            3,
            4,
            "affected-capabilities",
        );
        let unrelated_context = crate::device_observation::QualificationContextKey::new(
            &unrelated_handle,
            1,
            2,
            3,
            4,
            "unrelated-capabilities",
        );
        let root_review_for = |device_handle: &str,
                               context: &crate::device_observation::QualificationContextKey,
                               input_bound_destination: bool| {
            let mut retained = review();
            retained.device_handle = device_handle.to_string();
            retained.qualification_context = Some(context.clone());
            retained.response["plan"]["runtime_capabilities"] = json!({
                "adb_available": true,
                "apk_install": true,
                "shared_storage_write": true,
                "app_launch": true,
                "shell_command": true,
                "package_remove_for_user": false,
                "root_shell": true,
                "app_data_write": true,
            });
            retained.response["plan"]["inputs"] = if input_bound_destination {
                json!([{
                    "id": "destination",
                    "value": {
                        "type": "device_path",
                        "value": "/data/data/com.example.app/files",
                        "location": "device"
                    }
                }])
            } else {
                json!([])
            };
            retained.response["plan"]["steps"] = json!([{
                "id": "recipe.one/root-copy",
                "recipe_ref": "recipe.one",
                "type": "copy_files",
                "name": "Root copy",
                "note": "Root copy",
                "dependencies": [],
                "constraints": { "capabilities": [], "conflicts_with": [] },
                "params": {
                    "source": {
                        "value": {
                            "type": "file_path",
                            "value": "fixture.txt",
                            "location": "host"
                        }
                    },
                    "dest": if input_bound_destination {
                        json!({ "ref": "inputs.destination" })
                    } else {
                        json!({ "value": "/data/data/com.example.app/files" })
                    },
                    "copy_policy": { "value": "merge" }
                },
                "skip_if": [],
                "verify": []
            }]);
            retained.plan_digest = canonical_json_digest(&retained.response["plan"]).unwrap();
            retained
        };
        let non_root_review_for =
            |device_handle: &str, context: &crate::device_observation::QualificationContextKey| {
                let mut retained = review();
                retained.device_handle = device_handle.to_string();
                retained.qualification_context = Some(context.clone());
                retained
            };
        let (
            originating_root_review_handle,
            second_affected_root_review_handle,
            input_bound_affected_root_review_handle,
            affected_non_root_review_handle,
            unrelated_root_review_handle,
            unrelated_non_root_review_handle,
            originating_root_review,
        ) = {
            let mut handles = handles.lock().unwrap();
            handles.set_qualification_context(affected_context.clone());
            handles.set_qualification_context(unrelated_context.clone());
            let originating_root_review =
                root_review_for(&affected_handle, &affected_context, true);
            let originating_root_review_handle =
                handles.insert_review(originating_root_review.clone());
            let second_affected_root_review_handle =
                handles.insert_review(root_review_for(&affected_handle, &affected_context, false));
            let input_bound_affected_root_review_handle =
                handles.insert_review(root_review_for(&affected_handle, &affected_context, true));
            let affected_non_root_review_handle =
                handles.insert_review(non_root_review_for(&affected_handle, &affected_context));
            let unrelated_root_review_handle = handles.insert_review(root_review_for(
                &unrelated_handle,
                &unrelated_context,
                false,
            ));
            let unrelated_non_root_review_handle =
                handles.insert_review(non_root_review_for(&unrelated_handle, &unrelated_context));
            (
                originating_root_review_handle,
                second_affected_root_review_handle,
                input_bound_affected_root_review_handle,
                affected_non_root_review_handle,
                unrelated_root_review_handle,
                unrelated_non_root_review_handle,
                originating_root_review,
            )
        };
        let affected_key = RootQualificationKey::from_context(&affected_context);
        let unrelated_key = RootQualificationKey::from_context(&unrelated_context);
        let late_attempt = {
            let mut root = root.lock().unwrap();
            let unrelated_attempt = root.begin(unrelated_key.clone()).unwrap();
            assert!(root.complete(unrelated_attempt, RootQualificationState::Granted));
            let completed_affected = root.begin(affected_key.clone()).unwrap();
            assert!(root.complete(completed_affected, RootQualificationState::Granted));
            root.begin(affected_key.clone()).unwrap()
        };

        let executions = Mutex::new(ExecutionHandleStore::default());
        let execution_handle = {
            let mut executions = executions.lock().unwrap();
            executions.reserve_start(ExecutionKind::Real).unwrap();
            executions
                .bind_started(
                    ExecutionKind::Real,
                    "sidecar-root-terminal-expanded".to_string(),
                    originating_root_review_handle.clone(),
                    originating_root_review,
                )
                .public_handle
                .clone()
        };
        let terminal_response = || {
            Ok(json!({
                "execution": {
                    "executionId": "sidecar-root-terminal-expanded",
                    "status": "failed",
                    "errors": [{
                        "code": "root_authority_revoked",
                        "message": ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER,
                        "raw": "private root output: su -c id; uid=0; serial=sensitive-serial"
                    }],
                    "recipes": []
                }
            }))
        };
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![terminal_response(), terminal_response()]),
        };
        let generation_before = handles.lock().unwrap().device_generation();
        let affected_epoch_before = handles
            .lock()
            .unwrap()
            .session_epoch_for_test(&affected_handle)
            .unwrap();
        let unrelated_epoch_before = handles
            .lock()
            .unwrap()
            .session_epoch_for_test(&unrelated_handle)
            .unwrap();
        assert_eq!(
            root.lock().unwrap().get(&affected_key),
            Some(RootQualificationState::Granted)
        );
        assert_eq!(
            root.lock().unwrap().get(&unrelated_key),
            Some(RootQualificationState::Granted)
        );

        let (_temp, app) = test_app(executions, handles, root);
        let state = app.state::<AppState>();
        let handles = &state.handles;
        let root = &state.root_qualification;
        let executions = &state.executions;
        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        assert!(
            matches!(event, Some(RealExecutionMonitorEvent::Terminal { .. })),
            "the monitor must retain the authoritative terminal transition"
        );
        let first = get_real_execution_inner_with_runtime(&execution_handle, executions, &runtime)
            .expect("first expanded root terminal retrieval should succeed");
        assert_eq!(first["status"], "failed");
        assert_eq!(first["errors"][0]["remediation"]["kind"], "requalify_root");
        assert_eq!(first["completion"]["partialChangesPossible"], true);
        for forbidden in [
            "private root output",
            "su -c id",
            "uid=0",
            "sensitive-serial",
        ] {
            assert!(!first.to_string().contains(forbidden), "leaked {forbidden}");
        }
        assert_eq!(
            handles.lock().unwrap().device_generation(),
            generation_before
        );
        assert!(handles.lock().unwrap().device(&affected_handle).is_ok());
        assert_eq!(
            handles.lock().unwrap().facts(&affected_handle).unwrap(),
            &target_facts()
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .qualification_context(&affected_handle),
            Some(affected_context.clone())
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&affected_handle),
            Some(affected_epoch_before)
        );
        for review_handle in [
            &originating_root_review_handle,
            &second_affected_root_review_handle,
            &input_bound_affected_root_review_handle,
        ] {
            assert!(handles
                .lock()
                .unwrap()
                .review(review_handle)
                .unwrap_err()
                .contains("root_authority_changed"));
        }
        assert!(handles
            .lock()
            .unwrap()
            .review(&affected_non_root_review_handle)
            .is_ok());
        assert!(handles.lock().unwrap().device(&unrelated_handle).is_ok());
        assert_eq!(
            handles.lock().unwrap().facts(&unrelated_handle).unwrap(),
            &json!({
                "manufacturer": "Other",
                "model": "Pocket Other",
                "android_api_level": 33,
            })
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .qualification_context(&unrelated_handle),
            Some(unrelated_context.clone())
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&unrelated_handle),
            Some(unrelated_epoch_before)
        );
        assert!(handles
            .lock()
            .unwrap()
            .review(&unrelated_root_review_handle)
            .is_ok());
        assert!(handles
            .lock()
            .unwrap()
            .review(&unrelated_non_root_review_handle)
            .is_ok());
        assert!(handles
            .lock()
            .unwrap()
            .qualification_devices()
            .iter()
            .any(|device| device.handle == affected_handle && device.serial == "sensitive-serial"));
        assert!(
            handles
                .lock()
                .unwrap()
                .qualification_devices()
                .iter()
                .any(|device| device.handle == unrelated_handle
                    && device.serial == "unrelated-serial")
        );
        assert_eq!(root.lock().unwrap().get(&affected_key), None);
        assert_eq!(
            root.lock().unwrap().get(&unrelated_key),
            Some(RootQualificationState::Granted)
        );
        assert!(!root
            .lock()
            .unwrap()
            .complete(late_attempt, RootQualificationState::Granted));
        let originating_stale_error = handles
            .lock()
            .unwrap()
            .review(&originating_root_review_handle)
            .unwrap_err();
        assert!(executions
            .lock()
            .unwrap()
            .mapping(ExecutionKind::Real, &execution_handle, "missing")
            .is_ok());

        let repeated = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        assert!(
            repeated.is_none(),
            "an authoritative terminal transition must never be retained twice"
        );
        assert_eq!(
            handles.lock().unwrap().device_generation(),
            generation_before
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&affected_handle),
            Some(affected_epoch_before)
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&unrelated_handle),
            Some(unrelated_epoch_before)
        );
        assert!(handles
            .lock()
            .unwrap()
            .review(&affected_non_root_review_handle)
            .is_ok());
        assert!(handles
            .lock()
            .unwrap()
            .review(&unrelated_root_review_handle)
            .is_ok());
        assert!(handles
            .lock()
            .unwrap()
            .review(&unrelated_non_root_review_handle)
            .is_ok());
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .review(&originating_root_review_handle)
                .unwrap_err(),
            originating_stale_error
        );
        assert_eq!(
            root.lock().unwrap().get(&unrelated_key),
            Some(RootQualificationState::Granted)
        );

        let mapping = executions
            .lock()
            .unwrap()
            .mapping(ExecutionKind::Real, &execution_handle, "missing")
            .unwrap();
        let report = terminal_response().unwrap()["execution"].clone();
        let public = project_real_snapshot(&mapping, &report);
        let document =
            execution_report_document(&mapping, &report, &public, json!({ "status": "ready" }));
        let serialized = document.to_string();
        for forbidden in [
            ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER,
            "private root output",
            "su -c id",
            "uid=0",
            "serial=sensitive-serial",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        assert_eq!(
            handles.lock().unwrap().device_generation(),
            generation_before
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&affected_handle),
            Some(affected_epoch_before)
        );
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .session_epoch_for_test(&unrelated_handle),
            Some(unrelated_epoch_before)
        );
        assert_eq!(root.lock().unwrap().get(&affected_key), None);
        assert_eq!(
            root.lock().unwrap().get(&unrelated_key),
            Some(RootQualificationState::Granted)
        );
        assert!(handles
            .lock()
            .unwrap()
            .review(&affected_non_root_review_handle)
            .is_ok());
        assert!(handles
            .lock()
            .unwrap()
            .review(&unrelated_root_review_handle)
            .is_ok());
        assert!(handles
            .lock()
            .unwrap()
            .review(&unrelated_non_root_review_handle)
            .is_ok());
        assert!(executions
            .lock()
            .unwrap()
            .mapping(ExecutionKind::Real, &execution_handle, "missing")
            .is_ok());
    }

    /// Canonical tool runner for tests that never invoke the qualification
    /// tool: the session module only persists and materializes candidates.
    struct NoQualificationToolRunner;

    impl crate::qualification_repository::QualificationToolRunner for NoQualificationToolRunner {
        fn run(&self, _repo_root: &Path, _args: &[String]) -> Result<Vec<u8>, String> {
            Err(
                "terminal-monitor tests must not invoke the canonical qualification tool"
                    .to_string(),
            )
        }
    }

    fn qualification_build_json() -> Value {
        json!({
            "appVersion": "0.1.0",
            "gitCommit": "1".repeat(40),
            "materialBuildDigest": format!("sha256:{}", "a".repeat(64)),
            "realExecutionEnabled": true,
            "qualificationContract": 1,
        })
    }

    fn qualification_session_target() -> crate::qualification_session::QualificationTargetBinding {
        crate::qualification_session::QualificationTargetBinding {
            target_id: "target-test".to_string(),
            profile_id: "profile.test".to_string(),
            manufacturer: "Test".to_string(),
            model: "Device".to_string(),
            android_version: "15".to_string(),
            android_api: 35,
            abi_soc_class: "arm64".to_string(),
            root_state: crate::qualification_mode::QualificationRootState::NonRoot,
            connection_type: crate::qualification_mode::QualificationConnectionType::Usb3,
            firmware_build: "test/build".to_string(),
        }
    }

    fn qualification_session_workflow() -> crate::qualification_mode::QualificationWorkflow {
        crate::qualification_mode::QualificationWorkflow {
            id: "test-workflow".to_string(),
            version: 1,
            purpose: "test".to_string(),
            production_recipes: vec!["test.recipe".to_string()],
            required_capabilities: Vec::new(),
            prerequisites: Vec::new(),
            human_checkpoints: vec![crate::qualification_mode::QualificationWorkflowCheckpoint {
                id: "device_state_verified".to_string(),
                instruction: "Verify the device state".to_string(),
                fact: "device_state".to_string(),
                allowed_outcomes: vec![
                    crate::qualification_mode::QualificationCheckpointOutcome::Pass,
                    crate::qualification_mode::QualificationCheckpointOutcome::Fail,
                    crate::qualification_mode::QualificationCheckpointOutcome::UnableToVerify,
                ],
                required: true,
            }],
            compatibility_dimensions: Vec::new(),
            automated_observations: vec![
                crate::qualification_mode::QualificationWorkflowObservation {
                    id: "execution-report".to_string(),
                    required: true,
                },
            ],
        }
    }

    fn qualification_session_observation() -> crate::device_observation::SelectedDeviceObservation {
        qualification_session_observation_for("device-one")
    }

    fn qualification_session_observation_for(
        device_handle: &str,
    ) -> crate::device_observation::SelectedDeviceObservation {
        crate::device_observation::SelectedDeviceObservation {
            device_handle: device_handle.to_string(),
            session_epoch: Some(1),
            profile_id: Some("profile.test".to_string()),
            manufacturer: Some("Test".to_string()),
            model: Some("Device".to_string()),
            android_version: Some("15".to_string()),
            android_api: Some(35),
            abi_soc_class: Some("arm64".to_string()),
            firmware_build: Some("test/build".to_string()),
            root_state: Some(crate::device_qualification::RootQualificationState::Denied),
        }
    }

    fn qualification_admission_review() -> crate::qualification_session::ReviewObservation {
        qualification_admission_review_for("device-one")
    }

    fn qualification_admission_review_for(
        device_handle: &str,
    ) -> crate::qualification_session::ReviewObservation {
        crate::qualification_session::ReviewObservation {
            review_handle: "review-one".to_string(),
            device_handle: device_handle.to_string(),
            device_plan: "test-plan".to_string(),
            selected_recipes: vec!["test.recipe".to_string()],
            target_id: Some("target-test".to_string()),
            manufacturer: Some("Test".to_string()),
            model: Some("Device".to_string()),
            android_api: Some(35),
        }
    }

    /// Bind one real execution mapping for the monitor tests.
    fn bind_monitor_execution(
        executions: &Mutex<ExecutionHandleStore>,
        sidecar_id: &str,
    ) -> String {
        let mut executions = executions.lock().unwrap();
        executions.reserve_start(ExecutionKind::Real).unwrap();
        executions
            .bind_started(
                ExecutionKind::Real,
                sidecar_id.to_string(),
                "review-one".to_string(),
                review(),
            )
            .public_handle
            .clone()
    }

    #[test]
    fn product_terminal_monitor_retries_transient_failures_until_it_resolves() {
        let executions = Mutex::new(ExecutionHandleStore::default());
        let handles = Mutex::new(SessionHandles::default());
        let root = Mutex::new(RootQualificationStore::default());
        let execution_handle = bind_monitor_execution(&executions, "sidecar-retry");
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![
                Err(safe_error(
                    "execution_status_unavailable",
                    "transient status",
                )),
                Err(safe_error(
                    "execution_status_unavailable",
                    "transient status",
                )),
                Ok(json!({
                    "execution": {
                        "executionId": "sidecar-retry",
                        "status": "succeeded",
                        "recipes": []
                    }
                })),
            ]),
        };
        let (_temp, app) = test_app(executions, handles, root);
        let state = app.state::<AppState>();
        let mut waits = Vec::new();
        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |interval| {
            waits.push(interval)
        });

        assert!(
            matches!(event, Some(RealExecutionMonitorEvent::Terminal { .. })),
            "the monitor must keep observing until the execution is resolved"
        );
        assert_eq!(
            waits,
            vec![Duration::from_secs(2), Duration::from_secs(4)],
            "transient failures must back off instead of giving up"
        );
        assert!(state
            .executions
            .lock()
            .unwrap()
            .terminal_retained(ExecutionKind::Real, &execution_handle));
        assert!(monitor_resolution_message(&event.unwrap()).contains(&execution_handle));
    }

    #[test]
    fn product_terminal_monitor_resolves_a_lost_runtime_session_as_product_loss() {
        let executions = Mutex::new(ExecutionHandleStore::default());
        let handles = Mutex::new(SessionHandles::default());
        let root = Mutex::new(RootQualificationStore::default());
        let execution_handle = bind_monitor_execution(&executions, "sidecar-lost");
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![Err(safe_error(
                "runtime_session_lost",
                "the runtime session is gone",
            ))]),
        };
        let (_temp, app) = test_app(executions, handles, root);
        let state = app.state::<AppState>();
        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});

        let Some(RealExecutionMonitorEvent::Lost {
            execution_handle: lost,
        }) = event
        else {
            panic!("a lost runtime session must resolve through product loss semantics");
        };
        assert_eq!(lost, execution_handle);
        assert!(
            state
                .executions
                .lock()
                .unwrap()
                .mapping(ExecutionKind::Real, &execution_handle, "missing")
                .is_err(),
            "the lost runtime authority must drop the execution mapping"
        );
        let message = monitor_resolution_message(&RealExecutionMonitorEvent::Lost {
            execution_handle: execution_handle.clone(),
        });
        assert!(message.contains(&execution_handle));
        assert!(!message.contains("sensitive-serial"));
    }

    #[test]
    fn product_terminal_monitor_retries_when_runtime_loss_recovery_fails() {
        let executions = Mutex::new(ExecutionHandleStore::default());
        let handles = Mutex::new(SessionHandles::default());
        let root = Mutex::new(RootQualificationStore::default());
        let execution_handle = bind_monitor_execution(&executions, "sidecar-recovery-retry");
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![Err(safe_error(
                "runtime_session_lost",
                "the runtime session is gone",
            ))]),
        };
        let (_temp, app) = test_app(executions, handles, root);
        let state = app.state::<AppState>();
        let attempts = std::cell::Cell::new(0);
        let mut waits = Vec::new();

        let event = monitor_real_terminal_with_loss_recovery(
            &execution_handle,
            &state,
            &runtime,
            &mut |interval| waits.push(interval),
            &mut |state, handle, error| {
                let attempt = attempts.get() + 1;
                attempts.set(attempt);
                if attempt == 1 {
                    Err(safe_error(
                        "execution_recovery_unavailable",
                        "transient recovery failure",
                    ))
                } else {
                    recover_from_real_execution_loss(state, handle, error)
                }
            },
        );

        assert!(matches!(
            event,
            Some(RealExecutionMonitorEvent::Lost { .. })
        ));
        assert_eq!(
            attempts.get(),
            2,
            "recovery is retried after the first failure"
        );
        assert_eq!(waits, vec![Duration::from_secs(1), Duration::from_secs(2)]);
        assert!(state.executions.lock().unwrap().is_lost(&execution_handle));
    }

    #[test]
    fn product_terminal_monitor_resolves_a_disappeared_mapping_and_invalidates_qualification() {
        let (_repository_root, _app_root, app, execution_handle, candidate) =
            begin_monitor_qualification_attempt();
        let state = app.state::<AppState>();
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![Err(safe_error(
                "execution_status_unavailable",
                "transient status",
            ))]),
        };

        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {
            state
                .executions
                .lock()
                .unwrap()
                .forget_mapping(ExecutionKind::Real, &execution_handle);
        });

        assert!(matches!(
            event,
            Some(RealExecutionMonitorEvent::Lost { .. })
        ));
        assert!(state.executions.lock().unwrap().is_lost(&execution_handle));
        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid"),
            "mapping loss must reach the active qualification attempt as product loss"
        );
    }

    #[test]
    fn product_terminal_monitor_recovers_a_poisoned_execution_store_as_product_loss() {
        let (_repository_root, _app_root, app, execution_handle, candidate) =
            begin_monitor_qualification_attempt();
        let state = app.state::<AppState>();
        let poison_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _executions = state.executions.lock().unwrap();
            panic!("poison the execution store for the recovery regression");
        }));
        assert!(poison_result.is_err());
        assert!(state.executions.is_poisoned());
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(Vec::new()),
        };

        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});

        assert!(matches!(
            event,
            Some(RealExecutionMonitorEvent::Lost { .. })
        ));
        assert!(!state.executions.is_poisoned());
        assert!(state.executions.lock().unwrap().is_lost(&execution_handle));
        assert!(runtime.requests.lock().unwrap().is_empty());
        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid"),
            "the recovered runtime loss must reach the active qualification attempt"
        );
    }

    #[test]
    fn inventory_commit_cannot_be_overtaken_by_terminal_materialization() {
        let (_repository_root, _app_root, app, execution_handle, candidate, _device_handle) =
            begin_monitor_qualification_attempt_with_live_device();
        let state = app.state::<AppState>();
        let state = &*state;
        let mapping = state
            .executions
            .lock()
            .unwrap()
            .mapping(
                ExecutionKind::Real,
                &execution_handle,
                REAL_EXECUTION_UNAVAILABLE,
            )
            .unwrap();
        let report = json!({
            "executionId": "sidecar-qualification",
            "status": "succeeded",
            "errors": [],
            "recipes": []
        });
        let (inventory_committed_tx, inventory_committed_rx) = std::sync::mpsc::channel();
        let (continue_inventory_tx, continue_inventory_rx) = std::sync::mpsc::channel();
        let (terminal_started_tx, terminal_started_rx) = std::sync::mpsc::channel();
        let (terminal_done_tx, terminal_done_rx) = std::sync::mpsc::channel();

        let terminal_completed_early = std::thread::scope(|scope| {
            let inventory = scope.spawn(move || {
                crate::commands::reconcile_inventory_snapshot_with_state_and_hook(
                    state,
                    &json!({ "devices": [] }),
                    0,
                    0,
                    || {
                        inventory_committed_tx
                            .send(())
                            .expect("the inventory test must reach its commit barrier");
                        continue_inventory_rx
                            .recv()
                            .expect("the test must release the inventory notification");
                    },
                )
            });
            inventory_committed_rx
                .recv()
                .expect("product inventory authority should commit before notification");

            let terminal = scope.spawn(move || {
                terminal_started_tx
                    .send(())
                    .expect("the terminal attempt should start");
                let result = retain_terminal_real_execution(
                    state,
                    &mapping,
                    &report,
                    "2026-10-03T12:00:00Z",
                );
                terminal_done_tx
                    .send(result)
                    .expect("the terminal result should be delivered");
            });
            terminal_started_rx
                .recv()
                .expect("terminal retention should be attempted while inventory is paused");
            let completed_before_inventory_notification = terminal_done_rx.try_recv().is_ok();

            continue_inventory_tx
                .send(())
                .expect("the inventory notification should be released");
            inventory
                .join()
                .expect("inventory reconciliation should complete")
                .expect("the inventory transition should remain a successful product result");
            terminal
                .join()
                .expect("terminal retention should complete after inventory notification");
            completed_before_inventory_notification
        });
        assert!(
            !terminal_completed_early,
            "terminal retention must wait behind the committed inventory notification"
        );
        let terminal = terminal_done_rx
            .recv()
            .expect("the ordered terminal result should be returned")
            .expect("terminal retention should succeed")
            .expect("terminal retention should be the first terminal commit");
        assert!(matches!(
            terminal,
            RealExecutionMonitorEvent::Terminal { .. }
        ));

        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload["runValidity"], "invalid",
            "the committed disconnect must be observed before terminal evidence is materialized"
        );
    }

    #[test]
    fn terminal_commit_before_later_inventory_removal_keeps_the_completed_candidate_valid() {
        let (_repository_root, _app_root, app, execution_handle, candidate, _device_handle) =
            begin_monitor_qualification_attempt_with_live_device();
        let state = app.state::<AppState>();
        let state = &*state;
        let mapping = state
            .executions
            .lock()
            .unwrap()
            .mapping(
                ExecutionKind::Real,
                &execution_handle,
                REAL_EXECUTION_UNAVAILABLE,
            )
            .unwrap();
        let report = json!({
            "executionId": "sidecar-qualification",
            "status": "succeeded",
            "errors": [],
            "recipes": []
        });

        retain_terminal_real_execution(state, &mapping, &report, "2026-10-03T12:00:00Z")
            .expect("the terminal product transition should be retained")
            .expect("the terminal product transition should be new");
        crate::commands::reconcile_inventory_snapshot_with_state_and_hook(
            state,
            &json!({ "devices": [] }),
            0,
            0,
            || {},
        )
        .expect("a later inventory change must retain its ordinary product result");

        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(stored.payload["runValidity"], "valid");
    }

    #[test]
    fn authoritative_runtime_loss_retries_deferred_qualification_materialization() {
        let (repository_root, _app_root, app, execution_handle, candidate) =
            begin_monitor_qualification_attempt();
        let state = app.state::<AppState>();
        let recipe_path = repository_root
            .path()
            .join("authored/recipes/test.recipe.yaml");
        std::fs::write(
            &recipe_path,
            b"id: test.recipe\nsteps: [temporary change]\n",
        )
        .unwrap();
        let mapping = state
            .executions
            .lock()
            .unwrap()
            .mapping(
                ExecutionKind::Real,
                &execution_handle,
                REAL_EXECUTION_UNAVAILABLE,
            )
            .unwrap();
        let report = json!({
            "executionId": "sidecar-qualification",
            "status": "succeeded",
            "errors": [],
            "recipes": []
        });
        retain_terminal_real_execution(&state, &mapping, &report, "2026-10-03T12:00:00Z")
            .expect("the product terminal result should be retained")
            .expect("the terminal transition should be new");

        std::fs::write(&recipe_path, b"id: test.recipe\n").unwrap();
        let error = safe_error("runtime_session_lost", "the runtime session is gone");
        recover_from_real_execution_loss(&state, &execution_handle, &error)
            .expect("authoritative runtime loss should remain a successful product transition");

        assert!(state.executions.lock().unwrap().is_lost(&execution_handle));
        assert!(crate::qualification_session::session_status(&state)
            .unwrap()
            .is_none());
        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(stored.payload["runValidity"], "valid");
        assert!(stored.promotable);
    }

    /// Prepare one active qualification attempt bound to one real execution so
    /// a test can drive the product terminal monitor through the same seams
    /// production uses.
    ///
    /// The first returned directory holds the authored corpus and qualification
    /// repository root, the second holds the application state directories.
    fn begin_monitor_qualification_attempt() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        tauri::App<tauri::test::MockRuntime>,
        String,
        String,
    ) {
        begin_monitor_qualification_attempt_with_handles(SessionHandles::default(), "device-one")
    }

    fn begin_monitor_qualification_attempt_with_live_device() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        tauri::App<tauri::test::MockRuntime>,
        String,
        String,
        String,
    ) {
        let mut handles = SessionHandles::default();
        handles
            .update_devices(&json!({
                "devices": [{
                    "serial": "monitor-test-serial",
                    "state": "available",
                    "model": "Device",
                    "transportId": "monitor-test-transport"
                }]
            }))
            .unwrap();
        let device_handle = handles
            .single_available_device_handle()
            .expect("the fixture should retain one available native device");
        let (repository_root, app_root, app, execution_handle, candidate) =
            begin_monitor_qualification_attempt_with_handles(handles, &device_handle);
        (
            repository_root,
            app_root,
            app,
            execution_handle,
            candidate,
            device_handle,
        )
    }

    fn begin_monitor_qualification_attempt_with_handles(
        native_handles: SessionHandles,
        device_handle: &str,
    ) -> (
        tempfile::TempDir,
        tempfile::TempDir,
        tauri::App<tauri::test::MockRuntime>,
        String,
        String,
    ) {
        let repository_root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repository_root.path().join("authored/recipes")).unwrap();
        std::fs::write(
            repository_root
                .path()
                .join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\n",
        )
        .unwrap();
        let repository =
            crate::qualification_repository::QualificationRepository::new_for_test_with_source_state(
                repository_root.path().to_path_buf(),
                Box::new(NoQualificationToolRunner),
                serde_json::from_value(qualification_build_json()).unwrap(),
                crate::qualification_repository::QualificationSourceState {
                    head: "1".repeat(40),
                    tracked_worktree_clean: true,
                },
            );
        let candidate = repository
            .create_candidate(
                crate::qualification_repository::CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": qualification_build_json(),
                }),
                None,
            )
            .unwrap();
        let executions = Mutex::new(ExecutionHandleStore::default());
        let handles = Mutex::new(native_handles);
        let root = Mutex::new(RootQualificationStore::default());
        let execution_handle = bind_monitor_execution(&executions, "sidecar-qualification");
        let provider =
            crate::qualification_repository::QualificationRepositoryProvider::for_test(repository);
        let (app_root, app) = test_app_with_qualification(executions, handles, root, provider);
        let state = app.state::<AppState>();
        let session_handle =
            crate::qualification_session::session_handle_for_candidate(&candidate).unwrap();
        crate::qualification_session::begin(
            &state,
            crate::qualification_session::BeginSessionRequest {
                session_handle: session_handle.clone(),
                candidate_handle: candidate.clone(),
                captured_at: "2026-08-23T12:00:00Z".to_string(),
                device_plan: "test-plan".to_string(),
                target: qualification_session_target(),
                workflow: qualification_session_workflow(),
                build: serde_json::from_value(qualification_build_json()).unwrap(),
                runtime_contract: "real-execution-v1".to_string(),
                observation: qualification_session_observation_for(device_handle),
            },
        )
        .expect("the attempt should begin against the prepared candidate");
        crate::qualification_session::record_checkpoint(
            &state,
            &session_handle,
            "device_state_verified",
            crate::qualification_mode::QualificationCheckpointOutcome::Pass,
        )
        .expect("the required checkpoint should be recorded");
        crate::qualification_session::observe(
            &state,
            crate::qualification_session::QualificationLifecycleObservation::RealExecutionAdmitted(
                Box::new(
                    crate::qualification_session::ExecutionAdmissionObservation {
                        execution_handle: execution_handle.clone(),
                        review: qualification_admission_review_for(device_handle),
                        device_handle: device_handle.to_string(),
                    },
                ),
            ),
        );
        (repository_root, app_root, app, execution_handle, candidate)
    }

    /// Compose one scripted terminal execution report for the monitor tests.
    fn terminal_execution_json(execution_id: &str, status: &str, errors: Value) -> Value {
        json!({
            "execution": {
                "executionId": execution_id,
                "status": status,
                "errors": errors,
                "recipes": []
            }
        })
    }

    #[test]
    fn lost_execution_returns_a_terminal_empty_event_batch() {
        let executions = Mutex::new(ExecutionHandleStore::default());
        let execution_handle = bind_monitor_execution(&executions, "sidecar-lost-events");
        {
            let mut store = executions.lock().unwrap();
            let mapping = store
                .mapping(
                    ExecutionKind::Real,
                    &execution_handle,
                    REAL_EXECUTION_UNAVAILABLE,
                )
                .unwrap();
            store.mark_lost(&execution_handle, Some(mapping));
        }
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err("the sidecar session was lost".to_string()),
        };

        let snapshot =
            get_real_execution_inner_with_runtime(&execution_handle, &executions, &runtime)
                .expect("lost execution state remains an authoritative terminal projection");
        assert_eq!(snapshot["executionHandle"], execution_handle);
        assert_eq!(snapshot["status"], "failed");

        let events = get_real_execution_events_inner_with_runtime(
            &execution_handle,
            9,
            &executions,
            &runtime,
        )
        .expect("authoritative loss should project as an empty terminal batch");

        assert_eq!(events["executionHandle"], execution_handle);
        assert_eq!(events["events"], json!([]));
        assert_eq!(events["latestSequence"], 0);
        assert_eq!(events["terminal"], true);
        assert!(runtime.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn retained_terminal_event_poll_preserves_unseen_runtime_tail() {
        let executions = Mutex::new(ExecutionHandleStore::default());
        let execution_handle = bind_monitor_execution(&executions, "sidecar-retained-event-tail");
        {
            let mut store = executions.lock().unwrap();
            assert!(store.mark_terminal_with_report(
                ExecutionKind::Real,
                &execution_handle,
                json!({
                    "status": "succeeded",
                    "latestSequence": 25,
                    "recipes": [],
                    "errors": [],
                    "warnings": []
                }),
                json!({ "status": "ready" }),
            ));
        }
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Ok(json!({
                "events": [
                    {
                        "sequence": 21,
                        "timestamp": "2026-10-03T10:01:21Z",
                        "eventType": "execution_started",
                        "status": "running"
                    },
                    {
                        "sequence": 25,
                        "timestamp": "2026-10-03T10:01:25Z",
                        "eventType": "execution_completed",
                        "status": "succeeded"
                    }
                ],
                "latestSequence": 24,
                "terminal": false
            })),
        };

        let batch = get_real_execution_events_inner_with_runtime(
            &execution_handle,
            20,
            &executions,
            &runtime,
        )
        .expect("retained terminal state should include any still-readable event tail");

        assert_eq!(batch["executionHandle"], execution_handle);
        assert_eq!(
            batch["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| event["sequence"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![21, 25]
        );
        assert_eq!(batch["terminal"], true);
        assert_eq!(batch["latestSequence"], 25);
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "getExecutionEvents");
        assert_eq!(requests[0].1["afterSequence"], 20);
    }

    #[test]
    fn retained_terminal_snapshot_and_event_poll_survive_runtime_loss() {
        let executions = Mutex::new(ExecutionHandleStore::default());
        let execution_handle = bind_monitor_execution(&executions, "sidecar-retained-terminal");
        {
            let mut store = executions.lock().unwrap();
            let mapping = store
                .mapping(
                    ExecutionKind::Real,
                    &execution_handle,
                    REAL_EXECUTION_UNAVAILABLE,
                )
                .unwrap();
            let report = json!({
                "status": "succeeded",
                "startedAt": "2026-10-03T10:00:00Z",
                "finishedAt": "2026-10-03T10:01:00Z",
                "latestSequence": 4,
                "recipes": [],
                "errors": [],
                "warnings": []
            });
            store.launch_actions.insert(
                "launch-retained".to_string(),
                LaunchActionRecord {
                    action_handle: "launch-retained".to_string(),
                    label: "Open app".to_string(),
                    mapping,
                },
            );
            assert!(store.mark_terminal_with_report(
                ExecutionKind::Real,
                &execution_handle,
                report,
                json!({ "status": "ready" }),
            ));
        }
        let (_temp, app) = test_app(
            executions,
            Mutex::new(SessionHandles::default()),
            Mutex::new(RootQualificationStore::default()),
        );

        let snapshot = get_real_execution(execution_handle.clone(), app.state())
            .expect("the retained terminal report remains a pure projection");
        assert_eq!(snapshot["status"], "succeeded");
        assert_eq!(snapshot["launchAction"]["handle"], "launch-retained");

        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err("the sidecar session was lost".to_string()),
        };
        let events = get_real_execution_events_inner_with_runtime(
            &execution_handle,
            2,
            &app.state::<AppState>().executions,
            &runtime,
        )
        .expect("retained terminal state remains available after runtime loss");
        assert_eq!(events["executionHandle"], execution_handle);
        assert_eq!(events["events"], json!([]));
        assert_eq!(events["latestSequence"], 4);
        assert_eq!(events["terminal"], true);
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "getExecutionEvents");
        assert_eq!(requests[0].1["afterSequence"], 2);
        drop(requests);

        let still_retained = get_real_execution(execution_handle, app.state()).unwrap();
        assert_eq!(still_retained["status"], "succeeded");
        assert_eq!(still_retained["launchAction"]["handle"], "launch-retained");
    }

    #[test]
    fn terminal_retention_during_event_poll_preserves_unseen_runtime_events() {
        let executions = Mutex::new(ExecutionHandleStore::default());
        let execution_handle = bind_monitor_execution(&executions, "sidecar-terminal-during-poll");
        let (_temp, app) = test_app(
            executions,
            Mutex::new(SessionHandles::default()),
            Mutex::new(RootQualificationStore::default()),
        );
        let state = app.state::<AppState>();
        let runtime = TerminalDuringPollRuntime {
            executions: &state.executions,
            execution_handle: &execution_handle,
            response: json!({
                "events": [{
                    "sequence": 7,
                    "timestamp": "2026-10-03T10:01:30Z",
                    "status": "running"
                }],
                "latestSequence": 7,
                "terminal": false
            }),
        };

        let batch = get_real_execution_events_inner_with_runtime(
            &execution_handle,
            6,
            &state.executions,
            &runtime,
        )
        .expect("poll should retain runtime events and the authoritative terminal state");

        assert_eq!(batch["executionHandle"], execution_handle);
        assert_eq!(batch["events"][0]["sequence"], 7);
        assert_eq!(batch["latestSequence"], 8);
        assert_eq!(batch["terminal"], true);
    }

    #[test]
    fn product_terminal_monitor_feeds_the_committed_transition_to_qualification() {
        let (_repository_root, _app_root, app, execution_handle, candidate) =
            begin_monitor_qualification_attempt();
        let state = app.state::<AppState>();
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![Ok(terminal_execution_json(
                "sidecar-qualification",
                "succeeded",
                json!([]),
            ))]),
        };

        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        assert!(matches!(
            event,
            Some(RealExecutionMonitorEvent::Terminal { .. })
        ));

        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("valid"),
            "the product terminal transition must reach the qualification attempt"
        );
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("passed")
        );
        assert_eq!(
            stored
                .payload
                .pointer("/artifacts/0/path")
                .and_then(Value::as_str),
            Some("execution-report.json")
        );
        assert!(crate::qualification_session::session_status(&state)
            .unwrap()
            .is_none());
    }

    #[test]
    fn product_terminal_monitor_keeps_authority_loss_inside_qualification_evidence() {
        let (_repository_root, _app_root, app, execution_handle, candidate) =
            begin_monitor_qualification_attempt();
        let state = app.state::<AppState>();
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![Ok(terminal_execution_json(
                "sidecar-qualification",
                "failed",
                json!([{ "code": "device_identity_changed", "message": "identity detail" }]),
            ))]),
        };

        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        let Some(RealExecutionMonitorEvent::Terminal { observation }) = event else {
            panic!("the monitor must retain the authoritative terminal transition");
        };
        assert!(
            observation.authority_invalidated,
            "a terminal identity failure must be reported as invalidated authority"
        );

        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid"),
            "an invalidated authority transition can never produce valid evidence"
        );
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("not_observed")
        );
        assert!(stored
            .payload
            .get("automatedObservations")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));
        assert!(stored
            .payload
            .get("artifacts")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty));
        assert!(crate::qualification_session::session_status(&state)
            .unwrap()
            .is_none());
    }

    #[test]
    fn identity_failure_takes_precedence_over_root_invalidation() {
        let (handles, root, review_handle) = prepared_real_review(true);
        let device_handle = handles
            .lock()
            .unwrap()
            .review(&review_handle)
            .unwrap()
            .device_handle
            .clone();
        let review_snapshot = handles
            .lock()
            .unwrap()
            .review(&review_handle)
            .unwrap()
            .clone();
        let executions = Mutex::new(ExecutionHandleStore::default());
        let execution_handle = {
            let mut executions = executions.lock().unwrap();
            executions.reserve_start(ExecutionKind::Real).unwrap();
            executions
                .bind_started(
                    ExecutionKind::Real,
                    "sidecar-combined-terminal".to_string(),
                    review_handle.clone(),
                    review_snapshot,
                )
                .public_handle
                .clone()
        };
        let combined_terminal = || {
            Ok(json!({
                "execution": {
                    "executionId": "sidecar-combined-terminal",
                    "status": "failed",
                    "errors": [
                        { "code": "root_authority_revoked", "message": "root detail" },
                        { "code": "device_identity_changed", "message": "identity detail" }
                    ],
                    "recipes": []
                }
            }))
        };
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![combined_terminal(), combined_terminal()]),
        };
        let (_temp, app) = test_app(executions, handles, root);
        let state = app.state::<AppState>();
        let handles = &state.handles;
        let executions = &state.executions;
        let event = monitor_real_terminal(&execution_handle, &state, &runtime, &mut |_| {});
        assert!(
            matches!(event, Some(RealExecutionMonitorEvent::Terminal { .. })),
            "the monitor must retain the authoritative terminal transition"
        );
        let public = get_real_execution_inner_with_runtime(&execution_handle, executions, &runtime)
            .expect("combined terminal retrieval should succeed");
        assert_eq!(
            public["errors"][1]["remediation"]["kind"],
            "reconnect_device"
        );
        assert!(handles.lock().unwrap().device(&device_handle).is_err());
        assert!(handles
            .lock()
            .unwrap()
            .review(&review_handle)
            .unwrap_err()
            .contains("review_stale"));
    }

    #[test]
    fn integrated_real_preflight_rejects_every_listed_qualification_failure_without_start() {
        let cases = [
            (
                "unsupported",
                supported_inventory("transport-1"),
                json!({
                    "state": "online", "androidMajor": 10, "androidApiLevel": 29,
                    "abi": "arm64-v8a", "storage": "available", "packageManager": "available", "activityManager": "available"
                }),
                1,
                2,
            ),
            (
                "insufficient",
                supported_inventory("transport-1"),
                json!({
                    "state": "online", "androidMajor": 14, "androidApiLevel": 33,
                    "abi": "arm64-v8a", "storage": "available", "packageManager": "unknown", "activityManager": "available"
                }),
                1,
                2,
            ),
            (
                "unauthorized",
                json!({ "devices": [{ "serial": "sensitive-serial", "state": "unauthorized", "model": "Pocket S", "transportId": "transport-1" }] }),
                supported_qualification(),
                1,
                2,
            ),
            (
                "offline",
                json!({ "devices": [{ "serial": "sensitive-serial", "state": "offline", "model": "Pocket S", "transportId": "transport-1" }] }),
                supported_qualification(),
                1,
                2,
            ),
            (
                "no-device",
                json!({ "devices": [] }),
                supported_qualification(),
                1,
                2,
            ),
            (
                "multiple-device",
                json!({ "devices": [
                { "serial": "sensitive-serial", "state": "available", "model": "Pocket S", "transportId": "transport-1" },
                { "serial": "second-serial", "state": "available", "model": "Other", "transportId": "transport-2" }
            ] }),
                supported_qualification(),
                1,
                2,
            ),
            (
                "changed-transport",
                supported_inventory("transport-2"),
                supported_qualification(),
                1,
                2,
            ),
            (
                "runtime-mismatch",
                supported_inventory("transport-1"),
                supported_qualification(),
                9,
                2,
            ),
            (
                "revision-mismatch",
                supported_inventory("transport-1"),
                supported_qualification(),
                1,
                9,
            ),
            (
                "changed-fingerprint",
                supported_inventory("transport-1"),
                json!({
                    "state": "online", "androidMajor": 15, "androidApiLevel": 35,
                    "abi": "arm64-v8a", "storage": "available", "packageManager": "available", "activityManager": "available"
                }),
                1,
                2,
            ),
        ];
        for (name, inventory, qualification, runtime_generation, platform_tools_revision) in cases {
            let (handles, root, review_handle) = prepared_real_review(false);
            let (result, starts) = run_integrated_case(
                &review_handle,
                &handles,
                &root,
                inventory,
                Some(qualification),
                runtime_generation,
                platform_tools_revision,
            );
            assert_eq!(starts, 0, "{name} sent startExecution: {result:?}");
            assert!(result.is_err(), "{name} unexpectedly started");
        }
    }

    #[test]
    fn integrated_real_preflight_rejects_stale_handle_and_root_without_start() {
        let (handles, root, _review_handle) = prepared_real_review(false);
        let (result, starts) = run_integrated_case(
            "review_unknown",
            &handles,
            &root,
            supported_inventory("transport-1"),
            Some(supported_qualification()),
            1,
            2,
        );
        assert!(result.is_err());
        assert_eq!(starts, 0);

        let (handles, root, review_handle) = prepared_real_review(true);
        let (result, starts) = run_integrated_case(
            &review_handle,
            &handles,
            &root,
            supported_inventory("transport-1"),
            Some(supported_qualification()),
            1,
            2,
        );
        let error = result.unwrap_err();
        assert!(error.contains("root_qualification_required"), "{error}");
        assert_eq!(starts, 0);

        let (handles, root, review_handle) = prepared_real_review(true);
        {
            let mut root = root.lock().unwrap();
            let stale_attempt = root
                .begin(RootQualificationKey::new("stale-device", 7, 8))
                .unwrap();
            assert!(root.complete(stale_attempt, RootQualificationState::Granted));
        }
        let (result, starts) = run_integrated_case(
            &review_handle,
            &handles,
            &root,
            supported_inventory("transport-1"),
            Some(supported_qualification()),
            1,
            2,
        );
        let error = result.unwrap_err();
        assert!(error.contains("root_qualification_required"), "{error}");
        assert_eq!(starts, 0);

        let (handles, root, review_handle) = prepared_real_review_with_epoch(false, Some(0));
        let (result, starts) = run_integrated_case(
            &review_handle,
            &handles,
            &root,
            supported_inventory("transport-1"),
            Some(supported_qualification()),
            1,
            2,
        );
        assert!(result.unwrap_err().contains("review_stale"));
        assert_eq!(starts, 0);
    }

    #[test]
    fn final_probe_drift_invalidates_qualification_without_changing_product_admission() {
        let repository_root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repository_root.path().join("authored/recipes")).unwrap();
        std::fs::write(
            repository_root
                .path()
                .join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\n",
        )
        .unwrap();
        let repository =
            crate::qualification_repository::QualificationRepository::new_for_test_with_source_state(
                repository_root.path().to_path_buf(),
                Box::new(NoQualificationToolRunner),
                serde_json::from_value(qualification_build_json()).unwrap(),
                crate::qualification_repository::QualificationSourceState {
                    head: "1".repeat(40),
                    tracked_worktree_clean: true,
                },
            );
        let candidate = repository
            .create_candidate(
                crate::qualification_repository::CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": qualification_build_json(),
                }),
                None,
            )
            .unwrap();
        let (handles, root, review_handle) = prepared_real_review(false);
        let device_handle = handles
            .lock()
            .unwrap()
            .review(&review_handle)
            .unwrap()
            .device_handle
            .clone();
        let device_session_epoch = handles
            .lock()
            .unwrap()
            .device_session_epoch(&device_handle)
            .expect("prepared review must retain the device epoch");
        let provider =
            crate::qualification_repository::QualificationRepositoryProvider::for_test(repository);
        let (_app_root, app) = test_app_with_qualification(
            Mutex::new(ExecutionHandleStore::default()),
            handles,
            root,
            provider,
        );
        let state = app.state::<AppState>();
        let session_handle =
            crate::qualification_session::session_handle_for_candidate(&candidate).unwrap();
        let mut target = qualification_session_target();
        target.manufacturer = "AYANEO".to_string();
        target.model = "Pocket S".to_string();
        target.android_version = "15".to_string();
        target.android_api = 33;
        target.abi_soc_class = "arm64-v8a".to_string();
        target.firmware_build = "original/build".to_string();
        let mut observation = qualification_session_observation();
        observation.device_handle = device_handle.clone();
        observation.session_epoch = Some(device_session_epoch);
        observation.manufacturer = Some(target.manufacturer.clone());
        observation.model = Some(target.model.clone());
        observation.android_version = Some(target.android_version.clone());
        observation.android_api = Some(target.android_api);
        observation.abi_soc_class = Some(target.abi_soc_class.clone());
        observation.firmware_build = Some(target.firmware_build.clone());
        crate::qualification_session::begin(
            &state,
            crate::qualification_session::BeginSessionRequest {
                session_handle: session_handle.clone(),
                candidate_handle: candidate.clone(),
                captured_at: "2026-08-23T12:00:00Z".to_string(),
                device_plan: "test-plan".to_string(),
                target,
                workflow: qualification_session_workflow(),
                build: serde_json::from_value(qualification_build_json()).unwrap(),
                runtime_contract: "real-execution-v1".to_string(),
                observation,
            },
        )
        .unwrap();
        crate::qualification_session::record_checkpoint(
            &state,
            &session_handle,
            "device_state_verified",
            crate::qualification_mode::QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        let mut executions = ExecutionHandleStore::default();
        executions.reserve_start(ExecutionKind::Real).unwrap();
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![
                Ok(supported_inventory("transport-1")),
                Ok(json!({
                    "manufacturer": "AYANEO",
                    "model": "Pocket S",
                    "android_version": "15",
                    "android_api_level": 33,
                    "firmware_build": "changed/build",
                })),
                Ok(supported_qualification()),
                Ok(match_result("profile.test")),
                Ok(json!({ "execution": { "executionId": "sidecar-final-gate" } })),
            ]),
        };
        let platform_tools = PlatformToolsSnapshot {
            adb_path: "/trusted/adb",
            runtime_generation: 1,
            platform_tools_revision: 2,
        };

        let result = start_real_execution_inner_with_runtime(
            &review_handle,
            Some(&state),
            &state.handles,
            &state.root_qualification,
            &mut executions,
            &runtime,
            &platform_tools,
        );

        assert!(
            result.is_ok(),
            "qualification invalidation must not change product admission: {result:?}; requests: {:?}",
            runtime
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|(kind, _)| kind)
                .collect::<Vec<_>>()
        );
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|(request_type, _)| request_type == "probeDevice")
                .count(),
            1,
            "final device facts must come from the product probe"
        );
        let profile_match = requests
            .iter()
            .find(|(request_type, _)| request_type == "matchDevice")
            .expect("qualification should refresh the catalog profile from final probe facts");
        assert_eq!(
            profile_match.1["facts"]["firmware_build"], "changed/build",
            "the current profile match must consume the exact final probe payload"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|(request_type, _)| request_type == "startExecution")
                .count(),
            1
        );
        drop(requests);
        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid"),
            "the exact final probe observation must invalidate the drifted target"
        );
    }

    #[test]
    fn final_catalog_profile_drift_invalidates_qualification_without_blocking_execution() {
        let repository_root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repository_root.path().join("authored/recipes")).unwrap();
        std::fs::write(
            repository_root
                .path()
                .join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\n",
        )
        .unwrap();
        let repository =
            crate::qualification_repository::QualificationRepository::new_for_test_with_source_state(
                repository_root.path().to_path_buf(),
                Box::new(NoQualificationToolRunner),
                serde_json::from_value(qualification_build_json()).unwrap(),
                crate::qualification_repository::QualificationSourceState {
                    head: "1".repeat(40),
                    tracked_worktree_clean: true,
                },
            );
        let candidate = repository
            .create_candidate(
                crate::qualification_repository::CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": qualification_build_json(),
                }),
                None,
            )
            .unwrap();
        let (handles, root, review_handle) = prepared_real_review(false);
        let device_handle = handles
            .lock()
            .unwrap()
            .review(&review_handle)
            .unwrap()
            .device_handle
            .clone();
        let device_session_epoch = handles
            .lock()
            .unwrap()
            .device_session_epoch(&device_handle)
            .expect("prepared review must retain the device epoch");
        let provider =
            crate::qualification_repository::QualificationRepositoryProvider::for_test(repository);
        let (_app_root, app) = test_app_with_qualification(
            Mutex::new(ExecutionHandleStore::default()),
            handles,
            root,
            provider,
        );
        let state = app.state::<AppState>();
        let session_handle =
            crate::qualification_session::session_handle_for_candidate(&candidate).unwrap();
        let mut observation = qualification_session_observation();
        observation.device_handle = device_handle.clone();
        observation.session_epoch = Some(device_session_epoch);
        crate::qualification_session::begin(
            &state,
            crate::qualification_session::BeginSessionRequest {
                session_handle,
                candidate_handle: candidate.clone(),
                captured_at: "2026-08-23T12:00:00Z".to_string(),
                device_plan: "test-plan".to_string(),
                target: qualification_session_target(),
                workflow: qualification_session_workflow(),
                build: serde_json::from_value(qualification_build_json()).unwrap(),
                runtime_contract: "real-execution-v1".to_string(),
                observation,
            },
        )
        .unwrap();
        let mut executions = ExecutionHandleStore::default();
        executions.reserve_start(ExecutionKind::Real).unwrap();
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![
                Ok(supported_inventory("transport-1")),
                Ok(json!({
                    "manufacturer": "AYANEO",
                    "model": "Pocket S",
                    "android_version": "15",
                    "android_api_level": 33,
                    "firmware_build": "original/build",
                })),
                Ok(supported_qualification()),
                Ok(match_result("profile.changed")),
                Ok(json!({ "execution": { "executionId": "sidecar-final-gate" } })),
            ]),
        };
        let platform_tools = PlatformToolsSnapshot {
            adb_path: "/trusted/adb",
            runtime_generation: 1,
            platform_tools_revision: 2,
        };

        let result = start_real_execution_inner_with_runtime(
            &review_handle,
            Some(&state),
            &state.handles,
            &state.root_qualification,
            &mut executions,
            &runtime,
            &platform_tools,
        );

        assert!(
            result.is_ok(),
            "profile drift must remain qualification-only: {result:?}; requests: {:?}",
            runtime
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|(kind, _)| kind)
                .collect::<Vec<_>>()
        );
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|(request_type, _)| request_type == "probeDevice")
                .count(),
            1,
            "profile revalidation must reuse the product's final probe facts"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|(request_type, _)| request_type == "startExecution")
                .count(),
            1
        );
        drop(requests);
        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(stored.payload["runValidity"], "invalid");
    }

    #[test]
    fn final_probe_target_mismatch_is_observed_before_product_validation_returns() {
        let repository_root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repository_root.path().join("authored/recipes")).unwrap();
        std::fs::write(
            repository_root
                .path()
                .join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\n",
        )
        .unwrap();
        let repository =
            crate::qualification_repository::QualificationRepository::new_for_test_with_source_state(
                repository_root.path().to_path_buf(),
                Box::new(NoQualificationToolRunner),
                serde_json::from_value(qualification_build_json()).unwrap(),
                crate::qualification_repository::QualificationSourceState {
                    head: "1".repeat(40),
                    tracked_worktree_clean: true,
                },
            );
        let candidate = repository
            .create_candidate(
                crate::qualification_repository::CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": qualification_build_json(),
                }),
                None,
            )
            .unwrap();
        let (handles, root, review_handle) = prepared_real_review(false);
        let device_handle = handles
            .lock()
            .unwrap()
            .review(&review_handle)
            .unwrap()
            .device_handle
            .clone();
        let provider =
            crate::qualification_repository::QualificationRepositoryProvider::for_test(repository);
        let (_app_root, app) = test_app_with_qualification(
            Mutex::new(ExecutionHandleStore::default()),
            handles,
            root,
            provider,
        );
        let state = app.state::<AppState>();
        let session_handle =
            crate::qualification_session::session_handle_for_candidate(&candidate).unwrap();
        let mut target = qualification_session_target();
        target.manufacturer = "AYANEO".to_string();
        target.model = "Pocket S".to_string();
        target.android_version = "15".to_string();
        target.android_api = 33;
        target.abi_soc_class = "arm64-v8a".to_string();
        target.firmware_build = "original/build".to_string();
        let mut observation = qualification_session_observation();
        observation.device_handle = device_handle;
        observation.manufacturer = Some(target.manufacturer.clone());
        observation.model = Some(target.model.clone());
        observation.android_version = Some(target.android_version.clone());
        observation.android_api = Some(target.android_api);
        observation.abi_soc_class = Some(target.abi_soc_class.clone());
        observation.firmware_build = Some(target.firmware_build.clone());
        crate::qualification_session::begin(
            &state,
            crate::qualification_session::BeginSessionRequest {
                session_handle,
                candidate_handle: candidate.clone(),
                captured_at: "2026-08-23T12:00:00Z".to_string(),
                device_plan: "test-plan".to_string(),
                target,
                workflow: qualification_session_workflow(),
                build: serde_json::from_value(qualification_build_json()).unwrap(),
                runtime_contract: "real-execution-v1".to_string(),
                observation,
            },
        )
        .unwrap();
        let mut executions = ExecutionHandleStore::default();
        executions.reserve_start(ExecutionKind::Real).unwrap();
        let runtime = ScriptedRuntime {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(vec![
                Ok(supported_inventory("transport-1")),
                Ok(json!({
                    "manufacturer": "Different Manufacturer",
                    "model": "Pocket S",
                    "android_version": "15",
                    "android_api_level": 33,
                    "firmware_build": "original/build",
                })),
            ]),
        };
        let platform_tools = PlatformToolsSnapshot {
            adb_path: "/trusted/adb",
            runtime_generation: 1,
            platform_tools_revision: 2,
        };

        let result = start_real_execution_inner_with_runtime(
            &review_handle,
            Some(&state),
            &state.handles,
            &state.root_qualification,
            &mut executions,
            &runtime,
            &platform_tools,
        );

        assert!(
            result.is_err(),
            "product target validation must still reject drift"
        );
        let requests = runtime.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|(request_type, _)| request_type == "probeDevice")
                .count(),
            1,
            "qualification must consume the same final probe as product validation"
        );
        assert!(!requests
            .iter()
            .any(|(request_type, _)| request_type == "startExecution"));
        drop(requests);
        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(stored.payload["runValidity"], "invalid");
    }

    #[test]
    fn lost_execution_without_retained_report_does_not_offer_report_export() {
        let snapshot = lost_real_execution_snapshot("execution-lost", None);

        assert_eq!(
            snapshot["terminalPolicy"]["availableControls"],
            json!(["fresh_workflow"]),
            "report export is only available when the report is retained"
        );
    }

    #[test]
    fn integrated_real_preflight_starts_once_for_current_supported_non_root_context() {
        let (handles, root, review_handle) = prepared_real_review(false);
        let (result, starts) = run_integrated_case(
            &review_handle,
            &handles,
            &root,
            supported_inventory("transport-1"),
            Some(supported_qualification()),
            1,
            2,
        );
        assert!(
            result.is_ok(),
            "current supported context rejected: {result:?}"
        );
        assert_eq!(starts, 1);
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
    fn lost_runtime_session_resets_all_execution_mappings() {
        let runtime = FakeRuntime {
            requests: Mutex::new(Vec::new()),
            result: Err(json!({ "code": "runtime_session_lost" }).to_string()),
        };
        let mut store = ExecutionHandleStore::default();
        store.reserve_start(ExecutionKind::Simulated).unwrap();
        let terminal = store.bind_started(
            ExecutionKind::Simulated,
            "sidecar-terminal".into(),
            "terminal-review".into(),
            review(),
        );
        store.mark_terminal(ExecutionKind::Simulated, &terminal.public_handle);
        store.reserve_start(ExecutionKind::Real).unwrap();
        store.bind_started(
            ExecutionKind::Real,
            "sidecar-active".into(),
            "active-review".into(),
            review(),
        );

        let error =
            request_simulated_execution_events(&runtime, &mut store, &terminal.public_handle, 0)
                .unwrap_err();

        assert!(error.contains("execution_unavailable"));
        assert!(store
            .mapping(
                ExecutionKind::Simulated,
                &terminal.public_handle,
                "unavailable"
            )
            .is_err());
        store
            .reserve_start(ExecutionKind::Real)
            .expect("an irrecoverably lost runtime session must release its active slot");
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
        assert!(public["errors"][0].get("code").is_none());
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
        assert_eq!(public["events"][0]["label"], "[redacted]");
        assert!(public["events"][0].get("eventType").is_none());
        assert!(public["events"][0].get("phase").is_none());
        assert!(public["events"][0].get("stepId").is_none());
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

    #[test]
    fn phase6d6_terminal_projection_reports_possible_partial_change_for_completed_failure() {
        let report = json!({
            "status": "failed",
            "errors": [{ "code": "device_transport_lost" }],
            "recipes": [{
                "recipeId": "phase6d6-qualification",
                "name": "Reviewed device setup",
                "status": "failed",
                "steps": [
                    { "name": "Prepare reviewed setup", "status": "succeeded" },
                    { "name": "Apply reviewed changes", "status": "failed" },
                    { "name": "Verify completed setup", "status": "pending" }
                ]
            }]
        });
        let projected = project_phase6d6_terminal("phase6d6-projection-test", report);
        assert_eq!(projected["status"], "failed");
        assert_eq!(projected["terminal"], true);
        assert_eq!(
            projected["errors"][0]["message"],
            "The device connection was lost during execution."
        );
        assert_eq!(
            projected["errors"][0]["remediation"]["title"],
            "Reconnect and requalify"
        );
        let policy = &projected["terminalPolicy"];
        assert_eq!(
            policy["partialChangePresentation"],
            "possible_partial_change"
        );
        assert_eq!(policy["recoveryState"], "requalification_required");
        assert_eq!(policy["authorityInvalidated"], true);
        let controls = policy["availableControls"].as_array().unwrap();
        for expected in ["export_report", "repair_setup", "fresh_workflow"] {
            assert!(
                controls.contains(&Value::String(expected.to_string())),
                "missing {expected}"
            );
        }
        for forbidden in ["resume", "replay", "checkpoint", "ownership_transfer"] {
            assert!(
                !controls.contains(&Value::String(forbidden.to_string())),
                "forbidden {forbidden}"
            );
        }
        assert_eq!(projected["completion"]["counts"]["pending"], 1);
    }

    #[test]
    fn phase6d6_terminal_projection_reports_indeterminate_without_completed_work() {
        let report = json!({
            "status": "failed",
            "errors": [{ "code": "operation_timed_out" }],
            "recipes": [{
                "recipeId": "phase6d6-qualification",
                "name": "Reviewed device setup",
                "status": "failed",
                "steps": [
                    { "name": "Apply reviewed changes", "status": "failed" },
                    { "name": "Verify completed setup", "status": "pending" }
                ]
            }]
        });
        let projected = project_phase6d6_terminal("phase6d6-projection-test", report);
        assert_eq!(
            projected["terminalPolicy"]["partialChangePresentation"],
            "indeterminate"
        );
        assert_eq!(
            projected["terminalPolicy"]["recoveryState"],
            "fresh_qualification_required"
        );
        assert_eq!(projected["terminalPolicy"]["authorityInvalidated"], true);
        assert_eq!(projected["completion"]["counts"]["completed"], 0);
        assert_eq!(projected["completion"]["counts"]["pending"], 1);
    }

    #[test]
    fn phase6d6_terminal_projection_attaches_production_cancellation_guidance() {
        let report = json!({
            "status": "cancelled",
            "recipes": [{
                "recipeId": "phase6d6-qualification",
                "name": "Reviewed device setup",
                "status": "cancelled",
                "steps": [
                    { "name": "Prepare reviewed setup", "status": "succeeded" },
                    { "name": "Apply reviewed changes", "status": "cancelled" },
                    { "name": "Verify completed setup", "status": "pending" }
                ]
            }]
        });
        let projected = project_phase6d6_terminal("phase6d6-projection-test", report);
        assert_eq!(projected["cancellation"]["title"], "Execution cancelled");
        assert_eq!(
            projected["cancellation"]["message"],
            "This action was cancelled at a safe boundary."
        );
        assert!(projected["cancellation"]["remediation"]["message"]
            .as_str()
            .unwrap()
            .contains("The old execution cannot resume."));
        assert_eq!(
            projected["terminalPolicy"]["recoveryState"],
            "fresh_review_required"
        );
        assert_eq!(projected["terminalPolicy"]["authorityInvalidated"], false);
        assert_eq!(
            projected["terminalPolicy"]["partialChangePresentation"],
            "possible_partial_change"
        );
        assert_eq!(projected["completion"]["counts"]["pending"], 1);
    }

    #[test]
    fn terminal_policy_adds_launch_control_only_when_attached() {
        let report = json!({ "status": "succeeded", "recipes": [] });
        let mut projected = project_phase6d6_terminal("phase6d6-projection-test", report);
        let initial = projected["terminalPolicy"]["availableControls"]
            .as_array()
            .unwrap()
            .clone();
        assert!(!initial.contains(&Value::String("launch_configured_app".to_string())));
        attach_terminal_policy(&mut projected, true);
        let attached = projected["terminalPolicy"]["availableControls"]
            .as_array()
            .unwrap();
        assert!(attached.contains(&Value::String("launch_configured_app".to_string())));
    }
}
