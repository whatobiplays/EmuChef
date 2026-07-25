//! Backend-owned, read-only device qualification projection.
//!
//! This module deliberately does not execute ADB commands. Slice 6B.1 defines
//! deterministic classification, sanitization, and invalidation semantics;
//! bounded live inspection is added by a later slice.

use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

use crate::commands::{current_adb_path, safe_error, AppState};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceQualificationState {
    NotApplicable,
    NoDevice,
    Unauthorized,
    Offline,
    InsufficientlyQualified,
    Unsupported,
    Supported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RootQualificationState {
    NotChecked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceQualificationSnapshotDto {
    pub state: DeviceQualificationState,
    pub summary: &'static str,
    pub limitations: Vec<&'static str>,
    pub android_major: Option<u32>,
    pub android_api_level: Option<u32>,
    pub abi_class: Option<&'static str>,
    pub root: RootQualificationState,
    pub runtime_generation: u64,
    pub qualification_revision: u64,
    pub device_identity: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservedDeviceState {
    Unauthorized,
    Offline,
    Online,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedDevice<'a> {
    pub opaque_identity: &'a str,
    pub state: ObservedDeviceState,
    pub android_major: Option<u32>,
    pub android_api_level: Option<u32>,
    pub abi: Option<&'a str>,
}

pub fn classify(
    compiled: bool,
    runtime_generation: u64,
    qualification_revision: u64,
    devices: &[ObservedDevice<'_>],
) -> DeviceQualificationSnapshotDto {
    if !compiled {
        return snapshot(
            DeviceQualificationState::NotApplicable,
            "Real-device qualification is not compiled in this build.",
            vec!["Simulation remains available."],
            runtime_generation,
            qualification_revision,
            None,
            None,
            None,
            None,
        );
    }

    if devices.is_empty() {
        return snapshot(
            DeviceQualificationState::NoDevice,
            "No Android device is available for qualification.",
            vec!["Connect one device and refresh discovery."],
            runtime_generation,
            qualification_revision,
            None,
            None,
            None,
            None,
        );
    }

    if devices.len() != 1 {
        return snapshot(
            DeviceQualificationState::InsufficientlyQualified,
            "More than one Android device is connected.",
            vec!["Disconnect additional devices before qualification."],
            runtime_generation,
            qualification_revision,
            None,
            None,
            None,
            None,
        );
    }

    let device = &devices[0];
    match device.state {
        ObservedDeviceState::Unauthorized => snapshot(
            DeviceQualificationState::Unauthorized,
            "The connected device has not authorized this Mac.",
            vec!["Approve the USB debugging prompt on the device."],
            runtime_generation,
            qualification_revision,
            Some(device.opaque_identity),
            None,
            None,
            None,
        ),
        ObservedDeviceState::Offline => snapshot(
            DeviceQualificationState::Offline,
            "The connected device is offline.",
            vec!["Reconnect the device and refresh discovery."],
            runtime_generation,
            qualification_revision,
            Some(device.opaque_identity),
            None,
            None,
            None,
        ),
        ObservedDeviceState::Online => {
            classify_online(runtime_generation, qualification_revision, device)
        }
    }
}

fn classify_online(
    runtime_generation: u64,
    qualification_revision: u64,
    device: &ObservedDevice<'_>,
) -> DeviceQualificationSnapshotDto {
    let Some(android_major) = device.android_major else {
        return incomplete(runtime_generation, qualification_revision, device);
    };
    let Some(api_level) = device.android_api_level else {
        return incomplete(runtime_generation, qualification_revision, device);
    };
    let Some(abi_class) = normalize_abi(device.abi) else {
        return snapshot(
            DeviceQualificationState::Unsupported,
            "The connected device uses an unsupported processor architecture.",
            vec!["This device cannot begin real execution."],
            runtime_generation,
            qualification_revision,
            Some(device.opaque_identity),
            Some(android_major),
            Some(api_level),
            None,
        );
    };

    if android_major < 11 || api_level < 30 {
        return snapshot(
            DeviceQualificationState::Unsupported,
            "The connected device uses an unsupported Android version.",
            vec!["EmuChef requires Android 11 or newer."],
            runtime_generation,
            qualification_revision,
            Some(device.opaque_identity),
            Some(android_major),
            Some(api_level),
            Some(abi_class),
        );
    }

    snapshot(
        DeviceQualificationState::Supported,
        "The connected device meets the initial qualification contract.",
        vec!["Root access has not been checked."],
        runtime_generation,
        qualification_revision,
        Some(device.opaque_identity),
        Some(android_major),
        Some(api_level),
        Some(abi_class),
    )
}

fn incomplete(
    runtime_generation: u64,
    qualification_revision: u64,
    device: &ObservedDevice<'_>,
) -> DeviceQualificationSnapshotDto {
    snapshot(
        DeviceQualificationState::InsufficientlyQualified,
        "The connected device could not be fully qualified.",
        vec!["Refresh discovery before attempting real execution."],
        runtime_generation,
        qualification_revision,
        Some(device.opaque_identity),
        device.android_major,
        device.android_api_level,
        normalize_abi(device.abi),
    )
}

fn normalize_abi(abi: Option<&str>) -> Option<&'static str> {
    match abi {
        Some("arm64-v8a") => Some("arm64"),
        Some("armeabi-v7a") => Some("arm32"),
        Some("x86_64") => Some("x86_64"),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn snapshot(
    state: DeviceQualificationState,
    summary: &'static str,
    limitations: Vec<&'static str>,
    runtime_generation: u64,
    qualification_revision: u64,
    identity: Option<&str>,
    android_major: Option<u32>,
    android_api_level: Option<u32>,
    abi_class: Option<&'static str>,
) -> DeviceQualificationSnapshotDto {
    DeviceQualificationSnapshotDto {
        state,
        summary,
        limitations,
        android_major,
        android_api_level,
        abi_class,
        root: RootQualificationState::NotChecked,
        runtime_generation,
        qualification_revision,
        device_identity: identity.map(ToOwned::to_owned),
    }
}

#[tauri::command]
pub fn get_device_qualification(
    device_handle: String,
    state: State<'_, AppState>,
) -> Result<DeviceQualificationSnapshotDto, String> {
    let runtime_generation = state.sidecar.try_generation().map_err(|_| {
        safe_error(
            "runtime_generation_unavailable",
            "Device qualification state is temporarily unavailable.",
        )
    })?;
    let qualification_revision = state
        .adb
        .lock()
        .map_err(|_| {
            safe_error(
                "adb_state_unavailable",
                "Platform-Tools setup state is unavailable.",
            )
        })?
        .revision();

    if !cfg!(feature = "real-execution") {
        return Ok(classify(
            false,
            runtime_generation,
            qualification_revision,
            &[],
        ));
    }

    let adb_path = current_adb_path(&state)?;
    let observed = state
        .sidecar
        .request(
            "qualifyDevice",
            json!({ "adbPath": adb_path, "serial": device_handle }),
        )
        .map_err(|_| {
            safe_error(
                "device_qualification_failed",
                "Connected-device qualification could not be completed.",
            )
        })?;
    Ok(classify_observed(
        runtime_generation,
        qualification_revision,
        &observed,
    ))
}

fn classify_observed(
    runtime_generation: u64,
    qualification_revision: u64,
    observed: &Value,
) -> DeviceQualificationSnapshotDto {
    let state = observed.get("state").and_then(Value::as_str);
    let devices = match state {
        Some("no_device") => Vec::new(),
        Some("multiple_devices") => vec![online_placeholder("first"), online_placeholder("second")],
        Some("unauthorized") => vec![observed_device(observed, ObservedDeviceState::Unauthorized)],
        Some("offline") => vec![observed_device(observed, ObservedDeviceState::Offline)],
        Some("online") => vec![observed_device(observed, ObservedDeviceState::Online)],
        _ => vec![ObservedDevice {
            opaque_identity: "qualification-device",
            state: ObservedDeviceState::Online,
            android_major: None,
            android_api_level: None,
            abi: None,
        }],
    };
    classify(true, runtime_generation, qualification_revision, &devices)
}

fn observed_device<'a>(observed: &'a Value, state: ObservedDeviceState) -> ObservedDevice<'a> {
    ObservedDevice {
        opaque_identity: "qualification-device",
        state,
        android_major: observed
            .get("androidMajor")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        android_api_level: observed
            .get("androidApiLevel")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        abi: observed.get("abi").and_then(Value::as_str),
    }
}

fn online_placeholder(identity: &str) -> ObservedDevice<'_> {
    ObservedDevice {
        opaque_identity: identity,
        state: ObservedDeviceState::Online,
        android_major: None,
        android_api_level: None,
        abi: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn online<'a>(
        identity: &'a str,
        major: Option<u32>,
        api: Option<u32>,
        abi: Option<&'a str>,
    ) -> ObservedDevice<'a> {
        ObservedDevice {
            opaque_identity: identity,
            state: ObservedDeviceState::Online,
            android_major: major,
            android_api_level: api,
            abi,
        }
    }

    #[test]
    fn feature_disabled_build_is_not_applicable_and_never_identifies_a_device() {
        let result = classify(
            false,
            4,
            7,
            &[online("opaque", Some(14), Some(34), Some("arm64-v8a"))],
        );
        assert_eq!(result.state, DeviceQualificationState::NotApplicable);
        assert_eq!(result.device_identity, None);
        assert_eq!(result.root, RootQualificationState::NotChecked);
    }

    #[test]
    fn zero_and_multiple_devices_never_select_a_target() {
        let none = classify(true, 1, 2, &[]);
        assert_eq!(none.state, DeviceQualificationState::NoDevice);
        assert_eq!(none.device_identity, None);

        let multiple = classify(
            true,
            1,
            3,
            &[
                online("one", Some(13), Some(33), Some("arm64-v8a")),
                online("two", Some(13), Some(33), Some("arm64-v8a")),
            ],
        );
        assert_eq!(
            multiple.state,
            DeviceQualificationState::InsufficientlyQualified
        );
        assert_eq!(multiple.device_identity, None);
    }

    #[test]
    fn authorization_and_online_state_are_deterministic() {
        for (observed, expected) in [
            (
                ObservedDeviceState::Unauthorized,
                DeviceQualificationState::Unauthorized,
            ),
            (
                ObservedDeviceState::Offline,
                DeviceQualificationState::Offline,
            ),
        ] {
            let device = ObservedDevice {
                opaque_identity: "device-token",
                state: observed,
                android_major: None,
                android_api_level: None,
                abi: None,
            };
            assert_eq!(classify(true, 1, 1, &[device]).state, expected);
        }
    }

    #[test]
    fn supported_device_projects_only_normalized_facts() {
        let result = classify(
            true,
            8,
            13,
            &[online(
                "opaque-token",
                Some(14),
                Some(34),
                Some("arm64-v8a"),
            )],
        );
        assert_eq!(result.state, DeviceQualificationState::Supported);
        assert_eq!(result.android_major, Some(14));
        assert_eq!(result.android_api_level, Some(34));
        assert_eq!(result.abi_class, Some("arm64"));
        assert_eq!(result.root, RootQualificationState::NotChecked);
        assert_eq!(result.runtime_generation, 8);
        assert_eq!(result.qualification_revision, 13);
    }

    #[test]
    fn old_android_unknown_facts_and_unknown_abi_do_not_qualify() {
        assert_eq!(
            classify(
                true,
                1,
                1,
                &[online("old", Some(10), Some(29), Some("arm64-v8a"))]
            )
            .state,
            DeviceQualificationState::Unsupported,
        );
        assert_eq!(
            classify(
                true,
                1,
                1,
                &[online("unknown", None, None, Some("arm64-v8a"))]
            )
            .state,
            DeviceQualificationState::InsufficientlyQualified,
        );
        assert_eq!(
            classify(
                true,
                1,
                1,
                &[online("abi", Some(14), Some(34), Some("mips"))]
            )
            .state,
            DeviceQualificationState::Unsupported,
        );
    }

    #[test]
    fn generation_and_revision_are_part_of_every_snapshot() {
        let first = classify(true, 3, 5, &[]);
        let restarted = classify(true, 4, 6, &[]);
        assert_ne!(first.runtime_generation, restarted.runtime_generation);
        assert_ne!(
            first.qualification_revision,
            restarted.qualification_revision
        );
    }

    #[test]
    fn trusted_observation_is_classified_without_projecting_the_serial() {
        let result = classify_observed(
            9,
            11,
            &json!({
                "state": "online",
                "serial": "exact-sensitive-serial",
                "androidMajor": 14,
                "androidApiLevel": 34,
                "abi": "arm64-v8a",
            }),
        );
        assert_eq!(result.state, DeviceQualificationState::Supported);
        assert_eq!(
            result.device_identity.as_deref(),
            Some("qualification-device")
        );
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("exact-sensitive-serial"));
    }

    #[test]
    fn trusted_multiple_device_observation_never_selects_a_target() {
        let result = classify_observed(2, 3, &json!({ "state": "multiple_devices" }));
        assert_eq!(
            result.state,
            DeviceQualificationState::InsufficientlyQualified
        );
        assert_eq!(result.device_identity, None);
    }
}
