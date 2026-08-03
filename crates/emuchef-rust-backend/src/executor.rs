//! Safe executor implementation for dry-run, sandboxed filesystem operations,
//! and real-ADB execution.
//!
//! The real-ADB adapter remains crate-private. Protocol and CLI layers call the
//! executor through typed Rust interfaces, and filesystem effects remain
//! constrained to explicit sandbox roots.

pub mod adb;
pub(crate) mod identity;
pub(crate) mod root_authority;
pub(crate) mod root_requirements;

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::apk_manifest::inspect_apk_manifest;
use crate::artifact_resolver::{ArtifactResolveRequest, ArtifactResolver};
use crate::model::OrderedMap;
use crate::owned_process::ProcessCleanup;
use crate::planner::{
    ExecutionParamValue, ExecutionPlan, ExecutionStep, ExecutionStepCondition, RuntimeValue,
    TargetDeviceBinding,
};
use crate::remote_release_resolver::{resolve_github_latest, resolve_remote_latest};
use crate::validation::normalize_expected_sha256;

const APK_HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionRunResult {
    pub success: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub cancelled: bool,
    pub total_steps: usize,
    pub steps: Vec<StepRunRecord>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StepRunRecord {
    pub step_id: String,
    pub status: StepRunStatus,
    pub message: Option<String>,
    pub outputs: OrderedMap<RuntimeValue>,
    #[serde(skip)]
    pub(crate) failure_kind: Option<StepFailureKind>,
    #[serde(skip)]
    pub(crate) cleanup: Option<ProcessCleanup>,
}

/// Typed private failure kinds used to preserve safety decisions without
/// parsing presentation strings or changing the serialized execution schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepFailureKind {
    OperationTimedOut,
    OperationCommandFailed,
    OperationProcessFailed,
    DeviceOffline,
    DeviceUnauthorized,
    DeviceDisconnected,
    AdbServerUnavailable,
    TransportReset,
    TransportFailure,
    DeviceIdentityChanged(identity::IdentityCheckPhase),
    DeviceIdentityUnverified(identity::IdentityCheckPhase),
    RootAuthorityRevoked,
    RootAuthorityUnverified,
    RootDenied,
    RootUnavailable,
    OperationFailed,
}

impl StepFailureKind {
    /// Device safety failures terminate a real run at the current atomic step.
    pub(crate) const fn requires_device_fail_stop(self) -> bool {
        matches!(
            self,
            Self::OperationTimedOut
                | Self::DeviceOffline
                | Self::DeviceUnauthorized
                | Self::DeviceDisconnected
                | Self::AdbServerUnavailable
                | Self::TransportReset
                | Self::TransportFailure
                | Self::DeviceIdentityChanged(_)
                | Self::DeviceIdentityUnverified(_)
                | Self::RootAuthorityRevoked
                | Self::RootAuthorityUnverified
        )
    }
}

/// Internal device-operation error carrying stable classification and cleanup
/// evidence across the executor boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceOperationKind {
    TimedOut,
    CommandFailed,
    ProcessFailed,
    DeviceOffline,
    DeviceUnauthorized,
    DeviceDisconnected,
    AdbServerUnavailable,
    TransportReset,
    TransportFailure,
    DeviceIdentityChanged(identity::IdentityCheckPhase),
    DeviceIdentityUnverified(identity::IdentityCheckPhase),
    RootAuthorityRevoked,
    RootAuthorityUnverified,
    RootDenied,
    RootUnavailable,
    Other,
}

