//! Internal safe executor foundations for fixture-scoped dry-run and sandboxed
//! filesystem/artifact parity.
//!
//! Python remains the executor reference. This module intentionally mirrors only
//! the Python `ExecutionRunResult` shape and selected behaviors needed by Phase
//! 6O/6P tests. It does not expose protocol, CLI, Tauri, subprocess, ADB,
//! network, or unrestricted filesystem side-effect implementations.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::model::OrderedMap;
use crate::planner::{
    ExecutionArtifact, ExecutionParamValue, ExecutionPlan, ExecutionStep, ExecutionStepCondition,
    RuntimeValue,
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionRunResult {
    pub success: bool,
    pub total_steps: usize,
    pub steps: Vec<StepRunRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StepRunRecord {
    pub step_id: String,
    pub status: StepRunStatus,
    pub message: Option<String>,
    pub outputs: OrderedMap<RuntimeValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepRunStatus {
    Executed,
    Skipped,
    Blocked,
    Failed,
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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionProgressEvent {
    pub step_index: usize,
    pub total_steps: usize,
    pub step_id: String,
    pub step_name: String,
    pub phase: ProgressPhase,
    pub status: Option<ProgressStatus>,
    pub message: Option<String>,
}

#[derive(Debug, Default)]
pub struct ExecutorRunner {
    adapters: DryRunExecutorAdapters,
}

impl ExecutorRunner {
    pub fn new(adapters: DryRunExecutorAdapters) -> Self {
        Self { adapters }
    }

    pub fn adapters(&self) -> &DryRunExecutorAdapters {
        &self.adapters
    }

    pub fn run(&mut self, plan: &ExecutionPlan) -> ExecutionRunResult {
        let total_steps = plan.steps.len();
        let step_ids_in_plan = plan
            .steps
            .iter()
            .map(|step| step.id.clone())
            .collect::<HashSet<_>>();
        let mut state = ExecutionState::from_plan(plan);
        let mut records = Vec::new();

        for step in &plan.steps {
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
                records.push(record(
                    step,
                    StepRunStatus::Failed,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
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
                    records.push(record(
                        step,
                        StepRunStatus::Skipped,
                        Some("skip_if matched".to_string()),
                        OrderedMap::new(),
                    ));
                    continue;
                }
                Ok(false) => {}
                Err(message) => {
                    state.steps.insert(
                        step.id.clone(),
                        StepRuntimeState {
                            status: StepRuntimeStatus::Failed,
                            outputs: OrderedMap::new(),
                        },
                    );
                    records.push(record(
                        step,
                        StepRunStatus::Failed,
                        Some(message),
                        OrderedMap::new(),
                    ));
                    continue;
                }
            }

            let run_result = self.run_step(plan, &mut state, step);
            let (status, message, outputs, runtime_status) = match run_result {
                Ok(outputs) => {
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
                        ),
                        Ok(failed_verify) => (
                            StepRunStatus::Failed,
                            Some(format!("verify failed: {}", failed_verify.join(", "))),
                            OrderedMap::new(),
                            StepRuntimeStatus::Failed,
                        ),
                        Err(error) => (
                            StepRunStatus::Failed,
                            Some(error),
                            OrderedMap::new(),
                            StepRuntimeStatus::Failed,
                        ),
                    }
                }
                Err(failure) => (
                    StepRunStatus::Failed,
                    Some(failure.message),
                    failure.outputs,
                    StepRuntimeStatus::Failed,
                ),
            };

            state.steps.insert(
                step.id.clone(),
                StepRuntimeState {
                    status: runtime_status,
                    outputs: outputs.clone(),
                },
            );
            records.push(record(step, status, message, outputs));
        }

        let success = !records.iter().any(|record| {
            matches!(
                record.status,
                StepRunStatus::Failed | StepRunStatus::Blocked
            )
        });
        ExecutionRunResult {
            success,
            total_steps,
            steps: records,
        }
    }

    fn skip_if_matched(&mut self, step: &ExecutionStep) -> Result<bool, String> {
        step.skip_if.iter().try_fold(false, |matched, condition| {
            if matched {
                Ok(true)
            } else {
                self.evaluate_condition(condition)
            }
        })
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
            "resolve_artifacts" => self.execute_resolve_artifacts(plan, state, &resolved_params),
            "extract_artifacts" => self.execute_extract_artifacts(state, step, &resolved_params),
            "extract_archive" => self.execute_extract_archive(step, &resolved_params),
            "copy_files" => self.execute_copy_files(&resolved_params),
            "install_apk" => self.execute_install_apk(&resolved_params),
            "launch_app" => self.execute_launch_app(&resolved_params),
            "force_stop_app" => self.execute_force_stop_app(&resolved_params),
            other => Err(StepFailure::new(format!(
                "Unsupported executor step type in Rust Phase 6P skeleton: {other}"
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
                python_repr(raw_duration)
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
        let mut failure_message = None;

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
                Err(message) => {
                    let mut result = permission_result_base(step, &action);
                    result.insert("status".to_string(), json!("failed"));
                    result.insert("message".to_string(), json!(message));
                    action_results.push(Value::Object(result));
                    if action.required || policy.require_all || policy.on_failure == "fail" {
                        failure_message = Some(message);
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
        if let Some(message) = failure_message {
            return Err(StepFailure { message, outputs });
        }
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
        for artifact_id in string_list_param(resolved_params.get("artifacts")) {
            let Some(artifact) = artifacts_by_id.get(artifact_id.as_str()) else {
                return Err(StepFailure::new(format!(
                    "unknown_artifact_ref: Unknown artifact ref: 'artifacts.{artifact_id}'."
                )));
            };
            let result = self.resolve_artifact(artifact);
            let artifact_state = state
                .artifacts
                .get_mut(&artifact.id)
                .expect("artifact runtime state should be initialized from plan");
            match result {
                Ok(resolved) => {
                    artifact_state.status = ArtifactRuntimeStatus::Resolved;
                    artifact_state.local_path = Some(resolved.local_path);
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

    fn resolve_artifact(
        &mut self,
        artifact: &ExecutionArtifact,
    ) -> Result<ResolvedArtifact, StepFailure> {
        let sandbox = self.adapters.sandbox()?;
        let filename = artifact_filename(&artifact.id, &artifact.url);
        let local_filename = artifact_local_filename(&artifact.id, &artifact.url, &artifact.cache);
        let cache_hit;
        let local_path = if artifact.cache == "default" {
            let path = sandbox.cache_root.join(&local_filename);
            cache_hit = path.exists();
            path
        } else {
            cache_hit = false;
            sandbox.runtime_root.join("downloads").join(&local_filename)
        };

        sandbox.ensure_runtime_or_cache_write(&local_path)?;
        if !local_path.exists() {
            let Some(source_path) = file_url_to_path(&artifact.url) else {
                return Err(StepFailure::new(format!(
                    "artifact_download_failed: Failed to download artifact {:?} from {:?}: network downloads are disabled in Rust Phase 6P",
                    artifact.id, artifact.url
                )));
            };
            sandbox.ensure_read_allowed(&source_path)?;
            fs::create_dir_all(local_path.parent().unwrap())
                .map_err(|error| StepFailure::new(error.to_string()))?;
            fs::copy(&source_path, &local_path)
                .map_err(|error| StepFailure::new(error.to_string()))?;
        }

        Ok(ResolvedArtifact {
            local_path: local_path.to_string_lossy().to_string(),
            filename,
            cache_hit,
        })
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
        if extract_on != "host" {
            return Err(StepFailure::new(
                "extract_artifacts device extraction is outside Rust Phase 6P".to_string(),
            ));
        }
        let sandbox = self.adapters.sandbox()?;
        let extract_root = sandbox
            .runtime_root
            .join("extract")
            .join(sanitize_step_id(&step.id));
        let mut output_paths = Vec::new();
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
            output_paths.extend(
                members
                    .into_iter()
                    .map(|member| json!(member.to_string_lossy().to_string())),
            );
        }
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "extracted_paths".to_string(),
            RuntimeValue {
                type_name: "path_list".to_string(),
                value: Value::Array(output_paths),
                location: Some("host".to_string()),
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
        if extract_on != "host" {
            return Err(StepFailure::new(
                "extract_archive device extraction is outside Rust Phase 6P".to_string(),
            ));
        }
        let archive = runtime_value_param(resolved_params, "archive")?;
        if archive.location.as_deref() != Some("host") {
            return Err(StepFailure::new(
                "Host extraction requires a host-side archive path.".to_string(),
            ));
        }
        let Some(archive_path) = archive.value.as_str() else {
            return Err(StepFailure::new(
                "extract_archive archive runtime value must contain a string path".to_string(),
            ));
        };
        let archive_path = PathBuf::from(archive_path);
        let sandbox = self.adapters.sandbox()?;
        sandbox.ensure_read_allowed(&archive_path)?;
        let extract_root = sandbox
            .runtime_root
            .join("extract")
            .join(sanitize_step_id(&step.id));
        let members = sandbox.extract_zip_to_directory(&archive_path, &extract_root)?;
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
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let source = runtime_value_param(resolved_params, "source")?;
        if source.location.as_deref() != Some("host") {
            return Err(StepFailure::new(
                "copy_files device sources are outside Rust Phase 6P".to_string(),
            ));
        }
        let dest = resolved_params
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| StepFailure::new("copy_files dest must be a string".to_string()))?;
        let copy_policy = resolved_params
            .get("copy_policy")
            .and_then(Value::as_str)
            .unwrap_or("merge");
        let sandbox = self.adapters.sandbox()?;
        let copied_paths = sandbox.copy_host_source_to_fake_device(&source, dest, copy_policy)?;
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
        let apk_path = PathBuf::from(python_value_to_string(&app.value));
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
        let replace_existing = resolved_params
            .get("replace_existing")
            .map(python_truthy)
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
            .map(python_value_to_string)
            .unwrap_or_else(|| "None".to_string());
        let activity = resolved_params
            .get("activity")
            .filter(|value| !value.is_null())
            .map(python_value_to_string);
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
            .map(python_value_to_string)
            .unwrap_or_else(|| "None".to_string());
        if package_name.trim().is_empty() {
            return Err(StepFailure::new(
                "force_stop_app step requires a non-empty package_name.".to_string(),
            ));
        }
        self.adapters.device.force_stop_app(&package_name)?;
        Ok(OrderedMap::new())
    }

    fn evaluate_condition(&mut self, condition: &ExecutionStepCondition) -> Result<bool, String> {
        match condition.type_name.as_str() {
            "package_installed" => {
                let package_name = required_string_param(condition, "package_name")?;
                Ok(self.adapters.device.package_installed(&package_name))
            }
            "path_exists" => {
                let path = required_string_param(condition, "path")?;
                Ok(self.adapters.device.path_exists(&path))
            }
            "file_exists" => {
                let path = required_string_param(condition, "path")?;
                Ok(self.adapters.device.path_exists(&path)
                    && !self.adapters.device.path_is_dir(&path))
            }
            other => Err(format!("Unsupported condition type: {other}")),
        }
    }
}

#[derive(Debug, Default)]
pub struct DryRunExecutorAdapters {
    device: FakeDryRunDevice,
    sleep_calls: Vec<f64>,
    sandbox: Option<SandboxRoots>,
}

impl DryRunExecutorAdapters {
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
        }
    }

    pub fn device(&self) -> &FakeDryRunDevice {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut FakeDryRunDevice {
        &mut self.device
    }

    pub fn sleep_calls(&self) -> &[f64] {
        &self.sleep_calls
    }

    fn sleep(&mut self, seconds: f64) {
        self.sleep_calls.push(seconds);
    }

    fn sandbox(&self) -> Result<&SandboxRoots, StepFailure> {
        self.sandbox.as_ref().ok_or_else(|| {
            StepFailure::new(
                "Phase 6P filesystem/artifact handlers require explicit sandbox roots".to_string(),
            )
        })
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
}

impl FakeDryRunDevice {
    pub fn installed_packages_mut(&mut self) -> &mut HashSet<String> {
        &mut self.installed_packages
    }

    pub fn remote_paths_mut(&mut self) -> &mut HashSet<String> {
        &mut self.remote_paths
    }

    pub fn remote_dirs_mut(&mut self) -> &mut HashSet<String> {
        &mut self.remote_dirs
    }

    pub fn commands(&self) -> &[Vec<String>] {
        &self.commands
    }

    pub fn fail_run_plan_command(&mut self, command: Vec<String>, message: &str) {
        self.run_plan_failures.insert(command, message.to_string());
    }

    pub fn fail_install_apk(&mut self, apk_path: &Path, replace_existing: bool, message: &str) {
        self.install_failures.insert(
            (apk_path.to_string_lossy().to_string(), replace_existing),
            message.to_string(),
        );
    }

    pub fn fail_launch_app(&mut self, package_name: &str, activity: Option<&str>, message: &str) {
        self.launch_failures.insert(
            (package_name.to_string(), activity.unwrap_or("").to_string()),
            message.to_string(),
        );
    }

    pub fn fail_force_stop_app(&mut self, package_name: &str, message: &str) {
        self.force_stop_failures
            .insert(package_name.to_string(), message.to_string());
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
        let privileged = py_bool(is_app_private_path(path));
        self.commands.push(vec![
            "path_exists".to_string(),
            path.to_string(),
            privileged.to_string(),
        ]);
        self.remote_paths.contains(path)
    }

    fn path_is_dir(&mut self, path: &str) -> bool {
        let privileged = py_bool(is_app_private_path(path));
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

#[derive(Clone, Debug)]
struct SandboxRoots {
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    read_only_roots: Vec<PathBuf>,
}

impl SandboxRoots {
    fn ensure_read_allowed(&self, path: &Path) -> Result<(), StepFailure> {
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

    fn ensure_runtime_or_cache_write(&self, path: &Path) -> Result<(), StepFailure> {
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
        self.ensure_runtime_or_cache_write(dest_dir)?;
        let file =
            fs::File::open(archive_path).map_err(|error| StepFailure::new(error.to_string()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|_| StepFailure::new("File is not a zip file".to_string()))?;
        let mut entries = Vec::new();
        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .map_err(|error| StepFailure::new(error.to_string()))?;
            let entry_name = file.name().to_string();
            let relative = archive_relative_components(Path::new(&entry_name))?;
            let target = normalize_path(&dest_dir.join(relative));
            if !target.starts_with(&normalize_path(dest_dir)) {
                return Err(StepFailure::new(format!(
                    "unsafe archive entry rejected: {entry_name}"
                )));
            }
            self.ensure_runtime_or_cache_write(&target)?;
            entries.push((entry_name, target, file.is_dir()));
        }
        fs::create_dir_all(dest_dir).map_err(|error| StepFailure::new(error.to_string()))?;
        for (index, (entry_name, target, is_dir)) in entries.into_iter().enumerate() {
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
        if !normalized.starts_with(&normalize_path(&self.fake_device_root)) {
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
}

impl ExecutionState {
    fn from_plan(plan: &ExecutionPlan) -> Self {
        Self {
            steps: HashMap::new(),
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
    fn as_python_value(&self) -> &'static str {
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
struct ResolvedArtifact {
    local_path: String,
    filename: String,
    cache_hit: bool,
}

#[derive(Debug)]
struct StepFailure {
    message: String,
    outputs: OrderedMap<RuntimeValue>,
}

impl StepFailure {
    fn new(message: String) -> Self {
        Self {
            message,
            outputs: OrderedMap::new(),
        }
    }

    fn skipped() -> Self {
        Self::new("__emuchef_internal_skipped__".to_string())
    }
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
    StepRunRecord {
        step_id: step.id.clone(),
        status,
        message,
        outputs,
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
                value: json!(artifact.status.as_python_value()),
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

pub(crate) fn artifact_local_filename(artifact_id: &str, url: &str, cache: &str) -> String {
    let filename = artifact_filename(artifact_id, url);
    let hash_input = if cache == "default" {
        url.to_string()
    } else {
        format!("{artifact_id}{url}")
    };
    let digest = Sha256::digest(hash_input.as_bytes());
    let digest_hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{digest_hex}-{filename}")
}

fn artifact_filename(artifact_id: &str, url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path_with_query = if after_scheme.starts_with('/') {
        after_scheme
    } else {
        after_scheme
            .find('/')
            .map(|index| &after_scheme[index..])
            .unwrap_or("")
    };
    let path = path_with_query.split(['?', '#']).next().unwrap_or_default();
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .map(percent_decode)
        .unwrap_or_else(|| {
            format!(
                "{}.bin",
                artifact_id.rsplit('/').next().unwrap_or(artifact_id)
            )
        })
}

fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let path = if rest.starts_with('/') {
        rest
    } else {
        rest.find('/').map(|index| &rest[index..])?
    };
    Some(PathBuf::from(percent_decode(path)))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                output.push(hex);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).to_string()
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
    let mut relative = PathBuf::new();
    for component in path.components() {
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
            "symlink paths are not supported in Phase 6P sandbox operations: {}",
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

fn is_app_private_path(path: &str) -> bool {
    path.starts_with("/data/user/") || path.starts_with("/data/data/")
}

fn py_bool(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

fn python_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
    }
}

fn python_value_to_string(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => py_bool(*value).to_string(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => py_bool(*value).to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("{value:?}").replace('"', "'"),
        other => other.to_string(),
    }
}
