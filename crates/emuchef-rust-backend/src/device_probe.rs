//! Rust-owned device probing used by canonical planner dispatch.
//!
//! This module defines the command abstraction used to compose ADB `getprop`
//! facts into planner context. The canonical Rust `emuchef plan` route uses the
//! adapter when `--adb` or `--serial` requests live probing.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::owned_process::{run_owned_process, ProcessFailureKind, ProcessOperation};
use crate::planner::DeviceContext;

/// Device facts detected from a selected device.
///
/// Fields are optional so callers can layer detected values over a
/// profile-derived `DeviceContext` without inventing placeholder facts. The
/// Unknown serialized fields are rejected so misspelled facts cannot silently
/// weaken tests or other structured inputs.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DetectedDeviceFacts {
    pub serial: Option<String>,
    pub manufacturer: Option<String>,
    pub brand: Option<String>,
    pub model: Option<String>,
    pub product: Option<String>,
    pub device: Option<String>,
    pub board: Option<String>,
    pub hardware: Option<String>,
    #[serde(default)]
    pub abis: Vec<String>,
    pub android_version: Option<i64>,
    pub android_api_level: Option<i64>,
    #[serde(default)]
    pub device_tags: Vec<String>,
}

/// Stable error classifications for device probing.
///
/// Messages must remain deterministic and must not include host-specific data
/// such as paths, process ids, command timing, or volatile command output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeviceProbeError {
    Unavailable { message: String },
    Failed { message: String },
    TimedOut,
}

/// Output captured from a structured host command invocation.
///
/// This type intentionally stores only the stable process result shape needed by
/// the ADB probe adapter. Probe errors must not expose raw stderr or
/// host-specific launch errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Narrow runner seam for command execution used by the live ADB probe adapter.
///
/// Tests inject fake runners so normal verification never requires a connected
/// device or an installed `adb` binary.
pub(crate) trait CommandRunner {
    fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError>;

    fn run_for(
        &self,
        argv: &[String],
        _operation: ProcessOperation,
    ) -> Result<CommandOutput, DeviceProbeError> {
        self.run(argv)
    }
}

impl<T: CommandRunner + ?Sized> CommandRunner for &T {
    fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
        (*self).run(argv)
    }

    fn run_for(
        &self,
        argv: &[String],
        operation: ProcessOperation,
    ) -> Result<CommandOutput, DeviceProbeError> {
        (*self).run_for(argv, operation)
    }
}

/// Production command runner for the live ADB probe adapter.
///
/// This is the only device-probe adapter that delegates modeled argv to the
/// shared owned-process boundary. It never routes argv through a platform shell.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProcessCommandRunner;