impl DeviceOperationKind {
    pub(crate) const fn requires_device_fail_stop(self) -> bool {
        matches!(
            self,
            Self::TimedOut
                | Self::DeviceOffline
                | Self::DeviceUnauthorized
                | Self::DeviceDisconnected
                | Self::AdbServerUnavailable
                | Self::TransportReset
                | Self::TransportFailure
                | Self::DeviceIdentityChanged(_)
                | Self::DeviceIdentityUnverified(_)
                | Self::RootAuthorityRevoked
                | Self::RootAuthorityUnverified
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceOperationError {
    pub(crate) kind: DeviceOperationKind,
    pub(crate) message: String,
    pub(crate) cleanup: Option<ProcessCleanup>,
}

impl DeviceOperationError {
    pub(crate) fn other(message: impl Into<String>) -> Self {
        Self {
            kind: DeviceOperationKind::Other,
            message: message.into(),
            cleanup: None,
        }
    }

    pub(crate) fn timed_out(cleanup: ProcessCleanup) -> Self {
        Self {
            kind: DeviceOperationKind::TimedOut,
            message: "The device operation timed out.".to_string(),
            cleanup: Some(cleanup),
        }
    }

    pub(crate) fn transport(kind: DeviceOperationKind) -> Self {
        let message = match kind {
            DeviceOperationKind::DeviceOffline => "The reviewed device is offline.",
            DeviceOperationKind::DeviceUnauthorized => "The reviewed device is unauthorized.",
            DeviceOperationKind::DeviceDisconnected => {
                "The reviewed device is disconnected or missing."
            }
            DeviceOperationKind::AdbServerUnavailable => "The local ADB service is unavailable.",
            DeviceOperationKind::TransportReset => {
                "The device connection was reset during execution."
            }
            DeviceOperationKind::TransportFailure => {
                "The device connection was lost during execution."
            }
            _ => "The device connection was lost during execution.",
        };
        Self {
            kind,
            message: message.to_string(),
            cleanup: None,
        }
    }

    pub(crate) fn identity(kind: DeviceOperationKind, phase: identity::IdentityCheckPhase) -> Self {
        let message = if phase == identity::IdentityCheckPhase::PostOperation {
            identity::POST_OPERATION_IDENTITY_FAILURE_MARKER
        } else {
            "The reviewed device identity could not be confirmed."
        };
        Self {
            kind,
            message: message.to_string(),
            cleanup: None,
        }
    }
}

impl From<String> for DeviceOperationError {
    fn from(message: String) -> Self {
        Self::other(message)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepRunStatus {
    Executed,
    Skipped,
    Blocked,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    CheckingSkipConditions,
    Executing,
    Verifying,
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStatus {
    Skipped,
    Blocked,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionProgressEvent {
    pub step_index: usize,
    pub total_steps: usize,
    pub step_id: String,
    pub recipe_ref: String,
    pub step_name: String,
    pub note: String,
    pub phase: ProgressPhase,
    pub status: Option<ProgressStatus>,
    pub message: Option<String>,
}

/// Device boundary used by the executor. The operation surface intentionally
/// remains the current thirteen executor-visible methods; identity setup and
/// fake-filesystem selection are lifecycle/configuration hooks, not operations.
pub trait ExecutorDevice: std::fmt::Debug {
    /// Configure private real-device identity evidence from the reviewed plan.
    /// Dry-run devices intentionally keep the default no-op implementation.
    fn configure_identity_guard(&mut self, _target: Option<&TargetDeviceBinding>) {}

    /// Configure private execution-time root authority from the reviewed plan.
    /// This lifecycle hook is not part of the executor-visible operation surface.
    fn configure_root_authority(&mut self, _reviewed_root_authorized: bool) {}

    /// Identify adapters that revalidate root at each root-wrapped command.
    /// This lifecycle hook prevents duplicate runner-level preflight probes.
    fn owns_per_command_root_authority(&self) -> bool {
        false
    }

    fn revalidate_root(&mut self) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn uses_fake_device_filesystem(&self) -> bool {
        false
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError>;
    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError>;
    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError>;
    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError>;
    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError>;
    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError>;
    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError>;
    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError>;
    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError>;
    fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), DeviceOperationError>;
    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError>;
    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError>;
}

#[derive(Debug)]
pub struct ExecutorRunner<D: ExecutorDevice = FakeDryRunDevice> {
    adapters: ExecutorAdapters<D>,
    root_preflight: RootPreflightState,
    root_preflight_failure: Option<String>,
}

impl Default for ExecutorRunner<FakeDryRunDevice> {
    fn default() -> Self {
        Self {
            adapters: ExecutorAdapters::default(),
            root_preflight: RootPreflightState::NotRun,
            root_preflight_failure: None,
        }
    }
}

impl<D: ExecutorDevice> ExecutorRunner<D> {
    pub fn new(adapters: ExecutorAdapters<D>) -> Self {
        Self {
            adapters,
            root_preflight: RootPreflightState::NotRun,
            root_preflight_failure: None,
        }
    }

    #[cfg(test)]
    pub fn adapters(&self) -> &ExecutorAdapters<D> {
        &self.adapters
    }

    #[cfg(test)]
    pub fn run(&mut self, plan: &ExecutionPlan) -> ExecutionRunResult {
        self.run_with_progress(plan, |_| {})
    }

    pub fn run_with_progress(
        &mut self,
        plan: &ExecutionPlan,
        progress_callback: impl FnMut(ExecutionProgressEvent),
    ) -> ExecutionRunResult {
        self.run_with_progress_and_cancel(plan, progress_callback, || false)
    }

    /// Execute in plan order and observe cancellation only between atomic steps.
    ///
    /// A cancellation request never interrupts or rolls back the current device
    /// operation. Once observed, no later step resolves parameters, executes,
    /// or verifies. Completed progress remains intact; later runner records are
    /// internally cancelled while the session report leaves those never-started
    /// steps pending so a terminal projection can label them "Not attempted"
    /// without adding a serialized status.
    pub fn run_with_progress_and_cancel(
        &mut self,
        plan: &ExecutionPlan,
        mut progress_callback: impl FnMut(ExecutionProgressEvent),
        should_cancel: impl Fn() -> bool,
    ) -> ExecutionRunResult {
        let total_steps = plan.steps.len();
        let step_ids_in_plan = plan
            .steps
            .iter()
            .map(|step| step.id.clone())
            .collect::<HashSet<_>>();
        let mut state = ExecutionState::from_plan(plan);
        let mut records = Vec::new();
        let mut cancelled = false;
        let mut aborted = false;
        self.root_preflight = RootPreflightState::NotRun;
        self.root_preflight_failure = None;
        self.adapters
            .device
            .configure_identity_guard(plan.target_device.as_ref());
        self.adapters
            .device
            .configure_root_authority(reviewed_root_authorized(plan));

        for step in &plan.steps {
            if aborted {
                let message = "execution aborted after root preflight failure".to_string();
                records.push(record(
                    step,
                    StepRunStatus::Blocked,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }
            if cancelled || should_cancel() {
                cancelled = true;
                let message = "execution cancelled before step scheduling".to_string();
                records.push(record(
                    step,
                    StepRunStatus::Cancelled,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }
            progress_callback(progress_event(
                step,
                total_steps,
                plan,
                ProgressPhase::CheckingSkipConditions,
                None,
                None,
            ));

            let blocking_dependencies = blocking_dependencies(&state, &step.dependencies);
            if !blocking_dependencies.is_empty() {
                let message = format!("dependency blocked: {}", blocking_dependencies.join(", "));
                state.steps.insert(
                    step.id.clone(),
                    StepRuntimeState {
                        status: StepRuntimeStatus::Blocked,
                        outputs: OrderedMap::new(),
                    },
                );
                progress_callback(progress_event(
                    step,
                    total_steps,
                    plan,
                    ProgressPhase::Finished,
                    Some(ProgressStatus::Blocked),
                    Some(message.clone()),
                ));
                records.push(record(
                    step,
                    StepRunStatus::Blocked,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }

            if let Some(message) = missing_capabilities_message(plan, step) {
                state.steps.insert(
                    step.id.clone(),
                    StepRuntimeState {
                        status: StepRuntimeStatus::Failed,
                        outputs: OrderedMap::new(),
                    },
                );
                progress_callback(progress_event(
                    step,
                    total_steps,
                    plan,
                    ProgressPhase::Finished,
                    Some(ProgressStatus::Failed),
                    Some(message.clone()),
                ));
                records.push(record(
                    step,
                    StepRunStatus::Failed,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }

            if plan_step_root_requirements(plan, step).any() {
                if let Err(failure) = self.ensure_root_preflight() {
                    let device_fail_stop = failure
                        .failure_kind
                        .is_some_and(StepFailureKind::requires_device_fail_stop);
                    let message = failure.message.clone();
                    self.root_preflight_failure = Some(message.clone());
                    state.steps.insert(
                        step.id.clone(),
                        StepRuntimeState {
                            status: StepRuntimeStatus::Failed,
                            outputs: OrderedMap::new(),
                        },
                    );
                    progress_callback(progress_event(
                        step,
                        total_steps,
                        plan,
                        ProgressPhase::Finished,
                        Some(ProgressStatus::Failed),
                        Some(message.clone()),
                    ));
                    records.push(record_with_failure(
                        step,
                        StepRunStatus::Failed,
                        Some(message),
                        OrderedMap::new(),
                        failure.failure_kind,
                        failure.cleanup,
                    ));
                    if device_fail_stop {
                        break;
                    }
                    aborted = true;
                    continue;
                }
            }

            let active_conflicts = step
                .constraints
                .conflicts_with
                .iter()
                .filter(|conflict| step_ids_in_plan.contains(*conflict))
                .cloned()
                .collect::<Vec<_>>();
            if !active_conflicts.is_empty() {
                let message = format!("conflicting steps present: {}", active_conflicts.join(", "));
                state.steps.insert(
                    step.id.clone(),
                    StepRuntimeState {
                        status: StepRuntimeStatus::Failed,
                        outputs: OrderedMap::new(),
                    },
                );
                progress_callback(progress_event(
                    step,
                    total_steps,
                    plan,
                    ProgressPhase::Finished,
                    Some(ProgressStatus::Failed),
                    Some(message.clone()),
                ));
                records.push(record(
                    step,
                    StepRunStatus::Failed,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }

            match self.skip_if_matched(step) {
                Ok(true) => {
                    state.steps.insert(
                        step.id.clone(),
                        StepRuntimeState {
                            status: StepRuntimeStatus::Skipped,
                            outputs: OrderedMap::new(),
                        },
                    );
                    progress_callback(progress_event(
                        step,
                        total_steps,
                        plan,
                        ProgressPhase::Finished,
                        Some(ProgressStatus::Skipped),
                        Some("skip_if matched".to_string()),
                    ));
                    records.push(record(
                        step,
                        StepRunStatus::Skipped,
                        Some("skip_if matched".to_string()),
                        OrderedMap::new(),
                    ));
                    continue;
                }
                Ok(false) => {}
                Err(failure) => {
                    let device_fail_stop = failure
                        .failure_kind
                        .is_some_and(StepFailureKind::requires_device_fail_stop);
                    let message = failure.message.clone();
                    state.steps.insert(
                        step.id.clone(),
                        StepRuntimeState {
                            status: StepRuntimeStatus::Failed,
                            outputs: OrderedMap::new(),
                        },
                    );
                    progress_callback(progress_event(
                        step,
                        total_steps,
                        plan,
                        ProgressPhase::Finished,
                        Some(ProgressStatus::Failed),
                        Some(message.clone()),
                    ));
                    records.push(record_with_failure(
                        step,
                        StepRunStatus::Failed,
                        Some(message),
                        OrderedMap::new(),
                        failure.failure_kind,
                        failure.cleanup,
                    ));
                    if device_fail_stop {
                        break;
                    }
                    if self.root_preflight_failure.is_some() {
                        aborted = true;
                    }
                    continue;
                }
            }

            progress_callback(progress_event(
                step,
                total_steps,
                plan,
                ProgressPhase::Executing,
                None,
                None,
            ));
            let run_result = self.run_step(plan, &mut state, step);
            let (status, message, outputs, runtime_status, failure_kind, cleanup, device_fail_stop) =
                match run_result {
                    Ok(outputs) => {
                        progress_callback(progress_event(
                            step,
                            total_steps,
                            plan,
                            ProgressPhase::Verifying,
                            None,
                            None,
                        ));
                        let failed_verify = step
                            .verify
                            .iter()
                            .filter_map(|condition| match self.evaluate_condition(condition) {
                                Ok(true) => None,
                                Ok(false) => Some(Ok(condition.type_name.clone())),
                                Err(error) => Some(Err(error)),
                            })
                            .collect::<Result<Vec<_>, _>>();
                        match failed_verify {
                            Ok(failed_verify) if failed_verify.is_empty() => (
                                StepRunStatus::Executed,
                                None,
                                outputs.clone(),
                                StepRuntimeStatus::Succeeded,
                                None,
                                None,
                                false,
                            ),
                            Ok(failed_verify) => (
                                StepRunStatus::Failed,
                                Some(format!("verify failed: {}", failed_verify.join(", "))),
                                OrderedMap::new(),
                                StepRuntimeStatus::Failed,
                                None,
                                None,
                                false,
                            ),
                            Err(failure) => (
                                StepRunStatus::Failed,
                                Some(failure.message),
                                if failure
                                    .failure_kind
                                    .is_some_and(StepFailureKind::requires_device_fail_stop)
                                {
                                    outputs.clone()
                                } else {
                                    OrderedMap::new()
                                },
                                StepRuntimeStatus::Failed,
                                failure.failure_kind,
                                failure.cleanup,
                                failure
                                    .failure_kind
                                    .is_some_and(StepFailureKind::requires_device_fail_stop),
                            ),
                        }
                    }
                    Err(failure) => {
                        let device_fail_stop = failure
                            .failure_kind
                            .is_some_and(StepFailureKind::requires_device_fail_stop);
                        (
                            StepRunStatus::Failed,
                            Some(failure.message),
                            failure.outputs,
                            StepRuntimeStatus::Failed,
                            failure.failure_kind,
                            failure.cleanup,
                            device_fail_stop,
                        )
                    }
                };

            state.steps.insert(
                step.id.clone(),
                StepRuntimeState {
                    status: runtime_status,
                    outputs: outputs.clone(),
                },
            );
            let progress_status = match status {
                StepRunStatus::Executed => ProgressStatus::Succeeded,
                StepRunStatus::Skipped => ProgressStatus::Skipped,
                StepRunStatus::Blocked => ProgressStatus::Blocked,
                StepRunStatus::Failed => ProgressStatus::Failed,
                StepRunStatus::Cancelled => ProgressStatus::Cancelled,
            };
            progress_callback(progress_event(
                step,
                total_steps,
                plan,
                ProgressPhase::Finished,
                Some(progress_status),
                message.clone(),
            ));
            records.push(record_with_failure(
                step,
                status,
                message,
                outputs,
                failure_kind,
                cleanup,
            ));
            if device_fail_stop {
                break;
            }
            if self.root_preflight_failure.is_some() {
                aborted = true;
            }
        }

        let success = !records.iter().any(|record| {
            matches!(
                record.status,
                StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
            )
        });
        ExecutionRunResult {
            success,
            cancelled,
            total_steps,
            steps: records,
        }
    }

    fn skip_if_matched(&mut self, step: &ExecutionStep) -> Result<bool, StepFailure> {
        step.skip_if.iter().try_fold(false, |matched, condition| {
            if matched {
                Ok(true)
            } else {
                self.evaluate_condition(condition)
            }
        })
    }

    fn ensure_root_preflight(&mut self) -> Result<(), StepFailure> {
        if self.adapters.device.owns_per_command_root_authority() {
            return Ok(());
        }
        match &self.root_preflight {
            RootPreflightState::Granted => return Ok(()),
            RootPreflightState::Failed(message) => {
                return Err(StepFailure::new(message.clone()));
            }
            RootPreflightState::NotRun => {}
        }
        match self.adapters.device.revalidate_root() {
            Ok(()) => {
                self.root_preflight = RootPreflightState::Granted;
                Ok(())
            }
            Err(error) => {
                let failure: StepFailure = error.into();
                self.root_preflight = RootPreflightState::Failed(failure.message.clone());
                self.root_preflight_failure = Some(failure.message.clone());
                Err(failure)
            }
        }
    }

    fn run_step(
        &mut self,
        plan: &ExecutionPlan,
        state: &mut ExecutionState,
        step: &ExecutionStep,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let resolved_params = resolve_step_params(state, &step.params)?;
        match step.type_name.as_str() {
            "wait" => self.execute_wait(&resolved_params),
            "grant_permissions" => self.execute_grant_permissions(plan, step, &resolved_params),
            "resolve_remote_release" => self.execute_resolve_remote_release(&resolved_params),
            "resolve_github_release" => self.execute_resolve_github_release(&resolved_params),
            "download_remote_file" => self.execute_download_remote_file(step, &resolved_params),
            "resolve_artifacts" => self.execute_resolve_artifacts(plan, state, &resolved_params),
            "extract_artifacts" => self.execute_extract_artifacts(state, step, &resolved_params),
            "extract_archive" => self.execute_extract_archive(step, &resolved_params),
            "copy_files" => self.execute_copy_files(plan, step, &resolved_params),
            "install_apk" => self.execute_install_apk(&resolved_params),
            "launch_app" => self.execute_launch_app(&resolved_params),
            "force_stop_app" => self.execute_force_stop_app(&resolved_params),
            other => Err(StepFailure::new(format!(
                "Unsupported executor step type: {other}"
            ))),
        }
    }

    fn execute_wait(
        &mut self,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let raw_duration = resolved_params.get("duration_ms").unwrap_or(&Value::Null);
        let duration = raw_duration.as_i64().filter(|duration| *duration > 0);
        if raw_duration.is_boolean() || duration.is_none() {
            return Err(StepFailure::new(format!(
                "wait step requires a positive integer duration_ms: {}",
                stable_repr(raw_duration)
            )));
        }
        self.adapters.sleep(duration.unwrap() as f64 / 1000.0);
        Ok(OrderedMap::new())
    }

    fn execute_grant_permissions(
        &mut self,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let policy = permission_policy(resolved_params.get("policy"));
        let mut action_results = Vec::new();
        let mut failure = None;

        for action in permission_actions(resolved_params) {
            if let Some(reason) = permission_not_applicable_reason(
                action.when.as_ref(),
                plan.runtime_capabilities.root_shell,
                plan.device_context.android_api_level,
            ) {
                let mut result = permission_result_base(step, &action);
                insert_object_entries(&mut result, reason);
                result.insert("status".to_string(), json!("not_applicable"));
                action_results.push(Value::Object(result));
                continue;
            }

            match self
                .adapters
                .device
                .run_plan_command(permission_command(&action))
            {
                Ok(()) => {
                    let mut result = permission_result_base(step, &action);
                    result.insert("status".to_string(), json!("executed"));
                    action_results.push(Value::Object(result));
                }
                Err(error) => {
                    let message = error.message.clone();
                    let mut result = permission_result_base(step, &action);
                    result.insert("status".to_string(), json!("failed"));
                    result.insert("message".to_string(), json!(message));
                    action_results.push(Value::Object(result));
                    if action.required
                        || policy.require_all
                        || policy.on_failure == "fail"
                        || error.kind.requires_device_fail_stop()
                    {
                        failure = Some((error,));
                        break;
                    }
                }
            }
        }

        let mut outputs = OrderedMap::new();
        outputs.insert(
            "permission_results".to_string(),
            RuntimeValue {
                type_name: "object".to_string(),
                value: json!({ "actions": action_results }),
                location: None,
            },
        );
        if let Some((error,)) = failure {
            let mut failure: StepFailure = error.into();
            failure.outputs = outputs;
            return Err(failure);
        }
        Ok(outputs)
    }

    fn execute_resolve_remote_release(
        &mut self,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let provider = resolved_params
            .get("provider")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StepFailure::new("resolve_remote_release requires provider".to_string())
            })?;
        let base_url = resolved_params
            .get("base_url")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StepFailure::new("resolve_remote_release requires base_url".to_string())
            })?;
        let repository = resolved_params
            .get("repository")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StepFailure::new("resolve_remote_release requires repository".to_string())
            })?;
        let asset_pattern = resolved_params
            .get("asset_pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StepFailure::new("resolve_remote_release requires asset_pattern".to_string())
            })?;
        let include_prereleases = resolved_params
            .get("include_prereleases")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let resolved = resolve_remote_latest(
            provider,
            base_url,
            repository,
            include_prereleases,
            asset_pattern,
        )
        .map_err(StepFailure::new)?;
        Ok(remote_release_outputs(resolved))
    }

    fn execute_resolve_github_release(
        &mut self,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let repository = resolved_params
            .get("repository")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StepFailure::new("resolve_github_release requires repository".to_string())
            })?;
        let asset_pattern = resolved_params
            .get("asset_pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StepFailure::new("resolve_github_release requires asset_pattern".to_string())
            })?;
        let include_prereleases = resolved_params
            .get("include_prereleases")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let resolved = resolve_github_latest(repository, include_prereleases, asset_pattern)
            .map_err(StepFailure::new)?;
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "download_url".to_string(),
            RuntimeValue {
                type_name: "string".to_string(),
                value: Value::String(resolved.download_url),
                location: None,
            },
        );
        outputs.insert(
            "asset_name".to_string(),
            RuntimeValue {
                type_name: "string".to_string(),
                value: Value::String(resolved.asset_name),
                location: None,
            },
        );
        outputs.insert(
            "release_tag".to_string(),
            RuntimeValue {
                type_name: "string".to_string(),
                value: Value::String(resolved.release_tag),
                location: None,
            },
        );
        if let Some(published_at) = resolved.published_at {
            outputs.insert(
                "published_at".to_string(),
                RuntimeValue {
                    type_name: "string".to_string(),
                    value: Value::String(published_at),
                    location: None,
                },
            );
        }
        if let Some(size) = resolved.size {
            outputs.insert(
                "size".to_string(),
                RuntimeValue {
                    type_name: "integer".to_string(),
                    value: Value::from(size),
                    location: None,
                },
            );
        }
        Ok(outputs)
    }

    fn execute_download_remote_file(
        &mut self,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let url = resolved_params
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| StepFailure::new("download_remote_file requires url".to_string()))?;
        let cache = resolved_params
            .get("cache")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let artifact_id = format!("dynamic.{}", step.id);
        let mut resolver = ArtifactResolver::new(self.adapters.sandbox()?);
        let request = ArtifactResolveRequest {
            artifact_id: &artifact_id,
            type_name: "remote_file",
            url,
            cache_mode: cache,
        };
        let resolved = resolver
            .resolve(request)
            .map_err(|error| StepFailure::new(error.executor_message(request)))?;
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "local_path".to_string(),
            RuntimeValue {
                type_name: "file_path".to_string(),
                value: Value::String(resolved.local_path.to_string_lossy().into_owned()),
                location: Some("host".to_string()),
            },
        );
        outputs.insert(
            "filename".to_string(),
            RuntimeValue {
                type_name: "string".to_string(),
                value: Value::String(resolved.filename),
                location: None,
            },
        );
        Ok(outputs)
    }

    fn execute_resolve_artifacts(
        &mut self,
        plan: &ExecutionPlan,
        state: &mut ExecutionState,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let artifacts_by_id = plan
            .artifacts
            .iter()
            .map(|artifact| (artifact.id.as_str(), artifact))
            .collect::<HashMap<_, _>>();
        let mut resolver = ArtifactResolver::new(self.adapters.sandbox()?);
        for artifact_id in string_list_param(resolved_params.get("artifacts")) {
            let Some(artifact) = artifacts_by_id.get(artifact_id.as_str()) else {
                return Err(StepFailure::new(format!(
                    "unknown_artifact_ref: Unknown artifact ref: 'artifacts.{artifact_id}'."
                )));
            };
            let result = resolver
                .resolve(ArtifactResolveRequest {
                    artifact_id: &artifact.id,
                    type_name: &artifact.type_name,
                    url: &artifact.url,
                    cache_mode: &artifact.cache,
                })
                .map_err(|error| {
                    StepFailure::new(error.executor_message(ArtifactResolveRequest {
                        artifact_id: &artifact.id,
                        type_name: &artifact.type_name,
                        url: &artifact.url,
                        cache_mode: &artifact.cache,
                    }))
                });
            let artifact_state = state
                .artifacts
                .get_mut(&artifact.id)
                .expect("artifact runtime state should be initialized from plan");
            match result {
                Ok(resolved) => {
                    artifact_state.status = ArtifactRuntimeStatus::Resolved;
                    artifact_state.local_path =
                        Some(resolved.local_path.to_string_lossy().into_owned());
                    artifact_state.resolved_url = Some(artifact.url.clone());
                    artifact_state.filename = Some(resolved.filename);
                    artifact_state.cache_hit = resolved.cache_hit;
                    artifact_state.error = None;
                }
                Err(failure) => {
                    let message = failure.message.clone();
                    artifact_state.status = ArtifactRuntimeStatus::Failed;
                    artifact_state.error = Some(message);
                    return Err(failure);
                }
            }
        }
        Ok(OrderedMap::new())
    }

    fn execute_extract_artifacts(
        &mut self,
        state: &ExecutionState,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let extract_on = resolved_params
            .get("extract_on")
            .and_then(Value::as_str)
            .unwrap_or("host");
        if !matches!(extract_on, "host" | "device") {
            return Err(StepFailure::new(format!(
                "extract_artifacts extract_on must be 'host' or 'device', got {extract_on:?}"
            )));
        }
        let sandbox = self.adapters.sandbox()?.clone();
        let extract_root = sandbox
            .runtime_root
            .join("extract")
            .join(sanitize_step_id(&step.id));
        let mut output_paths = Vec::new();
        let device_base = format!("/data/local/tmp/emuchef/{}", sanitize_step_id(&step.id));
        for artifact_id in string_list_param(resolved_params.get("artifacts")) {
            let Some(artifact_state) = state.artifacts.get(&artifact_id) else {
                return Err(StepFailure::new(format!(
                    "unknown_artifact_ref: Unknown artifact ref: 'artifacts.{artifact_id}'."
                )));
            };
            if artifact_state.status != ArtifactRuntimeStatus::Resolved {
                return Err(StepFailure::new(format!(
                    "artifact_not_resolved: Artifact is not resolved: 'artifacts.{artifact_id}.local_path'."
                )));
            }
            let Some(local_path) = &artifact_state.local_path else {
                return Err(StepFailure::new(format!(
                    "artifact_not_resolved: Artifact is not resolved: 'artifacts.{artifact_id}.local_path'."
                )));
            };
            let archive_path = PathBuf::from(local_path);
            sandbox.ensure_read_allowed(&archive_path)?;
            let artifact_dir =
                extract_root.join(artifact_id.rsplit('/').next().unwrap_or(&artifact_id));
            let members = sandbox.extract_zip_to_directory(&archive_path, &artifact_dir)?;
            if extract_on == "host" {
                output_paths.extend(
                    members
                        .into_iter()
                        .map(|member| json!(member.to_string_lossy().to_string())),
                );
            } else {
                let device_dest = join_device_path(
                    &device_base,
                    artifact_id.rsplit('/').next().unwrap_or(&artifact_id),
                );
                self.adapters.device.mkdir_p(&device_dest)?;
                for member in members {
                    let name = member.file_name().ok_or_else(|| {
                        StepFailure::new(format!(
                            "extracted path has no basename: {}",
                            member.display()
                        ))
                    })?;
                    let target = join_device_path(&device_dest, &name.to_string_lossy());
                    self.adapters.device.push(&member, &target, false)?;
                }
                output_paths.push(json!(device_dest));
            }
        }
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "extracted_paths".to_string(),
            RuntimeValue {
                type_name: "path_list".to_string(),
                value: Value::Array(output_paths),
                location: Some(extract_on.to_string()),
            },
        );
        Ok(outputs)
    }

    fn execute_extract_archive(
        &mut self,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let extract_on = resolved_params
            .get("extract_on")
            .and_then(Value::as_str)
            .unwrap_or("host");
        if !matches!(extract_on, "host" | "device") {
            return Err(StepFailure::new(format!(
                "extract_archive extract_on must be 'host' or 'device', got {extract_on:?}"
            )));
        }
        let archive = runtime_value_param(resolved_params, "archive")?;
        if archive.location.as_deref() != Some("host") {
            return Err(StepFailure::new(
                "Rust host-side extraction requires a host-side archive path.".to_string(),
            ));
        }
        let Some(archive_path) = archive.value.as_str() else {
            return Err(StepFailure::new(
                "extract_archive archive runtime value must contain a string path".to_string(),
            ));
        };
        let archive_path = PathBuf::from(archive_path);
        let sandbox = self.adapters.sandbox()?.clone();
        sandbox.ensure_read_allowed(&archive_path)?;
        let extract_root = sandbox
            .runtime_root
            .join("extract")
            .join(sanitize_step_id(&step.id));
        let members = sandbox.extract_zip_to_directory(&archive_path, &extract_root)?;
        if extract_on == "device" {
            let operation = (|| {
                let dest = resolved_params
                    .get("dest")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StepFailure::new(
                            "extract_archive targeting a device requires a string dest".to_string(),
                        )
                    })?;
                self.adapters.device.mkdir_p(dest)?;
                for member in &members {
                    let name = member.file_name().ok_or_else(|| {
                        StepFailure::new(format!(
                            "extracted path has no basename: {}",
                            member.display()
                        ))
                    })?;
                    let target = join_device_path(dest, &name.to_string_lossy());
                    self.adapters.device.push(member, &target, false)?;
                }
                let mut outputs = OrderedMap::new();
                outputs.insert(
                    "extracted_path".to_string(),
                    RuntimeValue {
                        type_name: "directory_path".to_string(),
                        value: json!(dest),
                        location: Some("device".to_string()),
                    },
                );
                Ok(outputs)
            })();
            // Device-targeted extraction never exposes host staging as an output,
            // so callers cannot opt out of removing it.
            let cleanup = remove_host_staging(&extract_root);
            return combine_operation_and_cleanup(operation, cleanup);
        }
        let (type_name, value) = if members.len() == 1 {
            let member = &members[0];
            let type_name = if member.is_dir() {
                "directory_path"
            } else {
                "file_path"
            };
            (type_name, member.to_string_lossy().to_string())
        } else {
            ("directory_path", extract_root.to_string_lossy().to_string())
        };
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "extracted_path".to_string(),
            RuntimeValue {
                type_name: type_name.to_string(),
                value: json!(value),
                location: Some("host".to_string()),
            },
        );
        Ok(outputs)
    }

