//! In-memory product execution sessions for the additive Phase 0 sidecar API.
//!
//! The manager allows one active device attempt per sidecar process. Completed
//! attempts, ordered events, and step outputs remain inspectable until process
//! exit. Cancellation is cooperative between atomic steps and never rolls back
//! completed work. Retrying or repairing always creates a new execution id.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::artifact_resolver::{ArtifactResolveRequest, ArtifactResolver};
use crate::device_probe::{AdbDeviceProbe, AdbProbeConfig, DeviceProbe, ProcessCommandRunner};
use crate::errors::{ApiError, ApiErrorCode};
use crate::executor::adb::RealAdbDevice;
use crate::executor::{
    ExecutionProgressEvent, ExecutionRunResult, ExecutorAdapters, ExecutorDevice, ExecutorRunner,
    FakeDryRunDevice, ProgressPhase, ProgressStatus, SandboxRoots, StepRunStatus,
};
use crate::model::OrderedMap;
use crate::planner::{ExecutionParamValue, ExecutionPlan, RuntimeValue, TargetDeviceBinding};

/// Filesystem and executable policy fixed when the sidecar starts.
#[derive(Clone, Debug)]
pub struct SidecarRuntimeConfig {
    pub runtime_root: PathBuf,
    pub cache_root: PathBuf,
    pub adb_path: String,
}

impl Default for SidecarRuntimeConfig {
    fn default() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            runtime_root: root.join(".emuchef_runtime"),
            cache_root: root.join(".emuchef_cache").join("artifacts"),
            adb_path: "adb".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Real,
    DryRun,
}

