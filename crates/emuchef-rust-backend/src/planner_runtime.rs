//! Product planning orchestration for authored device plans.
//!
//! This module connects the CLI's parsed inputs to device probing and the
//! planner. It contains no process-level argument parsing or alternate planner
//! backend selection.

use std::path::PathBuf;

use serde_json::Value;

use crate::device_probe::{
    AdbDeviceProbe, AdbProbeConfig, CommandRunner, DeviceProbe, DeviceProbeError,
};
use crate::model::OrderedMap;
use crate::planner::{plan_execution, DeviceContext, PlanningResult};
use crate::planner_device_plan::{
    add_detected_profile_mismatch_warning, load_device_plan_profile_match_criteria,
};
use crate::runtime_configuration::{self, ConfigurationContextRequest};
use crate::ProcessOutput;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ExplicitDeviceContext {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub android_version: Option<i64>,
    pub device_tags: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct PlanningRequest {
    pub authored_root: PathBuf,
    pub configuration_root: Option<PathBuf>,
    pub user_configuration: Option<String>,
    pub device_plan: Option<String>,
    pub selected_recipes: Option<Vec<String>>,
    pub explicit_input_bindings: OrderedMap<Value>,
    pub explicit_context: ExplicitDeviceContext,
    pub adb_probe: Option<AdbProbeConfig>,
}

pub(crate) fn plan_with_adb_runner<R: CommandRunner>(
    request: PlanningRequest,
    adb_runner: &R,
) -> Result<PlanningResult, ProcessOutput> {
    let PlanningRequest {
        authored_root,
        configuration_root,
        user_configuration,
        device_plan,
        selected_recipes,
        explicit_input_bindings,
        explicit_context,
        adb_probe,
    } = request;
    let prepared = runtime_configuration::prepare_configuration(ConfigurationContextRequest {
        authored_root,
        configuration_root,
        user_configuration,
        device_plan,
        selected_recipes,
        explicit_bindings: explicit_input_bindings,
        device_context: None,
    })
    .map_err(configuration_context_error_output)?;
    let plan_id = format!("plan.{}.001", prepared.effective_device_plan);
    let mut input = prepared
        .planner_input(plan_id)
        .ok_or_else(|| prepared_configuration_error_output(&prepared))?;

    if let Some(config) = adb_probe {
        let probe = AdbDeviceProbe {
            config,
            runner: adb_runner,
        };
        let detected_facts = probe.detect().map_err(adb_probe_error_output)?;
        input.device_context = crate::device_probe::apply_detected_device_facts_to_context(
            input.device_context,
            &detected_facts,
        );
        let profile_match = load_device_plan_profile_match_criteria(
            &prepared.authored_root,
            &prepared.effective_device_plan,
        )
        .map_err(planner_load_error_output)?;
        let mut result = plan_execution(input);
        add_detected_profile_mismatch_warning(&mut result, &detected_facts, &profile_match);
        apply_explicit_device_context_to_result(&mut result, &explicit_context);
        return Ok(result);
    }

    apply_explicit_device_context(&mut input.device_context, &explicit_context);
    Ok(plan_execution(input))
}

fn configuration_context_error_output(
    error: runtime_configuration::ConfigurationContextError,
) -> ProcessOutput {
    ProcessOutput {
        exit_code: 1,
        stdout: String::new(),
        stderr: format!("Error: configuration_context_invalid: {error}\n"),
    }
}

fn prepared_configuration_error_output(
    prepared: &runtime_configuration::PreparedConfiguration,
) -> ProcessOutput {
    let diagnostic = prepared.diagnostics.first();
    ProcessOutput {
        exit_code: 1,
        stdout: String::new(),
        stderr: diagnostic.map_or_else(
            || "Error: configuration_context_invalid\n".to_string(),
            |diagnostic| format!("Error: {}: {}\n", diagnostic.code, diagnostic.message),
        ),
    }
}

fn planner_load_error_output(error: crate::planner::PlannerLoadError) -> ProcessOutput {
    ProcessOutput {
        exit_code: 1,
        stdout: String::new(),
        stderr: format!("Error: {}: {error}\n", error.code()),
    }
}

fn adb_probe_error_output(error: DeviceProbeError) -> ProcessOutput {
    let code = match error {
        DeviceProbeError::Unavailable { .. } => "adb_probe_unavailable",
        DeviceProbeError::Failed { .. } => "adb_probe_failed",
    };
    ProcessOutput {
        exit_code: 1,
        stdout: String::new(),
        stderr: format!("Error: {code}\n"),
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;

    use serde_json::json;

    use crate::device_probe::{CommandOutput, DeviceProbeError};

    use super::*;

    #[derive(Debug)]
    struct FakeRunner {
        calls: RefCell<Vec<Vec<String>>>,
        result: Result<CommandOutput, DeviceProbeError>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
            self.calls.borrow_mut().push(argv.to_vec());
            self.result.clone()
        }
    }

    fn authored_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("authored")
            .canonicalize()
            .expect("repo authored root should resolve")
    }

    fn request(adb_probe: Option<AdbProbeConfig>) -> PlanningRequest {
        PlanningRequest {
            authored_root: authored_root(),
            configuration_root: None,
            user_configuration: None,
            device_plan: Some("ayaneo.pocket_s_mini.base".to_string()),
            selected_recipes: None,
            explicit_input_bindings: OrderedMap::new(),
            explicit_context: ExplicitDeviceContext::default(),
            adb_probe,
        }
    }

    fn getprop(manufacturer: &str, model: &str, release: i64, api: i64) -> String {
        format!(
            "[ro.product.manufacturer]: [{manufacturer}]\n[ro.product.brand]: [{manufacturer}]\n[ro.product.model]: [{model}]\n[ro.build.version.release]: [{release}]\n[ro.build.version.sdk]: [{api}]\n"
        )
    }

    #[test]
    fn product_planning_without_probe_uses_current_plan_id() {
        let runner = FakeRunner {
            calls: RefCell::new(Vec::new()),
            result: Err(DeviceProbeError::Unavailable {
                message: "runner must not be called".to_string(),
            }),
        };

        let result = plan_with_adb_runner(request(None), &runner).expect("planning should pass");

        assert_eq!(
            result
                .execution_plan
                .expect("execution plan should exist")
                .id,
            "plan.ayaneo.pocket_s_mini.base.001"
        );
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn product_live_probe_uses_selected_adb_and_serial() {
        let runner = FakeRunner {
            calls: RefCell::new(Vec::new()),
            result: Ok(CommandOutput {
                status_code: Some(0),
                stdout: getprop("AYANEO", "AYANEO Pocket S mini", 13, 33),
                stderr: String::new(),
            }),
        };

        let result = plan_with_adb_runner(
            request(Some(AdbProbeConfig {
                adb_path: "/opt/android/adb".to_string(),
                serial: Some("SERIAL123".to_string()),
            })),
            &runner,
        )
        .expect("planning should pass");

        assert_eq!(
            runner.calls.into_inner(),
            vec![vec![
                "/opt/android/adb".to_string(),
                "-s".to_string(),
                "SERIAL123".to_string(),
                "shell".to_string(),
                "getprop".to_string(),
            ]]
        );
        assert_eq!(result.warnings, Vec::new());
    }

    #[test]
    fn explicit_context_changes_emitted_context_but_not_live_mismatch_evidence() {
        let runner = FakeRunner {
            calls: RefCell::new(Vec::new()),
            result: Ok(CommandOutput {
                status_code: Some(0),
                stdout: getprop("Valve", "Steam Deck", 12, 32),
                stderr: String::new(),
            }),
        };
        let mut request = request(Some(AdbProbeConfig {
            adb_path: "adb".to_string(),
            serial: None,
        }));
        request.explicit_context = ExplicitDeviceContext {
            manufacturer: Some("AYANEO".to_string()),
            model: Some("AYANEO Pocket S mini".to_string()),
            android_version: Some(13),
            device_tags: vec!["explicit_handheld".to_string()],
        };

        let result = plan_with_adb_runner(request, &runner).expect("planning should complete");

        assert_eq!(result.warnings[0].code, "device_profile_mismatch");
        assert_eq!(result.warnings[0].details["manufacturer"], json!("Valve"));
        assert_eq!(
            serde_json::to_value(
                result
                    .execution_plan
                    .expect("execution plan should exist")
                    .device_context
            )
            .expect("device context should serialize"),
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
    fn probe_launch_failures_return_stable_product_error() {
        let runner = FakeRunner {
            calls: RefCell::new(Vec::new()),
            result: Err(DeviceProbeError::Unavailable {
                message: "raw host error".to_string(),
            }),
        };

        let error = plan_with_adb_runner(
            request(Some(AdbProbeConfig {
                adb_path: "/tmp/adb".to_string(),
                serial: Some("SERIAL123".to_string()),
            })),
            &runner,
        )
        .expect_err("probe launch should fail");

        assert_eq!(error.stderr, "Error: adb_probe_unavailable\n");
        assert!(!error.stderr.contains("/tmp/adb"));
        assert!(!error.stderr.contains("SERIAL123"));
    }
}
