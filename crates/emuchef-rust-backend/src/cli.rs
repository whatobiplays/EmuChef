use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use crate::executor::{
    ExecutionProgressEvent, ExecutionRunResult, ExecutorRunner, ProgressPhase, ProgressStatus,
    StepRunStatus,
};
use crate::model::OrderedMap;
use crate::planner::{
    DeviceContext, ExecutionParamValue, ExecutionPlan, ExecutionPlanSource, ExecutionStep,
    ExecutionStepCondition, ExecutionStepConstraints, RuntimeCapabilities,
};
use crate::{validation, yaml, ProcessOutput};

const CLI_COMMANDS: &[&str] = &["validate", "apply"];

pub(crate) fn is_cli_command(arg: &str) -> bool {
    CLI_COMMANDS.contains(&arg)
}

pub(crate) fn run(args: &[String]) -> ProcessOutput {
    match args.first().map(String::as_str) {
        Some("validate") => run_validate(&args[1..]),
        Some("apply") => run_apply(&args[1..]),
        _ => ProcessOutput {
            exit_code: 2,
            stdout: String::new(),
            stderr: "usage: emuchef {validate,apply} ...\n".to_string(),
        },
    }
}

fn run_validate(args: &[String]) -> ProcessOutput {
    let mut authored_root: Option<String> = None;
    let mut path: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--authored-root" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error(
                        validate_usage(),
                        "emuchef validate: error: argument --authored-root: expected one argument",
                    );
                };
                authored_root = Some(value.clone());
            }
            "--verbose" | "--debug" => {
                return ProcessOutput {
                    exit_code: 2,
                    stdout: String::new(),
                    stderr: format!(
                        "Error: Rust Phase 6S validate supports only non-verbose selected fixtures; {} is deferred.\n",
                        args[index]
                    ),
                };
            }
            "--adb" => {
                return ProcessOutput {
                    exit_code: 2,
                    stdout: String::new(),
                    stderr:
                        "Error: Rust Phase 6S validate does not resolve ADB; --adb is deferred.\n"
                            .to_string(),
                };
            }
            "-h" | "--help" => {
                return ProcessOutput {
                    exit_code: 0,
                    stdout: format!("{}\n", validate_usage()),
                    stderr: String::new(),
                };
            }
            value if value.starts_with('-') => {
                return usage_error(
                    validate_usage(),
                    &format!("emuchef validate: error: unrecognized arguments: {value}"),
                );
            }
            value => {
                if path.is_some() {
                    return usage_error(
                        validate_usage(),
                        &format!("emuchef validate: error: unrecognized arguments: {value}"),
                    );
                }
                path = Some(value.to_string());
            }
        }
        index += 1;
    }

    let Some(target) = path else {
        return ProcessOutput {
            exit_code: 2,
            stdout: String::new(),
            stderr:
                "Error: Rust Phase 6S validate requires an explicit recipe path; catalog/default validation is deferred.\n"
                    .to_string(),
        };
    };
    let target_path = PathBuf::from(&target);
    let authored_root_path = authored_root.as_deref().map(PathBuf::from);
    if target_path.is_dir() {
        return ProcessOutput {
            exit_code: 2,
            stdout: String::new(),
            stderr:
                "Error: Rust Phase 6S validate supports recipe files only; catalog validation is deferred.\n"
                    .to_string(),
        };
    }
    let result = validate_single_path(&target_path, authored_root_path.as_deref());
    let exit_code = if result.has_errors() { 1 } else { 0 };
    ProcessOutput {
        exit_code,
        stdout: format_validation_summary(&result),
        stderr: String::new(),
    }
}