impl ExecutionMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "real" => Some(Self::Real),
            "dry_run" => Some(Self::DryRun),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Running,
    Succeeded,
    SucceededWithWarnings,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeExecutionStatus {
    Pending,
    Running,
    Succeeded,
    SucceededWithWarnings,
    Failed,
    Blocked,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepExecutionStatus {
    Pending,
    Running,
    Succeeded,
    Skipped,
    Failed,
    Blocked,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionIssue {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionStepReport {
    pub step_id: String,
    pub recipe_id: String,
    pub name: String,
    pub note: String,
    pub status: StepExecutionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub outputs: OrderedMap<RuntimeValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRecipeReport {
    pub recipe_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: RecipeExecutionStatus,
    pub steps: Vec<ExecutionStepReport>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReport {
    pub execution_id: String,
    pub plan_id: String,
    pub plan_digest: String,
    /// Full immutable plan content accepted by `startExecution`, retained so a
    /// terminal report is a self-contained record of reviewed intent.
    pub reviewed_plan: ExecutionPlan,
    pub mode: ExecutionMode,
    /// True only for fake-device dry runs. Such reports are not proof that any
    /// operation or verification ran against a real device.
    pub simulated: bool,
    pub verification_scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_device: Option<TargetDeviceBinding>,
    pub status: ExecutionStatus,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub latest_sequence: u64,
    pub recipes: Vec<ExecutionRecipeReport>,
    pub warnings: Vec<ExecutionIssue>,
    pub errors: Vec<ExecutionIssue>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionEvent {
    pub sequence: u64,
    pub timestamp: String,
    pub execution_id: String,
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<ExecutionIssue>,
}

#[derive(Debug)]
struct ExecutionRecord {
    report: ExecutionReport,
    events: Vec<ExecutionEvent>,
    cancel_requested: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
struct ExecutionState {
    next_execution_id: u64,
    active_execution_id: Option<String>,
    records: HashMap<String, ExecutionRecord>,
}

#[derive(Clone, Debug)]
pub struct ExecutionSessionManager {
    config: SidecarRuntimeConfig,
    state: Arc<Mutex<ExecutionState>>,
}

impl Default for ExecutionSessionManager {
    fn default() -> Self {
        Self::new(SidecarRuntimeConfig::default())
    }
}

impl ExecutionSessionManager {
    pub fn new(config: SidecarRuntimeConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(ExecutionState::default())),
        }
    }

    pub(crate) fn start(
        &self,
        plan: ExecutionPlan,
        required_digest: String,
        mode: ExecutionMode,
        supplied_target: Option<TargetDeviceBinding>,
    ) -> Result<Value, ApiError> {
        let actual_digest = crate::plan_digest::execution_plan_digest(&plan).map_err(|_| {
            ApiError::new(
                ApiErrorCode::InvalidExecutionPlan,
                "Execution plan could not be canonically hashed.",
                json!({}),
            )
        })?;
        if actual_digest != required_digest.to_lowercase() {
            return Err(ApiError::new(
                ApiErrorCode::PlanDigestMismatch,
                "Execution plan changed after review.",
                json!({ "expectedPlanDigest": required_digest, "actualPlanDigest": actual_digest }),
            ));
        }
        let target = preflight_target(&plan, mode, supplied_target, &self.config.adb_path)?;

        let (execution_id, report, cancel_requested) = {
            let mut state = lock(&self.state);
            if let Some(active) = &state.active_execution_id {
                return Err(ApiError::new(
                    ApiErrorCode::ExecutionInProgress,
                    "Only one execution may be active in a sidecar process.",
                    json!({ "activeExecutionId": active }),
                ));
            }
            let prospective_number = state.next_execution_id + 1;
            let execution_id = format!("execution-{prospective_number}");
            let attempt_root = self
                .config
                .runtime_root
                .join("executions")
                .join(&execution_id);
            admit_plan_artifacts(
                &plan,
                SandboxRoots {
                    runtime_root: attempt_root.clone(),
                    cache_root: self.config.cache_root.clone(),
                    fake_device_root: attempt_root.join("simulated-device"),
                    read_only_roots: read_only_roots(&plan),
                },
            )?;

            let cancel_requested = Arc::new(AtomicBool::new(false));
            state.next_execution_id = prospective_number;
            let report = initial_report(&execution_id, &plan, &actual_digest, mode, target.clone());
            let mut record = ExecutionRecord {
                report: report.clone(),
                events: Vec::new(),
                cancel_requested: Arc::clone(&cancel_requested),
            };
            append_event(
                &mut record,
                "execution_started",
                None,
                None,
                None,
                Some("running".to_string()),
                None,
                Some(if mode == ExecutionMode::DryRun {
                    "Simulated dry run started; no real-device verification is performed."
                        .to_string()
                } else {
                    "Execution started.".to_string()
                }),
                None,
            );
            let report = record.report.clone();
            state.active_execution_id = Some(execution_id.clone());
            state.records.insert(execution_id.clone(), record);
            (execution_id, report, cancel_requested)
        };

        let state = Arc::clone(&self.state);
        let config = self.config.clone();
        let worker_execution_id = execution_id.clone();
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                execute_attempt(
                    &state,
                    &worker_execution_id,
                    &plan,
                    mode,
                    target,
                    config,
                    cancel_requested,
                )
            }));
            match outcome {
                Ok(result) => finish_attempt(&state, &worker_execution_id, result),
                Err(_) => finish_panicked(&state, &worker_execution_id),
            }
            let mut state = lock(&state);
            if state.active_execution_id.as_deref() == Some(&worker_execution_id) {
                state.active_execution_id = None;
            }
        });

        Ok(json!({ "execution": report }))
    }

    pub(crate) fn get(&self, execution_id: &str) -> Result<Value, ApiError> {
        let state = lock(&self.state);
        let record = state
            .records
            .get(execution_id)
            .ok_or_else(|| unknown_execution(execution_id))?;
        Ok(json!({ "execution": record.report }))
    }

    pub(crate) fn events(
        &self,
        execution_id: &str,
        after_sequence: u64,
    ) -> Result<Value, ApiError> {
        let state = lock(&self.state);
        let record = state
            .records
            .get(execution_id)
            .ok_or_else(|| unknown_execution(execution_id))?;
        let events = record
            .events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .collect::<Vec<_>>();
        Ok(json!({
            "executionId": execution_id,
            "events": events,
            "latestSequence": record.report.latest_sequence,
            "terminal": record.report.status != ExecutionStatus::Running,
        }))
    }

    pub(crate) fn cancel(&self, execution_id: &str) -> Result<Value, ApiError> {
        let mut state = lock(&self.state);
        let record = state
            .records
            .get_mut(execution_id)
            .ok_or_else(|| unknown_execution(execution_id))?;
        if record.report.status != ExecutionStatus::Running {
            return Ok(json!({
                "executionId": execution_id,
                "accepted": false,
                "status": record.report.status,
            }));
        }
        record.cancel_requested.store(true, Ordering::Release);
        append_event(
            record,
            "cancel_requested",
            None,
            None,
            None,
            Some("running".to_string()),
            None,
            Some("Cancellation will be observed after the current atomic operation.".to_string()),
            None,
        );
        Ok(json!({
            "executionId": execution_id,
            "accepted": true,
            "status": "running",
        }))
    }

    /// Launch the single app proven eligible by a retained successful real execution.
    ///
    /// The execution remains reusable because one-shot authority belongs to the
    /// Tauri opaque-action store. This operation rederives eligibility on every
    /// invocation and never accepts package, activity, serial, path, or command
    /// input from its caller.
    pub(crate) fn launch_app(&self, execution_id: &str) -> Result<Value, ApiError> {
        let (plan, retained_target, package_name, activity) = {
            let state = lock(&self.state);
            let record = state
                .records
                .get(execution_id)
                .ok_or_else(|| unknown_execution(execution_id))?;
            let (package_name, activity) = eligible_launch_candidate(&record.report)?;
            (
                record.report.reviewed_plan.clone(),
                record.report.target_device.clone(),
                package_name,
                activity,
            )
        };

        let target = preflight_target(
            &plan,
            ExecutionMode::Real,
            retained_target,
            &self.config.adb_path,
        )?
        .ok_or_else(|| {
            ApiError::new(
                ApiErrorCode::LaunchUnavailable,
                "The retained execution no longer has an eligible launch target.",
                json!({}),
            )
        })?;
        RealAdbDevice::new(&self.config.adb_path, Some(target.serial))
            .launch_app(&package_name, activity.as_deref())
            .map_err(|_| {
                ApiError::new(
                    ApiErrorCode::LaunchFailed,
                    "The configured app could not be launched.",
                    json!({}),
                )
            })?;
        Ok(json!({ "launched": true }))
    }
}