    fn execute_copy_files(
        &mut self,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let source = runtime_value_param(resolved_params, "source")?;
        let dest = resolved_params
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| StepFailure::new("copy_files dest must be a string".to_string()))?;
        let copy_policy = resolved_params
            .get("copy_policy")
            .and_then(Value::as_str)
            .unwrap_or("merge");
        if !matches!(copy_policy, "merge" | "replace" | "sync") {
            return Err(StepFailure::new(format!(
                "copy_files copy_policy must be merge, replace, or sync, got {copy_policy:?}"
            )));
        }
        let sandbox = self.adapters.sandbox()?.clone();
        let app_private_dest = root_requirements::is_app_private_path(dest);
        let app_private_source = source
            .location
            .as_deref()
            .is_some_and(|location| location == "device")
            && root_requirements::is_app_private_path(&value_to_string(&source.value));
        if app_private_dest || app_private_source {
            self.ensure_root_preflight()?;
        }
        if app_private_dest
            && !(plan.runtime_capabilities.app_data_write && plan.runtime_capabilities.root_shell)
        {
            return Err(StepFailure::new(format!(
                "app_data_write_unavailable: Destination {dest:?} requires both app_data_write and root_shell runtime capabilities."
            )));
        }
        let copied_paths = if self.adapters.device.uses_fake_device_filesystem()
            && source.location.as_deref() == Some("host")
            && !app_private_dest
        {
            sandbox.copy_host_source_to_fake_device(&source, dest, copy_policy)?
        } else if source.location.as_deref() == Some("device") {
            copy_device_source(&mut self.adapters.device, &source, dest, copy_policy)?
        } else if app_private_dest {
            copy_host_source_to_app_private(
                &mut self.adapters.device,
                &sandbox,
                step,
                &source,
                dest,
                copy_policy,
            )?
        } else {
            copy_host_source(
                &mut self.adapters.device,
                &sandbox,
                &source,
                dest,
                copy_policy,
            )?
        };
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "copied_paths".to_string(),
            RuntimeValue {
                type_name: "path_list".to_string(),
                value: Value::Array(copied_paths.into_iter().map(Value::String).collect()),
                location: Some("device".to_string()),
            },
        );
        Ok(outputs)
    }

    fn execute_install_apk(
        &mut self,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let app = runtime_value_param(resolved_params, "app")?;
        if app.type_name != "file_path" || app.location.as_deref() != Some("host") {
            return Err(StepFailure::new(
                "install_apk requires a host-side file_path runtime value.".to_string(),
            ));
        }
        let apk_path = PathBuf::from(value_to_string(&app.value));
        if apk_path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| !extension.eq_ignore_ascii_case("apk"))
            .unwrap_or(true)
        {
            return Err(StepFailure::new(format!(
                "install_apk requires an .apk file, got: {}",
                apk_path.display()
            )));
        }
        if !apk_path.exists() {
            return Err(StepFailure::new(format!(
                "APK file not found: {}",
                apk_path.display()
            )));
        }
        if let Some(expected_package_name) = resolved_params.get("expected_package_name") {
            let expected_package_name = expected_package_name
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    StepFailure::new(
                        "install_apk expected_package_name must be a non-empty string literal."
                            .to_string(),
                    )
                })?;
            let manifest = inspect_apk_manifest(&apk_path).map_err(|error| {
                StepFailure::new(format!(
                    "apk_package_inspection_failed: manifest inspection failed with reason '{}'.",
                    error.code()
                ))
            })?;
            if manifest.package_name != expected_package_name {
                return Err(StepFailure::new(format!(
                    "apk_package_mismatch: expected package '{}', actual package '{}'.",
                    expected_package_name, manifest.package_name
                )));
            }
        }
        if let Some(expected_sha256) = resolved_params.get("expected_sha256") {
            let expected_sha256 = expected_sha256
                .as_str()
                .and_then(normalize_expected_sha256)
                .ok_or_else(|| {
                    StepFailure::new(
                        "install_apk expected_sha256 must be a 64-character hexadecimal string literal."
                            .to_string(),
                    )
                })?;
            let actual_sha256 = calculate_apk_sha256(&apk_path).map_err(|_| {
                StepFailure::new(
                    "apk_checksum_read_failed: APK checksum could not be calculated.".to_string(),
                )
            })?;
            if actual_sha256 != expected_sha256 {
                return Err(StepFailure::new(format!(
                    "apk_checksum_mismatch: expected SHA-256 '{expected_sha256}', actual SHA-256 '{actual_sha256}'."
                )));
            }
        }
        let replace_existing = resolved_params
            .get("replace_existing")
            .map(value_is_truthy)
            .unwrap_or(false);
        self.adapters
            .device
            .install_apk(&apk_path, replace_existing)?;
        Ok(OrderedMap::new())
    }

    fn execute_launch_app(
        &mut self,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let package_name = resolved_params
            .get("package_name")
            .map(value_to_string)
            .unwrap_or_else(|| "None".to_string());
        let activity = resolved_params
            .get("activity")
            .filter(|value| !value.is_null())
            .map(value_to_string);
        self.adapters
            .device
            .launch_app(&package_name, activity.as_deref())?;
        Ok(OrderedMap::new())
    }

    fn execute_force_stop_app(
        &mut self,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let package_name = resolved_params
            .get("package_name")
            .map(value_to_string)
            .unwrap_or_else(|| "None".to_string());
        if package_name.trim().is_empty() {
            return Err(StepFailure::new(
                "force_stop_app step requires a non-empty package_name.".to_string(),
            ));
        }
        self.adapters.device.force_stop_app(&package_name)?;
        Ok(OrderedMap::new())
    }

    fn evaluate_condition(
        &mut self,
        condition: &ExecutionStepCondition,
    ) -> Result<bool, StepFailure> {
        match condition.type_name.as_str() {
            "package_installed" => {
                let package_name =
                    required_string_param(condition, "package_name").map_err(StepFailure::new)?;
                self.adapters
                    .device
                    .package_installed(&package_name)
                    .map_err(StepFailure::from)
            }
            "path_exists" => {
                let path = required_string_param(condition, "path").map_err(StepFailure::new)?;
                if root_requirements::is_app_private_path(&path) {
                    self.ensure_root_preflight()?;
                }
                let device_exists = self
                    .adapters
                    .device
                    .path_exists(&path)
                    .map_err(StepFailure::from)?;
                if !self.adapters.device.uses_fake_device_filesystem() {
                    return Ok(device_exists);
                }
                let sandbox_exists = self
                    .adapters
                    .sandbox()
                    .and_then(|sandbox| sandbox.fake_device_path(&path))
                    .map(|path| path.exists())
                    .unwrap_or(false);
                Ok(device_exists || sandbox_exists)
            }
            "file_exists" => {
                let path = required_string_param(condition, "path").map_err(StepFailure::new)?;
                if root_requirements::is_app_private_path(&path) {
                    self.ensure_root_preflight()?;
                }
                let device_exists = self
                    .adapters
                    .device
                    .path_exists(&path)
                    .map_err(StepFailure::from)?;
                let device_is_dir = self
                    .adapters
                    .device
                    .path_is_dir(&path)
                    .map_err(StepFailure::from)?;
                if !self.adapters.device.uses_fake_device_filesystem() {
                    return Ok(device_exists && !device_is_dir);
                }
                let sandbox_is_file = self
                    .adapters
                    .sandbox()
                    .and_then(|sandbox| sandbox.fake_device_path(&path))
                    .map(|path| path.is_file())
                    .unwrap_or(false);
                Ok((device_exists && !device_is_dir) || sandbox_is_file)
            }
            other => Err(StepFailure::new(format!(
                "Unsupported condition type: {other}"
            ))),
        }
    }
}

