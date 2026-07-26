//! Backend-owned, read-only device qualification projection.
//!
//! Passive qualification and root authorization state stay native-owned. The
//! sidecar performs bounded ADB inspection; this module projects only sanitized
//! facts and generation-bound root evidence to the React client.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum RootQualificationState {
    Granted,
    Denied,
    Unavailable,
    CheckFailed {
        reason: RootQualificationFailureReason,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RootQualificationFailureReason {
    TimedOut,
    Transport,
    UnexpectedResponse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RootQualificationKey {
    pub(crate) device_handle: String,
    pub(crate) runtime_generation: u64,
    pub(crate) qualification_revision: u64,
}

impl RootQualificationKey {
    pub(crate) fn new(
        device_handle: impl Into<String>,
        runtime_generation: u64,
        qualification_revision: u64,
    ) -> Self {
        Self {
            device_handle: device_handle.into(),
            runtime_generation,
            qualification_revision,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RootQualificationAttempt {
    key: RootQualificationKey,
    id: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RootQualificationInvalidation {
    pub(crate) device_handle: Option<String>,
    pub(crate) cancelled_in_flight: bool,
}

#[derive(Debug, Default)]
pub(crate) struct RootQualificationStore {
    record: Option<(RootQualificationKey, RootQualificationState)>,
    in_flight: Option<RootQualificationAttempt>,
    next_id: u64,
}

impl RootQualificationStore {
    pub(crate) fn begin(
        &mut self,
        key: RootQualificationKey,
    ) -> Result<RootQualificationAttempt, &'static str> {
        if self.in_flight.is_some() {
            return Err("root_check_in_progress");
        }
        self.next_id = self.next_id.saturating_add(1).max(1);
        let attempt = RootQualificationAttempt {
            key,
            id: self.next_id,
        };
        self.in_flight = Some(attempt.clone());
        Ok(attempt)
    }

    pub(crate) fn complete(
        &mut self,
        attempt: RootQualificationAttempt,
        result: RootQualificationState,
    ) -> bool {
        if self.in_flight.as_ref() != Some(&attempt) {
            return false;
        }
        self.in_flight = None;
        self.record = Some((attempt.key, result));
        true
    }

    /// Cancel only the currently active attempt. A stale caller cannot clear a
    /// newer probe or disturb the last completed result.
    pub(crate) fn cancel(&mut self, attempt: &RootQualificationAttempt) -> bool {
        if self.in_flight.as_ref() != Some(attempt) {
            return false;
        }
        self.in_flight = None;
        self.next_id = self.next_id.saturating_add(1).max(1);
        true
    }

    pub(crate) fn get(&self, key: &RootQualificationKey) -> Option<RootQualificationState> {
        self.record
            .as_ref()
            .filter(|(record_key, _)| record_key == key)
            .map(|(_, result)| result.clone())
    }

    pub(crate) fn invalidate(&mut self) {
        self.record = None;
        self.in_flight = None;
        self.next_id = self.next_id.saturating_add(1).max(1);
    }

    pub(crate) fn invalidate_if_not_key(
        &mut self,
        key: Option<&RootQualificationKey>,
    ) -> RootQualificationInvalidation {
        let mut invalidation = RootQualificationInvalidation::default();
        let record_matches = self
            .record
            .as_ref()
            .is_some_and(|(record_key, _)| key.is_some_and(|expected| record_key == expected));
        let attempt_matches = self
            .in_flight
            .as_ref()
            .is_some_and(|attempt| key.is_some_and(|expected| &attempt.key == expected));
        if !record_matches {
            if let Some((record_key, _)) = self.record.take() {
                invalidation.device_handle = Some(record_key.device_handle);
            }
        }
        if !attempt_matches && self.in_flight.take().is_some() {
            invalidation.cancelled_in_flight = true;
        }
        if invalidation.device_handle.is_some() || invalidation.cancelled_in_flight {
            self.next_id = self.next_id.saturating_add(1).max(1);
        }
        invalidation
    }
}

/// Ensures a reserved root attempt is cancelled if orchestration returns
/// before the sidecar result is committed.
struct RootQualificationAttemptGuard<'a> {
    store: &'a Mutex<RootQualificationStore>,
    attempt: Option<RootQualificationAttempt>,
}

impl<'a> RootQualificationAttemptGuard<'a> {
    fn new(store: &'a Mutex<RootQualificationStore>, attempt: RootQualificationAttempt) -> Self {
        Self {
            store,
            attempt: Some(attempt),
        }
    }

    fn complete(mut self, result: RootQualificationState) -> Result<bool, String> {
        let mut store = self.store.lock().map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?;
        let attempt = self
            .attempt
            .take()
            .expect("root attempt guard must contain an attempt before completion");
        Ok(store.complete(attempt, result))
    }
}

impl Drop for RootQualificationAttemptGuard<'_> {
    fn drop(&mut self) {
        if let Some(attempt) = self.attempt.take() {
            if let Ok(mut store) = self.store.lock() {
                store.cancel(&attempt);
            }
        }
    }
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
    pub root: Option<RootQualificationState>,
    pub runtime_generation: u64,
    pub qualification_revision: u64,
    pub device_identity: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootQualificationCheckDto {
    pub qualification: RootQualificationState,
    pub runtime_generation: u64,
    pub qualification_revision: u64,
    pub device_identity: String,
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
        root: None,
        runtime_generation,
        qualification_revision,
        device_identity: identity.map(ToOwned::to_owned),
    }
}

#[tauri::command]
pub fn get_device_qualification(
    device_handle: Option<String>,
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

    let devices = state
        .handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?
        .qualification_devices();
    if devices.is_empty() {
        return Ok(classify(
            true,
            runtime_generation,
            qualification_revision,
            &[],
        ));
    }
    if devices.len() != 1 {
        let placeholders = [online_placeholder("first"), online_placeholder("second")];
        return Ok(classify(
            true,
            runtime_generation,
            qualification_revision,
            &placeholders,
        ));
    }

    let target = devices.into_iter().next().expect("one device exists");
    if device_handle
        .as_deref()
        .is_some_and(|requested| requested != target.handle)
    {
        return Ok(classify(
            true,
            runtime_generation,
            qualification_revision,
            &[],
        ));
    }

    let identity = target.handle.clone();
    if target.state == "unauthorized" || target.state == "offline" {
        let state = if target.state == "unauthorized" {
            ObservedDeviceState::Unauthorized
        } else {
            ObservedDeviceState::Offline
        };
        let observed = [ObservedDevice {
            opaque_identity: &identity,
            state,
            android_major: None,
            android_api_level: None,
            abi: None,
        }];
        return Ok(classify(
            true,
            runtime_generation,
            qualification_revision,
            &observed,
        ));
    }
    if target.state != "available" {
        return Ok(classify(
            true,
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
            json!({ "adbPath": adb_path, "serial": target.serial }),
        )
        .map_err(|_| {
            safe_error(
                "device_qualification_failed",
                "Connected-device qualification could not be completed.",
            )
        })?;
    let snapshot = classify_observed(
        runtime_generation,
        qualification_revision,
        &observed,
        Some(&identity),
    );
    attach_cached_root(snapshot, &state, &identity)
}

fn attach_cached_root(
    mut snapshot: DeviceQualificationSnapshotDto,
    state: &AppState,
    identity: &str,
) -> Result<DeviceQualificationSnapshotDto, String> {
    if snapshot.state != DeviceQualificationState::Supported {
        return Ok(snapshot);
    }
    let key = RootQualificationKey::new(
        identity,
        snapshot.runtime_generation,
        snapshot.qualification_revision,
    );
    snapshot.root = state
        .root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .get(&key);
    Ok(snapshot)
}

#[tauri::command]
pub fn check_device_root(
    device_handle: String,
    state: State<'_, AppState>,
) -> Result<RootQualificationCheckDto, String> {
    if !cfg!(feature = "real-execution") {
        return Err(safe_error(
            "real_execution_unavailable",
            "Root access checks are unavailable in this development build.",
        ));
    }
    let runtime_generation = state.sidecar.try_generation().map_err(|_| {
        safe_error(
            "runtime_generation_unavailable",
            "Device qualification state is temporarily unavailable.",
        )
    })?;
    let (target, device_count) = {
        let handles = state.handles.lock().map_err(|_| {
            safe_error(
                "session_state_unavailable",
                "Device session state is unavailable.",
            )
        })?;
        let devices = handles.qualification_devices();
        let count = devices.len();
        (
            devices
                .into_iter()
                .find(|device| device.handle == device_handle),
            count,
        )
    };
    if device_count != 1 {
        return Err(safe_error(
            "root_check_requires_one_device",
            "Connect exactly one supported Android device before checking root access.",
        ));
    }
    let target = target.ok_or_else(|| {
        safe_error(
            "device_handle_stale",
            "The selected device changed. Refresh device discovery and try again.",
        )
    })?;
    if target.state != "available" {
        return Err(safe_error(
            "root_check_device_unavailable",
            "The selected device is not ready for a root access check.",
        ));
    }
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
    let key = RootQualificationKey::new(&device_handle, runtime_generation, qualification_revision);
    let adb_path = current_adb_path(&state)?;
    let previous = state
        .root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .get(&key);
    let attempt = state
        .root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .begin(key.clone())
        .map_err(|code| safe_error(code, "A root access check is already in progress."))?;
    let attempt_guard = RootQualificationAttemptGuard::new(&state.root_qualification, attempt);
    let qualification = match state.sidecar.request(
        "checkRoot",
        json!({ "adbPath": adb_path, "serial": target.serial }),
    ) {
        Ok(value) => serde_json::from_value::<RootQualificationState>(value).unwrap_or(
            RootQualificationState::CheckFailed {
                reason: RootQualificationFailureReason::UnexpectedResponse,
                message: "The root access check returned an unexpected response.".to_string(),
            },
        ),
        Err(_) => RootQualificationState::CheckFailed {
            reason: RootQualificationFailureReason::UnexpectedResponse,
            message: "The root access check could not be completed.".to_string(),
        },
    };
    let changed = previous.as_ref() != Some(&qualification);
    let committed = attempt_guard.complete(qualification.clone())?;
    if !committed {
        return Err(safe_error(
            "root_check_stale",
            "The device changed while root access was being checked. Try again.",
        ));
    }
    if changed {
        state
            .handles
            .lock()
            .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?
            .invalidate_reviews_for_device(&device_handle, "root_qualification_changed");
    }
    Ok(RootQualificationCheckDto {
        qualification,
        runtime_generation,
        qualification_revision,
        device_identity: device_handle,
    })
}

fn classify_observed(
    runtime_generation: u64,
    qualification_revision: u64,
    observed: &Value,
    identity: Option<&str>,
) -> DeviceQualificationSnapshotDto {
    let state = observed.get("state").and_then(Value::as_str);
    let devices = match state {
        Some("no_device") => Vec::new(),
        Some("multiple_devices") => vec![online_placeholder("first"), online_placeholder("second")],
        Some("unauthorized") => vec![observed_device(
            observed,
            ObservedDeviceState::Unauthorized,
            identity.unwrap_or("qualification-device"),
        )],
        Some("offline") => vec![observed_device(
            observed,
            ObservedDeviceState::Offline,
            identity.unwrap_or("qualification-device"),
        )],
        Some("online") => vec![observed_device(
            observed,
            ObservedDeviceState::Online,
            identity.unwrap_or("qualification-device"),
        )],
        _ => vec![ObservedDevice {
            opaque_identity: identity.unwrap_or("qualification-device"),
            state: ObservedDeviceState::Online,
            android_major: None,
            android_api_level: None,
            abi: None,
        }],
    };
    classify(true, runtime_generation, qualification_revision, &devices)
}

fn observed_device<'a>(
    observed: &'a Value,
    state: ObservedDeviceState,
    identity: &'a str,
) -> ObservedDevice<'a> {
    ObservedDevice {
        opaque_identity: identity,
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
    use serde_json::json;

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
        assert_eq!(result.root, None);
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
        assert_eq!(result.root, None);
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
    fn root_qualification_serializes_the_approved_compact_shape() {
        let granted = serde_json::to_value(RootQualificationState::Granted).unwrap();
        assert_eq!(granted, json!({ "status": "granted" }));

        let failed = serde_json::to_value(RootQualificationState::CheckFailed {
            reason: RootQualificationFailureReason::TimedOut,
            message: "Root authorization timed out. Try again.".to_string(),
        })
        .unwrap();
        assert_eq!(
            failed,
            json!({
                "status": "checkFailed",
                "reason": "timedOut",
                "message": "Root authorization timed out. Try again."
            })
        );
    }

    #[test]
    fn root_store_is_single_flight_generation_bound_and_invalidatable() {
        let mut store = RootQualificationStore::default();
        let key = RootQualificationKey::new("opaque-device", 4, 9);
        let token = store.begin(key.clone()).unwrap();
        assert!(store.begin(key.clone()).is_err());
        assert!(store.complete(token, RootQualificationState::Granted));
        assert_eq!(store.get(&key), Some(RootQualificationState::Granted));

        store.invalidate();
        assert_eq!(store.get(&key), None);
    }

    #[test]
    fn cancelled_prerequisite_attempt_can_be_retried_without_sidecar_work() {
        let mut store = RootQualificationStore::default();
        let key = RootQualificationKey::new("opaque-device", 4, 9);
        let attempt = store.begin(key.clone()).unwrap();

        assert!(store.cancel(&attempt));
        assert!(!store.cancel(&attempt));
        assert!(store.begin(key).is_ok());
    }

    #[test]
    fn dropping_attempt_guard_cancels_uncommitted_orchestration() {
        let store = Mutex::new(RootQualificationStore::default());
        let key = RootQualificationKey::new("opaque-device", 4, 9);
        let attempt = store.lock().unwrap().begin(key.clone()).unwrap();

        drop(RootQualificationAttemptGuard::new(&store, attempt));

        assert!(store.lock().unwrap().begin(key).is_ok());
    }

    #[test]
    fn stale_cancellation_and_completion_cannot_clear_or_overwrite_new_attempt() {
        let mut store = RootQualificationStore::default();
        let stale = store
            .begin(RootQualificationKey::new("opaque-device", 4, 9))
            .unwrap();
        assert!(store.cancel(&stale));

        let current = store
            .begin(RootQualificationKey::new("opaque-device", 5, 10))
            .unwrap();
        assert!(!store.cancel(&stale));
        assert!(!store.complete(stale, RootQualificationState::Granted));
        assert!(store.complete(current.clone(), RootQualificationState::Denied));
        assert_eq!(
            store.get(&current.key),
            Some(RootQualificationState::Denied)
        );
    }

    #[test]
    fn cancelling_later_attempt_preserves_completed_root_result() {
        let mut store = RootQualificationStore::default();
        let key = RootQualificationKey::new("opaque-device", 4, 9);
        let first = store.begin(key.clone()).unwrap();
        assert!(store.complete(first, RootQualificationState::Granted));

        let later = store.begin(key.clone()).unwrap();
        assert!(store.cancel(&later));
        assert_eq!(store.get(&key), Some(RootQualificationState::Granted));
    }

    #[test]
    fn invalidation_reports_removed_completed_handle_and_inflight_cancellation() {
        let mut store = RootQualificationStore::default();
        let old_key = RootQualificationKey::new("opaque-device-a", 4, 9);
        let old_attempt = store.begin(old_key.clone()).unwrap();
        assert!(store.complete(old_attempt, RootQualificationState::Granted));
        let newer_attempt = store
            .begin(RootQualificationKey::new("opaque-device-b", 5, 10))
            .unwrap();

        let invalidation = store.invalidate_if_not_key(None);
        assert_eq!(
            invalidation.device_handle.as_deref(),
            Some("opaque-device-a")
        );
        assert!(invalidation.cancelled_in_flight);
        assert!(!store.complete(newer_attempt, RootQualificationState::Denied));
    }

    #[test]
    fn matching_key_preserves_completed_and_inflight_evidence() {
        let mut store = RootQualificationStore::default();
        let key = RootQualificationKey::new("opaque-device", 4, 9);
        let completed = store.begin(key.clone()).unwrap();
        assert!(store.complete(completed, RootQualificationState::Granted));
        let in_flight = store.begin(key.clone()).unwrap();

        assert_eq!(store.invalidate_if_not_key(Some(&key)), Default::default());
        assert_eq!(store.get(&key), Some(RootQualificationState::Granted));
        assert!(store.complete(in_flight, RootQualificationState::Granted));
    }

    #[test]
    fn invalidating_only_an_inflight_attempt_does_not_report_a_review_device() {
        let mut store = RootQualificationStore::default();
        let attempt = store
            .begin(RootQualificationKey::new("opaque-device", 4, 9))
            .unwrap();

        let invalidation = store.invalidate_if_not_key(None);
        assert_eq!(invalidation.device_handle, None);
        assert!(invalidation.cancelled_in_flight);
        assert!(!store.complete(attempt, RootQualificationState::Granted));
    }

    #[test]
    fn observed_serial_is_never_used_as_the_frontend_device_identity() {
        let snapshot = classify_observed(
            3,
            8,
            &json!({
                "state": "online",
                "serial": "exact-sensitive-serial",
                "androidMajor": 14,
                "androidApiLevel": 34,
                "abi": "arm64-v8a"
            }),
            Some("opaque-device-handle"),
        );
        assert_eq!(
            snapshot.device_identity.as_deref(),
            Some("opaque-device-handle")
        );
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("exact-sensitive-serial"));
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
            None,
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
        let result = classify_observed(2, 3, &json!({ "state": "multiple_devices" }), None);
        assert_eq!(
            result.state,
            DeviceQualificationState::InsufficientlyQualified
        );
        assert_eq!(result.device_identity, None);
    }
}
