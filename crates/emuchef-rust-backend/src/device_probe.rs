//! Rust-side device probing foundation for future planner ownership.
//!
//! This module defines the small abstraction needed to compose detected device
//! facts into planner context later. It intentionally has no live adapter and no
//! route wiring; current callers can use only explicit/profile-derived context.

use crate::planner::DeviceContext;

/// Device facts a future probe adapter may detect from a selected device.
///
/// Fields are optional so callers can layer detected values over a
/// profile-derived `DeviceContext` without inventing placeholder facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DetectedDeviceFacts {
    pub serial: Option<String>,
    pub manufacturer: Option<String>,
    pub brand: Option<String>,
    pub model: Option<String>,
    pub android_version: Option<i64>,
    pub android_api_level: Option<i64>,
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
    fn fake_probe_path_has_no_live_behavior_dependencies() {
        let source = include_str!("device_probe.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("source should include production section");

        for forbidden in [
            "std::process",
            "Command::new",
            "std::env",
            "std::fs",
            "TcpStream",
            "UdpSocket",
            "reqwest",
            "ureq",
            "hyper",
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
}