fn calculate_apk_sha256(path: &Path) -> io::Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; APK_HASH_BUFFER_BYTES];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect())
}

pub(crate) fn reviewed_plan_requires_root(plan: &ExecutionPlan) -> bool {
    plan.steps
        .iter()
        .any(|step| plan_step_root_requirements(plan, step).any())
}

pub(crate) fn reviewed_root_authorized(plan: &ExecutionPlan) -> bool {
    if !reviewed_plan_requires_root(plan) {
        return false;
    }
    let requirements = plan
        .steps
        .iter()
        .map(|step| plan_step_root_requirements(plan, step))
        .fold(
            root_requirements::RootRequirements::default(),
            |mut all, current| {
                all.merge(current);
                all
            },
        );
    requirements.any()
        && (!requirements.root_shell || plan.runtime_capabilities.root_shell)
        && (!requirements.app_data_write || plan.runtime_capabilities.app_data_write)
}

fn plan_step_root_requirements(
    plan: &ExecutionPlan,
    step: &ExecutionStep,
) -> root_requirements::RootRequirements {
    let operation = root_requirements::root_work_operation(&step.type_name);
    let (root_shell_constraint, app_data_write_constraint) =
        root_requirements::root_requirement_constraints(
            step.constraints.capabilities.iter().map(String::as_str),
        );
    let condition_requires_root = step
        .skip_if
        .iter()
        .chain(step.verify.iter())
        .any(condition_requires_root);
    let source_requires_root = step
        .params
        .get("source")
        .is_some_and(|value| execution_param_runtime_value_requires_root(plan, value));
    let destination_requires_root = step
        .params
        .get("dest")
        .is_some_and(|value| execution_param_destination_requires_root(plan, value));
    let extracts_to_device = step
        .params
        .get("extract_on")
        .is_some_and(|value| execution_param_is_string(plan, value, "device"));
    root_requirements::classify_root_requirement_step(root_requirements::RootRequirementStep {
        operation,
        root_shell_constraint,
        app_data_write_constraint,
        condition_requires_root,
        source_requires_root,
        destination_requires_root,
        extracts_to_device,
    })
}

fn input_runtime_value<'a>(plan: &'a ExecutionPlan, ref_value: &str) -> Option<&'a RuntimeValue> {
    let input_id = ref_value.strip_prefix("inputs.")?;
    if input_id.is_empty() {
        return None;
    }
    plan.inputs
        .iter()
        .find(|input| input.id == input_id)
        .map(|input| &input.value)
}

fn execution_param_runtime_value_requires_root(
    plan: &ExecutionPlan,
    value: &ExecutionParamValue,
) -> bool {
    match value {
        ExecutionParamValue::Literal { value } => {
            runtime_value_json_is_app_private_device_path(value)
        }
        ExecutionParamValue::Ref { ref_value } => input_runtime_value(plan, ref_value)
            .is_some_and(runtime_value_device_value_requires_root),
    }
}

fn execution_param_destination_requires_root(
    plan: &ExecutionPlan,
    value: &ExecutionParamValue,
) -> bool {
    match value {
        ExecutionParamValue::Literal { value } => value
            .as_str()
            .is_some_and(root_requirements::is_app_private_path),
        ExecutionParamValue::Ref { ref_value } => input_runtime_value(plan, ref_value)
            .is_some_and(runtime_value_device_path_requires_root),
    }
}

fn execution_param_is_string(
    plan: &ExecutionPlan,
    value: &ExecutionParamValue,
    expected: &str,
) -> bool {
    match value {
        ExecutionParamValue::Literal { value } => value.as_str() == Some(expected),
        ExecutionParamValue::Ref { ref_value } => {
            input_runtime_value(plan, ref_value).and_then(|value| value.value.as_str())
                == Some(expected)
        }
    }
}

fn runtime_value_device_value_requires_root(value: &RuntimeValue) -> bool {
    value.location.as_deref() == Some("device")
        && match &value.value {
            Value::String(path) => root_requirements::is_app_private_path(path),
            Value::Array(paths) => paths
                .iter()
                .filter_map(Value::as_str)
                .any(root_requirements::is_app_private_path),
            _ => false,
        }
}

fn runtime_value_device_path_requires_root(value: &RuntimeValue) -> bool {
    value.location.as_deref() == Some("device")
        && value
            .value
            .as_str()
            .is_some_and(root_requirements::is_app_private_path)
}

fn runtime_value_json_is_app_private_device_path(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.get("location").and_then(Value::as_str) != Some("device") {
        return false;
    }
    match object.get("value") {
        Some(Value::String(path)) => root_requirements::is_app_private_path(path),
        Some(Value::Array(paths)) => paths
            .iter()
            .filter_map(Value::as_str)
            .any(root_requirements::is_app_private_path),
        _ => false,
    }
}

fn condition_requires_root(condition: &ExecutionStepCondition) -> bool {
    root_requirements::condition_requires_root(
        &condition.type_name,
        condition.params.get("path").and_then(Value::as_str),
    )
}

fn remote_release_outputs(
    resolved: crate::remote_release_resolver::ResolvedRemoteRelease,
) -> OrderedMap<RuntimeValue> {
    let mut outputs = OrderedMap::new();
    outputs.insert(
        "download_url".to_string(),
        RuntimeValue {
            type_name: "string".to_string(),
            value: Value::String(resolved.download_url),
            location: None,
        },
    );
    outputs.insert(
        "asset_name".to_string(),
        RuntimeValue {
            type_name: "string".to_string(),
            value: Value::String(resolved.asset_name),
            location: None,
        },
    );
    outputs.insert(
        "release_tag".to_string(),
        RuntimeValue {
            type_name: "string".to_string(),
            value: Value::String(resolved.release_tag),
            location: None,
        },
    );
    if let Some(published_at) = resolved.published_at {
        outputs.insert(
            "published_at".to_string(),
            RuntimeValue {
                type_name: "string".to_string(),
                value: Value::String(published_at),
                location: None,
            },
        );
    }
    if let Some(size) = resolved.size {
        outputs.insert(
            "size".to_string(),
            RuntimeValue {
                type_name: "integer".to_string(),
                value: Value::from(size),
                location: None,
            },
        );
    }
    outputs
}

fn copy_host_source<D: ExecutorDevice>(
    device: &mut D,
    sandbox: &SandboxRoots,
    source: &RuntimeValue,
    dest: &str,
    copy_policy: &str,
) -> Result<Vec<String>, StepFailure> {
    match source.type_name.as_str() {
        "directory_path" => {
            let source_path = source_path_from_value(source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for child in sorted_children(&source_path)? {
                let child_name = child
                    .file_name()
                    .expect("source child should have a basename")
                    .to_string_lossy();
                let target = join_device_path(dest, &child_name);
                device.push(&child, &target, copy_policy == "sync")?;
                copied.push(target);
            }
            Ok(copied)
        }
        "path_list" => {
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for source_path in source_path_list(source)? {
                sandbox.ensure_read_allowed(&source_path)?;
                let child_name = source_path
                    .file_name()
                    .ok_or_else(|| {
                        StepFailure::new(format!(
                            "copy_files source path has no basename: {}",
                            source_path.display()
                        ))
                    })?
                    .to_string_lossy();
                let target = join_device_path(dest, &child_name);
                device.push(&source_path, &target, copy_policy == "sync")?;
                copied.push(target);
            }
            Ok(copied)
        }
        "file_path" => {
            let source_path = source_path_from_value(source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            let target = if device.path_is_dir(dest)? {
                let name = source_path.file_name().ok_or_else(|| {
                    StepFailure::new(format!(
                        "copy_files source path has no basename: {}",
                        source_path.display()
                    ))
                })?;
                join_device_path(dest, &name.to_string_lossy())
            } else {
                if let Some(parent) = device_parent(dest) {
                    device.mkdir_p(parent)?;
                }
                dest.to_string()
            };
            if copy_policy == "replace" {
                device.remove_file(&target)?;
            }
            device.push(&source_path, &target, copy_policy == "sync")?;
            Ok(vec![target])
        }
        other => Err(StepFailure::new(format!(
            "copy_files does not support source runtime type {other:?}."
        ))),
    }
}

fn copy_device_source<D: ExecutorDevice>(
    device: &mut D,
    source: &RuntimeValue,
    dest: &str,
    copy_policy: &str,
) -> Result<Vec<String>, StepFailure> {
    match source.type_name.as_str() {
        "directory_path" => {
            let source_path = value_to_string(&source.value);
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            device.copy_on_device(
                &format!("{}/.", source_path.trim_end_matches('/')),
                dest,
                true,
                false,
            )?;
            Ok(vec![dest.to_string()])
        }
        "path_list" => {
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for item in source.value.as_array().cloned().unwrap_or_default() {
                let source_path = item.as_str().ok_or_else(|| {
                    StepFailure::new(
                        "copy_files path_list source values must be strings".to_string(),
                    )
                })?;
                device.copy_on_device(source_path, dest, true, false)?;
                copied.push(join_device_path(dest, device_basename(source_path)));
            }
            Ok(copied)
        }
        "file_path" => {
            let source_path = value_to_string(&source.value);
            let target = if device.path_is_dir(dest)? {
                join_device_path(dest, device_basename(&source_path))
            } else {
                if let Some(parent) = device_parent(dest) {
                    device.mkdir_p(parent)?;
                }
                dest.to_string()
            };
            if copy_policy == "replace" {
                device.remove_file(&target)?;
            }
            device.copy_on_device(&source_path, &target, false, false)?;
            Ok(vec![target])
        }
        other => Err(StepFailure::new(format!(
            "copy_files does not support device source runtime type {other:?}."
        ))),
    }
}

fn copy_host_source_to_app_private<D: ExecutorDevice>(
    device: &mut D,
    sandbox: &SandboxRoots,
    step: &ExecutionStep,
    source: &RuntimeValue,
    dest: &str,
    copy_policy: &str,
) -> Result<Vec<String>, StepFailure> {
    let stage_root = format!("/data/local/tmp/emuchef/{}", sanitize_step_id(&step.id));
    device.remove_tree(&stage_root)?;
    device.mkdir_p(&stage_root)?;
    let result = (|| match source.type_name.as_str() {
        "directory_path" => {
            let source_path = source_path_from_value(source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for child in sorted_children(&source_path)? {
                let name = child
                    .file_name()
                    .expect("source child should have a basename")
                    .to_string_lossy();
                let staged = join_device_path(&stage_root, &name);
                device.push(&child, &staged, false)?;
                device.copy_on_device(&staged, dest, child.is_dir(), true)?;
                copied.push(join_device_path(dest, &name));
            }
            Ok(copied)
        }
        "path_list" => {
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for source_path in source_path_list(source)? {
                sandbox.ensure_read_allowed(&source_path)?;
                let name = source_path
                    .file_name()
                    .ok_or_else(|| {
                        StepFailure::new(format!(
                            "copy_files source path has no basename: {}",
                            source_path.display()
                        ))
                    })?
                    .to_string_lossy();
                let staged = join_device_path(&stage_root, &name);
                device.push(&source_path, &staged, false)?;
                device.copy_on_device(&staged, dest, source_path.is_dir(), true)?;
                copied.push(join_device_path(dest, &name));
            }
            Ok(copied)
        }
        "file_path" => {
            let source_path = source_path_from_value(source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            let name = source_path
                .file_name()
                .ok_or_else(|| {
                    StepFailure::new(format!(
                        "copy_files source path has no basename: {}",
                        source_path.display()
                    ))
                })?
                .to_string_lossy();
            let staged = join_device_path(&stage_root, &name);
            device.push(&source_path, &staged, false)?;
            let target = if device.path_is_dir(dest)? {
                join_device_path(dest, &name)
            } else {
                if let Some(parent) = device_parent(dest) {
                    device.mkdir_p(parent)?;
                }
                dest.to_string()
            };
            if copy_policy == "replace" {
                device.remove_file(&target)?;
            }
            device.copy_on_device(&staged, &target, false, true)?;
            Ok(vec![target])
        }
        other => Err(StepFailure::new(format!(
            "copy_files does not support source runtime type {other:?}."
        ))),
    })();
    let cleanup = device.remove_tree(&stage_root).map_err(StepFailure::from);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(copied), Ok(())) => Ok(copied),
    }
}

fn source_path_list(source: &RuntimeValue) -> Result<Vec<PathBuf>, StepFailure> {
    source
        .value
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            item.as_str().map(PathBuf::from).ok_or_else(|| {
                StepFailure::new("copy_files path_list source values must be strings".to_string())
            })
        })
        .collect()
}