fn eligible_launch_candidate(
    report: &ExecutionReport,
) -> Result<(String, Option<String>), ApiError> {
    if report.mode != ExecutionMode::Real
        || !matches!(
            report.status,
            ExecutionStatus::Succeeded | ExecutionStatus::SucceededWithWarnings
        )
    {
        return Err(ApiError::new(
            ApiErrorCode::LaunchUnavailable,
            "This execution is not eligible to launch an app.",
            json!({}),
        ));
    }

    let succeeded_steps = report
        .recipes
        .iter()
        .flat_map(|recipe| recipe.steps.iter())
        .filter(|step| step.status == StepExecutionStatus::Succeeded)
        .map(|step| step.step_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut candidates = BTreeSet::new();
    for step in &report.reviewed_plan.steps {
        if step.type_name != "launch_app" || !succeeded_steps.contains(step.id.as_str()) {
            continue;
        }
        let Some(ExecutionParamValue::Literal {
            value: package_name,
        }) = step.params.get("package_name")
        else {
            continue;
        };
        let Some(package_name) = package_name
            .as_str()
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let activity = match step.params.get("activity") {
            None | Some(ExecutionParamValue::Literal { value: Value::Null }) => None,
            Some(ExecutionParamValue::Literal { value }) => {
                let Some(value) = value.as_str().filter(|value| !value.trim().is_empty()) else {
                    continue;
                };
                Some(value.to_string())
            }
            Some(ExecutionParamValue::Ref { .. }) => continue,
        };
        candidates.insert((package_name.to_string(), activity));
    }

    if candidates.len() != 1 {
        return Err(ApiError::new(
            ApiErrorCode::LaunchUnavailable,
            "The retained execution does not establish exactly one safe launch candidate.",
            json!({}),
        ));
    }
    Ok(candidates.into_iter().next().expect("one candidate"))
}

/// Admit every retained artifact while the caller holds the execution-state
/// lock. This helper performs only bounded local metadata/readability checks and
/// URL parsing; it must never acquire execution state or perform network or
/// filesystem mutation.
fn admit_plan_artifacts(plan: &ExecutionPlan, sandbox: SandboxRoots) -> Result<(), ApiError> {
    let resolver = ArtifactResolver::new(&sandbox);
    for artifact in &plan.artifacts {
        if let Err(error) = resolver.admit(ArtifactResolveRequest {
            artifact_id: &artifact.id,
            type_name: &artifact.type_name,
            url: &artifact.url,
            cache_mode: &artifact.cache,
        }) {
            return Err(ApiError::new(
                ApiErrorCode::ExecutionStartFailed,
                "Execution artifacts are not ready.",
                json!({
                    "code": "artifact_not_ready",
                    "artifactCode": error.code(),
                }),
            ));
        }
    }
    Ok(())
}

fn preflight_target(
    plan: &ExecutionPlan,
    mode: ExecutionMode,
    supplied: Option<TargetDeviceBinding>,
    adb_path: &str,
) -> Result<Option<TargetDeviceBinding>, ApiError> {
    let reviewed = plan.target_device.as_ref();
    if let (Some(reviewed), Some(supplied)) = (reviewed, supplied.as_ref()) {
        reject_target_mismatch(reviewed, supplied)?;
    }
    if mode == ExecutionMode::DryRun {
        return Ok(supplied.or_else(|| reviewed.cloned()));
    }
    let reviewed = reviewed.ok_or_else(|| {
        ApiError::new(
            ApiErrorCode::InvalidExecutionPlan,
            "Real execution requires reviewed target-device binding data.",
            json!({ "field": "plan.target_device" }),
        )
    })?;
    let serial = supplied
        .as_ref()
        .map(|target| target.serial.clone())
        .unwrap_or_else(|| reviewed.serial.clone());
    let facts = AdbDeviceProbe {
        config: AdbProbeConfig {
            adb_path: adb_path.to_string(),
            serial: Some(serial.clone()),
        },
        runner: ProcessCommandRunner,
    }
    .detect()
    .map_err(|_| {
        ApiError::new(
            ApiErrorCode::ExecutionStartFailed,
            "Target-device preflight could not read stable ADB facts.",
            json!({ "code": "target_device_probe_failed" }),
        )
    })?;
    let actual = TargetDeviceBinding {
        serial,
        manufacturer: facts.manufacturer,
        model: facts.model,
        android_api_level: facts.android_api_level,
    };
    reject_target_mismatch(reviewed, &actual)?;
    Ok(Some(actual))
}

fn reject_target_mismatch(
    reviewed: &TargetDeviceBinding,
    actual: &TargetDeviceBinding,
) -> Result<(), ApiError> {
    if let Some(field) = crate::planner::target_mismatch_field(reviewed, actual) {
        return Err(ApiError::new(
            ApiErrorCode::TargetDeviceMismatch,
            "Target device does not match the reviewed plan.",
            json!({ "field": field }),
        ));
    }
    Ok(())
}

fn initial_report(
    execution_id: &str,
    plan: &ExecutionPlan,
    digest: &str,
    mode: ExecutionMode,
    target: Option<TargetDeviceBinding>,
) -> ExecutionReport {
    let mut recipes = plan
        .recipes
        .iter()
        .map(|recipe| ExecutionRecipeReport {
            recipe_id: recipe.id.clone(),
            name: recipe.name.clone(),
            description: recipe.description.clone(),
            status: RecipeExecutionStatus::Pending,
            steps: Vec::new(),
        })
        .collect::<Vec<_>>();
    for step in &plan.steps {
        let index = recipes
            .iter()
            .position(|recipe| recipe.recipe_id == step.recipe_ref)
            .unwrap_or_else(|| {
                recipes.push(ExecutionRecipeReport {
                    recipe_id: step.recipe_ref.clone(),
                    name: step.recipe_ref.clone(),
                    description: None,
                    status: RecipeExecutionStatus::Pending,
                    steps: Vec::new(),
                });
                recipes.len() - 1
            });
        recipes[index].steps.push(ExecutionStepReport {
            step_id: step.id.clone(),
            recipe_id: step.recipe_ref.clone(),
            name: step.name.clone(),
            note: crate::planner::normalized_plan_step_note(
                Some(&step.note),
                &step.name,
                &step.type_name,
                &step.id,
            ),
            status: StepExecutionStatus::Pending,
            message: None,
            outputs: OrderedMap::new(),
        });
    }
    ExecutionReport {
        execution_id: execution_id.to_string(),
        plan_id: plan.id.clone(),
        plan_digest: digest.to_string(),
        reviewed_plan: plan.clone(),
        mode,
        simulated: mode == ExecutionMode::DryRun,
        verification_scope: if mode == ExecutionMode::DryRun {
            "simulated_only"
        } else {
            "real_device"
        },
        target_device: target,
        status: ExecutionStatus::Running,
        started_at: now(),
        finished_at: None,
        latest_sequence: 0,
        recipes,
        warnings: Vec::new(),
        errors: Vec::new(),
    }
}

fn execute_attempt(
    state: &Arc<Mutex<ExecutionState>>,
    execution_id: &str,
    plan: &ExecutionPlan,
    mode: ExecutionMode,
    target: Option<TargetDeviceBinding>,
    config: SidecarRuntimeConfig,
    cancel_requested: Arc<AtomicBool>,
) -> ExecutionRunResult {
    #[cfg(test)]
    if plan.id == "__test_execution_worker_panics__" {
        panic!("injected execution worker panic");
    }
    let attempt_root = config.runtime_root.join("executions").join(execution_id);
    let fake_device_root = attempt_root.join("simulated-device");
    let read_only_roots = read_only_roots(plan);
    match mode {
        ExecutionMode::DryRun => {
            #[cfg(test)]
            if plan.id == "__test_execution_can_cancel__" {
                return run_with_device(
                    state,
                    execution_id,
                    plan,
                    ExecutorAdapters::with_device_and_sandbox_roots(
                        FakeDryRunDevice::default(),
                        attempt_root,
                        config.cache_root,
                        fake_device_root,
                        read_only_roots,
                        true,
                    ),
                    cancel_requested,
                );
            }
            run_with_device(
                state,
                execution_id,
                plan,
                ExecutorAdapters::<FakeDryRunDevice>::with_sandbox_roots(
                    attempt_root,
                    config.cache_root,
                    fake_device_root,
                    read_only_roots,
                ),
                cancel_requested,
            )
        }
        ExecutionMode::Real => {
            let serial = target.map(|target| target.serial);
            run_with_device(
                state,
                execution_id,
                plan,
                ExecutorAdapters::with_device_and_sandbox_roots(
                    RealAdbDevice::new(config.adb_path, serial),
                    attempt_root,
                    config.cache_root,
                    fake_device_root,
                    read_only_roots,
                    true,
                ),
                cancel_requested,
            )
        }
    }
}

fn run_with_device<D: ExecutorDevice>(
    state: &Arc<Mutex<ExecutionState>>,
    execution_id: &str,
    plan: &ExecutionPlan,
    adapters: ExecutorAdapters<D>,
    cancel_requested: Arc<AtomicBool>,
) -> ExecutionRunResult {
    let mut runner = ExecutorRunner::new(adapters);
    runner.run_with_progress_and_cancel(
        plan,
        |event| record_progress(state, execution_id, event),
        || cancel_requested.load(Ordering::Acquire),
    )
}

fn record_progress(
    state: &Arc<Mutex<ExecutionState>>,
    execution_id: &str,
    event: ExecutionProgressEvent,
) {
    let mut state = lock(state);
    let Some(record) = state.records.get_mut(execution_id) else {
        return;
    };
    let status = match (event.phase.clone(), event.status.clone()) {
        (ProgressPhase::Finished, Some(ProgressStatus::Succeeded)) => {
            StepExecutionStatus::Succeeded
        }
        (ProgressPhase::Finished, Some(ProgressStatus::Skipped)) => StepExecutionStatus::Skipped,
        (ProgressPhase::Finished, Some(ProgressStatus::Blocked)) => StepExecutionStatus::Blocked,
        (ProgressPhase::Finished, Some(ProgressStatus::Failed)) => StepExecutionStatus::Failed,
        (ProgressPhase::Finished, Some(ProgressStatus::Cancelled)) => {
            StepExecutionStatus::Cancelled
        }
        _ => StepExecutionStatus::Running,
    };
    if let Some(step) = find_step_mut(&mut record.report, &event.step_id) {
        step.status = status;
        step.message = event.message.clone();
    }
    refresh_recipe_statuses(&mut record.report);
    append_event(
        record,
        "step_progress",
        Some(event.recipe_ref),
        Some(event.step_id),
        Some(enum_json(&event.phase)),
        event.status.as_ref().map(enum_json),
        Some(event.note),
        event.message,
        None,
    );
}

fn finish_attempt(
    state: &Arc<Mutex<ExecutionState>>,
    execution_id: &str,
    result: ExecutionRunResult,
) {
    let mut state = lock(state);
    let Some(record) = state.records.get_mut(execution_id) else {
        return;
    };
    for step_record in &result.steps {
        let recipe_id = record
            .report
            .recipes
            .iter()
            .flat_map(|recipe| recipe.steps.iter())
            .find(|step| step.step_id == step_record.step_id)
            .map(|step| step.recipe_id.clone());
        if let Some(step) = find_step_mut(&mut record.report, &step_record.step_id) {
            step.outputs = step_record.outputs.clone();
        }
        match step_record.status {
            StepRunStatus::Failed | StepRunStatus::Blocked => {
                record.report.errors.push(ExecutionIssue {
                    code: issue_code(step_record.status.clone(), step_record.message.as_deref()),
                    message: step_record
                        .message
                        .clone()
                        .unwrap_or_else(|| "Required execution work did not complete.".to_string()),
                    recipe_id,
                    step_id: Some(step_record.step_id.clone()),
                });
            }
            StepRunStatus::Executed => collect_permission_warnings(
                &mut record.report,
                recipe_id,
                &step_record.step_id,
                &step_record.outputs,
            ),
            StepRunStatus::Skipped | StepRunStatus::Cancelled => {}
        }
    }
    record.report.status = overall_status(&result, !record.report.warnings.is_empty());
    record.report.finished_at = Some(now());
    refresh_recipe_statuses(&mut record.report);
    apply_recipe_warning_statuses(&mut record.report);
    let status = enum_json(&record.report.status);
    append_event(
        record,
        "execution_completed",
        None,
        None,
        None,
        Some(status),
        None,
        Some(if record.report.simulated {
            "Simulated dry run completed; results do not establish real-device verification."
                .to_string()
        } else {
            "Execution completed.".to_string()
        }),
        None,
    );
}

fn finish_panicked(state: &Arc<Mutex<ExecutionState>>, execution_id: &str) {
    let mut state = lock(state);
    let Some(record) = state.records.get_mut(execution_id) else {
        return;
    };
    let issue = ExecutionIssue {
        code: "execution_worker_panicked".to_string(),
        message: "Execution worker terminated unexpectedly.".to_string(),
        recipe_id: None,
        step_id: None,
    };
    record.report.status = ExecutionStatus::Failed;
    record.report.finished_at = Some(now());
    record.report.errors.push(issue.clone());
    append_event(
        record,
        "execution_worker_panicked",
        None,
        None,
        None,
        Some("failed".to_string()),
        None,
        Some(issue.message.clone()),
        Some(issue),
    );
}

fn collect_permission_warnings(
    report: &mut ExecutionReport,
    recipe_id: Option<String>,
    step_id: &str,
    outputs: &OrderedMap<RuntimeValue>,
) {
    let actions = outputs
        .get("permission_results")
        .and_then(|value| value.value.get("actions"))
        .and_then(Value::as_array);
    for action in actions.into_iter().flatten() {
        if action.get("status").and_then(Value::as_str) == Some("failed") {
            report.warnings.push(ExecutionIssue {
                code: "optional_permission_failed".to_string(),
                message: action
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("An optional permission action failed.")
                    .to_string(),
                recipe_id: recipe_id.clone(),
                step_id: Some(step_id.to_string()),
            });
        }
    }
}

fn overall_status(result: &ExecutionRunResult, has_warnings: bool) -> ExecutionStatus {
    if result.cancelled {
        ExecutionStatus::Cancelled
    } else if !result.success {
        ExecutionStatus::Failed
    } else if has_warnings {
        ExecutionStatus::SucceededWithWarnings
    } else {
        ExecutionStatus::Succeeded
    }
}

fn issue_code(status: StepRunStatus, message: Option<&str>) -> String {
    if status == StepRunStatus::Blocked {
        return "dependency_blocked".to_string();
    }
    let message = message.unwrap_or_default();
    for code in [
        "artifact_tls_verification_failed",
        "artifact_http_status",
        "artifact_transport_failed",
        "artifact_digest_mismatch",
        "artifact_size_mismatch",
        "unknown_artifact_ref",
    ] {
        if message.starts_with(code) {
            return code.to_string();
        }
    }
    if message.starts_with("verify failed") {
        "verification_failed".to_string()
    } else if message.starts_with("missing capabilities") {
        "missing_capability".to_string()
    } else if message.starts_with("conflicting steps") {
        "step_conflict".to_string()
    } else {
        "step_execution_failed".to_string()
    }
}

fn refresh_recipe_statuses(report: &mut ExecutionReport) {
    for recipe in &mut report.recipes {
        recipe.status = if recipe
            .steps
            .iter()
            .any(|step| step.status == StepExecutionStatus::Failed)
        {
            RecipeExecutionStatus::Failed
        } else if recipe
            .steps
            .iter()
            .any(|step| step.status == StepExecutionStatus::Blocked)
        {
            RecipeExecutionStatus::Blocked
        } else if recipe
            .steps
            .iter()
            .any(|step| step.status == StepExecutionStatus::Cancelled)
        {
            RecipeExecutionStatus::Cancelled
        } else if recipe
            .steps
            .iter()
            .any(|step| step.status == StepExecutionStatus::Running)
        {
            RecipeExecutionStatus::Running
        } else if recipe
            .steps
            .iter()
            .all(|step| step.status == StepExecutionStatus::Pending)
        {
            RecipeExecutionStatus::Pending
        } else {
            RecipeExecutionStatus::Succeeded
        };
    }
}

fn apply_recipe_warning_statuses(report: &mut ExecutionReport) {
    let recipe_ids = report
        .warnings
        .iter()
        .filter_map(|warning| warning.recipe_id.clone())
        .collect::<Vec<_>>();
    for recipe in &mut report.recipes {
        if recipe.status == RecipeExecutionStatus::Succeeded
            && recipe_ids.contains(&recipe.recipe_id)
        {
            recipe.status = RecipeExecutionStatus::SucceededWithWarnings;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn append_event(
    record: &mut ExecutionRecord,
    event_type: &str,
    recipe_id: Option<String>,
    step_id: Option<String>,
    phase: Option<String>,
    status: Option<String>,
    note: Option<String>,
    message: Option<String>,
    issue: Option<ExecutionIssue>,
) {
    let sequence = record.report.latest_sequence + 1;
    record.report.latest_sequence = sequence;
    record.events.push(ExecutionEvent {
        sequence,
        timestamp: now(),
        execution_id: record.report.execution_id.clone(),
        event_type: event_type.to_string(),
        recipe_id,
        step_id,
        phase,
        status,
        note,
        message,
        issue,
    });
}

fn find_step_mut<'a>(
    report: &'a mut ExecutionReport,
    step_id: &str,
) -> Option<&'a mut ExecutionStepReport> {
    report
        .recipes
        .iter_mut()
        .flat_map(|recipe| recipe.steps.iter_mut())
        .find(|step| step.step_id == step_id)
}

fn read_only_roots(plan: &ExecutionPlan) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    for input in &plan.inputs {
        if input.value.location.as_deref() != Some("host") {
            continue;
        }
        if let Some(path) = input.value.value.as_str() {
            roots.push(PathBuf::from(path));
        }
        if let Some(paths) = input.value.value.as_array() {
            roots.extend(paths.iter().filter_map(Value::as_str).map(PathBuf::from));
        }
    }
    roots.extend(plan.artifacts.iter().filter_map(|artifact| {
        artifact
            .url
            .strip_prefix("file://")
            .map(|path| Path::new(path).to_path_buf())
    }));
    roots
}

fn enum_json<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToString::to_string))
        .unwrap_or_default()
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("RFC 3339 formatting for UTC time should succeed")
}

