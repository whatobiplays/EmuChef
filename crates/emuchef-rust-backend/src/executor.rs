//! Internal safe executor foundations for fixture-scoped dry-run parity.
//!
//! Python remains the executor reference. This module intentionally mirrors only
//! the Python `ExecutionRunResult` shape and selected dry-run behaviors needed by
//! Phase 6O tests. It does not expose protocol, CLI, Tauri, subprocess, ADB,
//! network, archive, or real filesystem side-effect implementations.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::{json, Value};

use crate::model::OrderedMap;
use crate::planner::{
    ExecutionParamValue, ExecutionPlan, ExecutionStep, ExecutionStepCondition, RuntimeValue,
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
        let mut state = ExecutionState::default();
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
            other => Err(StepFailure::new(format!(
                "Unsupported executor step type in Rust Phase 6O skeleton: {other}"
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
}

impl DryRunExecutorAdapters {
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
}

#[derive(Debug, Default)]
pub struct FakeDryRunDevice {
    installed_packages: HashSet<String>,
    remote_paths: HashSet<String>,
    remote_dirs: HashSet<String>,
    commands: Vec<Vec<String>>,
    run_plan_failures: HashMap<Vec<String>, String>,
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
}

#[derive(Debug, Default)]
struct ExecutionState {
    steps: HashMap<String, StepRuntimeState>,
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

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => py_bool(*value).to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("{value:?}").replace('"', "'"),
        other => other.to_string(),
    }
}
