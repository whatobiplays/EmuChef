//! Read-only product operations used by the end-user device workflow.
//!
//! These DTOs are intentionally sidecar-internal. They may contain exact ADB
//! serials and catalog roots because the trusted Tauri backend consumes them;
//! the React-facing bridge must project separate path- and serial-free DTOs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::catalog_source::CatalogSnapshot;
use crate::device_probe::{
    AdbDeviceProbe, AdbProbeConfig, CommandOutput, CommandRunner, DetectedDeviceFacts, DeviceProbe,
    DeviceProbeError, ProcessCommandRunner,
};
use crate::errors::ApiError;
use crate::executor::adb::{probe_root, ProcessAdbCommandExecutor};

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdbInventoryEntry {
    serial: String,
    state: &'static str,
    model: Option<String>,
    transport_id: Option<String>,
}

const PASSIVE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn list_adb_devices(adb_path: &str) -> Result<Value, ApiError> {
    list_adb_devices_with_runner(adb_path, &ProcessCommandRunner)
}

fn list_adb_devices_with_runner(
    adb_path: &str,
    runner: &impl CommandRunner,
) -> Result<Value, ApiError> {
    let argv = vec![
        adb_path.to_string(),
        "devices".to_string(),
        "-l".to_string(),
    ];
    let output = runner
        .run_bounded(&argv, PASSIVE_PROBE_TIMEOUT)
        .map_err(|_| {
            ApiError::command_failed(
                "ADB device inventory could not be started.",
                json!({ "reason": "adb_inventory_unavailable" }),
            )
        })?;
    if output.status_code != Some(0) {
        return Err(ApiError::command_failed(
            "ADB device inventory failed.",
            json!({ "reason": "adb_inventory_failed" }),
        ));
    }
    let entries = parse_adb_devices(&output.stdout);
    Ok(json!({
        "state": if entries.is_empty() { "no_devices" } else { "devices" },
        "devices": entries.into_iter().map(|entry| json!({
            "serial": entry.serial,
            "state": entry.state,
            "model": entry.model,
            "transportId": entry.transport_id,
        })).collect::<Vec<_>>(),
    }))
}

fn parse_adb_devices(stdout: &str) -> Vec<AdbInventoryEntry> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("List of devices attached")
                && !line.starts_with('*')
        })
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?.trim();
            let raw_state = fields.next()?.trim();
            if serial.is_empty() {
                return None;
            }
            let state = match raw_state {
                "device" => "available",
                "unauthorized" => "unauthorized",
                "offline" => "offline",
                _ => return None,
            };
            let model = fields
                .clone()
                .find_map(|field| field.strip_prefix("model:"))
                .filter(|value| !value.is_empty())
                .map(|value| value.replace('_', " "));
            let transport_id = fields
                .find_map(|field| field.strip_prefix("transport_id:"))
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            Some(AdbInventoryEntry {
                serial: serial.to_string(),
                state,
                model,
                transport_id,
            })
        })
        .collect()
}

pub(crate) fn probe_device(adb_path: &str, serial: &str) -> Result<Value, ApiError> {
    let probe = AdbDeviceProbe {
        config: AdbProbeConfig {
            adb_path: adb_path.to_string(),
            serial: Some(serial.to_string()),
        },
        runner: ProcessCommandRunner,
    };
    let facts = probe.detect().map_err(|_| {
        ApiError::command_failed(
            "Selected device properties could not be read.",
            json!({ "reason": "adb_probe_failed" }),
        )
    })?;
    serde_json::to_value(facts).map_err(|_| {
        ApiError::command_failed(
            "Selected device properties could not be represented.",
            json!({ "reason": "adb_probe_serialization_failed" }),
        )
    })
}

/// Perform one bounded, read-only qualification pass for the requested serial.
/// Exact serials remain in the trusted sidecar response and are replaced at the
/// Tauri IPC boundary.
pub(crate) fn qualify_device(adb_path: &str, serial: &str) -> Result<Value, ApiError> {
    qualify_device_with_runner(adb_path, serial, &ProcessCommandRunner)
}

/// Run the sole privileged capability probe. The exact ADB command is owned by
/// the executor boundary; this function only serializes its sanitized outcome.
pub(crate) fn check_root(adb_path: &str, serial: &str) -> Result<Value, ApiError> {
    let mut executor = ProcessAdbCommandExecutor;
    Ok(probe_root(&mut executor, adb_path, serial, Duration::from_secs(30)).status_json())
}