fn run_apply(args: &[String]) -> ProcessOutput {
    let mut plan_file: Option<String> = None;
    let mut dry_run = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--plan-file" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error(
                        apply_usage(),
                        "emuchef apply: error: argument --plan-file: expected one argument",
                    );
                };
                plan_file = Some(value.clone());
            }
            "--dry-run" => dry_run = true,
            "--serial" => {
                index += 1;
                if args.get(index).is_none() {
                    return usage_error(
                        apply_usage(),
                        &format!(
                            "emuchef apply: error: argument {}: expected one argument",
                            args[index - 1]
                        ),
                    );
                }
            }
            "--adb" => {
                return ProcessOutput {
                    exit_code: 2,
                    stdout: String::new(),
                    stderr: "Error: Rust Phase 6S apply --dry-run does not resolve ADB; --adb is deferred.\n"
                        .to_string(),
                };
            }
            "--verbose" | "--debug" => {
                return ProcessOutput {
                    exit_code: 2,
                    stdout: String::new(),
                    stderr: format!(
                        "Error: Rust Phase 6S apply supports only non-verbose selected fixtures; {} is deferred.\n",
                        args[index]
                    ),
                };
            }
            "-h" | "--help" => {
                return ProcessOutput {
                    exit_code: 0,
                    stdout: format!("{}\n", apply_usage()),
                    stderr: String::new(),
                };
            }
            value => {
                return usage_error(
                    apply_usage(),
                    &format!("emuchef apply: error: unrecognized arguments: {value}"),
                );
            }
        }
        index += 1;
    }

    let Some(plan_file) = plan_file else {
        return usage_error(
            apply_usage(),
            "emuchef apply: error: the following arguments are required: --plan-file",
        );
    };
    if !dry_run {
        return ProcessOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "Error: Rust Phase 6S apply supports only --dry-run.\n".to_string(),
        };
    }

    let plan_path = PathBuf::from(&plan_file);
    let plan = match load_execution_plan_file(&plan_path) {
        Ok(plan) => plan,
        Err(CliError::IoNotFound { path }) => {
            return ProcessOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!(
                    "FileNotFoundError: [Errno 2] No such file or directory: '{}'\n",
                    path.display()
                ),
            };
        }
        Err(error) => {
            return ProcessOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!("Error: {error}\n"),
            };
        }
    };

    let mut stdout = String::new();
    let mut runner = ExecutorRunner::default();
    let result = runner.run_with_progress(&plan, |event| {
        stdout.push_str(&format_execution_progress_event(&event, true));
    });
    stdout.push_str(&format_execution_summary(&result, true));
    ProcessOutput {
        exit_code: if result.success { 0 } else { 1 },
        stdout,
        stderr: format_execution_errors(&result),
    }
}

#[derive(Debug)]
struct ValidationCliResult {
    validated_paths: Vec<String>,
    diagnostics: Vec<JsonValue>,
}

impl ValidationCliResult {
    fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic["severity"] == "error")
    }

    fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic["severity"] == "warning")
    }
}