fn device_parent(path: &str) -> Option<&str> {
    let path = path.trim_end_matches('/');
    let (parent, _) = path.rsplit_once('/')?;
    if parent.is_empty() {
        Some("/")
    } else {
        Some(parent)
    }
}

fn device_basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

#[cfg(test)]
pub type DryRunExecutorAdapters = ExecutorAdapters<FakeDryRunDevice>;

#[derive(Debug)]
pub struct ExecutorAdapters<D: ExecutorDevice = FakeDryRunDevice> {
    device: D,
    sleep_calls: Vec<f64>,
    sandbox: Option<SandboxRoots>,
    perform_sleep: bool,
}

impl Default for ExecutorAdapters<FakeDryRunDevice> {
    fn default() -> Self {
        Self {
            device: FakeDryRunDevice::default(),
            sleep_calls: Vec::new(),
            sandbox: None,
            perform_sleep: false,
        }
    }
}

impl<D: ExecutorDevice> ExecutorAdapters<D> {
    #[cfg(test)]
    pub fn with_device(device: D) -> Self {
        Self {
            device,
            sleep_calls: Vec::new(),
            sandbox: None,
            perform_sleep: false,
        }
    }

    pub fn with_device_and_sandbox_roots(
        device: D,
        runtime_root: PathBuf,
        cache_root: PathBuf,
        fake_device_root: PathBuf,
        read_only_roots: Vec<PathBuf>,
        perform_sleep: bool,
    ) -> Self {
        Self {
            device,
            sleep_calls: Vec::new(),
            sandbox: Some(SandboxRoots {
                runtime_root,
                cache_root,
                fake_device_root,
                read_only_roots,
            }),
            perform_sleep,
        }
    }

    #[cfg(test)]
    pub fn device(&self) -> &D {
        &self.device
    }

    #[cfg(test)]
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    #[cfg(test)]
    pub fn sleep_calls(&self) -> &[f64] {
        &self.sleep_calls
    }

    fn sleep(&mut self, seconds: f64) {
        self.sleep_calls.push(seconds);
        if self.perform_sleep {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
        }
    }

    fn sandbox(&self) -> Result<&SandboxRoots, StepFailure> {
        self.sandbox.as_ref().ok_or_else(|| {
            StepFailure::new(
                "Filesystem and artifact handlers require explicit sandbox roots".to_string(),
            )
        })
    }
}

impl ExecutorAdapters<FakeDryRunDevice> {
    pub fn with_sandbox_roots(
        runtime_root: PathBuf,
        cache_root: PathBuf,
        fake_device_root: PathBuf,
        read_only_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            device: FakeDryRunDevice::default(),
            sleep_calls: Vec::new(),
            sandbox: Some(SandboxRoots {
                runtime_root,
                cache_root,
                fake_device_root,
                read_only_roots,
            }),
            perform_sleep: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct FakeDryRunDevice {
    installed_packages: HashSet<String>,
    remote_paths: HashSet<String>,
    remote_dirs: HashSet<String>,
    commands: Vec<Vec<String>>,
    run_plan_failures: HashMap<Vec<String>, String>,
    install_failures: HashMap<(String, bool), String>,
    launch_failures: HashMap<(String, String), String>,
    force_stop_failures: HashMap<String, String>,
    root_preflight_result: Option<Result<(), String>>,
    root_preflight_calls: usize,
}

impl FakeDryRunDevice {
    #[cfg(test)]
    pub fn installed_packages_mut(&mut self) -> &mut HashSet<String> {
        &mut self.installed_packages
    }

    #[cfg(test)]
    pub fn remote_paths_mut(&mut self) -> &mut HashSet<String> {
        &mut self.remote_paths
    }

    #[cfg(test)]
    pub fn remote_dirs_mut(&mut self) -> &mut HashSet<String> {
        &mut self.remote_dirs
    }

    #[cfg(test)]
    pub fn commands(&self) -> &[Vec<String>] {
        &self.commands
    }

    #[cfg(test)]
    pub fn set_root_preflight_result(&mut self, result: Result<(), String>) {
        self.root_preflight_result = Some(result);
    }

    #[cfg(test)]
    pub fn root_preflight_calls(&self) -> usize {
        self.root_preflight_calls
    }

    #[cfg(test)]
    pub fn fail_run_plan_command(&mut self, command: Vec<String>, message: &str) {
        self.run_plan_failures.insert(command, message.to_string());
    }

    #[cfg(test)]
    pub fn fail_launch_app(&mut self, package_name: &str, activity: Option<&str>, message: &str) {
        self.launch_failures.insert(
            (package_name.to_string(), activity.unwrap_or("").to_string()),
            message.to_string(),
        );
    }

    fn install_apk(&mut self, apk_path: &Path, replace_existing: bool) -> Result<(), String> {
        let apk_path = apk_path.to_string_lossy().to_string();
        self.commands.push(vec![
            "install_apk".to_string(),
            apk_path.clone(),
            py_bool(replace_existing).to_string(),
        ]);
        if let Some(message) = self.install_failures.get(&(apk_path, replace_existing)) {
            return Err(message.clone());
        }
        Ok(())
    }

    fn package_installed(&mut self, package_name: &str) -> bool {
        self.commands.push(vec![
            "package_installed".to_string(),
            package_name.to_string(),
        ]);
        self.installed_packages.contains(package_name)
    }

    fn path_exists(&mut self, path: &str) -> bool {
        let privileged = py_bool(root_requirements::is_app_private_path(path));
        self.commands.push(vec![
            "path_exists".to_string(),
            path.to_string(),
            privileged.to_string(),
        ]);
        self.remote_paths.contains(path)
    }

    fn path_is_dir(&mut self, path: &str) -> bool {
        let privileged = py_bool(root_requirements::is_app_private_path(path));
        self.commands.push(vec![
            "path_is_dir".to_string(),
            path.to_string(),
            privileged.to_string(),
        ]);
        self.remote_dirs.contains(path)
    }

    fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), String> {
        let mut recorded = vec!["run_plan_command".to_string()];
        recorded.extend(command.iter().cloned());
        self.commands.push(recorded);
        if let Some(message) = self.run_plan_failures.get(&command) {
            return Err(message.clone());
        }
        Ok(())
    }

    fn launch_app(&mut self, package_name: &str, activity: Option<&str>) -> Result<(), String> {
        let activity = activity.unwrap_or("");
        self.commands.push(vec![
            "launch_app".to_string(),
            package_name.to_string(),
            activity.to_string(),
        ]);
        if let Some(message) = self
            .launch_failures
            .get(&(package_name.to_string(), activity.to_string()))
        {
            return Err(message.clone());
        }
        Ok(())
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), String> {
        self.commands
            .push(vec!["force_stop_app".to_string(), package_name.to_string()]);
        if let Some(message) = self.force_stop_failures.get(package_name) {
            return Err(message.clone());
        }
        Ok(())
    }
}

impl ExecutorDevice for FakeDryRunDevice {
    fn revalidate_root(&mut self) -> Result<(), DeviceOperationError> {
        self.root_preflight_calls = self.root_preflight_calls.saturating_add(1);
        self.commands.push(vec!["root_preflight".to_string()]);
        self.root_preflight_result
            .clone()
            .unwrap_or(Ok(()))
            .map_err(DeviceOperationError::from)
    }

    fn uses_fake_device_filesystem(&self) -> bool {
        true
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        FakeDryRunDevice::install_apk(self, apk_path, replace_existing)
            .map_err(DeviceOperationError::from)
    }

    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError> {
        self.commands.push(vec![
            if sync { "push_sync" } else { "push" }.to_string(),
            source.to_string_lossy().to_string(),
            dest.to_string(),
        ]);
        self.remote_paths.insert(dest.to_string());
        if source.is_dir() {
            self.remote_dirs.insert(dest.to_string());
        }
        Ok(())
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.commands
            .push(vec!["mkdir_p".to_string(), path.to_string()]);
        self.remote_paths.insert(path.to_string());
        self.remote_dirs.insert(path.to_string());
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.commands
            .push(vec!["remove_file".to_string(), path.to_string()]);
        self.remote_paths.remove(path);
        self.remote_dirs.remove(path);
        Ok(())
    }

    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.commands
            .push(vec!["remove_tree".to_string(), path.to_string()]);
        let prefix = format!("{}/", path.trim_end_matches('/'));
        self.remote_paths
            .retain(|candidate| candidate != path && !candidate.starts_with(&prefix));
        self.remote_dirs
            .retain(|candidate| candidate != path && !candidate.starts_with(&prefix));
        Ok(())
    }

    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError> {
        self.commands.push(vec![
            "copy_on_device".to_string(),
            source.to_string(),
            dest.to_string(),
            py_bool(recursive).to_string(),
            py_bool(privileged).to_string(),
        ]);
        self.remote_paths.insert(dest.to_string());
        if recursive {
            self.remote_dirs.insert(dest.to_string());
        }
        Ok(())
    }

    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError> {
        Ok(FakeDryRunDevice::package_installed(self, package_name))
    }

    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        Ok(FakeDryRunDevice::path_exists(self, path))
    }

    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        Ok(FakeDryRunDevice::path_is_dir(self, path))
    }

    fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), DeviceOperationError> {
        FakeDryRunDevice::run_plan_command(self, command).map_err(DeviceOperationError::from)
    }

    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError> {
        FakeDryRunDevice::launch_app(self, package_name, activity)
            .map_err(DeviceOperationError::from)
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError> {
        FakeDryRunDevice::force_stop_app(self, package_name).map_err(DeviceOperationError::from)
    }
}