fn qualify_device_with_runner(
    adb_path: &str,
    requested_serial: &str,
    runner: &impl CommandRunner,
) -> Result<Value, ApiError> {
    let inventory = list_adb_devices_with_runner(adb_path, runner)?;
    let devices = inventory
        .get("devices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if devices.is_empty() {
        return Ok(json!({ "state": "no_device" }));
    }
    if devices.len() != 1 {
        return Ok(json!({ "state": "multiple_devices" }));
    }

    let device = &devices[0];
    let serial = device
        .get("serial")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::command_failed(
                "ADB device inventory was incomplete.",
                json!({ "reason": "adb_inventory_invalid" }),
            )
        })?;
    if serial != requested_serial {
        return Ok(json!({ "state": "no_device" }));
    }
    match device.get("state").and_then(Value::as_str) {
        Some("unauthorized") => Ok(json!({
            "state": "unauthorized",
            "serial": serial,
        })),
        Some("offline") => Ok(json!({
            "state": "offline",
            "serial": serial,
        })),
        Some("available") => {
            let probe = AdbDeviceProbe {
                config: AdbProbeConfig {
                    adb_path: adb_path.to_string(),
                    serial: Some(serial.to_string()),
                },
                runner,
            };
            let facts = probe.detect_bounded(PASSIVE_PROBE_TIMEOUT).map_err(|_| {
                ApiError::command_failed(
                    "Connected device qualification facts could not be read.",
                    json!({ "reason": "adb_qualification_probe_failed" }),
                )
            })?;
            let storage = probe_storage(adb_path, serial, runner);
            let package_manager = probe_manager(
                adb_path,
                serial,
                runner,
                &["cmd", "package", "path", "android"],
                &["pm", "path", "android"],
                manager_kind::PACKAGE,
            );
            let activity_manager = probe_manager(
                adb_path,
                serial,
                runner,
                &["cmd", "activity", "get-config"],
                &["am", "get-config"],
                manager_kind::ACTIVITY,
            );
            Ok(json!({
                "state": "online",
                "serial": serial,
                "androidMajor": facts.android_version,
                "androidApiLevel": facts.android_api_level,
                "abi": facts.abis.first(),
                "storage": storage,
                "packageManager": package_manager,
                "activityManager": activity_manager,
            }))
        }
        _ => Err(ApiError::command_failed(
            "ADB device inventory contained an unsupported state.",
            json!({ "reason": "adb_inventory_invalid" }),
        )),
    }
}

fn serial_shell_command(adb_path: &str, serial: &str, command: &[&str]) -> Vec<String> {
    let mut argv = vec![
        adb_path.to_string(),
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
    ];
    argv.extend(command.iter().map(|part| (*part).to_string()));
    argv
}

fn probe_storage(adb_path: &str, serial: &str, runner: &impl CommandRunner) -> &'static str {
    let argv = serial_shell_command(
        adb_path,
        serial,
        &[
            "sh",
            "-c",
            "test -d /sdcard && test -w /sdcard && df -k /sdcard",
        ],
    );
    match runner.run_bounded(&argv, PASSIVE_PROBE_TIMEOUT) {
        Ok(output) if output.status_code == Some(0) && storage_output_is_valid(&output.stdout) => {
            "available"
        }
        Ok(output)
            if output.status_code != Some(0)
                && output.stdout.trim().is_empty()
                && output.stderr.trim().is_empty() =>
        {
            "unsupported"
        }
        _ => "unknown",
    }
}

mod manager_kind {
    pub(super) const PACKAGE: u8 = 1;
    pub(super) const ACTIVITY: u8 = 2;
}

fn probe_manager(
    adb_path: &str,
    serial: &str,
    runner: &impl CommandRunner,
    primary: &[&str],
    fallback: &[&str],
    kind: u8,
) -> &'static str {
    let primary_argv = serial_shell_command(adb_path, serial, primary);
    let primary_result = runner.run_bounded(&primary_argv, PASSIVE_PROBE_TIMEOUT);
    if primary_result
        .as_ref()
        .is_ok_and(|output| cmd_interface_is_unavailable(output))
    {
        let fallback_argv = serial_shell_command(adb_path, serial, fallback);
        return primary_result_to_manager(
            runner.run_bounded(&fallback_argv, PASSIVE_PROBE_TIMEOUT),
            kind,
        );
    }
    primary_result_to_manager(primary_result, kind)
}