fn validate_single_path(path: &Path, authored_root: Option<&Path>) -> ValidationCliResult {
    let result = validation::validate_recipe_path_result(path, authored_root);
    ValidationCliResult {
        validated_paths: vec![yaml::resolved_path_string(path)],
        diagnostics: result["diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    }
}

fn format_validation_summary(result: &ValidationCliResult) -> String {
    let status = if result.has_errors() {
        "error"
    } else if result.has_warnings() {
        "warning"
    } else {
        "success"
    };
    let mut lines = vec![
        format!("Validation status: {status}"),
        "Validated paths:".to_string(),
    ];
    lines.extend(bullet_lines(&result.validated_paths));
    let grouped = group_validation_issues(result);
    if !grouped.is_empty() {
        lines.push("Issues:".to_string());
        for (file_path, diagnostics) in grouped {
            lines.push(file_path);
            for diagnostic in diagnostics {
                lines.push(format!(
                    "  - {}: {}",
                    string_field(diagnostic, "code"),
                    string_field(diagnostic, "message")
                ));
                let field = string_field(diagnostic, "field");
                if !field.is_empty() {
                    lines.push(format!("    field: {field}"));
                }
            }
        }
    }
    lines.join("\n") + "\n"
}

fn group_validation_issues(result: &ValidationCliResult) -> BTreeMap<String, Vec<&JsonValue>> {
    let mut grouped = BTreeMap::<String, Vec<&JsonValue>>::new();
    for diagnostic in &result.diagnostics {
        let file = string_field(diagnostic, "file");
        grouped
            .entry(if file.is_empty() {
                "(unknown file)".to_string()
            } else {
                file
            })
            .or_default()
            .push(diagnostic);
    }
    grouped
}

fn bullet_lines(items: &[String]) -> Vec<String> {
    if items.is_empty() {
        vec!["- (none)".to_string()]
    } else {
        items.iter().map(|item| format!("- {item}")).collect()
    }
}

fn format_execution_progress_event(event: &ExecutionProgressEvent, dry_run: bool) -> String {
    if event.phase == ProgressPhase::Finished {
        let status = match event.status {
            Some(ProgressStatus::Skipped) => "skipped",
            Some(ProgressStatus::Blocked) => "blocked",
            Some(ProgressStatus::Succeeded) => "succeeded",
            Some(ProgressStatus::Failed) => "failed",
            None => "finished",
        };
        return format!(
            "[{}/{}] {}: {status}\n",
            event.step_index, event.total_steps, event.step_name
        );
    }

    let mut phase_label = match event.phase {
        ProgressPhase::CheckingSkipConditions => "checking skip conditions",
        ProgressPhase::Executing => "executing",
        ProgressPhase::Verifying => "verifying",
        ProgressPhase::Finished => unreachable!(),
    };
    if dry_run && event.phase == ProgressPhase::Executing {
        phase_label = "executing (dry-run)";
    }
    format!(
        "[{}/{}] {}: {phase_label}\n",
        event.step_index, event.total_steps, event.step_name
    )
}

fn format_execution_summary(result: &ExecutionRunResult, dry_run: bool) -> String {
    let prefix = if dry_run { "Dry run" } else { "Execution" };
    let succeeded = count_steps(result, StepRunStatus::Executed);
    let skipped = count_steps(result, StepRunStatus::Skipped);
    let blocked = count_steps(result, StepRunStatus::Blocked);
    let failed = count_steps(result, StepRunStatus::Failed);
    let not_run = result.total_steps.saturating_sub(result.steps.len());
    let mut lines = vec![
        format!(
            "{prefix}: {}",
            if result.success { "success" } else { "failed" }
        ),
        format!("- total: {}", result.total_steps),
        format!("- succeeded: {succeeded}"),
        format!("- skipped: {skipped}"),
        format!("- blocked: {blocked}"),
        format!("- failed: {failed}"),
        format!("- not run: {not_run}"),
    ];
    let permission_results = collect_permission_results(result);
    if !permission_results.is_empty() {
        let permission_executed = permission_results
            .iter()
            .filter(|record| string_field(record, "status") == "executed")
            .count();
        let permission_not_applicable = permission_results
            .iter()
            .filter(|record| string_field(record, "status") == "not_applicable")
            .count();
        let permission_failed = permission_results
            .iter()
            .filter(|record| string_field(record, "status") == "failed")
            .count();
        lines.extend([
            "Permission actions:".to_string(),
            format!("- executed: {permission_executed}"),
            format!("- not_applicable: {permission_not_applicable}"),
            format!("- failed: {permission_failed}"),
        ]);
        lines.extend(
            permission_results
                .iter()
                .map(|record| format!("- {}", format_permission_result(record))),
        );
    }
    lines.join("\n") + "\n"
}

fn count_steps(result: &ExecutionRunResult, status: StepRunStatus) -> usize {
    result
        .steps
        .iter()
        .filter(|record| record.status == status)
        .count()
}

fn collect_permission_results(result: &ExecutionRunResult) -> Vec<&JsonValue> {
    let mut records = Vec::new();
    for step in &result.steps {
        let Some(output) = step.outputs.get("permission_results") else {
            continue;
        };
        let Some(actions) = output.value.get("actions").and_then(JsonValue::as_array) else {
            continue;
        };
        records.extend(actions);
    }
    records
}

fn format_permission_result(record: &JsonValue) -> String {
    let kind = non_empty_or(string_field(record, "kind"), "permission");
    let package_name = string_field(record, "package_name");
    let permission = string_field(record, "permission");
    let op = string_field(record, "op");
    let action_name = if !permission.is_empty() {
        permission
    } else if !op.is_empty() {
        op
    } else {
        kind.clone()
    };
    let detail = format!("{kind} {package_name} {action_name}")
        .trim()
        .to_string();
    let provenance = format!(
        "{} -> {}:{}",
        string_field(record, "step_id"),
        string_field(record, "source_recipe_id"),
        string_field(record, "source_section")
    );
    let message = string_field(record, "message");
    if message.is_empty() {
        format!(
            "{}: {detail} ({provenance})",
            string_field(record, "status")
        )
    } else {
        format!(
            "{}: {detail} ({provenance}) - {message}",
            string_field(record, "status")
        )
    }
}

fn format_execution_errors(result: &ExecutionRunResult) -> String {
    let mut stderr = String::new();
    for record in &result.steps {
        if record.status == StepRunStatus::Failed {
            stderr.push_str(&format!(
                "ERROR emuchef.executor.runner: Step failed: {}\n",
                record.step_id
            ));
            if let Some(message) = &record.message {
                stderr.push_str(&format!("ValueError: {message}\n"));
            }
        }
    }
    stderr
}

#[derive(Debug)]
enum CliError {
    IoNotFound { path: PathBuf },
    Message(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::IoNotFound { path } => {
                write!(formatter, "File not found: {}", path.display())
            }
            CliError::Message(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CliError {}

fn load_execution_plan_file(path: &Path) -> Result<ExecutionPlan, CliError> {
    let text = fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CliError::IoNotFound {
                path: path.to_path_buf(),
            }
        } else {
            CliError::Message(error.to_string())
        }
    })?;
    let mut raw = serde_yaml::from_str::<YamlValue>(&text)
        .map_err(|error| CliError::Message(error.to_string()))?;
    let raw_mapping = as_mapping(
        &raw,
        "Execution plan file must contain a top-level mapping.",
    )?;
    if string_value(raw_mapping, "kind") == "planning_result" {
        raw = raw_mapping
            .get(&yaml_key("execution_plan"))
            .cloned()
            .ok_or_else(|| {
                CliError::Message("Planning result does not contain an execution_plan.".to_string())
            })?;
        as_mapping(&raw, "planning_result.execution_plan must be a mapping.")?;
    }
    let mapping = as_mapping(
        &raw,
        "Execution plan file must contain a top-level mapping.",
    )?;
    if string_value(mapping, "kind") != "execution_plan" {
        return Err(CliError::Message(format!(
            "Unsupported plan kind: {:?}",
            optional_string_value(mapping, "kind")
        )));
    }
    parse_execution_plan(mapping)
}

fn parse_execution_plan(data: &serde_yaml::Mapping) -> Result<ExecutionPlan, CliError> {
    let source = mapping_value(data, "source")?;
    let device_context = mapping_value(data, "device_context")?;
    let runtime_capabilities = mapping_value(data, "runtime_capabilities")?;
    reject_non_empty_sequence(
        data,
        "inputs",
        "Rust Phase 6S apply --dry-run supports only selected no-input plan fixtures.",
    )?;
    reject_non_empty_sequence(
        data,
        "artifacts",
        "Rust Phase 6S apply --dry-run supports only selected no-artifact plan fixtures.",
    )?;
    Ok(ExecutionPlan {
        id: required_string(data, "id")?,
        source: ExecutionPlanSource {
            device_profile_ref: required_string(source, "device_profile_ref")?,
            device_plan_ref: required_string(source, "device_plan_ref")?,
            selected_recipe_refs: string_list(source, "selected_recipe_refs")?,
            expanded_recipe_refs: string_list(source, "expanded_recipe_refs")?,
        },
        device_context: DeviceContext {
            manufacturer: required_string(device_context, "manufacturer")?,
            model: required_string(device_context, "model")?,
            android_version: required_i64(device_context, "android_version")?,
            android_api_level: optional_i64(device_context, "android_api_level")?,
            device_tags: string_list(device_context, "device_tags")?,
        },
        runtime_capabilities: RuntimeCapabilities {
            adb_available: required_bool(runtime_capabilities, "adb_available")?,
            apk_install: required_bool(runtime_capabilities, "apk_install")?,
            shared_storage_write: required_bool(runtime_capabilities, "shared_storage_write")?,
            app_launch: required_bool(runtime_capabilities, "app_launch")?,
            shell_command: required_bool(runtime_capabilities, "shell_command")?,
            package_remove_for_user: required_bool(
                runtime_capabilities,
                "package_remove_for_user",
            )?,
            root_shell: required_bool(runtime_capabilities, "root_shell")?,
            app_data_write: required_bool(runtime_capabilities, "app_data_write")?,
        },
        inputs: Vec::new(),
        artifacts: Vec::new(),
        steps: parse_steps(data)?,
        schema_version: required_i64(data, "schema_version")?,
        kind: "execution_plan",
    })
}

fn parse_steps(data: &serde_yaml::Mapping) -> Result<Vec<ExecutionStep>, CliError> {
    sequence_value(data, "steps")?
        .iter()
        .map(|item| {
            let mapping = as_mapping(item, "execution step must be a mapping")?;
            let constraints = optional_mapping_value(mapping, "constraints")?;
            Ok(ExecutionStep {
                id: required_string(mapping, "id")?,
                recipe_ref: required_string(mapping, "recipe_ref")?,
                type_name: required_string(mapping, "type")?,
                name: required_string(mapping, "name")?,
                dependencies: string_list(mapping, "dependencies")?,
                constraints: ExecutionStepConstraints {
                    capabilities: constraints
                        .map(|value| string_list(value, "capabilities"))
                        .transpose()?
                        .unwrap_or_default(),
                    conflicts_with: constraints
                        .map(|value| string_list(value, "conflicts_with"))
                        .transpose()?
                        .unwrap_or_default(),
                },
                params: parse_params(mapping)?,
                skip_if: parse_conditions(mapping, "skip_if")?,
                verify: parse_conditions(mapping, "verify")?,
            })
        })
        .collect()
}

fn reject_non_empty_sequence(
    mapping: &serde_yaml::Mapping,
    key: &str,
    message: &str,
) -> Result<(), CliError> {
    if !sequence_value(mapping, key)?.is_empty() {
        return Err(CliError::Message(message.to_string()));
    }
    Ok(())
}

fn parse_params(
    mapping: &serde_yaml::Mapping,
) -> Result<OrderedMap<ExecutionParamValue>, CliError> {
    let Some(params) = optional_mapping_value(mapping, "params")? else {
        return Ok(OrderedMap::new());
    };
    let mut parsed = OrderedMap::new();
    for (key, value) in params {
        let Some(key) = key.as_str() else {
            return Err(CliError::Message(
                "execution step param keys must be strings".to_string(),
            ));
        };
        parsed.insert(key.to_string(), parse_param_value(value)?);
    }
    Ok(parsed)
}

fn parse_param_value(value: &YamlValue) -> Result<ExecutionParamValue, CliError> {
    if let YamlValue::Mapping(mapping) = value {
        if mapping.len() == 1 {
            if let Some(ref_value) = mapping.get(&yaml_key("ref")).and_then(YamlValue::as_str) {
                return Ok(ExecutionParamValue::Ref {
                    ref_value: ref_value.to_string(),
                });
            }
            if let Some(value) = mapping.get(&yaml_key("value")) {
                return Ok(ExecutionParamValue::Literal {
                    value: yaml_to_json(value)?,
                });
            }
        }
    }
    Ok(ExecutionParamValue::Literal {
        value: yaml_to_json(value)?,
    })
}

fn parse_conditions(
    mapping: &serde_yaml::Mapping,
    key: &str,
) -> Result<Vec<ExecutionStepCondition>, CliError> {
    sequence_value(mapping, key)?
        .iter()
        .map(|item| {
            let condition = as_mapping(item, "execution condition must be a mapping")?;
            let mut params = OrderedMap::new();
            if let Some(raw_params) = optional_mapping_value(condition, "params")? {
                for (key, value) in raw_params {
                    let Some(key) = key.as_str() else {
                        return Err(CliError::Message(
                            "execution condition param keys must be strings".to_string(),
                        ));
                    };
                    params.insert(key.to_string(), yaml_to_json(value)?);
                }
            }
            Ok(ExecutionStepCondition {
                type_name: required_string(condition, "type")?,
                params,
            })
        })
        .collect()
}

fn yaml_to_json(value: &YamlValue) -> Result<JsonValue, CliError> {
    serde_json::to_value(value).map_err(|error| CliError::Message(error.to_string()))
}

fn as_mapping<'a>(
    value: &'a YamlValue,
    message: &str,
) -> Result<&'a serde_yaml::Mapping, CliError> {
    value
        .as_mapping()
        .ok_or_else(|| CliError::Message(message.to_string()))
}