impl CommandRunner for ProcessCommandRunner {
    fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
        self.run_for(argv, ProcessOperation::Probe)
    }

    fn run_for(
        &self,
        argv: &[String],
        operation: ProcessOperation,
    ) -> Result<CommandOutput, DeviceProbeError> {
        let Some((executable, args)) = argv.split_first() else {
            return Err(adb_probe_start_error());
        };
        let output =
            run_owned_process(executable, args, operation).map_err(|error| match error.kind {
                ProcessFailureKind::Spawn => adb_probe_start_error(),
                ProcessFailureKind::TimedOut => DeviceProbeError::TimedOut,
                _ => DeviceProbeError::Failed {
                    message: "ADB probe process failed".to_string(),
                },
            })?;
        Ok(CommandOutput {
            status_code: output.status_code,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Configuration for the live ADB-backed getprop probe command.
///
/// The configuration preserves caller-supplied argv strings and does not
/// validate paths or look up additional environment state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdbProbeConfig {
    pub adb_path: String,
    pub serial: Option<String>,
}

/// Build the argv for collecting all device properties through getprop.
///
/// This pure helper does not execute the returned command. Empty or
/// whitespace-only serial values are treated as absent so callers do not model
/// a selected empty device id.
pub(crate) fn build_adb_getprop_command(config: &AdbProbeConfig) -> Vec<String> {
    let mut command = vec![config.adb_path.clone()];
    if let Some(serial) = present_text(config.serial.as_deref()) {
        command.push("-s".to_string());
        command.push(serial.to_string());
    }
    command.push("shell".to_string());
    command.push("getprop".to_string());
    command
}

/// Parse supplied Android getprop stdout into detected device facts.
///
/// The parser accepts standard lines shaped like `[key]: [value]`, ignores
/// unknown or malformed lines, and never infers data from host state or live
/// device commands. Serial comes only from the explicit argument.
pub(crate) fn detected_facts_from_getprop_output(
    getprop_stdout: &str,
    serial: Option<String>,
) -> DetectedDeviceFacts {
    let mut facts = DetectedDeviceFacts {
        serial: present_text(serial.as_deref()).map(ToString::to_string),
        ..DetectedDeviceFacts::default()
    };
    let mut primary_abis = None;
    let mut fallback_abis = Vec::new();

    for line in getprop_stdout.lines() {
        let Some((key, value)) = parse_getprop_line(line) else {
            continue;
        };
        match key {
            "ro.product.manufacturer" => {
                facts.manufacturer = present_text(Some(value)).map(ToString::to_string);
            }
            "ro.product.brand" => {
                facts.brand = present_text(Some(value)).map(ToString::to_string);
            }
            "ro.product.model" => {
                facts.model = present_text(Some(value)).map(ToString::to_string);
            }
            "ro.product.name" => {
                facts.product = present_text(Some(value)).map(ToString::to_string);
            }
            "ro.product.device" => {
                facts.device = present_text(Some(value)).map(ToString::to_string);
            }
            "ro.product.board" => {
                facts.board = present_text(Some(value)).map(ToString::to_string);
            }
            "ro.hardware" => {
                facts.hardware = present_text(Some(value)).map(ToString::to_string);
            }
            "ro.product.cpu.abilist" => {
                primary_abis = Some(normalize_abis(value.split(',')));
            }
            "ro.product.cpu.abi" | "ro.product.cpu.abi2" => {
                fallback_abis.push(value);
            }
            "ro.build.version.release" => {
                facts.android_version = parse_android_release(value);
            }
            "ro.build.version.sdk" => {
                facts.android_api_level = parse_android_api_level(value);
            }
            _ => {}
        }
    }

    facts.abis = primary_abis
        .filter(|abis| !abis.is_empty())
        .unwrap_or_else(|| normalize_abis(fallback_abis));

    facts
}

/// Preserve reported ABI preference order while removing blanks and duplicates.
fn normalize_abis<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| present_text(Some(value)))
        .filter(|value| seen.insert((*value).to_string()))
        .map(ToString::to_string)
        .collect()
}

fn parse_getprop_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let (raw_key, raw_value) = line.split_once("]: [")?;
    let key = raw_key.strip_prefix('[')?;
    let value = raw_value.strip_suffix(']')?;
    Some((key, value))
}

fn present_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_android_release(value: &str) -> Option<i64> {
    let value = value.trim();
    let digit_count = value
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }
    value[..digit_count].parse::<i64>().ok()
}

fn parse_android_api_level(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok()
}

fn adb_probe_start_error() -> DeviceProbeError {
    DeviceProbeError::Unavailable {
        message: "adb probe command could not be started".to_string(),
    }
}

fn adb_getprop_failed_error() -> DeviceProbeError {
    DeviceProbeError::Failed {
        message: "adb getprop probe command failed".to_string(),
    }
}

/// Abstraction over a source of detected device facts.
pub(crate) trait DeviceProbe {
    fn detect(&self) -> Result<DetectedDeviceFacts, DeviceProbeError>;
}

/// Live ADB-backed probe adapter for collecting Android getprop facts.
///
/// The product planner uses this crate-private adapter when `--adb` or
/// `--serial` requests live device detection.
#[derive(Clone, Debug)]
pub(crate) struct AdbDeviceProbe<R> {
    pub config: AdbProbeConfig,
    pub runner: R,
}

impl<R: CommandRunner> DeviceProbe for AdbDeviceProbe<R> {
    fn detect(&self) -> Result<DetectedDeviceFacts, DeviceProbeError> {
        let command = build_adb_getprop_command(&self.config);
        let output = self.runner.run_for(&command, ProcessOperation::Probe)?;
        if output.status_code != Some(0) {
            return Err(adb_getprop_failed_error());
        }
        Ok(detected_facts_from_getprop_output(
            &output.stdout,
            self.config.serial.clone(),
        ))
    }
}