fn primary_result_to_manager(
    result: Result<CommandOutput, DeviceProbeError>,
    kind: u8,
) -> &'static str {
    let Ok(output) = result else {
        return "unknown";
    };
    if output.status_code == Some(0) {
        let valid = match kind {
            manager_kind::PACKAGE => output
                .stdout
                .lines()
                .any(|line| line.trim_start().starts_with("package:")),
            manager_kind::ACTIVITY => output
                .stdout
                .lines()
                .any(|line| line.trim_start().starts_with("config")),
            _ => false,
        };
        return if valid { "available" } else { "unknown" };
    }
    if output.stdout.trim().is_empty() && output.stderr.trim().is_empty() {
        "unsupported"
    } else {
        "unknown"
    }
}

fn cmd_interface_is_unavailable(output: &CommandOutput) -> bool {
    if output.status_code == Some(127) {
        return true;
    }
    let stderr = output.stderr.to_ascii_lowercase();
    stderr.contains("cmd: not found")
        || stderr.contains("cmd: inaccessible or not found")
        || stderr.contains("not found")
        || stderr.contains("unknown command")
        || stderr.contains("unsupported command")
        || stderr.contains("unrecognized option")
}

fn storage_output_is_valid(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        fields.len() >= 6 && fields.last() == Some(&"/sdcard")
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ProfileMatchYaml {
    #[serde(default)]
    manufacturer_contains: Vec<String>,
    #[serde(default)]
    brand_contains: Vec<String>,
    #[serde(default)]
    model_patterns: Vec<String>,
    android_version: Option<AndroidRangeYaml>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct AndroidRangeYaml {
    min: Option<i64>,
    max: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct ProfileYaml {
    id: String,
    name: String,
    description: Option<String>,
    #[serde(rename = "match", default)]
    match_: ProfileMatchYaml,
    #[serde(default)]
    metadata: Value,
}

#[derive(Clone, Debug, Deserialize)]
struct PlanYaml {
    id: String,
    name: String,
    description: Option<String>,
    device_profile_ref: String,
}

#[derive(Clone, Debug, Default)]
struct MatchEvidence {
    matched: usize,
    declared: usize,
    missing: usize,
    conflicts: usize,
    family_matches: usize,
    family_conflicts: usize,
    android_conflict: bool,
    reasons: Vec<String>,
}

pub(crate) fn match_device(
    snapshot: &CatalogSnapshot,
    facts: &DetectedDeviceFacts,
) -> Result<Value, ApiError> {
    let profiles: Vec<ProfileYaml> = load_yaml_directory(snapshot.root(), "device_profiles")?;
    let plans: Vec<PlanYaml> = load_yaml_directory(snapshot.root(), "device_plans")?;
    let mut plans_by_profile = BTreeMap::<String, Vec<&PlanYaml>>::new();
    for plan in &plans {
        plans_by_profile
            .entry(plan.device_profile_ref.clone())
            .or_default()
            .push(plan);
    }

    let mut candidates = Vec::new();
    let mut safe_generic_plans = Vec::new();
    for profile in profiles {
        let evidence = evaluate_profile(facts, &profile.match_);
        let generic = profile_is_generic(&profile);
        let safe_generic = generic
            && evidence.family_matches > 0
            && evidence.family_conflicts == 0
            && !evidence.android_conflict;
        let profile_plans = plans_by_profile
            .get(&profile.id)
            .cloned()
            .unwrap_or_default();

        if safe_generic {
            safe_generic_plans.extend(profile_plans.iter().map(|plan| {
                json!({
                    "planId": plan.id,
                    "name": plan.name,
                    "description": plan.description,
                    "profileId": profile.id,
                    "profileName": profile.name,
                    "reasons": evidence.reasons,
                    "requiresExplicitChoice": true,
                })
            }));
            continue;
        }
        if generic || evidence.conflicts > 0 || evidence.matched == 0 {
            continue;
        }
        let confidence = if evidence.missing == 0 && evidence.matched == evidence.declared {
            "exact"
        } else if evidence.matched >= 2 {
            "high"
        } else {
            "low"
        };
        candidates.extend(profile_plans.iter().map(|plan| {
            json!({
                "planId": plan.id,
                "name": plan.name,
                "description": plan.description,
                "profileId": profile.id,
                "profileName": profile.name,
                "profileDescription": profile.description,
                "confidence": confidence,
                "reasons": evidence.reasons,
            })
        }));
    }

    candidates.sort_by(|left, right| {
        confidence_rank(right["confidence"].as_str().unwrap_or("none"))
            .cmp(&confidence_rank(
                left["confidence"].as_str().unwrap_or("none"),
            ))
            .then_with(|| left["planId"].as_str().cmp(&right["planId"].as_str()))
    });
    safe_generic_plans
        .sort_by(|left, right| left["planId"].as_str().cmp(&right["planId"].as_str()));
    let confidence = candidates
        .first()
        .and_then(|candidate| candidate["confidence"].as_str())
        .unwrap_or("none");
    let top_count = candidates
        .iter()
        .take_while(|candidate| candidate["confidence"] == confidence)
        .count();
    let recommended_plan_id = if matches!(confidence, "exact" | "high") && top_count == 1 {
        candidates[0]["planId"].as_str()
    } else {
        None
    };
    let blank_setup_plans = candidates
        .iter()
        .chain(safe_generic_plans.iter())
        .map(|plan| {
            let plan_name = plan
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("approved setup");
            json!({
                "planId": plan.get("planId"),
                "name": format!("Start from scratch with {plan_name}"),
                "description": format!("Use the {plan_name} device profile and choose recipes manually."),
                "profileId": plan.get("profileId"),
                "profileName": plan.get("profileName"),
                "confidence": plan.get("confidence"),
                "reasons": plan.get("reasons"),
                "requiresExplicitChoice": true,
                "selectionMode": "blank",
            })
        })
        .collect::<Vec<_>>();
    let blocked = candidates.is_empty() && safe_generic_plans.is_empty();
    Ok(json!({
        "confidence": confidence,
        "recommendedPlanId": recommended_plan_id,
        "requiresExplicitChoice": recommended_plan_id.is_none() && !blocked,
        "candidates": candidates,
        "safeGenericPlans": safe_generic_plans,
        "blankSetupPlans": blank_setup_plans,
        "blocked": blocked,
        "blockReason": blocked.then_some("No backend-approved device plan is safe for the detected device."),
    }))
}

fn load_yaml_directory<T: for<'de> Deserialize<'de>>(
    root: &Path,
    directory: &str,
) -> Result<Vec<T>, ApiError> {
    let mut paths = fs::read_dir(root.join(directory))
        .map_err(|_| catalog_match_load_error(directory))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_yaml(path))
        .collect::<Vec<PathBuf>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(path).map_err(|_| catalog_match_load_error(directory))?;
            serde_yaml::from_slice(&bytes).map_err(|_| catalog_match_load_error(directory))
        })
        .collect()
}