fn mapping_value<'a>(
    mapping: &'a serde_yaml::Mapping,
    key: &str,
) -> Result<&'a serde_yaml::Mapping, CliError> {
    let value = mapping
        .get(&yaml_key(key))
        .ok_or_else(|| CliError::Message(format!("Execution plan missing {key}.")))?;
    as_mapping(value, &format!("Execution plan {key} must be a mapping."))
}

fn optional_mapping_value<'a>(
    mapping: &'a serde_yaml::Mapping,
    key: &str,
) -> Result<Option<&'a serde_yaml::Mapping>, CliError> {
    mapping
        .get(&yaml_key(key))
        .map(|value| as_mapping(value, &format!("Execution plan {key} must be a mapping.")))
        .transpose()
}

fn sequence_value<'a>(
    mapping: &'a serde_yaml::Mapping,
    key: &str,
) -> Result<&'a Vec<YamlValue>, CliError> {
    match mapping.get(&yaml_key(key)) {
        Some(YamlValue::Sequence(sequence)) => Ok(sequence),
        Some(_) => Err(CliError::Message(format!(
            "Execution plan {key} must be a list."
        ))),
        None => Ok(empty_sequence()),
    }
}

fn empty_sequence() -> &'static Vec<YamlValue> {
    static EMPTY: std::sync::OnceLock<Vec<YamlValue>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Vec::new)
}