/// Test probe that returns a preconfigured detection result.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct FakeDeviceProbe {
    result: Result<DetectedDeviceFacts, DeviceProbeError>,
}

#[cfg(test)]
impl FakeDeviceProbe {
    pub(crate) fn new(result: Result<DetectedDeviceFacts, DeviceProbeError>) -> Self {
        Self { result }
    }
}

#[cfg(test)]
impl DeviceProbe for FakeDeviceProbe {
    fn detect(&self) -> Result<DetectedDeviceFacts, DeviceProbeError> {
        self.result.clone()
    }
}

/// Apply detected facts over an existing profile-derived planner context.
///
/// Intended future precedence is:
/// synthetic/profile context -> detected facts -> explicit CLI overrides.
/// Detected facts are applied only through the product planning runtime
/// binary live-probe mode. Other routes keep using their existing context
/// sources.
pub(crate) fn apply_detected_device_facts_to_context(
    mut context: DeviceContext,
    facts: &DetectedDeviceFacts,
) -> DeviceContext {
    if let Some(manufacturer) = &facts.manufacturer {
        context.manufacturer = manufacturer.clone();
    }
    if let Some(model) = &facts.model {
        context.model = model.clone();
    }
    if let Some(android_version) = facts.android_version {
        context.android_version = android_version;
    }
    if let Some(android_api_level) = facts.android_api_level {
        context.android_api_level = Some(android_api_level);
    }
    if !facts.device_tags.is_empty() {
        context.device_tags = facts.device_tags.clone();
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::DeviceContext;
    use std::cell::RefCell;

    fn base_context() -> DeviceContext {
        DeviceContext {
            manufacturer: "Profile Manufacturer".to_string(),
            model: "Profile Model".to_string(),
            android_version: 12,
            android_api_level: Some(32),
            device_tags: vec!["profile_tag".to_string(), "handheld".to_string()],
        }
    }

    #[derive(Debug)]
    struct FakeCommandRunner {
        calls: RefCell<Vec<Vec<String>>>,
        result: Result<CommandOutput, DeviceProbeError>,
    }

    impl FakeCommandRunner {
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

        fn failed_to_launch() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                result: Err(DeviceProbeError::Unavailable {
                    message: "adb probe command could not be started".to_string(),
                }),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl CommandRunner for FakeCommandRunner {
        fn run(&self, argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
            self.calls.borrow_mut().push(argv.to_vec());
            self.result.clone()
        }
    }

    #[derive(Debug, Default)]
    struct DeadlineRecordingRunner {
        operations: RefCell<Vec<ProcessOperation>>,
    }

    impl CommandRunner for DeadlineRecordingRunner {
        fn run(&self, _argv: &[String]) -> Result<CommandOutput, DeviceProbeError> {
            panic!("production getprop detection must use the bounded runner path")
        }

        fn run_for(
            &self,
            _argv: &[String],
            operation: ProcessOperation,
        ) -> Result<CommandOutput, DeviceProbeError> {
            self.operations.borrow_mut().push(operation);
            Ok(CommandOutput {
                status_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn production_getprop_detection_uses_the_fixed_probe_deadline() {
        let probe = AdbDeviceProbe {
            config: AdbProbeConfig {
                adb_path: "adb".to_string(),
                serial: Some("SERIAL123".to_string()),
            },
            runner: DeadlineRecordingRunner::default(),
        };

        probe.detect().expect("bounded probe should succeed");

        assert_eq!(
            *probe.runner.operations.borrow(),
            [crate::owned_process::ProcessOperation::Probe]
        );
    }

    #[test]
    fn fake_probe_returns_configured_detected_facts() {
        let facts = DetectedDeviceFacts {
            serial: Some("FAKE123".to_string()),
            manufacturer: Some("AYANEO".to_string()),
            brand: Some("AYANEO".to_string()),
            model: Some("Pocket S Mini".to_string()),
            android_version: Some(13),
            android_api_level: Some(33),
            device_tags: vec!["detected".to_string()],
            ..DetectedDeviceFacts::default()
        };
        let probe = FakeDeviceProbe::new(Ok(facts.clone()));

        assert_eq!(probe.detect(), Ok(facts));
    }

    #[test]
    fn fake_probe_returns_configured_error() {
        let error = DeviceProbeError::Unavailable {
            message: "device probing unavailable".to_string(),
        };
        let probe = FakeDeviceProbe::new(Err(error.clone()));

        assert_eq!(probe.detect(), Err(error));
    }

    #[test]
    fn adb_device_probe_builds_getprop_command_without_serial() {
        let runner = FakeCommandRunner::completed(Some(0), "", "");
        let probe = AdbDeviceProbe {
            config: AdbProbeConfig {
                adb_path: "adb".to_string(),
                serial: None,
            },
            runner,
        };

        let _ = probe.detect().expect("probe should parse empty facts");

        assert_eq!(
            probe.runner.calls(),
            vec![vec![
                "adb".to_string(),
                "shell".to_string(),
                "getprop".to_string()
            ]]
        );
    }

    #[test]
    fn adb_device_probe_builds_getprop_command_with_serial() {
        let runner = FakeCommandRunner::completed(Some(0), "", "");
        let probe = AdbDeviceProbe {
            config: AdbProbeConfig {
                adb_path: "adb".to_string(),
                serial: Some("SERIAL123".to_string()),
            },
            runner,
        };

        let _ = probe.detect().expect("probe should parse empty facts");

        assert_eq!(
            probe.runner.calls(),
            vec![vec![
                "adb".to_string(),
                "-s".to_string(),
                "SERIAL123".to_string(),
                "shell".to_string(),
                "getprop".to_string(),
            ]]
        );
    }

    #[test]
    fn adb_device_probe_successful_stdout_returns_detected_facts() {
        let runner = FakeCommandRunner::completed(
            Some(0),
            "\
[ro.product.manufacturer]: [AYANEO]
[ro.product.brand]: [AYANEO]
[ro.product.model]: [Pocket S Mini]
[ro.build.version.release]: [13]
[ro.build.version.sdk]: [33]
",
            "",
        );
        let probe = AdbDeviceProbe {
            config: AdbProbeConfig {
                adb_path: "adb".to_string(),
                serial: None,
            },
            runner,
        };

        let facts = probe.detect().expect("probe should parse getprop facts");

        assert_eq!(
            facts,
            DetectedDeviceFacts {
                serial: None,
                manufacturer: Some("AYANEO".to_string()),
                brand: Some("AYANEO".to_string()),
                model: Some("Pocket S Mini".to_string()),
                android_version: Some(13),
                android_api_level: Some(33),
                device_tags: Vec::new(),
                ..DetectedDeviceFacts::default()
            }
        );
    }

    #[test]
    fn adb_device_probe_includes_configured_serial_in_detected_facts() {
        let runner =
            FakeCommandRunner::completed(Some(0), "[ro.product.model]: [Pocket S Mini]\n", "");
        let probe = AdbDeviceProbe {
            config: AdbProbeConfig {
                adb_path: "adb".to_string(),
                serial: Some("SERIAL123".to_string()),
            },
            runner,
        };

        let facts = probe
            .detect()
            .expect("probe should include configured serial");

        assert_eq!(facts.serial, Some("SERIAL123".to_string()));
        assert_eq!(facts.model, Some("Pocket S Mini".to_string()));
    }

    #[test]
    fn adb_device_probe_non_zero_status_returns_stable_failed_error_without_stderr() {
        let runner = FakeCommandRunner::completed(
            Some(1),
            "[ro.product.model]: [Pocket S Mini]\n",
            "device-specific failure details",
        );
        let probe = AdbDeviceProbe {
            config: AdbProbeConfig {
                adb_path: "/host/specific/adb".to_string(),
                serial: Some("SERIAL123".to_string()),
            },
            runner,
        };

        let error = probe.detect().expect_err("non-zero status should fail");

        assert_eq!(
            error,
            DeviceProbeError::Failed {
                message: "adb getprop probe command failed".to_string(),
            }
        );
        assert_error_message_excludes(
            &error,
            &[
                "device-specific failure details",
                "/host/specific/adb",
                "SERIAL123",
            ],
        );
    }

    #[test]
    fn adb_device_probe_launch_failure_returns_stable_unavailable_error() {
        let probe = AdbDeviceProbe {
            config: AdbProbeConfig {
                adb_path: "/host/specific/adb".to_string(),
                serial: Some("SERIAL123".to_string()),
            },
            runner: FakeCommandRunner::failed_to_launch(),
        };

        let error = probe.detect().expect_err("launch failure should fail");

        assert_eq!(
            error,
            DeviceProbeError::Unavailable {
                message: "adb probe command could not be started".to_string(),
            }
        );
        assert_error_message_excludes(&error, &["/host/specific/adb", "SERIAL123"]);
    }

    #[test]
    fn adb_device_probe_empty_or_malformed_stdout_returns_absent_facts() {
        for stdout in ["", "   \n", "not a getprop line\n[missing.end: [ignored]\n"] {
            let probe = AdbDeviceProbe {
                config: AdbProbeConfig {
                    adb_path: "adb".to_string(),
                    serial: None,
                },
                runner: FakeCommandRunner::completed(Some(0), stdout, ""),
            };

            let facts = probe
                .detect()
                .expect("successful empty output is absent facts");

            assert_eq!(facts, DetectedDeviceFacts::default(), "stdout={stdout:?}");
        }
    }

    #[test]
    fn process_command_runner_empty_argv_returns_stable_unavailable_error() {
        let runner = ProcessCommandRunner;

        let error = runner
            .run(&[])
            .expect_err("empty argv should not start a process");

        assert_eq!(
            error,
            DeviceProbeError::Unavailable {
                message: "adb probe command could not be started".to_string(),
            }
        );
    }

    #[test]
    fn default_detected_facts_are_absent() {
        assert_eq!(
            DetectedDeviceFacts::default(),
            DetectedDeviceFacts {
                serial: None,
                manufacturer: None,
                brand: None,
                model: None,
                android_version: None,
                android_api_level: None,
                device_tags: Vec::new(),
                ..DetectedDeviceFacts::default()
            }
        );
    }

    #[test]
    fn detected_manufacturer_overrides_base_context() {
        let context = apply_detected_device_facts_to_context(
            base_context(),
            &DetectedDeviceFacts {
                manufacturer: Some("Detected Manufacturer".to_string()),
                ..DetectedDeviceFacts::default()
            },
        );

        assert_eq!(context.manufacturer, "Detected Manufacturer");
        assert_eq!(context.model, "Profile Model");
    }

    #[test]
    fn detected_model_overrides_base_context() {
        let context = apply_detected_device_facts_to_context(
            base_context(),
            &DetectedDeviceFacts {
                model: Some("Detected Model".to_string()),
                ..DetectedDeviceFacts::default()
            },
        );

        assert_eq!(context.model, "Detected Model");
        assert_eq!(context.manufacturer, "Profile Manufacturer");
    }

    #[test]
    fn detected_android_version_overrides_base_context() {
        let context = apply_detected_device_facts_to_context(
            base_context(),
            &DetectedDeviceFacts {
                android_version: Some(14),
                ..DetectedDeviceFacts::default()
            },
        );

        assert_eq!(context.android_version, 14);
        assert_eq!(context.android_api_level, Some(32));
    }

    #[test]
    fn detected_android_api_level_overrides_base_context() {
        let context = apply_detected_device_facts_to_context(
            base_context(),
            &DetectedDeviceFacts {
                android_api_level: Some(34),
                ..DetectedDeviceFacts::default()
            },
        );

        assert_eq!(context.android_api_level, Some(34));
        assert_eq!(context.android_version, 12);
    }

    #[test]
    fn absent_detected_fields_preserve_base_context_fields() {
        let context =
            apply_detected_device_facts_to_context(base_context(), &DetectedDeviceFacts::default());

        assert_eq!(context, base_context());
    }

    #[test]
    fn non_empty_detected_tags_replace_base_tags_in_order() {
        let context = apply_detected_device_facts_to_context(
            base_context(),
            &DetectedDeviceFacts {
                device_tags: vec!["detected_first".to_string(), "detected_second".to_string()],
                ..DetectedDeviceFacts::default()
            },
        );

        assert_eq!(
            context.device_tags,
            vec!["detected_first".to_string(), "detected_second".to_string()]
        );
    }

    #[test]
    fn empty_detected_tags_preserve_base_tags() {
        let context =
            apply_detected_device_facts_to_context(base_context(), &DetectedDeviceFacts::default());

        assert_eq!(
            context.device_tags,
            vec!["profile_tag".to_string(), "handheld".to_string()]
        );
    }

    #[test]
    fn adb_probe_command_without_serial_returns_adb_shell_getprop() {
        let command = build_adb_getprop_command(&AdbProbeConfig {
            adb_path: "adb".to_string(),
            serial: None,
        });

        assert_eq!(
            command,
            vec![
                "adb".to_string(),
                "shell".to_string(),
                "getprop".to_string()
            ]
        );
    }

    #[test]
    fn adb_probe_command_with_serial_returns_adb_s_serial_shell_getprop() {
        let command = build_adb_getprop_command(&AdbProbeConfig {
            adb_path: "adb".to_string(),
            serial: Some("SERIAL123".to_string()),
        });

        assert_eq!(
            command,
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "SERIAL123".to_string(),
                "shell".to_string(),
                "getprop".to_string(),
            ]
        );
    }

    #[test]
    fn adb_probe_command_preserves_configured_adb_path() {
        let command = build_adb_getprop_command(&AdbProbeConfig {
            adb_path: "/opt/android platform tools/adb".to_string(),
            serial: None,
        });

        assert_eq!(command[0], "/opt/android platform tools/adb");
    }

    #[test]
    fn adb_probe_command_treats_empty_or_whitespace_serial_as_absent() {
        for serial in ["", "   ", "\t\n"] {
            let command = build_adb_getprop_command(&AdbProbeConfig {
                adb_path: "adb".to_string(),
                serial: Some(serial.to_string()),
            });

            assert_eq!(
                command,
                vec![
                    "adb".to_string(),
                    "shell".to_string(),
                    "getprop".to_string()
                ]
            );
        }
    }

    #[test]
    fn adb_probe_getprop_parser_extracts_supported_detected_facts() {
        let output = "\
[ro.product.manufacturer]: [AYANEO]
[ro.product.brand]: [AYANEO]
[ro.product.model]: [Pocket S Mini]
[ro.build.version.release]: [13]
[ro.build.version.sdk]: [33]
";

        let facts = detected_facts_from_getprop_output(output, None);

        assert_eq!(
            facts,
            DetectedDeviceFacts {
                serial: None,
                manufacturer: Some("AYANEO".to_string()),
                brand: Some("AYANEO".to_string()),
                model: Some("Pocket S Mini".to_string()),
                android_version: Some(13),
                android_api_level: Some(33),
                device_tags: Vec::new(),
                ..DetectedDeviceFacts::default()
            }
        );
    }

    #[test]
    fn adb_probe_getprop_parser_parses_android_release_leading_integer_forms() {
        for release in ["13", "13.0", "13 QPR", " 13 QPR "] {
            let output = format!("[ro.build.version.release]: [{release}]\n");

            let facts = detected_facts_from_getprop_output(&output, None);

            assert_eq!(facts.android_version, Some(13), "release={release:?}");
        }
    }

    #[test]
    fn adb_probe_getprop_parser_ignores_invalid_release_and_sdk_values() {
        let output = "\
[ro.build.version.release]: [Android 13]
[ro.build.version.sdk]: [33 preview]
";

        let facts = detected_facts_from_getprop_output(output, None);

        assert_eq!(facts.android_version, None);
        assert_eq!(facts.android_api_level, None);
    }

    #[test]
    fn adb_probe_getprop_parser_treats_empty_values_as_absent() {
        let output = "\
[ro.product.manufacturer]: []
[ro.product.brand]: [   ]
[ro.product.model]: []
[ro.build.version.release]: [ ]
[ro.build.version.sdk]: []
";

        let facts = detected_facts_from_getprop_output(output, None);

        assert_eq!(facts, DetectedDeviceFacts::default());
    }

    #[test]
    fn adb_probe_getprop_parser_ignores_unknown_and_malformed_lines() {
        let output = "\
[ro.product.manufacturer]: [AYANEO]
[ro.unknown.key]: [ignored]
not a getprop line
[missing.end: [ignored]
[ro.product.model]: [Pocket S Mini]
";

        let facts = detected_facts_from_getprop_output(output, None);

        assert_eq!(facts.manufacturer, Some("AYANEO".to_string()));
        assert_eq!(facts.model, Some("Pocket S Mini".to_string()));
        assert_eq!(facts.brand, None);
    }

    #[test]
    fn adb_probe_getprop_parser_leaves_device_tags_empty() {
        let facts = detected_facts_from_getprop_output(
            "[ro.product.model]: [Pocket S Mini]\n",
            Some("SERIAL123".to_string()),
        );

        assert!(facts.device_tags.is_empty());
    }

    #[test]
    fn adb_probe_getprop_parser_includes_supplied_serial() {
        let facts = detected_facts_from_getprop_output(
            "[ro.product.model]: [Pocket S Mini]\n",
            Some("SERIAL123".to_string()),
        );

        assert_eq!(facts.serial, Some("SERIAL123".to_string()));
    }

    #[test]
    fn adb_probe_getprop_parser_treats_empty_or_whitespace_serial_as_absent() {
        for serial in ["", "   ", "\t\n"] {
            let facts = detected_facts_from_getprop_output(
                "[ro.product.model]: [Pocket S Mini]\n",
                Some(serial.to_string()),
            );

            assert_eq!(facts.serial, None, "serial={serial:?}");
        }
    }

    #[test]
    fn authored_planner_data_layer_does_not_invoke_live_adb_probe() {
        for (name, source) in [(
            "planner_device_plan.rs",
            include_str!("planner_device_plan.rs"),
        )] {
            let code = source_without_line_comments(source);
            for forbidden in [
                "AdbDeviceProbe",
                "ProcessCommandRunner",
                "build_adb_getprop_command",
            ] {
                assert!(
                    !code.contains(forbidden),
                    "{name} must not invoke live ADB probing marker {forbidden:?}"
                );
            }
        }
    }

    #[test]
    fn probe_error_classifications_are_stable_messages() {
        let errors = [
            DeviceProbeError::Unavailable {
                message: "probe unavailable".to_string(),
            },
            DeviceProbeError::Failed {
                message: "probe failed".to_string(),
            },
        ];

        assert!(matches!(
            errors[0],
            DeviceProbeError::Unavailable { ref message } if message == "probe unavailable"
        ));
        assert!(matches!(
            errors[1],
            DeviceProbeError::Failed { ref message } if message == "probe failed"
        ));
    }

    #[test]
    fn getprop_parser_expands_safe_identity_facts_and_preserves_abi_order() {
        let facts = detected_facts_from_getprop_output(
            "\
[ro.product.name]: [pocket_s_mini]
[ro.product.device]: [pocket_s_mini]
[ro.product.board]: [kalama]
[ro.hardware]: [qcom]
[ro.product.cpu.abilist]: [ arm64-v8a,armeabi-v7a,arm64-v8a, ,x86_64 ]
",
            None,
        );
        assert_eq!(facts.product.as_deref(), Some("pocket_s_mini"));
        assert_eq!(facts.device.as_deref(), Some("pocket_s_mini"));
        assert_eq!(facts.board.as_deref(), Some("kalama"));
        assert_eq!(facts.hardware.as_deref(), Some("qcom"));
        assert_eq!(facts.abis, ["arm64-v8a", "armeabi-v7a", "x86_64"]);
    }

    #[test]
    fn getprop_parser_uses_ordered_abi_fallback_only_when_abilist_is_absent() {
        let fallback = detected_facts_from_getprop_output(
            "\
[ro.product.cpu.abi]: [arm64-v8a]
[ro.product.cpu.abi2]: [ armeabi-v7a ]
[ro.product.cpu.abi2]: [arm64-v8a]
",
            None,
        );
        assert_eq!(fallback.abis, ["arm64-v8a", "armeabi-v7a"]);

        let primary = detected_facts_from_getprop_output(
            "\
[ro.product.cpu.abi]: [fallback]
[ro.product.cpu.abilist]: [primary,secondary]
",
            None,
        );
        assert_eq!(primary.abis, ["primary", "secondary"]);
    }

    fn source_without_line_comments(source: &str) -> String {
        source
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !(trimmed.starts_with("//")
                    || trimmed.starts_with("///")
                    || trimmed.starts_with("//!"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_error_message_excludes(error: &DeviceProbeError, needles: &[&str]) {
        let message = match error {
            DeviceProbeError::Unavailable { message } | DeviceProbeError::Failed { message } => {
                message
            }
            DeviceProbeError::TimedOut => "probe timed out",
        };
        for needle in needles {
            assert!(
                !message.contains(needle),
                "probe error message {message:?} should not contain host-specific detail {needle:?}"
            );
        }
    }
}