fn catalog_match_load_error(directory: &str) -> ApiError {
    ApiError::load_failed(
        "Catalog matching data is unavailable.",
        json!({ "reason": "catalog_match_data_invalid", "directory": directory }),
    )
}

fn is_yaml(path: &Path) -> bool {
    path.is_file()
        && matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        )
}

fn profile_is_generic(profile: &ProfileYaml) -> bool {
    profile
        .metadata
        .get("safe_generic")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || profile.id.split('.').any(|part| part == "generic")
}

fn evaluate_profile(facts: &DetectedDeviceFacts, criteria: &ProfileMatchYaml) -> MatchEvidence {
    let mut result = MatchEvidence::default();
    evaluate_contains(
        "manufacturer",
        facts.manufacturer.as_deref(),
        &criteria.manufacturer_contains,
        true,
        &mut result,
    );
    evaluate_contains(
        "brand",
        facts.brand.as_deref(),
        &criteria.brand_contains,
        true,
        &mut result,
    );
    evaluate_patterns(
        facts.model.as_deref(),
        &criteria.model_patterns,
        &mut result,
    );
    if let Some(range) = &criteria.android_version {
        result.declared += 1;
        match facts.android_version {
            None => {
                result.missing += 1;
                result
                    .reasons
                    .push("Android version was unavailable.".to_string());
            }
            Some(version)
                if range.min.is_some_and(|minimum| version < minimum)
                    || range.max.is_some_and(|maximum| version > maximum) =>
            {
                result.conflicts += 1;
                result.android_conflict = true;
                result
                    .reasons
                    .push("Android version is outside the supported range.".to_string());
            }
            Some(_) => {
                result.matched += 1;
                result
                    .reasons
                    .push("Android version is supported.".to_string());
            }
        }
    }
    result
}

