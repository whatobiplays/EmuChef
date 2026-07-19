//! Read-only product operations used by the end-user device workflow.
//!
//! These DTOs are intentionally sidecar-internal. They may contain exact ADB
//! serials and catalog roots because the trusted Tauri backend consumes them;
//! the React-facing bridge must project separate path- and serial-free DTOs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::catalog_source::CatalogSnapshot;
use crate::device_probe::{
    AdbDeviceProbe, AdbProbeConfig, CommandRunner, DetectedDeviceFacts, DeviceProbe,
    ProcessCommandRunner,
};
use crate::errors::ApiError;

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdbInventoryEntry {
    serial: String,
    state: &'static str,
    model: Option<String>,
}

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
    let output = runner.run(&argv).map_err(|_| {
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
                .find_map(|field| field.strip_prefix("model:"))
                .filter(|value| !value.is_empty())
                .map(|value| value.replace('_', " "));
            Some(AdbInventoryEntry {
                serial: serial.to_string(),
                state,
                model,
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