pub(crate) fn map_adb_error(error: adb::AdbCommandError) -> DeviceOperationError {
    match error {
        adb::AdbCommandError::TimedOut { cleanup, .. } => DeviceOperationError::timed_out(cleanup),
        adb::AdbCommandError::CommandFailed => DeviceOperationError {
            kind: DeviceOperationKind::CommandFailed,
            message: "The device operation failed.".to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::DeviceOffline => {
            DeviceOperationError::transport(DeviceOperationKind::DeviceOffline)
        }
        adb::AdbCommandError::DeviceUnauthorized => {
            DeviceOperationError::transport(DeviceOperationKind::DeviceUnauthorized)
        }
        adb::AdbCommandError::DeviceDisconnected => {
            DeviceOperationError::transport(DeviceOperationKind::DeviceDisconnected)
        }
        adb::AdbCommandError::AdbServerUnavailable => {
            DeviceOperationError::transport(DeviceOperationKind::AdbServerUnavailable)
        }
        adb::AdbCommandError::TransportReset => {
            DeviceOperationError::transport(DeviceOperationKind::TransportReset)
        }
        adb::AdbCommandError::TransportFailure => {
            DeviceOperationError::transport(DeviceOperationKind::TransportFailure)
        }
        adb::AdbCommandError::DeviceIdentityChanged { phase } => {
            DeviceOperationError::identity(DeviceOperationKind::DeviceIdentityChanged(phase), phase)
        }
        adb::AdbCommandError::DeviceIdentityUnverified { phase } => DeviceOperationError::identity(
            DeviceOperationKind::DeviceIdentityUnverified(phase),
            phase,
        ),
        adb::AdbCommandError::ProcessFailed { cleanup, .. } => DeviceOperationError {
            kind: DeviceOperationKind::ProcessFailed,
            message: "The device process failed before a trustworthy result was available."
                .to_string(),
            cleanup: Some(cleanup),
        },
        adb::AdbCommandError::RootDenied => DeviceOperationError {
            kind: DeviceOperationKind::RootDenied,
            message: "Root access was denied by the device.".to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::RootUnavailable => DeviceOperationError {
            kind: DeviceOperationKind::RootUnavailable,
            message: "Root access is unavailable on this device.".to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::RootAuthorityRevoked => DeviceOperationError {
            kind: DeviceOperationKind::RootAuthorityRevoked,
            message: "Root authority was revoked during execution.".to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::RootAuthorityUnverified => DeviceOperationError {
            kind: DeviceOperationKind::RootAuthorityUnverified,
            message: "Continued root authority could not be confirmed safely.".to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::Resolution(_)
        | adb::AdbCommandError::RootCheckFailed
        | adb::AdbCommandError::InvalidPlanCommand(_) => DeviceOperationError::other(
            "The device operation could not be started or completed safely.",
        ),
    }
}

fn real_adb_error<E: adb::AdbCommandExecutor>(
    device: &mut adb::RealAdbDevice<E>,
    fallback: String,
) -> DeviceOperationError {
    let mutation_precedes_root_failure = device.root_authority_failure_after_mutation();
    device
        .take_last_error()
        .map(|error| {
            let root_failure = matches!(
                error,
                adb::AdbCommandError::RootAuthorityRevoked
                    | adb::AdbCommandError::RootAuthorityUnverified
            );
            let mut mapped = map_adb_error(error);
            if root_failure && mutation_precedes_root_failure {
                mapped.message =
                    crate::executor::root_authority::ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER
                        .to_string();
            }
            mapped
        })
        .unwrap_or_else(|| DeviceOperationError::other(fallback))
}

impl<E: adb::AdbCommandExecutor> ExecutorDevice for adb::RealAdbDevice<E> {
    fn configure_identity_guard(&mut self, target: Option<&TargetDeviceBinding>) {
        adb::RealAdbDevice::configure_identity_guard(self, target);
    }

    fn configure_root_authority(&mut self, reviewed_root_authorized: bool) {
        adb::RealAdbDevice::configure_root_authority(self, reviewed_root_authorized);
    }

    fn owns_per_command_root_authority(&self) -> bool {
        true
    }

    fn revalidate_root(&mut self) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::check_root(self).map_err(|message| real_adb_error(self, message))
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::install_apk(self, apk_path, replace_existing)
            .map_err(|message| real_adb_error(self, message))
    }

    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::push(self, source, dest, sync)
            .map_err(|message| real_adb_error(self, message))
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::mkdir_p(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::remove_file(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::remove_tree(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::copy_on_device(self, source, dest, recursive, privileged)
            .map_err(|message| real_adb_error(self, message))
    }

    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError> {
        adb::RealAdbDevice::package_installed(self, package_name)
            .map_err(|message| real_adb_error(self, message))
    }

    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        adb::RealAdbDevice::path_exists(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        adb::RealAdbDevice::path_is_dir(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::run_plan_command(self, command)
            .map_err(|message| real_adb_error(self, message))
    }

    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::launch_app(self, package_name, activity)
            .map_err(|message| real_adb_error(self, message))
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::force_stop_app(self, package_name)
            .map_err(|message| real_adb_error(self, message))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SandboxRoots {
    pub(crate) runtime_root: PathBuf,
    pub(crate) cache_root: PathBuf,
    pub(crate) fake_device_root: PathBuf,
    pub(crate) read_only_roots: Vec<PathBuf>,
}

impl SandboxRoots {
    pub(crate) fn ensure_read_allowed(&self, path: &Path) -> Result<(), StepFailure> {
        let normalized = path
            .canonicalize()
            .map(|path| normalize_path(&path))
            .unwrap_or_else(|_| normalize_path(path));
        if self
            .allowed_read_roots()
            .iter()
            .any(|root| normalized.starts_with(root))
        {
            return Ok(());
        }
        Err(StepFailure::new(format!(
            "sandbox read rejected outside allowed roots: {}",
            path.display()
        )))
    }

    pub(crate) fn ensure_runtime_or_cache_write(&self, path: &Path) -> Result<(), StepFailure> {
        let normalized = normalize_path(path);
        let runtime_root = normalize_path(&self.runtime_root);
        let cache_root = normalize_path(&self.cache_root);
        if normalized.starts_with(&runtime_root) {
            reject_symlink_ancestor(&self.runtime_root, path)?;
            reject_existing_symlink(path)?;
            return Ok(());
        }
        if normalized.starts_with(&cache_root) {
            reject_symlink_ancestor(&self.cache_root, path)?;
            reject_existing_symlink(path)?;
            return Ok(());
        }
        Err(StepFailure::new(format!(
            "sandbox write rejected outside runtime/cache roots: {}",
            path.display()
        )))
    }

    fn fake_device_path(&self, device_path: &str) -> Result<PathBuf, StepFailure> {
        if !device_path.starts_with('/') {
            return Err(StepFailure::new(format!(
                "fake device path must be absolute: {device_path}"
            )));
        }
        let relative = fake_device_relative_components(Path::new(device_path))?;
        let path = normalize_path(&self.fake_device_root.join(relative));
        let fake_device_root = normalize_path(&self.fake_device_root);
        if path.starts_with(&fake_device_root) {
            Ok(path)
        } else {
            Err(StepFailure::new(format!(
                "path traversal rejected for fake device path: {device_path}"
            )))
        }
    }

    fn extract_zip_to_directory(
        &self,
        archive_path: &Path,
        dest_dir: &Path,
    ) -> Result<Vec<PathBuf>, StepFailure> {
        let operation = self.extract_zip_to_directory_inner(archive_path, dest_dir);
        if operation.is_err() {
            return combine_operation_and_cleanup(operation, remove_host_staging(dest_dir));
        }
        operation
    }

    fn extract_zip_to_directory_inner(
        &self,
        archive_path: &Path,
        dest_dir: &Path,
    ) -> Result<Vec<PathBuf>, StepFailure> {
        self.ensure_runtime_or_cache_write(dest_dir)?;
        let file =
            fs::File::open(archive_path).map_err(|error| StepFailure::new(error.to_string()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|_| StepFailure::new("File is not a zip file".to_string()))?;
        let mut entries = Vec::new();
        let mut declared_paths = HashMap::new();
        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .map_err(|error| StepFailure::new(error.to_string()))?;
            let entry_name = file.name().to_string();
            if file.is_symlink() {
                return Err(StepFailure::new(format!(
                    "symlink archive entry rejected: {entry_name}"
                )));
            }
            let relative = archive_relative_components(Path::new(&entry_name))?;
            let is_dir = file.is_dir() || entry_name.ends_with('/');
            if declared_paths.insert(relative.clone(), is_dir).is_some() {
                return Err(StepFailure::new(format!(
                    "duplicate archive entry rejected: {entry_name}"
                )));
            }
            let target = normalize_path(&dest_dir.join(relative));
            if !target.starts_with(normalize_path(dest_dir)) {
                return Err(StepFailure::new(format!(
                    "unsafe archive entry rejected: {entry_name}"
                )));
            }
            self.ensure_runtime_or_cache_write(&target)?;
            entries.push((index, entry_name, target, is_dir));
        }
        for (path, _) in &declared_paths {
            for ancestor in path.ancestors().skip(1) {
                if ancestor.as_os_str().is_empty() {
                    break;
                }
                if declared_paths.get(ancestor) == Some(&false) {
                    return Err(StepFailure::new(format!(
                        "archive path conflict rejected: {}",
                        path.display()
                    )));
                }
            }
        }
        fs::create_dir_all(dest_dir).map_err(|error| StepFailure::new(error.to_string()))?;
        for (index, entry_name, target, is_dir) in entries {
            let mut file = archive
                .by_index(index)
                .map_err(|error| StepFailure::new(error.to_string()))?;
            reject_symlink_ancestor(dest_dir, &target)?;
            reject_existing_symlink(&target)?;
            if is_dir || entry_name.ends_with('/') {
                fs::create_dir_all(&target).map_err(|error| StepFailure::new(error.to_string()))?;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| StepFailure::new(error.to_string()))?;
                }
                let mut output = fs::File::create(&target)
                    .map_err(|error| StepFailure::new(error.to_string()))?;
                io::copy(&mut file, &mut output)
                    .map_err(|error| StepFailure::new(error.to_string()))?;
            }
        }
        let mut children = fs::read_dir(dest_dir)
            .map_err(|error| StepFailure::new(error.to_string()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| StepFailure::new(error.to_string()))?;
        children.sort();
        if children.is_empty() {
            Ok(vec![dest_dir.to_path_buf()])
        } else {
            Ok(children)
        }
    }

    fn copy_host_source_to_fake_device(
        &self,
        source: &RuntimeValue,
        dest: &str,
        copy_policy: &str,
    ) -> Result<Vec<String>, StepFailure> {
        match source.type_name.as_str() {
            "file_path" => {
                let source_path = source_path_from_value(source)?;
                self.ensure_read_allowed(&source_path)?;
                let (target, dest_is_dir) = self.file_target_path(&source_path, dest)?;
                if copy_policy == "replace" && target.exists() {
                    self.remove_fake_device_path(&target)?;
                }
                self.copy_path(&source_path, &target)?;
                Ok(vec![device_file_target(&source_path, dest, dest_is_dir)])
            }
            "directory_path" => {
                let source_path = source_path_from_value(source)?;
                self.ensure_read_allowed(&source_path)?;
                let target_dir = self.fake_device_path(dest)?;
                if copy_policy == "replace" && target_dir.exists() {
                    self.remove_fake_device_path(&target_dir)?;
                }
                reject_symlink_ancestor(&self.fake_device_root, &target_dir)?;
                reject_existing_symlink(&target_dir)?;
                fs::create_dir_all(&target_dir)
                    .map_err(|error| StepFailure::new(error.to_string()))?;
                let mut copied = Vec::new();
                for child in sorted_children(&source_path)? {
                    let child_name = child
                        .file_name()
                        .expect("source child should have a basename")
                        .to_string_lossy()
                        .to_string();
                    let target = target_dir.join(&child_name);
                    self.copy_path(&child, &target)?;
                    copied.push(join_device_path(dest, &child_name));
                }
                Ok(copied)
            }
            "path_list" => {
                let target_dir = self.fake_device_path(dest)?;
                if copy_policy == "replace" && target_dir.exists() {
                    self.remove_fake_device_path(&target_dir)?;
                }
                reject_symlink_ancestor(&self.fake_device_root, &target_dir)?;
                reject_existing_symlink(&target_dir)?;
                fs::create_dir_all(&target_dir)
                    .map_err(|error| StepFailure::new(error.to_string()))?;
                let mut copied = Vec::new();
                for item in source.value.as_array().cloned().unwrap_or_default() {
                    let Some(item) = item.as_str() else {
                        return Err(StepFailure::new(
                            "copy_files path_list source values must be strings".to_string(),
                        ));
                    };
                    let source_path = PathBuf::from(item);
                    self.ensure_read_allowed(&source_path)?;
                    let child_name = source_path
                        .file_name()
                        .ok_or_else(|| {
                            StepFailure::new(format!(
                                "copy_files source path has no basename: {item}"
                            ))
                        })?
                        .to_string_lossy()
                        .to_string();
                    let target = target_dir.join(&child_name);
                    self.copy_path(&source_path, &target)?;
                    copied.push(join_device_path(dest, &child_name));
                }
                Ok(copied)
            }
            other => Err(StepFailure::new(format!(
                "copy_files does not support source runtime type {other:?}."
            ))),
        }
    }

    fn file_target_path(
        &self,
        source_path: &Path,
        dest: &str,
    ) -> Result<(PathBuf, bool), StepFailure> {
        let dest_path = self.fake_device_path(dest)?;
        if dest_path.is_dir() {
            Ok((
                dest_path.join(
                    source_path
                        .file_name()
                        .expect("source file should have basename"),
                ),
                true,
            ))
        } else {
            Ok((dest_path, false))
        }
    }

    fn copy_path(&self, source: &Path, target: &Path) -> Result<(), StepFailure> {
        reject_existing_symlink(source)?;
        reject_symlink_ancestor(&self.fake_device_root, target)?;
        reject_existing_symlink(target)?;
        let fake_device_root = normalize_path(&self.fake_device_root);
        let normalized_target = normalize_path(target);
        if !normalized_target.starts_with(&fake_device_root) {
            return Err(StepFailure::new(format!(
                "sandbox write rejected outside fake device root: {}",
                target.display()
            )));
        }
        if source.is_dir() {
            copy_dir_recursive(source, target, &self.fake_device_root)
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| StepFailure::new(error.to_string()))?;
            }
            fs::copy(source, target)
                .map(|_| ())
                .map_err(|error| StepFailure::new(error.to_string()))
        }
    }

    fn remove_fake_device_path(&self, path: &Path) -> Result<(), StepFailure> {
        let normalized = normalize_path(path);
        if !normalized.starts_with(normalize_path(&self.fake_device_root)) {
            return Err(StepFailure::new(format!(
                "delete rejected outside fake device root: {}",
                path.display()
            )));
        }
        reject_symlink_ancestor(&self.fake_device_root, path)?;
        reject_existing_symlink(path)?;
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|error| StepFailure::new(error.to_string()))
        } else if path.exists() {
            fs::remove_file(path).map_err(|error| StepFailure::new(error.to_string()))
        } else {
            Ok(())
        }
    }

    fn allowed_read_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![
            normalize_path(&self.runtime_root),
            normalize_path(&self.cache_root),
            normalize_path(&self.fake_device_root),
        ];
        roots.extend(
            [&self.runtime_root, &self.cache_root, &self.fake_device_root]
                .into_iter()
                .filter_map(|root| root.canonicalize().ok())
                .map(|root| normalize_path(&root)),
        );
        roots.extend(self.read_only_roots.iter().map(|root| normalize_path(root)));
        roots.extend(
            self.read_only_roots
                .iter()
                .filter_map(|root| root.canonicalize().ok())
                .map(|root| normalize_path(&root)),
        );
        roots
    }
}

#[derive(Debug, Default)]
struct ExecutionState {
    steps: HashMap<String, StepRuntimeState>,
    artifacts: HashMap<String, ArtifactRuntimeState>,
    inputs: HashMap<String, RuntimeValue>,
}

impl ExecutionState {
    fn from_plan(plan: &ExecutionPlan) -> Self {
        Self {
            steps: HashMap::new(),
            inputs: plan
                .inputs
                .iter()
                .map(|input| (input.id.clone(), input.value.clone()))
                .collect(),
            artifacts: plan
                .artifacts
                .iter()
                .map(|artifact| {
                    (
                        artifact.id.clone(),
                        ArtifactRuntimeState {
                            status: ArtifactRuntimeStatus::Pending,
                            local_path: None,
                            resolved_url: None,
                            filename: None,
                            cache_hit: false,
                            error: None,
                        },
                    )
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug)]
struct StepRuntimeState {
    status: StepRuntimeStatus,
    outputs: OrderedMap<RuntimeValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StepRuntimeStatus {
    Skipped,
    Blocked,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ArtifactRuntimeStatus {
    Pending,
    Resolved,
    Failed,
}

impl ArtifactRuntimeStatus {
    fn as_status_value(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Resolved => "resolved",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
struct ArtifactRuntimeState {
    status: ArtifactRuntimeStatus,
    local_path: Option<String>,
    resolved_url: Option<String>,
    filename: Option<String>,
    cache_hit: bool,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct StepFailure {
    pub(crate) message: String,
    outputs: OrderedMap<RuntimeValue>,
    pub(crate) failure_kind: Option<StepFailureKind>,
    pub(crate) cleanup: Option<ProcessCleanup>,
}

impl StepFailure {
    fn new(message: String) -> Self {
        Self {
            message,
            outputs: OrderedMap::new(),
            failure_kind: None,
            cleanup: None,
        }
    }
}

impl From<DeviceOperationError> for StepFailure {
    fn from(error: DeviceOperationError) -> Self {
        let failure_kind = match error.kind {
            DeviceOperationKind::TimedOut => Some(StepFailureKind::OperationTimedOut),
            DeviceOperationKind::CommandFailed => Some(StepFailureKind::OperationCommandFailed),
            DeviceOperationKind::ProcessFailed => Some(StepFailureKind::OperationProcessFailed),
            DeviceOperationKind::DeviceOffline => Some(StepFailureKind::DeviceOffline),
            DeviceOperationKind::DeviceUnauthorized => Some(StepFailureKind::DeviceUnauthorized),
            DeviceOperationKind::DeviceDisconnected => Some(StepFailureKind::DeviceDisconnected),
            DeviceOperationKind::AdbServerUnavailable => {
                Some(StepFailureKind::AdbServerUnavailable)
            }
            DeviceOperationKind::TransportReset => Some(StepFailureKind::TransportReset),
            DeviceOperationKind::TransportFailure => Some(StepFailureKind::TransportFailure),
            DeviceOperationKind::DeviceIdentityChanged(phase) => {
                Some(StepFailureKind::DeviceIdentityChanged(phase))
            }
            DeviceOperationKind::DeviceIdentityUnverified(phase) => {
                Some(StepFailureKind::DeviceIdentityUnverified(phase))
            }
            DeviceOperationKind::RootAuthorityRevoked => {
                Some(StepFailureKind::RootAuthorityRevoked)
            }
            DeviceOperationKind::RootAuthorityUnverified => {
                Some(StepFailureKind::RootAuthorityUnverified)
            }
            DeviceOperationKind::RootDenied => Some(StepFailureKind::RootDenied),
            DeviceOperationKind::RootUnavailable => Some(StepFailureKind::RootUnavailable),
            DeviceOperationKind::Other => Some(StepFailureKind::OperationFailed),
        };
        Self {
            message: error.message,
            outputs: OrderedMap::new(),
            failure_kind,
            cleanup: error.cleanup,
        }
    }
}

fn host_staging_cleanup_failure() -> StepFailure {
    let mut outputs = OrderedMap::new();
    outputs.insert(
        "cleanup_report".to_string(),
        RuntimeValue {
            type_name: "object".to_string(),
            value: json!({
                "classification": "host_staging_cleanup_failed",
                "status": "failed",
            }),
            location: None,
        },
    );
    StepFailure {
        message: "host_staging_cleanup_failed: temporary extraction staging could not be removed."
            .to_string(),
        outputs,
        failure_kind: None,
        cleanup: None,
    }
}

fn remove_host_staging(path: &Path) -> Result<(), StepFailure> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(host_staging_cleanup_failure()),
    }
}

fn combine_operation_and_cleanup<T>(
    operation: Result<T, StepFailure>,
    cleanup: Result<(), StepFailure>,
) -> Result<T, StepFailure> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_failure)) => Err(cleanup_failure),
        (Err(operation_failure), Ok(())) => Err(operation_failure),
        (Err(mut operation_failure), Err(cleanup_failure)) => {
            operation_failure.outputs.extend(cleanup_failure.outputs);
            Err(operation_failure)
        }
    }
}

#[cfg(test)]
mod cleanup_combination_tests {
    use super::*;

    #[test]
    fn cleanup_failure_is_separate_and_preserves_the_original_operation_failure() {
        let operation = Err::<(), _>(StepFailure::new(
            "verification_failed: controlled original failure".to_string(),
        ));
        let failure = combine_operation_and_cleanup(operation, Err(host_staging_cleanup_failure()))
            .unwrap_err();

        assert_eq!(
            failure.message,
            "verification_failed: controlled original failure"
        );
        assert_eq!(
            failure.outputs["cleanup_report"].value,
            json!({
                "classification": "host_staging_cleanup_failed",
                "status": "failed",
            })
        );
    }

    #[test]
    fn cleanup_failure_after_success_has_its_own_fixed_classification() {
        let failure = combine_operation_and_cleanup(
            Ok::<(), StepFailure>(()),
            Err(host_staging_cleanup_failure()),
        )
        .unwrap_err();

        assert_eq!(
            failure.outputs["cleanup_report"].value["classification"],
            "host_staging_cleanup_failed"
        );
        assert!(!failure.message.contains('/'));
    }
}

#[derive(Clone, Debug)]
enum RootPreflightState {
    NotRun,
    Granted,
    Failed(String),
}

impl From<String> for StepFailure {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

#[derive(Clone, Debug)]
struct PermissionPolicy {
    on_failure: String,
    require_all: bool,
}

#[derive(Clone, Debug)]
struct PermissionAction {
    kind: PermissionActionKind,
    package_name: String,
    permission: Option<String>,
    op: Option<String>,
    desired_mode: Option<String>,
    required: bool,
    when: Option<Value>,
    source_section: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PermissionActionKind {
    RuntimePermission,
    Appop,
}

fn record(
    step: &ExecutionStep,
    status: StepRunStatus,
    message: Option<String>,
    outputs: OrderedMap<RuntimeValue>,
) -> StepRunRecord {
    record_with_failure(step, status, message, outputs, None, None)
}

fn record_with_failure(
    step: &ExecutionStep,
    status: StepRunStatus,
    message: Option<String>,
    outputs: OrderedMap<RuntimeValue>,
    failure_kind: Option<StepFailureKind>,
    cleanup: Option<ProcessCleanup>,
) -> StepRunRecord {
    StepRunRecord {
        step_id: step.id.clone(),
        status,
        message,
        outputs,
        failure_kind,
        cleanup,
    }
}

fn progress_event(
    step: &ExecutionStep,
    total_steps: usize,
    plan: &ExecutionPlan,
    phase: ProgressPhase,
    status: Option<ProgressStatus>,
    message: Option<String>,
) -> ExecutionProgressEvent {
    let step_index = plan
        .steps
        .iter()
        .position(|candidate| candidate.id == step.id)
        .map(|index| index + 1)
        .unwrap_or(0);
    ExecutionProgressEvent {
        step_index,
        total_steps,
        step_id: step.id.clone(),
        recipe_ref: step.recipe_ref.clone(),
        step_name: step.name.clone(),
        note: crate::planner::normalized_plan_step_note(
            Some(&step.note),
            &step.name,
            &step.type_name,
            &step.id,
        ),
        phase,
        status,
        message,
    }
}

fn blocking_dependencies(state: &ExecutionState, dependencies: &[String]) -> Vec<String> {
    dependencies
        .iter()
        .filter(|dependency_id| {
            state.steps.get(*dependency_id).is_some_and(|step_state| {
                matches!(
                    step_state.status,
                    StepRuntimeStatus::Failed | StepRuntimeStatus::Blocked
                )
            })
        })
        .cloned()
        .collect()
}

fn missing_capabilities_message(plan: &ExecutionPlan, step: &ExecutionStep) -> Option<String> {
    let missing = step
        .constraints
        .capabilities
        .iter()
        .filter(|capability| !runtime_capability_enabled(plan, capability))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "missing required capabilities: {}",
            missing.join(", ")
        ))
    }
}

fn runtime_capability_enabled(plan: &ExecutionPlan, capability: &str) -> bool {
    match capability {
        "adb_available" => plan.runtime_capabilities.adb_available,
        "apk_install" => plan.runtime_capabilities.apk_install,
        "shared_storage_write" => plan.runtime_capabilities.shared_storage_write,
        "app_launch" => plan.runtime_capabilities.app_launch,
        "shell_command" => plan.runtime_capabilities.shell_command,
        "package_remove_for_user" => plan.runtime_capabilities.package_remove_for_user,
        "root_shell" => plan.runtime_capabilities.root_shell,
        "app_data_write" => plan.runtime_capabilities.app_data_write,
        _ => false,
    }
}

fn resolve_step_params(
    state: &ExecutionState,
    params: &OrderedMap<ExecutionParamValue>,
) -> Result<OrderedMap<Value>, StepFailure> {
    let mut resolved = OrderedMap::new();
    for (name, value) in params {
        match value {
            ExecutionParamValue::Literal { value } => {
                resolved.insert(name.clone(), value.clone());
            }
            ExecutionParamValue::Ref { ref_value } => {
                resolved.insert(name.clone(), resolve_runtime_ref(state, ref_value)?);
            }
        }
    }
    Ok(resolved)
}

fn resolve_runtime_ref(state: &ExecutionState, ref_value: &str) -> Result<Value, StepFailure> {
    if let Some(input_ref) = ref_value.strip_prefix("inputs.") {
        let Some(input) = state.inputs.get(input_ref) else {
            return Err(StepFailure::new(format!(
                "unknown_input_ref: Unknown input ref: {ref_value:?}."
            )));
        };
        if matches!(
            input.type_name.as_str(),
            "file_path" | "directory_path" | "path_list"
        ) {
            return serde_json::to_value(input)
                .map_err(|error| StepFailure::new(error.to_string()));
        }
        return Ok(input.value.clone());
    }
    if let Some(artifact_ref) = ref_value.strip_prefix("artifacts.") {
        let Some((artifact_id, field)) = artifact_ref.rsplit_once('.') else {
            return Err(StepFailure::new(format!(
                "invalid_ref_format: Invalid runtime ref: {ref_value:?}."
            )));
        };
        let Some(artifact) = state.artifacts.get(artifact_id) else {
            return Err(StepFailure::new(format!(
                "unknown_artifact_ref: Unknown artifact ref: {ref_value:?}."
            )));
        };
        if artifact.status != ArtifactRuntimeStatus::Resolved {
            return Err(StepFailure::new(format!(
                "artifact_not_resolved: Artifact is not resolved: {ref_value:?}."
            )));
        }
        let runtime_value = match field {
            "status" => RuntimeValue {
                type_name: "string".to_string(),
                value: json!(artifact.status.as_status_value()),
                location: None,
            },
            "local_path" => RuntimeValue {
                type_name: "file_path".to_string(),
                value: json!(artifact.local_path),
                location: Some("host".to_string()),
            },
            "filename" => RuntimeValue {
                type_name: "string".to_string(),
                value: json!(artifact.filename),
                location: None,
            },
            "resolved_url" => RuntimeValue {
                type_name: "string".to_string(),
                value: json!(artifact.resolved_url),
                location: None,
            },
            "cache_hit" => RuntimeValue {
                type_name: "boolean".to_string(),
                value: json!(artifact.cache_hit),
                location: None,
            },
            "error" => RuntimeValue {
                type_name: "string".to_string(),
                value: json!(artifact.error.clone().unwrap_or_default()),
                location: None,
            },
            _ => {
                return Err(StepFailure::new(format!(
                    "unknown_artifact_field: Unknown artifact field in ref: {ref_value:?}."
                )))
            }
        };
        return serde_json::to_value(runtime_value)
            .map_err(|error| StepFailure::new(error.to_string()));
    }
    let Some(step_ref) = ref_value.strip_prefix("steps.") else {
        return Err(StepFailure::new(format!(
            "invalid_ref_format: Invalid runtime ref: {ref_value:?}."
        )));
    };
    let Some((step_id, output_name)) = step_ref.split_once(".outputs.") else {
        return Err(StepFailure::new(format!(
            "invalid_ref_format: Invalid runtime ref: {ref_value:?}."
        )));
    };
    let Some(step_state) = state.steps.get(step_id) else {
        return Err(StepFailure::new(format!(
            "unknown_step_ref: Unknown step ref: {ref_value:?}."
        )));
    };
    if step_state.status != StepRuntimeStatus::Succeeded {
        return Err(StepFailure::new(format!(
            "step_output_unavailable: Step output is unavailable because step {step_id:?} did not succeed."
        )));
    }
    let Some(value) = step_state.outputs.get(output_name) else {
        return Err(StepFailure::new(format!(
            "unknown_step_output: Unknown step output in ref: {ref_value:?}."
        )));
    };
    serde_json::to_value(value).map_err(|error| StepFailure::new(error.to_string()))
}

fn string_list_param(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn runtime_value_param(
    params: &OrderedMap<Value>,
    name: &str,
) -> Result<RuntimeValue, StepFailure> {
    let Some(Value::Object(value)) = params.get(name) else {
        return Err(StepFailure::new(format!(
            "{name} must be a runtime value object"
        )));
    };
    Ok(RuntimeValue {
        type_name: value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        value: value.get("value").cloned().unwrap_or(Value::Null),
        location: value
            .get("location")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn source_path_from_value(source: &RuntimeValue) -> Result<PathBuf, StepFailure> {
    source.value.as_str().map(PathBuf::from).ok_or_else(|| {
        StepFailure::new("copy_files source runtime value must contain a string path".to_string())
    })
}

fn fake_device_relative_components(path: &Path) -> Result<PathBuf, StepFailure> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(StepFailure::new(format!(
                    "path traversal rejected: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(relative)
}

fn archive_relative_components(path: &Path) -> Result<PathBuf, StepFailure> {
    let raw = path.to_string_lossy();
    let portable = raw.replace('\\', "/");
    let rooted_or_drive = portable.starts_with('/')
        || portable
            .as_bytes()
            .get(1)
            .is_some_and(|separator| *separator == b':')
            && portable
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic);
    if rooted_or_drive {
        return Err(StepFailure::new(format!(
            "unsafe archive entry rejected: {}",
            path.display()
        )));
    }
    let mut relative = PathBuf::new();
    for component in Path::new(&portable).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::RootDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(StepFailure::new(format!(
                    "unsafe archive entry rejected: {}",
                    path.display()
                )));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(StepFailure::new(format!(
            "unsafe archive entry rejected: {}",
            path.display()
        )));
    }
    Ok(relative)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn sanitize_step_id(step_id: &str) -> String {
    step_id.replace('/', "_")
}

fn sorted_children(path: &Path) -> Result<Vec<PathBuf>, StepFailure> {
    let mut children = fs::read_dir(path)
        .map_err(|error| StepFailure::new(error.to_string()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| StepFailure::new(error.to_string()))?;
    children.sort();
    Ok(children)
}

fn reject_existing_symlink(path: &Path) -> Result<(), StepFailure> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(StepFailure::new(format!(
            "symlink paths are not supported in sandbox operations: {}",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink_ancestor(root: &Path, path: &Path) -> Result<(), StepFailure> {
    reject_existing_symlink(root)?;
    let root = normalize_path(root);
    let path = normalize_path(path);
    let relative = path.strip_prefix(&root).map_err(|_| {
        StepFailure::new(format!(
            "sandbox path is outside root: {}",
            path.to_string_lossy()
        ))
    })?;
    let mut current = root;
    for component in relative.components() {
        current.push(component.as_os_str());
        reject_existing_symlink(&current)?;
    }
    Ok(())
}

fn copy_dir_recursive(
    source: &Path,
    target: &Path,
    fake_device_root: &Path,
) -> Result<(), StepFailure> {
    reject_existing_symlink(source)?;
    reject_symlink_ancestor(fake_device_root, target)?;
    reject_existing_symlink(target)?;
    fs::create_dir_all(target).map_err(|error| StepFailure::new(error.to_string()))?;
    for child in sorted_children(source)? {
        reject_existing_symlink(&child)?;
        let child_target = target.join(
            child
                .file_name()
                .expect("directory child should have a basename"),
        );
        reject_symlink_ancestor(fake_device_root, &child_target)?;
        reject_existing_symlink(&child_target)?;
        if child.is_dir() {
            copy_dir_recursive(&child, &child_target, fake_device_root)?;
        } else {
            if let Some(parent) = child_target.parent() {
                fs::create_dir_all(parent).map_err(|error| StepFailure::new(error.to_string()))?;
            }
            fs::copy(&child, &child_target)
                .map(|_| ())
                .map_err(|error| StepFailure::new(error.to_string()))?;
        }
    }
    Ok(())
}

fn join_device_path(parent: &str, child: &str) -> String {
    format!("{}/{}", parent.trim_end_matches('/'), child)
}

fn device_file_target(source_path: &Path, dest: &str, dest_is_dir: bool) -> String {
    if dest_is_dir {
        join_device_path(
            dest,
            &source_path
                .file_name()
                .expect("source file should have basename")
                .to_string_lossy(),
        )
    } else {
        dest.to_string()
    }
}

fn required_string_param(condition: &ExecutionStepCondition, name: &str) -> Result<String, String> {
    condition
        .params
        .get(name)
        .map(|value| match value {
            Value::String(value) => value.clone(),
            other => match other {
                Value::Null => "None".to_string(),
                Value::Bool(value) => py_bool(*value).to_string(),
                Value::Number(value) => value.to_string(),
                _ => other.to_string(),
            },
        })
        .ok_or_else(|| format!("Unsupported condition params for {}", condition.type_name))
}

fn permission_policy(value: Option<&Value>) -> PermissionPolicy {
    let Some(Value::Object(policy)) = value else {
        return PermissionPolicy {
            on_failure: "warn".to_string(),
            require_all: false,
        };
    };
    PermissionPolicy {
        on_failure: policy
            .get("on_failure")
            .and_then(Value::as_str)
            .unwrap_or("warn")
            .to_string(),
        require_all: policy
            .get("require_all")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn permission_actions(resolved_params: &OrderedMap<Value>) -> Vec<PermissionAction> {
    let mut actions = Vec::new();
    for (index, item) in value_array(resolved_params.get("runtime"))
        .iter()
        .enumerate()
    {
        let Some(item) = item.as_object() else {
            continue;
        };
        actions.push(PermissionAction {
            kind: PermissionActionKind::RuntimePermission,
            package_name: string_field(item, "package_name"),
            permission: Some(string_field(item, "name")),
            op: None,
            desired_mode: None,
            required: item
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            when: item.get("when").cloned(),
            source_section: format!("params.runtime[{index}]"),
        });
    }
    for (index, item) in value_array(resolved_params.get("appops"))
        .iter()
        .enumerate()
    {
        let Some(item) = item.as_object() else {
            continue;
        };
        actions.push(PermissionAction {
            kind: PermissionActionKind::Appop,
            package_name: string_field(item, "package_name"),
            permission: None,
            op: Some(string_field(item, "op")),
            desired_mode: Some(string_field(item, "mode")),
            required: item
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            when: item.get("when").cloned(),
            source_section: format!("params.appops[{index}]"),
        });
    }
    actions
}

fn value_array(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn string_field(item: &serde_json::Map<String, Value>, field: &str) -> String {
    item.get(field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| item.get(field).map_or_else(String::new, Value::to_string))
}

fn permission_command(action: &PermissionAction) -> Vec<String> {
    match action.kind {
        PermissionActionKind::RuntimePermission => vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            action.package_name.clone(),
            action.permission.clone().unwrap_or_default(),
        ],
        PermissionActionKind::Appop => vec![
            "adb".to_string(),
            "shell".to_string(),
            "appops".to_string(),
            "set".to_string(),
            action.package_name.clone(),
            action.op.clone().unwrap_or_default(),
            action.desired_mode.clone().unwrap_or_default(),
        ],
    }
}

fn permission_result_base(
    step: &ExecutionStep,
    action: &PermissionAction,
) -> serde_json::Map<String, Value> {
    let mut result = serde_json::Map::new();
    result.insert("step_id".to_string(), json!(step.id));
    result.insert(
        "kind".to_string(),
        json!(match action.kind {
            PermissionActionKind::RuntimePermission => "runtime_permission",
            PermissionActionKind::Appop => "appop",
        }),
    );
    result.insert("package_name".to_string(), json!(action.package_name));
    result.insert("source_recipe_id".to_string(), json!(step.recipe_ref));
    result.insert("source_section".to_string(), json!(action.source_section));
    if let Some(permission) = &action.permission {
        result.insert("permission".to_string(), json!(permission));
    }
    if let Some(op) = &action.op {
        result.insert("op".to_string(), json!(op));
    }
    if let Some(desired_mode) = &action.desired_mode {
        result.insert("desired_mode".to_string(), json!(desired_mode));
    }
    result
}

fn permission_not_applicable_reason(
    when: Option<&Value>,
    rooted: bool,
    android_api_level: Option<i64>,
) -> Option<serde_json::Map<String, Value>> {
    let Some(Value::Object(when)) = when else {
        return None;
    };
    if when.get("rooted").and_then(Value::as_bool) == Some(true) && !rooted {
        return Some(object_entries(json!({
            "reason_code": "requires_root",
            "message": "Device is not rooted."
        })));
    }
    if when.get("rooted").and_then(Value::as_bool) == Some(false) && rooted {
        return Some(object_entries(json!({
            "reason_code": "requires_unrooted",
            "message": "Device is rooted."
        })));
    }
    let api_min = when.get("android_api_min").and_then(Value::as_i64);
    let api_max = when.get("android_api_max").and_then(Value::as_i64);
    if (api_min.is_some() || api_max.is_some()) && android_api_level.is_none() {
        return Some(object_entries(json!({
            "reason_code": "missing_android_api_level",
            "message": "Device Android API level is unknown."
        })));
    }
    if let (Some(api_min), Some(android_api_level)) = (api_min, android_api_level) {
        if android_api_level < api_min {
            return Some(object_entries(json!({
                "reason_code": "android_api_out_of_range",
                "message": format!("Device Android API {android_api_level} is below minimum {api_min}.")
            })));
        }
    }
    if let (Some(api_max), Some(android_api_level)) = (api_max, android_api_level) {
        if android_api_level > api_max {
            return Some(object_entries(json!({
                "reason_code": "android_api_out_of_range",
                "message": format!("Device Android API {android_api_level} is above maximum {api_max}.")
            })));
        }
    }
    None
}

fn object_entries(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(entries) => entries,
        _ => serde_json::Map::new(),
    }
}

fn insert_object_entries(
    target: &mut serde_json::Map<String, Value>,
    source: serde_json::Map<String, Value>,
) {
    for (key, value) in source {
        target.insert(key, value);
    }
}

fn py_bool(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

fn value_is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => py_bool(*value).to_string(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn stable_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => py_bool(*value).to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("{value:?}").replace('"', "'"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod phase_5b10_tests {
    use super::*;
    use crate::planner::{
        DeviceContext, ExecutionPlanSource, ExecutionStepConstraints, RuntimeCapabilities,
    };

    #[test]
    fn permission_fail_policy_stops_after_deterministic_mixed_outcomes() {
        let mut params = OrderedMap::new();
        params.insert(
            "runtime".to_string(),
            ExecutionParamValue::Literal {
                value: json!([
                    {
                        "package_name": "com.example.root",
                        "name": "android.permission.CAMERA",
                        "required": false,
                        "when": { "rooted": true }
                    },
                    {
                        "package_name": "com.example.success",
                        "name": "android.permission.POST_NOTIFICATIONS",
                        "required": false
                    }
                ]),
            },
        );
        params.insert(
            "appops".to_string(),
            ExecutionParamValue::Literal {
                value: json!([
                    {
                        "package_name": "com.example.fail",
                        "op": "RUN_IN_BACKGROUND",
                        "mode": "ignore",
                        "required": false
                    },
                    {
                        "package_name": "com.example.after",
                        "op": "MANAGE_EXTERNAL_STORAGE",
                        "mode": "allow",
                        "required": false
                    }
                ]),
            },
        );
        params.insert(
            "policy".to_string(),
            ExecutionParamValue::Literal {
                value: json!({ "on_failure": "fail", "require_all": false }),
            },
        );
        let step = ExecutionStep {
            id: "phase5b10.permissions/mixed".to_string(),
            recipe_ref: "phase5b10.permissions".to_string(),
            type_name: "grant_permissions".to_string(),
            name: "Grant mixed permissions".to_string(),
            note: "Grant mixed permissions".to_string(),
            dependencies: Vec::new(),
            constraints: ExecutionStepConstraints {
                capabilities: vec!["shell_command".to_string()],
                conflicts_with: Vec::new(),
            },
            params,
            skip_if: Vec::new(),
            verify: Vec::new(),
        };
        let plan = ExecutionPlan {
            id: "plan.phase5b10.permissions".to_string(),
            source: ExecutionPlanSource {
                device_profile_ref: "example.device_profile".to_string(),
                device_plan_ref: "example.device_plan".to_string(),
                selected_recipe_refs: vec!["phase5b10.permissions".to_string()],
                expanded_recipe_refs: vec!["phase5b10.permissions".to_string()],
                catalog: None,
            },
            recipes: Vec::new(),
            target_device: None,
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Example".to_string(),
                android_version: 14,
                android_api_level: Some(34),
                device_tags: Vec::new(),
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: true,
                shared_storage_write: true,
                app_launch: true,
                shell_command: true,
                package_remove_for_user: false,
                root_shell: false,
                app_data_write: false,
            },
            inputs: Vec::new(),
            artifacts: Vec::new(),
            steps: vec![step],
            schema_version: 1,
            kind: "execution_plan",
        };
        let successful_command = vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            "com.example.success".to_string(),
            "android.permission.POST_NOTIFICATIONS".to_string(),
        ];
        let failing_command = vec![
            "adb".to_string(),
            "shell".to_string(),
            "appops".to_string(),
            "set".to_string(),
            "com.example.fail".to_string(),
            "RUN_IN_BACKGROUND".to_string(),
            "ignore".to_string(),
        ];
        let mut adapters = DryRunExecutorAdapters::default();
        adapters
            .device_mut()
            .fail_run_plan_command(failing_command.clone(), "app-op denied");
        let mut runner = ExecutorRunner::new(adapters);

        let actual = serde_json::to_value(runner.run(&plan))
            .expect("execution result should serialize deterministically");

        assert_eq!(actual["success"], false);
        assert_eq!(actual["steps"][0]["status"], "failed");
        assert_eq!(actual["steps"][0]["message"], "app-op denied");
        assert_eq!(
            actual["steps"][0]["outputs"]["permission_results"]["value"]["actions"],
            json!([
                {
                    "step_id": "phase5b10.permissions/mixed",
                    "kind": "runtime_permission",
                    "package_name": "com.example.root",
                    "source_recipe_id": "phase5b10.permissions",
                    "source_section": "params.runtime[0]",
                    "permission": "android.permission.CAMERA",
                    "reason_code": "requires_root",
                    "message": "Device is not rooted.",
                    "status": "not_applicable"
                },
                {
                    "step_id": "phase5b10.permissions/mixed",
                    "kind": "runtime_permission",
                    "package_name": "com.example.success",
                    "source_recipe_id": "phase5b10.permissions",
                    "source_section": "params.runtime[1]",
                    "permission": "android.permission.POST_NOTIFICATIONS",
                    "status": "executed"
                },
                {
                    "step_id": "phase5b10.permissions/mixed",
                    "kind": "appop",
                    "package_name": "com.example.fail",
                    "source_recipe_id": "phase5b10.permissions",
                    "source_section": "params.appops[0]",
                    "op": "RUN_IN_BACKGROUND",
                    "desired_mode": "ignore",
                    "status": "failed",
                    "message": "app-op denied"
                }
            ])
        );
        assert_eq!(
            runner.adapters().device().commands(),
            &[
                [vec!["run_plan_command".to_string()], successful_command].concat(),
                [vec!["run_plan_command".to_string()], failing_command].concat(),
            ]
        );
        assert!(!actual.to_string().contains("com.example.after"));
    }
}
