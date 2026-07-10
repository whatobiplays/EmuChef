//! Shared Rust planning implementation plus the development-only shadow command.
//!
//! The product `emuchef plan` command and the `emuchef-plan-shadow` reference
//! binary call the same planning function. The shadow binary remains a manual
//! JSON inspection surface and is not used by product runtime or packaging.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::device_probe::{
    AdbDeviceProbe, AdbProbeConfig, CommandRunner, DetectedDeviceFacts, DeviceProbe,
    DeviceProbeError, ProcessCommandRunner,
};
use crate::model::OrderedMap;
use crate::planner::{plan_execution, DeviceContext, PlannerInput, PlanningResult, PlanningStatus};
use crate::planner_device_plan::plan_from_authored_device_plan_with_detected_facts;
use crate::ProcessOutput;

const USAGE: &str = "usage: emuchef-plan-shadow --authored-root <path> --device-plan <id> [--detected-facts-json <path> | --probe-adb-getprop [--adb-path <path>] [--serial <serial>]] [--manufacturer <value>] [--model <value>] [--android-version <integer>] [--device-tag <value>]... [--bind <recipe_ref>/<input_id>=<value>]...\n";

#[derive(Debug, PartialEq)]
struct ShadowConfig {
    authored_root: PathBuf,
    device_plan: String,
    detected_facts_source: DetectedFactsSource,
    input_bindings: OrderedMap<Value>,
    explicit_context: ExplicitDeviceContext,
}

#[derive(Debug, PartialEq)]
enum DetectedFactsSource {
    None,
    FixtureJson(PathBuf),
    LiveAdbGetprop {
        adb_path: String,
        serial: Option<String>,
    },
}