fn evaluate_contains(
    label: &str,
    actual: Option<&str>,
    expected: &[String],
    family: bool,
    result: &mut MatchEvidence,
) {
    if expected.is_empty() {
        return;
    }
    result.declared += 1;
    let Some(actual) = actual else {
        result.missing += 1;
        result
            .reasons
            .push(format!("Device {label} was unavailable."));
        return;
    };
    let actual = actual.to_lowercase();
    if expected
        .iter()
        .any(|value| actual.contains(&value.to_lowercase()))
    {
        result.matched += 1;
        if family {
            result.family_matches += 1;
        }
        result.reasons.push(format!("Device {label} matched."));
    } else {
        result.conflicts += 1;
        if family {
            result.family_conflicts += 1;
        }
        result
            .reasons
            .push(format!("Device {label} did not match."));
    }
}

fn evaluate_patterns(actual: Option<&str>, patterns: &[String], result: &mut MatchEvidence) {
    if patterns.is_empty() {
        return;
    }
    result.declared += 1;
    let Some(actual) = actual else {
        result.missing += 1;
        result
            .reasons
            .push("Device model was unavailable.".to_string());
        return;
    };
    if patterns.iter().any(|pattern| {
        regex::Regex::new(pattern)
            .map(|regex| regex.is_match(actual))
            .unwrap_or(false)
    }) {
        result.matched += 1;
        result.reasons.push("Device model matched.".to_string());
    } else {
        result.conflicts += 1;
        result
            .reasons
            .push("Device model did not match.".to_string());
    }
}

