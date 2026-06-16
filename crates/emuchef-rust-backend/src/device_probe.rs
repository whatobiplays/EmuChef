//! Rust-side device probing foundation for future planner ownership.
//!
//! This module defines the small abstraction needed to compose detected device
//! facts into planner context later. It intentionally has no live adapter and no
//! route wiring; current callers can use only explicit/profile-derived context.

use serde::Deserialize;

use crate::planner::DeviceContext;

/// Device facts a future probe adapter may detect from a selected device.
///
/// Fields are optional so callers can layer detected values over a
/// profile-derived `DeviceContext` without inventing placeholder facts. The
/// struct also deserializes strict local JSON fixtures for the dev-only shadow
/// harness; unknown fixture fields are rejected so misspelled facts do not
/// silently weaken migration evidence.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetectedDeviceFacts {
    pub serial: Option<String>,
    pub manufacturer: Option<String>,
    pub brand: Option<String>,
    pub model: Option<String>,
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
    InvalidOutput { message: String },
}

/// Configuration for modeling a future ADB-backed getprop probe command.
///
/// This is only a command specification. It preserves caller-supplied argv
/// strings and never validates paths, looks up environment state, or starts a
/// process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdbProbeConfig {
    pub adb_path: String,
    pub serial: Option<String>,
}

/// Build the argv for collecting all device properties through getprop.
///
/// The returned command is suitable as a future adapter input, but this helper
/// intentionally does not execute it. Empty or whitespace-only serial values are
/// treated as absent so callers do not model a selected empty device id.
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
            "ro.build.version.release" => {
                facts.android_version = parse_android_release(value);
            }
            "ro.build.version.sdk" => {
                facts.android_api_level = parse_android_api_level(value);
            }
            _ => {}
        }
    }

    facts
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

/// Abstraction over a source of detected device facts.
pub(crate) trait DeviceProbe {
    fn detect(&self) -> Result<DetectedDeviceFacts, DeviceProbeError>;
}

/// Test probe that returns a preconfigured detection result.
#[derive(Clone, Debug)]
pub(crate) struct FakeDeviceProbe {
    result: Result<DetectedDeviceFacts, DeviceProbeError>,
}

impl FakeDeviceProbe {
    pub(crate) fn new(result: Result<DetectedDeviceFacts, DeviceProbeError>) -> Self {
        Self { result }
    }
}

impl DeviceProbe for FakeDeviceProbe {
    fn detect(&self) -> Result<DetectedDeviceFacts, DeviceProbeError> {
        self.result.clone()
    }
}

/// Apply detected facts over an existing profile-derived planner context.
///
/// Intended future precedence is:
/// synthetic/profile context -> detected facts -> explicit CLI overrides.
/// P8N only supplies this helper and does not wire probing into any route.
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

    fn base_context() -> DeviceContext {
        DeviceContext {
            manufacturer: "Profile Manufacturer".to_string(),
            model: "Profile Model".to_string(),
            android_version: 12,
            android_api_level: Some(32),
            device_tags: vec!["profile_tag".to_string(), "handheld".to_string()],
        }
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
    fn fake_probe_path_has_no_live_behavior_dependencies() {
        let production_source = production_source_without_line_comments();

        for forbidden in [
            "std::process",
            "Command::new",
            "std::env",
            "std::net",
            "TcpStream",
            "UdpSocket",
            "tokio::net",
            "reqwest",
            "ureq",
            "hyper",
            "adb ",
            "adb.exe",
        ] {
            assert!(
                !production_source.contains(forbidden),
                "device probe foundation must not contain live behavior marker {forbidden:?}"
            );
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
            DeviceProbeError::InvalidOutput {
                message: "invalid probe output".to_string(),
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
        assert!(matches!(
            errors[2],
            DeviceProbeError::InvalidOutput { ref message } if message == "invalid probe output"
        ));
    }

    fn production_source_without_line_comments() -> String {
        include_str!("device_probe.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("source should include production section")
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
}