impl DetectedFactsSource {
    fn uses_detected_facts(&self) -> bool {
        !matches!(self, DetectedFactsSource::None)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ExplicitDeviceContext {
    manufacturer: Option<String>,
    model: Option<String>,
    android_version: Option<i64>,
    device_tags: Vec<String>,
}

/// Run the dev-only planner shadow command with already-split process args.
///
/// Planner results are emitted as pretty JSON on stdout, including planner
/// validation failures such as missing required bindings. Argument and authored
/// load failures are process-level errors and write stable text to stderr.
pub fn run(args: &[String]) -> ProcessOutput {
    run_with_adb_runner(args, &ProcessCommandRunner)
}

pub(crate) fn run_with_adb_runner<R: CommandRunner>(
    args: &[String],
    adb_runner: &R,
) -> ProcessOutput {
    match planning_result_with_adb_runner(args, adb_runner) {
        Ok(result) => emit_planning_result(result),
        Err(output) => output,
    }
}

pub(crate) fn planning_result_with_adb_runner<R: CommandRunner>(
    args: &[String],
    adb_runner: &R,
) -> Result<PlanningResult, ProcessOutput> {
    let config = match parse_args(args) {
        Ok(config) => config,
        Err(ShadowArgError::Help) => {
            return Err(ProcessOutput {
                exit_code: 0,
                stdout: USAGE.to_string(),
                stderr: String::new(),
            });
        }
        Err(ShadowArgError::Usage(message)) => {
            return Err(ProcessOutput {
                exit_code: 2,
                stdout: String::new(),
                stderr: format!("{message}\n{USAGE}"),
            });
        }
    };

    let ShadowConfig {
        authored_root,
        device_plan,
        detected_facts_source,
        input_bindings,
        explicit_context,
    } = config;
    let plan_id = format!("plan.shadow.{device_plan}.001");
    let uses_detected_facts = detected_facts_source.uses_detected_facts();
    let mut result = match detected_facts_source {
        DetectedFactsSource::FixtureJson(path) => {
            let detected_facts = match load_detected_facts_fixture(&path) {
                Ok(facts) => facts,
                Err(error) => return Err(error.into_process_output()),
            };
            match plan_from_authored_device_plan_with_detected_facts(
                &authored_root,
                &device_plan,
                plan_id,
                input_bindings,
                &detected_facts,
            ) {
                Ok(result) => result,
                Err(error) => {
                    return Err(ProcessOutput {
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: format!("Error: {}: {error}\n", error.code()),
                    });
                }
            }
        }
        DetectedFactsSource::LiveAdbGetprop { adb_path, serial } => {
            let probe = AdbDeviceProbe {
                config: AdbProbeConfig { adb_path, serial },
                runner: adb_runner,
            };
            let detected_facts = match probe.detect() {
                Ok(facts) => facts,
                Err(error) => return Err(adb_probe_error_output(error)),
            };
            match plan_from_authored_device_plan_with_detected_facts(
                &authored_root,
                &device_plan,
                plan_id,
                input_bindings,
                &detected_facts,
            ) {
                Ok(result) => result,
                Err(error) => {
                    return Err(ProcessOutput {
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: format!("Error: {}: {error}\n", error.code()),
                    });
                }
            }
        }
        DetectedFactsSource::None => {
            let mut input = match PlannerInput::from_authored_device_plan(
                &authored_root,
                &device_plan,
                plan_id,
                input_bindings,
            ) {
                Ok(input) => input,
                Err(error) => {
                    return Err(ProcessOutput {
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: format!("Error: {}: {error}\n", error.code()),
                    });
                }
            };
            apply_explicit_device_context(&mut input.device_context, &explicit_context);
            plan_execution(input)
        }
    };
    if uses_detected_facts {
        apply_explicit_device_context_to_result(&mut result, &explicit_context);
    }

    Ok(result)
}

fn adb_probe_error_output(error: DeviceProbeError) -> ProcessOutput {
    let code = match error {
        DeviceProbeError::Unavailable { .. } => "adb_probe_unavailable",
        DeviceProbeError::Failed { .. } | DeviceProbeError::InvalidOutput { .. } => {
            "adb_probe_failed"
        }
    };
    ProcessOutput {
        exit_code: 1,
        stdout: String::new(),
        stderr: format!("Error: {code}\n"),
    }
}

fn emit_planning_result(result: PlanningResult) -> ProcessOutput {
    let exit_code = if matches!(result.status, PlanningStatus::Success) {
        0
    } else {
        1
    };
    ProcessOutput {
        exit_code,
        stdout: format!(
            "{}\n",
            serde_json::to_string_pretty(&result)
                .expect("serializing shadow planning result should not fail")
        ),
        stderr: String::new(),
    }
}

#[derive(Debug, PartialEq)]
enum ShadowArgError {
    Help,
    Usage(String),
}

fn parse_args(args: &[String]) -> Result<ShadowConfig, ShadowArgError> {
    let mut authored_root: Option<PathBuf> = None;
    let mut device_plan: Option<String> = None;
    let mut detected_facts_json: Option<PathBuf> = None;
    let mut probe_adb_getprop = false;
    let mut adb_path: Option<String> = None;
    let mut serial: Option<String> = None;
    let mut raw_bindings: Vec<String> = Vec::new();
    let mut explicit_context = ExplicitDeviceContext::default();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--authored-root" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--authored-root requires one argument");
                };
                authored_root = Some(PathBuf::from(value));
            }
            "--device-plan" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--device-plan requires one argument");
                };
                device_plan = Some(value.clone());
            }
            "--detected-facts-json" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--detected-facts-json requires one argument");
                };
                detected_facts_json = Some(PathBuf::from(value));
            }
            "--probe-adb-getprop" => {
                probe_adb_getprop = true;
            }
            "--adb-path" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--adb-path requires one argument");
                };
                adb_path = Some(value.clone());
            }
            "--serial" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--serial requires one argument");
                };
                serial = Some(value.clone());
            }
            "--manufacturer" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--manufacturer requires one argument");
                };
                explicit_context.manufacturer = Some(value.clone());
            }
            "--model" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--model requires one argument");
                };
                explicit_context.model = Some(value.clone());
            }
            "--android-version" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--android-version requires one argument");
                };
                explicit_context.android_version = Some(parse_android_version(value)?);
            }
            "--device-tag" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--device-tag requires one argument");
                };
                explicit_context.device_tags.push(value.clone());
            }
            "--bind" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--bind requires one argument");
                };
                raw_bindings.push(value.clone());
            }
            "-h" | "--help" => return Err(ShadowArgError::Help),
            value if value.starts_with('-') => {
                return usage_error(&format!("unrecognized argument: {value}"));
            }
            value => {
                return usage_error(&format!("unrecognized argument: {value}"));
            }
        }
        index += 1;
    }

    let missing = [
        ("--authored-root", authored_root.is_none()),
        ("--device-plan", device_plan.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, is_missing)| is_missing.then_some(name))
    .collect::<Vec<_>>();
    if !missing.is_empty() {
        return usage_error(&format!(
            "the following arguments are required: {}",
            missing.join(", ")
        ));
    }
    if detected_facts_json.is_some() && probe_adb_getprop {
        return usage_error("--probe-adb-getprop cannot be combined with --detected-facts-json");
    }
    if !probe_adb_getprop && adb_path.is_some() {
        return usage_error("--adb-path is only valid with --probe-adb-getprop");
    }
    if !probe_adb_getprop && serial.is_some() {
        return usage_error("--serial is only valid with --probe-adb-getprop");
    }
    let detected_facts_source = if let Some(path) = detected_facts_json {
        DetectedFactsSource::FixtureJson(path)
    } else if probe_adb_getprop {
        DetectedFactsSource::LiveAdbGetprop {
            adb_path: adb_path.unwrap_or_else(|| "adb".to_string()),
            serial,
        }
    } else {
        DetectedFactsSource::None
    };

    Ok(ShadowConfig {
        authored_root: authored_root.expect("missing authored_root was checked"),
        device_plan: device_plan.expect("missing device_plan was checked"),
        detected_facts_source,
        input_bindings: parse_bindings(&raw_bindings)?,
        explicit_context,
    })
}