fn string_list(mapping: &serde_yaml::Mapping, key: &str) -> Result<Vec<String>, CliError> {
    sequence_value(mapping, key)?
        .iter()
        .map(|item| {
            item.as_str().map(ToString::to_string).ok_or_else(|| {
                CliError::Message(format!("Execution plan {key} entries must be strings."))
            })
        })
        .collect()
}

fn required_string(mapping: &serde_yaml::Mapping, key: &str) -> Result<String, CliError> {
    mapping
        .get(&yaml_key(key))
        .and_then(YamlValue::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| CliError::Message(format!("Execution plan missing string {key}.")))
}

fn required_i64(mapping: &serde_yaml::Mapping, key: &str) -> Result<i64, CliError> {
    mapping
        .get(&yaml_key(key))
        .and_then(YamlValue::as_i64)
        .ok_or_else(|| CliError::Message(format!("Execution plan missing integer {key}.")))
}

fn optional_i64(mapping: &serde_yaml::Mapping, key: &str) -> Result<Option<i64>, CliError> {
    mapping
        .get(&yaml_key(key))
        .map(|value| {
            if value.is_null() {
                Ok(None)
            } else {
                value.as_i64().map(Some).ok_or_else(|| {
                    CliError::Message(format!("Execution plan {key} must be an integer."))
                })
            }
        })
        .unwrap_or(Ok(None))
}