fn unknown_execution(execution_id: &str) -> ApiError {
    ApiError::new(
        ApiErrorCode::UnknownExecution,
        format!("Unknown execution id: {execution_id}"),
        json!({ "executionId": execution_id }),
    )
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::planner::{
        DeviceContext, ExecutionArtifact, ExecutionParamValue, ExecutionPlanSource,
        ExecutionRecipeSnapshot, ExecutionStep, ExecutionStepConstraints, RuntimeCapabilities,
    };

    #[test]
    fn backend_default_cache_root_remains_working_directory_local() {
        let current = std::env::current_dir().unwrap();
        let config = SidecarRuntimeConfig::default();
        assert_eq!(config.cache_root, current.join(".emuchef_cache/artifacts"));
        assert_eq!(config.runtime_root, current.join(".emuchef_runtime"));
    }

    #[test]
    fn target_matching_normalizes_display_text_but_not_serial_or_api() {
        let reviewed = TargetDeviceBinding {
            serial: "SERIAL-1".to_string(),
            manufacturer: Some("AYANEO  Corp".to_string()),
            model: Some("Pocket S2".to_string()),
            android_api_level: Some(35),
        };
        let equivalent = TargetDeviceBinding {
            serial: "SERIAL-1".to_string(),
            manufacturer: Some(" ayaneo corp ".to_string()),
            model: Some("POCKET   S2".to_string()),
            android_api_level: Some(35),
        };
        assert!(reject_target_mismatch(&reviewed, &equivalent).is_ok());
        let different = TargetDeviceBinding {
            serial: "SERIAL-2".to_string(),
            ..equivalent.clone()
        };
        assert_eq!(
            reject_target_mismatch(&reviewed, &different)
                .unwrap_err()
                .code,
            ApiErrorCode::TargetDeviceMismatch
        );
        let mismatches = [
            (
                TargetDeviceBinding {
                    manufacturer: Some("Different".to_string()),
                    ..equivalent.clone()
                },
                "manufacturer",
            ),
            (
                TargetDeviceBinding {
                    model: None,
                    ..equivalent.clone()
                },
                "model",
            ),
            (
                TargetDeviceBinding {
                    android_api_level: Some(34),
                    ..equivalent
                },
                "android_api_level",
            ),
        ];
        for (actual, field) in mismatches {
            let error = reject_target_mismatch(&reviewed, &actual).unwrap_err();
            assert_eq!(error.details["field"], field);
        }
    }

    #[test]
    fn overall_status_distinguishes_warnings_failure_and_cancellation() {
        let mut result = ExecutionRunResult {
            success: true,
            cancelled: false,
            total_steps: 0,
            steps: Vec::new(),
        };
        assert_eq!(overall_status(&result, false), ExecutionStatus::Succeeded);
        assert_eq!(
            overall_status(&result, true),
            ExecutionStatus::SucceededWithWarnings
        );
        result.success = false;
        assert_eq!(overall_status(&result, false), ExecutionStatus::Failed);
        result.cancelled = true;
        assert_eq!(overall_status(&result, false), ExecutionStatus::Cancelled);
    }

    #[test]
    fn dry_run_reports_are_grouped_simulated_and_emit_incremental_events() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(temp.path());
        let plan = test_plan("plan.phase0");
        let digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
        let started = manager
            .start(plan, digest.clone(), ExecutionMode::DryRun, None)
            .unwrap();
        let execution_id = started["execution"]["executionId"]
            .as_str()
            .unwrap()
            .to_string();
        let report = wait_for_terminal(&manager, &execution_id);

        assert_eq!(report["status"], "succeeded");
        assert_eq!(report["planDigest"], digest);
        assert_eq!(report["reviewedPlan"]["id"], "plan.phase0");
        assert_eq!(report["simulated"], true);
        assert_eq!(report["verificationScope"], "simulated_only");
        assert_eq!(report["recipes"][0]["name"], "Example Recipe");
        assert_eq!(report["recipes"][0]["description"], "Example description");
        assert_eq!(report["recipes"][0]["steps"][0]["note"], "Waiting briefly");
        assert!(report["startedAt"].as_str().unwrap().ends_with('Z'));
        assert!(report["finishedAt"].as_str().unwrap().ends_with('Z'));

        let all = manager.events(&execution_id, 0).unwrap();
        let events = all["events"].as_array().unwrap();
        assert!(events.len() >= 5);
        assert!(events
            .iter()
            .enumerate()
            .all(|(index, event)| event["sequence"] == json!(index + 1)));
        let after = manager.events(&execution_id, 1).unwrap();
        assert!(after["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["sequence"].as_u64().unwrap() > 1));
    }

    #[test]
    fn failed_artifact_admission_allocates_no_attempt_and_leaves_the_slot_reusable() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(temp.path());
        let plan = plan_with_artifact(
            "plan.rejected",
            "ftp://user:password@example.com/archive.zip?token=secret#private",
            "default",
        );
        let digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
        let error = manager
            .start(plan, digest, ExecutionMode::DryRun, None)
            .unwrap_err();
        assert_eq!(error.code, ApiErrorCode::ExecutionStartFailed);
        assert_eq!(error.message, "Execution artifacts are not ready.");
        assert_eq!(error.details["code"], "artifact_not_ready");
        assert_eq!(error.details["artifactCode"], "artifact_scheme_unsupported");
        let serialized = error.to_value().to_string();
        for sensitive in ["user", "password", "secret", "private", "example.com"] {
            assert!(!serialized.contains(sensitive));
        }
        assert_eq!(
            manager.get("execution-1").unwrap_err().code,
            ApiErrorCode::UnknownExecution
        );
        assert_eq!(
            manager.events("execution-1", 0).unwrap_err().code,
            ApiErrorCode::UnknownExecution
        );
        assert!(!temp.path().join("runtime").exists());
        assert!(!temp.path().join("cache").exists());

        let valid = test_plan("plan.after-admission-failure");
        let digest = crate::plan_digest::execution_plan_digest(&valid).unwrap();
        let started = manager
            .start(valid, digest, ExecutionMode::DryRun, None)
            .unwrap();
        assert_eq!(started["execution"]["executionId"], "execution-1");
        let report = wait_for_terminal(&manager, "execution-1");
        assert_eq!(report["status"], "succeeded");
    }

    #[test]
    fn cold_http_artifact_admission_starts_without_contacting_the_source() {
        use std::net::TcpListener;

        let temp = tempfile::tempdir().unwrap();
        let manager = manager(temp.path());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/artifact.bin", listener.local_addr().unwrap());
        let plan = plan_with_artifact("plan.cold-http", &url, "none");
        let digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
        let started = manager
            .start(plan, digest, ExecutionMode::DryRun, None)
            .unwrap();
        assert_eq!(started["execution"]["executionId"], "execution-1");
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let report = wait_for_terminal(&manager, "execution-1");
        assert_eq!(report["status"], "succeeded");
    }

    #[cfg(unix)]
    #[test]
    fn real_mode_admission_runs_after_target_preflight_without_allocating_on_failure() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let fake_adb = temp.path().join("adb");
        fs::write(
            &fake_adb,
            "#!/bin/sh\nprintf '[ro.product.manufacturer]: [Example]\\n[ro.product.model]: [Example]\\n[ro.build.version.sdk]: [35]\\n'\n",
        )
        .unwrap();
        fs::set_permissions(&fake_adb, fs::Permissions::from_mode(0o755)).unwrap();
        let manager = ExecutionSessionManager::new(SidecarRuntimeConfig {
            runtime_root: temp.path().join("runtime"),
            cache_root: temp.path().join("cache"),
            adb_path: fake_adb.to_string_lossy().into_owned(),
        });
        let mut plan =
            plan_with_artifact("plan.real-rejected", "file:relative/artifact.bin", "none");
        plan.target_device = Some(TargetDeviceBinding {
            serial: "REAL-1".to_string(),
            manufacturer: Some("Example".to_string()),
            model: Some("Example".to_string()),
            android_api_level: Some(35),
        });
        let mut mismatched = plan.clone();
        mismatched.target_device.as_mut().unwrap().model = Some("Different".to_string());
        let mismatch_digest = crate::plan_digest::execution_plan_digest(&mismatched).unwrap();
        let mismatch = manager
            .start(mismatched, mismatch_digest, ExecutionMode::Real, None)
            .unwrap_err();
        assert_eq!(mismatch.code, ApiErrorCode::TargetDeviceMismatch);

        let digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
        let error = manager
            .start(plan, digest, ExecutionMode::Real, None)
            .unwrap_err();
        assert_eq!(error.details["code"], "artifact_not_ready");
        assert_eq!(error.details["artifactCode"], "artifact_url_invalid");
        assert_eq!(
            manager.get("execution-1").unwrap_err().code,
            ApiErrorCode::UnknownExecution
        );
        assert!(!temp.path().join("runtime").exists());
        assert!(!temp.path().join("cache").exists());

        let mut valid = test_plan("plan.real-after-admission-failure");
        valid.target_device = Some(TargetDeviceBinding {
            serial: "REAL-1".to_string(),
            manufacturer: Some("Example".to_string()),
            model: Some("Example".to_string()),
            android_api_level: Some(35),
        });
        let digest = crate::plan_digest::execution_plan_digest(&valid).unwrap();
        let started = manager
            .start(valid, digest, ExecutionMode::Real, None)
            .unwrap();
        assert_eq!(started["execution"]["executionId"], "execution-1");
        let report = wait_for_terminal(&manager, "execution-1");
        assert_eq!(report["status"], "succeeded");
    }

    #[test]
    fn worker_panic_leaves_terminal_report_and_releases_active_slot() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(temp.path());
        let panic_plan = test_plan("__test_execution_worker_panics__");
        let digest = crate::plan_digest::execution_plan_digest(&panic_plan).unwrap();
        let started = manager
            .start(panic_plan, digest, ExecutionMode::DryRun, None)
            .unwrap();
        let execution_id = started["execution"]["executionId"].as_str().unwrap();
        let report = wait_for_terminal(&manager, execution_id);
        assert_eq!(report["status"], "failed");
        assert_eq!(report["errors"][0]["code"], "execution_worker_panicked");

        let next_plan = test_plan("plan.after-panic");
        let digest = crate::plan_digest::execution_plan_digest(&next_plan).unwrap();
        for attempt in 0..100 {
            if manager
                .start(
                    next_plan.clone(),
                    digest.clone(),
                    ExecutionMode::DryRun,
                    None,
                )
                .is_ok()
            {
                return;
            }
            assert!(attempt < 99, "active execution slot was not released");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn session_cancellation_is_evented_and_terminal_without_rollback_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(temp.path());
        let mut plan = test_plan("__test_execution_can_cancel__");
        if let ExecutionParamValue::Literal { value } = plan.steps[0]
            .params
            .get_mut("duration_ms")
            .expect("wait duration")
        {
            *value = json!(50);
        }
        let mut second = plan.steps[0].clone();
        second.id = "recipe.example/second".to_string();
        second.name = "Second".to_string();
        plan.steps.push(second);
        let digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
        let started = manager
            .start(plan, digest, ExecutionMode::DryRun, None)
            .unwrap();
        let execution_id = started["execution"]["executionId"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(manager.cancel(&execution_id).unwrap()["accepted"], true);
        let report = wait_for_terminal(&manager, &execution_id);
        assert_eq!(report["status"], "cancelled");
        assert!(report["recipes"][0]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["status"] == "cancelled"));
        let events = manager.events(&execution_id, 0).unwrap();
        assert!(events["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["eventType"] == "cancel_requested"));
    }

    fn eligible_report(status: ExecutionStatus) -> ExecutionReport {
        let mut plan = test_plan("plan.launch");
        plan.steps[0].id = "recipe.example/launch".to_string();
        plan.steps[0].type_name = "launch_app".to_string();
        plan.steps[0].params.clear();
        plan.steps[0].params.insert(
            "package_name".to_string(),
            ExecutionParamValue::Literal {
                value: json!("com.example.app"),
            },
        );
        plan.steps[0].params.insert(
            "activity".to_string(),
            ExecutionParamValue::Literal {
                value: json!(".MainActivity"),
            },
        );
        let mut report = initial_report(
            "execution-launch",
            &plan,
            "digest",
            ExecutionMode::Real,
            None,
        );
        report.status = status;
        report.recipes[0].status = RecipeExecutionStatus::Succeeded;
        report.recipes[0].steps[0].status = StepExecutionStatus::Succeeded;
        report
    }

    #[test]
    fn launch_candidate_is_rederived_without_sidecar_consumption() {
        let report = eligible_report(ExecutionStatus::SucceededWithWarnings);
        let first = eligible_launch_candidate(&report).unwrap();
        let second = eligible_launch_candidate(&report).unwrap();
        assert_eq!(
            first,
            (
                "com.example.app".to_string(),
                Some(".MainActivity".to_string())
            )
        );
        assert_eq!(first, second);
    }

    #[test]
    fn launch_candidate_rejects_non_success_dynamic_and_ambiguous_plans() {
        let mut failed = eligible_report(ExecutionStatus::Failed);
        assert_eq!(
            eligible_launch_candidate(&failed).unwrap_err().code,
            ApiErrorCode::LaunchUnavailable
        );

        failed.status = ExecutionStatus::Succeeded;
        failed.reviewed_plan.steps[0].params.insert(
            "package_name".to_string(),
            ExecutionParamValue::Ref {
                ref_value: "steps.other.outputs.package".to_string(),
            },
        );
        assert!(eligible_launch_candidate(&failed).is_err());

        let mut ambiguous = eligible_report(ExecutionStatus::Succeeded);
        let mut second = ambiguous.reviewed_plan.steps[0].clone();
        second.id = "recipe.example/launch-two".to_string();
        second.params.insert(
            "package_name".to_string(),
            ExecutionParamValue::Literal {
                value: json!("com.example.other"),
            },
        );
        ambiguous.reviewed_plan.steps.push(second);
        let mut second_report = ambiguous.recipes[0].steps[0].clone();
        second_report.step_id = "recipe.example/launch-two".to_string();
        ambiguous.recipes[0].steps.push(second_report);
        assert!(eligible_launch_candidate(&ambiguous).is_err());
    }

    fn manager(root: &Path) -> ExecutionSessionManager {
        ExecutionSessionManager::new(SidecarRuntimeConfig {
            runtime_root: root.join("runtime"),
            cache_root: root.join("cache"),
            adb_path: "adb".to_string(),
        })
    }

    fn wait_for_terminal(manager: &ExecutionSessionManager, execution_id: &str) -> Value {
        for _ in 0..100 {
            let report = manager.get(execution_id).unwrap()["execution"].clone();
            if report["status"] != "running" {
                return report;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("execution did not become terminal");
    }

    fn test_plan(id: &str) -> ExecutionPlan {
        let mut params = OrderedMap::new();
        params.insert(
            "duration_ms".to_string(),
            ExecutionParamValue::Literal { value: json!(1) },
        );
        ExecutionPlan {
            id: id.to_string(),
            source: ExecutionPlanSource {
                device_profile_ref: "profile.example".to_string(),
                device_plan_ref: "plan.example".to_string(),
                selected_recipe_refs: vec!["recipe.example".to_string()],
                expanded_recipe_refs: vec!["recipe.example".to_string()],
                catalog: None,
            },
            recipes: vec![ExecutionRecipeSnapshot {
                id: "recipe.example".to_string(),
                name: "Example Recipe".to_string(),
                description: Some("Example description".to_string()),
            }],
            target_device: None,
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Example".to_string(),
                android_version: 14,
                android_api_level: Some(35),
                device_tags: Vec::new(),
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: true,
                shared_storage_write: true,
                app_launch: true,
                shell_command: true,
                package_remove_for_user: true,
                root_shell: false,
                app_data_write: false,
            },
            inputs: Vec::new(),
            artifacts: Vec::new(),
            steps: vec![ExecutionStep {
                id: "recipe.example/wait".to_string(),
                recipe_ref: "recipe.example".to_string(),
                type_name: "wait".to_string(),
                name: "Wait".to_string(),
                note: "Waiting briefly".to_string(),
                dependencies: Vec::new(),
                constraints: ExecutionStepConstraints {
                    capabilities: Vec::new(),
                    conflicts_with: Vec::new(),
                },
                params,
                skip_if: Vec::new(),
                verify: Vec::new(),
            }],
            schema_version: 1,
            kind: "execution_plan",
        }
    }

    fn plan_with_artifact(id: &str, url: &str, cache: &str) -> ExecutionPlan {
        let mut plan = test_plan(id);
        plan.artifacts.push(ExecutionArtifact {
            id: "recipe.example/artifact".to_string(),
            type_name: "remote_file".to_string(),
            url: url.to_string(),
            cache: cache.to_string(),
        });
        plan
    }
}