fn parse_android_version(value: &str) -> Result<i64, ShadowArgError> {
    match value.parse::<i64>() {
        Ok(version) if version >= 0 => Ok(version),
        _ => usage_error("--android-version must be a non-negative integer"),
    }
}

fn apply_explicit_device_context(
    device_context: &mut DeviceContext,
    explicit_context: &ExplicitDeviceContext,
) {
    if let Some(manufacturer) = &explicit_context.manufacturer {
        device_context.manufacturer = manufacturer.clone();
    }
    if let Some(model) = &explicit_context.model {
        device_context.model = model.clone();
    }
    if let Some(android_version) = explicit_context.android_version {
        device_context.android_version = android_version;
    }
    if !explicit_context.device_tags.is_empty() {
        device_context.device_tags = explicit_context.device_tags.clone();
    }
}

fn apply_explicit_device_context_to_result(
    result: &mut PlanningResult,
    explicit_context: &ExplicitDeviceContext,
) {
    if let Some(execution_plan) = &mut result.execution_plan {
        apply_explicit_device_context(&mut execution_plan.device_context, explicit_context);
    }
}

#[derive(Debug, PartialEq)]
enum DetectedFactsFixtureError {
    ReadFailed { file_name: String },
    Invalid { file_name: String, message: String },
}

impl DetectedFactsFixtureError {
    fn into_process_output(self) -> ProcessOutput {
        match self {
            DetectedFactsFixtureError::ReadFailed { file_name } => ProcessOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!(
                    "Error: detected_facts_fixture_read_failed: could not read detected facts fixture '{file_name}'.\n"
                ),
            },
            DetectedFactsFixtureError::Invalid { file_name, message } => ProcessOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!(
                    "Error: detected_facts_fixture_invalid: fixture '{file_name}' is not valid DetectedDeviceFacts JSON: {message}.\n"
                ),
            },
        }
    }
}

/// Load already-detected facts from a local fixture file.
///
/// This is a dev/test harness input, not a probe adapter. Error text reports
/// stable fixture names and classifications without depending on host-specific
/// absolute paths or OS error strings.
fn load_detected_facts_fixture(
    path: &Path,
) -> Result<DetectedDeviceFacts, DetectedFactsFixtureError> {
    let file_name = stable_file_name(path);
    let text = fs::read_to_string(path).map_err(|_| DetectedFactsFixtureError::ReadFailed {
        file_name: file_name.clone(),
    })?;
    serde_json::from_str::<DetectedDeviceFacts>(&text).map_err(|error| {
        DetectedFactsFixtureError::Invalid {
            file_name,
            message: error.to_string(),
        }
    })
}