fn required_bool(mapping: &serde_yaml::Mapping, key: &str) -> Result<bool, CliError> {
    mapping
        .get(&yaml_key(key))
        .and_then(YamlValue::as_bool)
        .ok_or_else(|| CliError::Message(format!("Execution plan missing boolean {key}.")))
}

fn string_value(mapping: &serde_yaml::Mapping, key: &str) -> String {
    optional_string_value(mapping, key).unwrap_or_default()
}

fn optional_string_value(mapping: &serde_yaml::Mapping, key: &str) -> Option<String> {
    mapping
        .get(&yaml_key(key))
        .and_then(YamlValue::as_str)
        .map(ToString::to_string)
}

fn yaml_key(key: &str) -> YamlValue {
    YamlValue::String(key.to_string())
}

fn string_field(value: &JsonValue, key: &str) -> String {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string()
}

fn non_empty_or(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn usage_error(usage: &str, message: &str) -> ProcessOutput {
    ProcessOutput {
        exit_code: 2,
        stdout: String::new(),
        stderr: format!("{usage}\n{message}\n"),
    }
}

fn validate_usage() -> &'static str {
    "usage: emuchef validate [-h] [--verbose] [--debug] [--adb ADB]\n                        [--authored-root AUTHORED_ROOT]\n                        [path]"
}

fn apply_usage() -> &'static str {
    "usage: emuchef apply [-h] [--verbose] [--debug] [--adb ADB]\n                     --plan-file PLAN_FILE [--serial SERIAL] [--dry-run]"
}