fn confidence_rank(confidence: &str) -> u8 {
    match confidence {
        "exact" => 3,
        "high" => 2,
        "low" => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::catalog_source::CatalogSnapshot;
    use crate::device_probe::{CommandOutput, DeviceProbeError};

    struct FakeRunner {
        calls: RefCell<Vec<Vec<String>>>,
        output: Result<CommandOutput, DeviceProbeError>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
            self.calls.borrow_mut().push(argv.to_vec());
            self.output.clone()
        }
    }

    #[test]
    fn inventory_parses_supported_states_and_models() {
        let runner = FakeRunner {
            calls: RefCell::new(Vec::new()),
            output: Ok(CommandOutput {
                status_code: Some(0),
                stdout: "List of devices attached\nabc device model:Pocket_S_mini transport_id:1\ndef unauthorized usb:1\nghi offline\n".to_string(),
                stderr: String::new(),
            }),
        };
        let result = list_adb_devices_with_runner("/managed/adb", &runner).unwrap();
        assert_eq!(result["devices"][0]["state"], "available");
        assert_eq!(result["devices"][0]["model"], "Pocket S mini");
        assert_eq!(result["devices"][1]["state"], "unauthorized");
        assert_eq!(result["devices"][2]["state"], "offline");
        assert_eq!(
            runner.calls.into_inner(),
            vec![vec!["/managed/adb", "devices", "-l"]]
        );
    }

    #[test]
    fn inventory_failure_is_stable_and_redacted() {
        let runner = FakeRunner {
            calls: RefCell::new(Vec::new()),
            output: Ok(CommandOutput {
                status_code: Some(1),
                stdout: String::new(),
                stderr: "secret host output".to_string(),
            }),
        };
        let error = list_adb_devices_with_runner("/private/adb", &runner).unwrap_err();
        assert_eq!(error.details["reason"], "adb_inventory_failed");
        assert!(!error.message.contains("/private"));
        assert!(!error.message.contains("secret"));
    }

    #[test]
    fn inventory_distinguishes_no_devices_and_multiple_available_devices() {
        let empty = FakeRunner {
            calls: RefCell::new(Vec::new()),
            output: Ok(CommandOutput {
                status_code: Some(0),
                stdout: "List of devices attached\n\n".to_string(),
                stderr: String::new(),
            }),
        };
        assert_eq!(
            list_adb_devices_with_runner("/managed/adb", &empty).unwrap()["state"],
            "no_devices"
        );

        let multiple = FakeRunner {
            calls: RefCell::new(Vec::new()),
            output: Ok(CommandOutput {
                status_code: Some(0),
                stdout: "List of devices attached\none device model:One\ntwo device model:Two\n"
                    .to_string(),
                stderr: String::new(),
            }),
        };
        let result = list_adb_devices_with_runner("/managed/adb", &multiple).unwrap();
        assert_eq!(result["devices"].as_array().unwrap().len(), 2);
        assert!(result["devices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|device| device["state"] == "available"));
    }

    #[test]
    fn qualification_returns_inventory_states_without_probing() {
        for (inventory, expected) in [
            ("List of devices attached\n\n", "no_device"),
            (
                "List of devices attached\none unauthorized\n",
                "unauthorized",
            ),
            ("List of devices attached\none offline\n", "offline"),
            (
                "List of devices attached\none device\ntwo device\n",
                "multiple_devices",
            ),
        ] {
            let runner = FakeRunner {
                calls: RefCell::new(Vec::new()),
                output: Ok(CommandOutput {
                    status_code: Some(0),
                    stdout: inventory.to_string(),
                    stderr: String::new(),
                }),
            };
            let result = qualify_device_with_runner("/managed/adb", "one", &runner).unwrap();
            assert_eq!(result["state"], expected);
            assert_eq!(runner.calls.borrow().len(), 1);
        }
    }

    struct SequenceRunner {
        calls: RefCell<Vec<Vec<String>>>,
        outputs: RefCell<Vec<CommandOutput>>,
    }

    impl CommandRunner for SequenceRunner {
        fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
            self.calls.borrow_mut().push(argv.to_vec());
            Ok(self.outputs.borrow_mut().remove(0))
        }
    }

    struct ScriptedRunner {
        calls: RefCell<Vec<Vec<String>>>,
        outputs: RefCell<Vec<Result<CommandOutput, DeviceProbeError>>>,
    }

    impl ScriptedRunner {
        fn new(outputs: Vec<Result<CommandOutput, DeviceProbeError>>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                outputs: RefCell::new(outputs),
            }
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
            self.calls.borrow_mut().push(argv.to_vec());
            self.outputs.borrow_mut().remove(0)
        }
    }

    #[test]
    fn qualification_probes_only_one_online_device() {
        let runner = SequenceRunner {
            calls: RefCell::new(Vec::new()),
            outputs: RefCell::new(vec![
                CommandOutput {
                    status_code: Some(0),
                    stdout: "List of devices attached\nsecret-serial device model:Pocket\n"
                        .to_string(),
                    stderr: String::new(),
                },
                CommandOutput {
                    status_code: Some(0),
                    stdout: "[ro.build.version.release]: [14]\n[ro.build.version.sdk]: [34]\n[ro.product.cpu.abilist]: [arm64-v8a,armeabi-v7a]\n".to_string(),
                    stderr: String::new(),
                },
                CommandOutput {
                    status_code: Some(0),
                    stdout: "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/vold 100 10 90 10% /sdcard\n".to_string(),
                    stderr: String::new(),
                },
                CommandOutput {
                    status_code: Some(0),
                    stdout: "package:/system/framework/framework-res.apk\n".to_string(),
                    stderr: String::new(),
                },
                CommandOutput {
                    status_code: Some(0),
                    stdout: "config\n".to_string(),
                    stderr: String::new(),
                },
            ]),
        };
        let result = qualify_device_with_runner("/managed/adb", "secret-serial", &runner).unwrap();
        assert_eq!(result["state"], "online");
        assert_eq!(result["serial"], "secret-serial");
        assert_eq!(result["androidMajor"], 14);
        assert_eq!(result["androidApiLevel"], 34);
        assert_eq!(result["abi"], "arm64-v8a");
        assert_eq!(runner.calls.borrow().len(), 5);
        assert_eq!(
            runner.calls.borrow()[1],
            vec!["/managed/adb", "-s", "secret-serial", "shell", "getprop"]
        );
    }

    #[test]
    fn qualification_reports_required_capabilities_without_privileged_commands() {
        let runner = ScriptedRunner::new(vec![
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "List of devices attached\nsecret-serial device model:Pocket\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "[ro.build.version.release]: [14]\n[ro.build.version.sdk]: [34]\n[ro.product.cpu.abilist]: [arm64-v8a,armeabi-v7a]\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/vold 100 10 90 10% /sdcard\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "package:/system/framework/framework-res.apk\n".to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "config\n  mcc 310\n".to_string(),
                stderr: String::new(),
            }),
        ]);

        let result = qualify_device_with_runner("/managed/adb", "secret-serial", &runner).unwrap();

        assert_eq!(result["storage"], "available");
        assert_eq!(result["packageManager"], "available");
        assert_eq!(result["activityManager"], "available");
        assert_eq!(runner.calls.borrow().len(), 5);
        assert!(runner
            .calls
            .borrow()
            .iter()
            .all(|argv| !argv.iter().any(|part| part == "su")));
        assert_eq!(
            runner.calls.borrow()[2],
            vec![
                "/managed/adb",
                "-s",
                "secret-serial",
                "shell",
                "sh",
                "-c",
                "test -d /sdcard && test -w /sdcard && df -k /sdcard",
            ]
        );
        assert_eq!(
            runner.calls.borrow()[3],
            vec![
                "/managed/adb",
                "-s",
                "secret-serial",
                "shell",
                "cmd",
                "package",
                "path",
                "android",
            ]
        );
        assert_eq!(
            runner.calls.borrow()[4],
            vec![
                "/managed/adb",
                "-s",
                "secret-serial",
                "shell",
                "cmd",
                "activity",
                "get-config",
            ]
        );
    }

    #[test]
    fn qualification_falls_back_only_for_confirmed_unavailable_cmd_interfaces() {
        let runner = ScriptedRunner::new(vec![
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "List of devices attached\nsecret-serial device model:Pocket\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "[ro.build.version.release]: [14]\n[ro.build.version.sdk]: [34]\n[ro.product.cpu.abilist]: [arm64-v8a]\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/vold 100 10 90 10% /sdcard\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(127),
                stdout: String::new(),
                stderr: "cmd: not found\n".to_string(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "package:/system/framework/framework-res.apk\n".to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(127),
                stdout: String::new(),
                stderr: "Unknown command: get-config\n".to_string(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "config\n  mcc 310\n".to_string(),
                stderr: String::new(),
            }),
        ]);

        let result = qualify_device_with_runner("/managed/adb", "secret-serial", &runner).unwrap();

        assert_eq!(result["packageManager"], "available");
        assert_eq!(result["activityManager"], "available");
        assert_eq!(runner.calls.borrow().len(), 7);
        assert_eq!(runner.calls.borrow()[4][4..], ["pm", "path", "android"]);
        assert_eq!(runner.calls.borrow()[6][4..], ["am", "get-config"]);
    }

    #[test]
    fn qualification_does_not_fallback_for_timeout_or_malformed_cmd_output() {
        let runner = ScriptedRunner::new(vec![
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "List of devices attached\nsecret-serial device model:Pocket\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "[ro.build.version.release]: [14]\n[ro.build.version.sdk]: [34]\n[ro.product.cpu.abilist]: [arm64-v8a]\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/vold 100 10 90 10% /sdcard\n"
                    .to_string(),
                stderr: String::new(),
            }),
            Err(DeviceProbeError::TimedOut),
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            }),
        ]);

        let result = qualify_device_with_runner("/managed/adb", "secret-serial", &runner).unwrap();

        assert_eq!(result["packageManager"], "unknown");
        assert_eq!(result["activityManager"], "unknown");
        assert_eq!(runner.calls.borrow().len(), 5);
        assert!(runner.calls.borrow()[3][4..]
            .starts_with(["cmd".to_string(), "package".to_string()].as_slice()));
        assert!(runner.calls.borrow()[4][4..]
            .starts_with(["cmd".to_string(), "activity".to_string()].as_slice()));
    }

    #[test]
    fn qualification_probe_failure_is_stable_and_redacted() {
        let runner = SequenceRunner {
            calls: RefCell::new(Vec::new()),
            outputs: RefCell::new(vec![
                CommandOutput {
                    status_code: Some(0),
                    stdout: "List of devices attached\nprivate-serial device\n".to_string(),
                    stderr: String::new(),
                },
                CommandOutput {
                    status_code: Some(1),
                    stdout: String::new(),
                    stderr: "private device failure".to_string(),
                },
            ]),
        };
        let error =
            qualify_device_with_runner("/private/adb", "private-serial", &runner).unwrap_err();
        assert_eq!(error.details["reason"], "adb_qualification_probe_failed");
        assert!(!error.message.contains("private-serial"));
        assert!(!error.message.contains("/private"));
    }

    #[test]
    fn qualification_does_not_probe_a_different_serial() {
        let runner = FakeRunner {
            calls: RefCell::new(Vec::new()),
            output: Ok(CommandOutput {
                status_code: Some(0),
                stdout: "List of devices attached\nother-serial device model:Other\n".to_string(),
                stderr: String::new(),
            }),
        };

        let result =
            qualify_device_with_runner("/managed/adb", "selected-serial", &runner).unwrap();

        assert_eq!(result["state"], "no_device");
        assert_eq!(runner.calls.borrow().len(), 1);
    }

    #[test]
    fn exact_and_generic_evidence_are_conservative() {
        let facts = DetectedDeviceFacts {
            manufacturer: Some("AYANEO".to_string()),
            brand: Some("AYANEO".to_string()),
            model: Some("Pocket S mini".to_string()),
            android_version: Some(13),
            ..DetectedDeviceFacts::default()
        };
        let criteria = ProfileMatchYaml {
            manufacturer_contains: vec!["AYANEO".to_string()],
            brand_contains: vec!["AYANEO".to_string()],
            model_patterns: vec!["Pocket S mini".to_string()],
            android_version: Some(AndroidRangeYaml {
                min: Some(13),
                max: None,
            }),
        };
        let evidence = evaluate_profile(&facts, &criteria);
        assert_eq!(evidence.matched, evidence.declared);
        assert_eq!(evidence.conflicts, 0);
        assert_eq!(evidence.family_matches, 2);
    }

    #[test]
    fn matching_reports_high_low_none_and_backend_approved_generic_choices() {
        let high = match_fixture(
            "manufacturer_contains: [Acme]\nbrand_contains: [Acme]\nmodel_patterns: ['^Model$']",
            DetectedDeviceFacts {
                manufacturer: Some("Acme".to_string()),
                brand: Some("Acme".to_string()),
                ..DetectedDeviceFacts::default()
            },
            false,
        );
        assert_eq!(high["confidence"], "high");
        assert_eq!(high["recommendedPlanId"], "plan.test");
        assert_eq!(high["blankSetupPlans"][0]["planId"], "plan.test");
        assert_eq!(high["blankSetupPlans"][0]["selectionMode"], "blank");

        let low = match_fixture(
            "manufacturer_contains: [Acme]\nbrand_contains: [Acme]\nmodel_patterns: ['^Model$']",
            DetectedDeviceFacts {
                manufacturer: Some("Acme".to_string()),
                ..DetectedDeviceFacts::default()
            },
            false,
        );
        assert_eq!(low["confidence"], "low");
        assert!(low["recommendedPlanId"].is_null());
        assert_eq!(low["requiresExplicitChoice"], true);

        let none = match_fixture(
            "manufacturer_contains: [Acme]",
            DetectedDeviceFacts {
                manufacturer: Some("Other".to_string()),
                ..DetectedDeviceFacts::default()
            },
            false,
        );
        assert_eq!(none["confidence"], "none");
        assert_eq!(none["blocked"], true);

        let generic = match_fixture(
            "manufacturer_contains: [Acme]",
            DetectedDeviceFacts {
                manufacturer: Some("Acme".to_string()),
                ..DetectedDeviceFacts::default()
            },
            true,
        );
        assert_eq!(generic["confidence"], "none");
        assert_eq!(generic["blocked"], false);
        assert!(generic["recommendedPlanId"].is_null());
        assert_eq!(
            generic["safeGenericPlans"][0]["requiresExplicitChoice"],
            true
        );
    }

    fn match_fixture(match_yaml: &str, facts: DetectedDeviceFacts, safe_generic: bool) -> Value {
        let temp = tempfile::tempdir().unwrap();
        for directory in ["apps", "recipes", "device_profiles", "device_plans"] {
            fs::create_dir(temp.path().join(directory)).unwrap();
        }
        let profile_id = if safe_generic {
            "profile.generic"
        } else {
            "profile.test"
        };
        fs::write(
            temp.path().join("device_profiles/profile.yaml"),
            format!(
                "id: {profile_id}\nname: Test profile\ndescription: Test\nmatch:\n{}\nmetadata:\n  safe_generic: {}\n",
                match_yaml
                    .lines()
                    .map(|line| format!("  {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                safe_generic
            ),
        )
        .unwrap();
        fs::write(
            temp.path().join("device_plans/plan.yaml"),
            format!(
                "id: plan.test\nname: Test plan\ndescription: Test\ndevice_profile_ref: {profile_id}\n"
            ),
        )
        .unwrap();
        let snapshot = CatalogSnapshot::legacy_local(temp.path()).unwrap();
        match_device(&snapshot, &facts).unwrap()
    }
}