fn stable_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<fixture>")
        .to_string()
}

fn parse_bindings(raw_bindings: &[String]) -> Result<OrderedMap<Value>, ShadowArgError> {
    let mut grouped: OrderedMap<Vec<String>> = OrderedMap::new();
    for raw_binding in raw_bindings {
        let (binding_ref, raw_value) = parse_binding(raw_binding)?;
        grouped
            .entry(binding_ref)
            .or_default()
            .push(raw_value.to_string());
    }

    Ok(grouped
        .into_iter()
        .map(|(binding_ref, raw_values)| {
            let value = if raw_values.len() == 1 {
                Value::String(
                    raw_values
                        .into_iter()
                        .next()
                        .expect("single binding value should exist"),
                )
            } else {
                Value::Array(raw_values.into_iter().map(Value::String).collect())
            };
            (binding_ref, value)
        })
        .collect())
}

fn parse_binding(raw_binding: &str) -> Result<(String, &str), ShadowArgError> {
    let Some((binding_ref, raw_value)) = raw_binding.split_once('=') else {
        return invalid_bind(raw_binding);
    };
    let Some((recipe_ref, input_id)) = binding_ref.split_once('/') else {
        return invalid_bind(raw_binding);
    };
    if recipe_ref.is_empty() || input_id.is_empty() || input_id.contains('/') {
        return invalid_bind(raw_binding);
    }
    Ok((binding_ref.to_string(), raw_value))
}

fn usage_error<T>(message: &str) -> Result<T, ShadowArgError> {
    Err(ShadowArgError::Usage(format!(
        "emuchef-plan-shadow: error: {message}"
    )))
}

fn invalid_bind<T>(raw_binding: &str) -> Result<T, ShadowArgError> {
    usage_error(&format!(
        "Invalid --bind value: '{raw_binding}'. Expected <recipe_ref>/<input_id>=<value>."
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use crate::device_probe::{CommandOutput, CommandRunner, DeviceProbeError};

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn repo_authored_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root should resolve from crate manifest dir")
            .join("authored")
    }

    fn shadow_args(extra_args: &[&str]) -> Vec<String> {
        let mut args = vec![
            "--authored-root".to_string(),
            repo_authored_root().to_string_lossy().into_owned(),
            "--device-plan".to_string(),
            "ayaneo.pocket_s_mini.base".to_string(),
        ];
        args.extend(extra_args.iter().map(|value| (*value).to_string()));
        args
    }

    fn stdout_json(output: &ProcessOutput) -> serde_json::Value {
        serde_json::from_str(&output.stdout)
            .unwrap_or_else(|error| panic!("stdout should be JSON: {error}\n{}", output.stdout))
    }

    fn matching_getprop_output() -> &'static str {
        "\
[ro.product.manufacturer]: [AYANEO]
[ro.product.brand]: [AYANEO]
[ro.product.model]: [AYANEO Pocket S mini]
[ro.build.version.release]: [13]
[ro.build.version.sdk]: [33]
"
    }

    fn mismatching_getprop_output() -> &'static str {
        "\
[ro.product.manufacturer]: [Valve]
[ro.product.brand]: [Valve]
[ro.product.model]: [Steam Deck]
[ro.build.version.release]: [12]
[ro.build.version.sdk]: [32]
"
    }

    #[derive(Debug)]
    struct FakeAdbRunner {
        calls: RefCell<Vec<Vec<String>>>,
        result: Result<CommandOutput, DeviceProbeError>,
    }

    impl FakeAdbRunner {
        fn completed(status_code: Option<i32>, stdout: &str, stderr: &str) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                result: Ok(CommandOutput {
                    status_code,
                    stdout: stdout.to_string(),
                    stderr: stderr.to_string(),
                }),
            }
        }

        fn unavailable(message: &str) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                result: Err(DeviceProbeError::Unavailable {
                    message: message.to_string(),
                }),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl CommandRunner for FakeAdbRunner {
        fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
            self.calls.borrow_mut().push(argv.to_vec());
            self.result.clone()
        }
    }

    #[test]
    fn adb_probe_shadow_default_builds_adb_shell_getprop() {
        let runner = FakeAdbRunner::completed(Some(0), matching_getprop_output(), "");

        let output = run_with_adb_runner(&shadow_args(&["--probe-adb-getprop"]), &runner);

        assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
        assert_eq!(
            runner.calls(),
            vec![vec![
                "adb".to_string(),
                "shell".to_string(),
                "getprop".to_string(),
            ]]
        );
    }

    #[test]
    fn adb_probe_shadow_preserves_supplied_adb_path() {
        let runner = FakeAdbRunner::completed(Some(0), matching_getprop_output(), "");

        let output = run_with_adb_runner(
            &shadow_args(&[
                "--probe-adb-getprop",
                "--adb-path",
                "/opt/android platform tools/adb",
            ]),
            &runner,
        );

        assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
        assert_eq!(runner.calls()[0][0], "/opt/android platform tools/adb");
    }

    #[test]
    fn adb_probe_shadow_forwards_serial_to_command_and_detected_facts() {
        let runner = FakeAdbRunner::completed(Some(0), mismatching_getprop_output(), "");

        let output = run_with_adb_runner(
            &shadow_args(&["--probe-adb-getprop", "--serial", "SERIAL123"]),
            &runner,
        );

        assert_eq!(output.exit_code, 1);
        assert_eq!(
            runner.calls()[0],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "SERIAL123".to_string(),
                "shell".to_string(),
                "getprop".to_string(),
            ]
        );
        let result = stdout_json(&output);
        assert_eq!(
            result["warnings"][0]["details"]["serial"],
            json!("SERIAL123")
        );
    }

    #[test]
    fn adb_probe_shadow_success_routes_detected_facts_through_planning_result() {
        let runner = FakeAdbRunner::completed(Some(0), matching_getprop_output(), "");

        let output = run_with_adb_runner(&shadow_args(&["--probe-adb-getprop"]), &runner);

        assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
        let result = stdout_json(&output);
        assert_eq!(result["status"], "success");
        assert_eq!(
            result["execution_plan"]["device_context"],
            json!({
                "manufacturer": "AYANEO",
                "model": "AYANEO Pocket S mini",
                "android_version": 13,
                "android_api_level": 33,
                "device_tags": ["handheld_android", "brand_ayaneo"],
            })
        );
    }

    #[test]
    fn adb_probe_shadow_matching_facts_do_not_emit_device_profile_mismatch() {
        let runner = FakeAdbRunner::completed(Some(0), matching_getprop_output(), "");

        let output = run_with_adb_runner(&shadow_args(&["--probe-adb-getprop"]), &runner);

        assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
        let result = stdout_json(&output);
        assert_eq!(result["warnings"], json!([]));
    }

    #[test]
    fn adb_probe_shadow_mismatching_facts_emit_one_device_profile_mismatch() {
        let runner = FakeAdbRunner::completed(Some(0), mismatching_getprop_output(), "");

        let output = run_with_adb_runner(&shadow_args(&["--probe-adb-getprop"]), &runner);

        assert_eq!(output.exit_code, 1);
        let result = stdout_json(&output);
        let warnings = result["warnings"]
            .as_array()
            .expect("warnings should be an array");
        assert_eq!(result["status"], "warning");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0]["code"], "device_profile_mismatch");
    }

    #[test]
    fn adb_probe_shadow_explicit_context_overrides_emitted_context_after_live_facts() {
        let runner = FakeAdbRunner::completed(Some(0), mismatching_getprop_output(), "");

        let output = run_with_adb_runner(
            &shadow_args(&[
                "--probe-adb-getprop",
                "--manufacturer",
                "AYANEO",
                "--model",
                "AYANEO Pocket S mini",
                "--android-version",
                "13",
                "--device-tag",
                "explicit_handheld",
            ]),
            &runner,
        );

        assert_eq!(output.exit_code, 1);
        let result = stdout_json(&output);
        assert_eq!(
            result["execution_plan"]["device_context"],
            json!({
                "manufacturer": "AYANEO",
                "model": "AYANEO Pocket S mini",
                "android_version": 13,
                "android_api_level": 32,
                "device_tags": ["explicit_handheld"],
            })
        );
    }

    #[test]
    fn adb_probe_shadow_mismatch_warning_uses_live_facts_not_explicit_overrides() {
        let runner = FakeAdbRunner::completed(Some(0), mismatching_getprop_output(), "");

        let output = run_with_adb_runner(
            &shadow_args(&[
                "--probe-adb-getprop",
                "--manufacturer",
                "AYANEO",
                "--model",
                "AYANEO Pocket S mini",
                "--android-version",
                "13",
            ]),
            &runner,
        );

        assert_eq!(output.exit_code, 1);
        let result = stdout_json(&output);
        assert_eq!(result["warnings"][0]["code"], "device_profile_mismatch");
        assert_eq!(result["warnings"][0]["details"]["manufacturer"], "Valve");
        assert_eq!(result["warnings"][0]["details"]["model"], "Steam Deck");
    }

    #[test]
    fn adb_probe_shadow_rejects_fixture_json_source_combination() {
        let runner = FakeAdbRunner::completed(Some(0), matching_getprop_output(), "");

        let output = run_with_adb_runner(
            &shadow_args(&["--detected-facts-json", "facts.json", "--probe-adb-getprop"]),
            &runner,
        );

        assert_eq!(output.exit_code, 2);
        assert_eq!(output.stdout, "");
        assert!(output
            .stderr
            .contains("--probe-adb-getprop cannot be combined with --detected-facts-json"));
        assert!(output.stderr.contains("usage: emuchef-plan-shadow"));
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn adb_probe_shadow_rejects_adb_path_without_probe_mode() {
        let runner = FakeAdbRunner::completed(Some(0), matching_getprop_output(), "");

        let output = run_with_adb_runner(&shadow_args(&["--adb-path", "adb"]), &runner);

        assert_eq!(output.exit_code, 2);
        assert_eq!(output.stdout, "");
        assert!(output
            .stderr
            .contains("--adb-path is only valid with --probe-adb-getprop"));
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn adb_probe_shadow_rejects_serial_without_probe_mode() {
        let runner = FakeAdbRunner::completed(Some(0), matching_getprop_output(), "");

        let output = run_with_adb_runner(&shadow_args(&["--serial", "SERIAL123"]), &runner);

        assert_eq!(output.exit_code, 2);
        assert_eq!(output.stdout, "");
        assert!(output
            .stderr
            .contains("--serial is only valid with --probe-adb-getprop"));
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn adb_probe_shadow_launch_failure_returns_stable_process_error() {
        let runner = FakeAdbRunner::unavailable("raw OS error /tmp/adb SERIAL123");

        let output = run_with_adb_runner(
            &shadow_args(&[
                "--probe-adb-getprop",
                "--adb-path",
                "/tmp/adb",
                "--serial",
                "SERIAL123",
            ]),
            &runner,
        );

        assert_eq!(output.exit_code, 1);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, "Error: adb_probe_unavailable\n");
        assert!(!output.stderr.contains("raw OS error"));
        assert!(!output.stderr.contains("/tmp/adb"));
        assert!(!output.stderr.contains("SERIAL123"));
        assert!(!output.stderr.contains("usage:"));
    }

    #[test]
    fn adb_probe_shadow_non_zero_status_returns_stable_process_error() {
        let runner =
            FakeAdbRunner::completed(Some(1), "", "raw adb stderr for SERIAL123 from /tmp/adb");

        let output = run_with_adb_runner(
            &shadow_args(&[
                "--probe-adb-getprop",
                "--adb-path",
                "/tmp/adb",
                "--serial",
                "SERIAL123",
            ]),
            &runner,
        );

        assert_eq!(output.exit_code, 1);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, "Error: adb_probe_failed\n");
        assert!(!output.stderr.contains("raw adb stderr"));
        assert!(!output.stderr.contains("/tmp/adb"));
        assert!(!output.stderr.contains("SERIAL123"));
        assert!(!output.stderr.contains("usage:"));
    }

    #[test]
    fn adb_probe_shadow_omitted_probe_mode_preserves_existing_behavior_without_runner_call() {
        let runner = FakeAdbRunner::unavailable("runner must not be called");

        let output = run_with_adb_runner(&shadow_args(&[]), &runner);

        assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
        assert_eq!(output.stderr, "");
        assert_eq!(stdout_json(&output)["status"], "success");
        assert!(
            runner.calls().is_empty(),
            "no-probe path must not execute the ADB runner"
        );
    }

    #[test]
    fn parse_args_accepts_explicit_device_context_args_in_order() {
        let config = parse_args(&strings(&[
            "--authored-root",
            "authored",
            "--device-plan",
            "ayaneo.pocket_s_mini.base",
            "--manufacturer",
            "AYANEO",
            "--model",
            "Pocket S Mini",
            "--android-version",
            "13",
            "--device-tag",
            "handheld",
            "--device-tag",
            "landscape",
        ]))
        .expect("explicit device context args should parse");

        assert_eq!(
            config.explicit_context.manufacturer.as_deref(),
            Some("AYANEO")
        );
        assert_eq!(
            config.explicit_context.model.as_deref(),
            Some("Pocket S Mini")
        );
        assert_eq!(config.explicit_context.android_version, Some(13));
        assert_eq!(
            config.explicit_context.device_tags,
            vec!["handheld".to_string(), "landscape".to_string()]
        );
    }

    #[test]
    fn parse_args_default_has_no_explicit_device_context() {
        let config = parse_args(&strings(&[
            "--authored-root",
            "authored",
            "--device-plan",
            "ayaneo.pocket_s_mini.base",
        ]))
        .expect("minimal args should parse");

        assert_eq!(config.explicit_context, ExplicitDeviceContext::default());
    }

    #[test]
    fn parse_args_rejects_invalid_android_versions() {
        for raw_version in ["not-an-int", "-1"] {
            let error = parse_args(&strings(&[
                "--authored-root",
                "authored",
                "--device-plan",
                "ayaneo.pocket_s_mini.base",
                "--android-version",
                raw_version,
            ]))
            .expect_err("invalid android version should fail");

            assert!(
                matches!(error, ShadowArgError::Usage(message) if message.contains("--android-version must be a non-negative integer"))
            );
        }
    }

    #[test]
    fn parse_args_rejects_missing_explicit_context_values() {
        for flag in [
            "--manufacturer",
            "--model",
            "--android-version",
            "--device-tag",
        ] {
            let error = parse_args(&strings(&[
                "--authored-root",
                "authored",
                "--device-plan",
                "ayaneo.pocket_s_mini.base",
                flag,
            ]))
            .expect_err("missing value should fail");

            assert!(
                matches!(error, ShadowArgError::Usage(message) if message.contains(&format!("{flag} requires one argument")))
            );
        }
    }

    #[test]
    fn parse_bindings_matches_python_repeated_ref_grouping() {
        let bindings = parse_bindings(&strings(&[
            "recipe.one/input=/tmp/one",
            "recipe.two/input=/tmp/two",
            "recipe.one/input=/tmp/three",
        ]))
        .expect("bindings should parse");

        assert_eq!(
            bindings["recipe.one/input"],
            json!(["/tmp/one", "/tmp/three"])
        );
        assert_eq!(bindings["recipe.two/input"], json!("/tmp/two"));
        assert_eq!(
            bindings.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["recipe.one/input", "recipe.two/input"]
        );
    }

    #[test]
    fn parse_bindings_rejects_malformed_refs() {
        for raw in [
            "missing-equals",
            "/input=/tmp/value",
            "recipe/=/tmp/value",
            "recipe/input/extra=/tmp/value",
        ] {
            let error = parse_bindings(&strings(&[raw])).expect_err("binding should fail");
            assert!(
                matches!(error, ShadowArgError::Usage(message) if message.contains("Invalid --bind value"))
            );
        }
    }
}
