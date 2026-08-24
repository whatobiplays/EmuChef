//! Backend-owned, read-only device qualification projection.
//!
//! Passive qualification and root authorization state stay native-owned. The
//! sidecar performs bounded ADB inspection; this module projects only sanitized
//! facts and generation-bound root evidence to the React client.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

use crate::commands::{current_adb_path, list_and_reconcile_inventory, safe_error, AppState};
use crate::handles::{DeviceDto, SessionHandles};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CapabilityOutcome {
    Available,
    Unsupported,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityAvailabilityDto {
    Available,
    Unavailable,
    Unknown,
}

impl CapabilityOutcome {
    fn dto(self) -> CapabilityAvailabilityDto {
        match self {
            Self::Available => CapabilityAvailabilityDto::Available,
            Self::Unsupported => CapabilityAvailabilityDto::Unavailable,
            Self::Unknown => CapabilityAvailabilityDto::Unknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QualificationContextKey {
    pub(crate) device_handle: String,
    pub(crate) session_epoch: u64,
    pub(crate) runtime_generation: u64,
    pub(crate) platform_tools_revision: u64,
    pub(crate) qualification_revision: u64,
    pub(crate) capability_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CurrentQualification {
    pub(crate) snapshot: DeviceQualificationSnapshotDto,
    pub(crate) context: Option<QualificationContextKey>,
}

/// Apply one inventory to native session and root authority using explicit
/// generation inputs. The execution seam uses this same function with a
/// deterministic runtime requester.
pub(crate) fn reconcile_inventory_with_context(
    handles: &Mutex<SessionHandles>,
    root_qualification: &Mutex<RootQualificationStore>,
    inventory: &Value,
    runtime_generation: u64,
    platform_tools_revision: u64,
) -> Result<Vec<DeviceDto>, String> {
    let (devices, current_key) = {
        let mut handles = handles.lock().map_err(|_| {
            safe_error("session_state_unavailable", "Session state is unavailable.")
        })?;
        let devices = handles.update_devices(inventory)?;
        let current_key = handles
            .single_available_device_handle()
            .and_then(|handle| handles.qualification_context(&handle))
            .filter(|context| {
                context.runtime_generation == runtime_generation
                    && context.platform_tools_revision == platform_tools_revision
            })
            .map(|context| RootQualificationKey::from_context(&context));
        (devices, current_key)
    };
    let invalidation = root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .invalidate_if_not_key(current_key.as_ref());
    if let Some(device_handle) = invalidation.device_handle.as_deref() {
        handles
            .lock()
            .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?
            .invalidate_reviews_for_device(device_handle, "root_qualification_changed");
    }
    Ok(devices)
}

impl QualificationContextKey {
    pub(crate) fn new(
        device_handle: impl Into<String>,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
        qualification_revision: u64,
        capability_fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            device_handle: device_handle.into(),
            session_epoch,
            runtime_generation,
            platform_tools_revision,
            qualification_revision,
            capability_fingerprint: capability_fingerprint.into(),
        }
    }
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RootQualificationKey {
    pub(crate) device_handle: String,
    pub(crate) runtime_generation: u64,
    pub(crate) qualification_revision: u64,
    pub(crate) session_epoch: u64,
    pub(crate) capability_fingerprint: String,
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
            session_epoch: 0,
            capability_fingerprint: String::new(),
        }
    }

    pub(crate) fn from_context(context: &QualificationContextKey) -> Self {
        Self {
            device_handle: context.device_handle.clone(),
            runtime_generation: context.runtime_generation,
            qualification_revision: context.qualification_revision,
            session_epoch: context.session_epoch,
            capability_fingerprint: context.capability_fingerprint.clone(),
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

/// Retains generation-bound root results independently for each device context
/// while serializing the native check that can be in flight at one time.
#[derive(Debug, Default)]
pub(crate) struct RootQualificationStore {
    records: HashMap<RootQualificationKey, RootQualificationState>,
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
        self.records.insert(attempt.key, result);
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
        self.records.get(key).cloned()
    }

    pub(crate) fn invalidate(&mut self) {
        self.records.clear();
        self.in_flight = None;
        self.next_id = self.next_id.saturating_add(1).max(1);
    }

    /// Invalidate completed and in-flight root evidence for one device handle.
    /// Bumping the attempt generation fences any late completion callback.
    pub(crate) fn invalidate_for_device(
        &mut self,
        device_handle: &str,
    ) -> RootQualificationInvalidation {
        let mut invalidation = RootQualificationInvalidation::default();
        let removed_record = self
            .records
            .keys()
            .any(|key| key.device_handle == device_handle);
        if removed_record {
            self.records
                .retain(|key, _| key.device_handle != device_handle);
            invalidation.device_handle = Some(device_handle.to_string());
        }
        if self
            .in_flight
            .as_ref()
            .is_some_and(|attempt| attempt.key.device_handle == device_handle)
        {
            self.in_flight = None;
            invalidation.cancelled_in_flight = true;
        }
        if invalidation.device_handle.is_some() || invalidation.cancelled_in_flight {
            self.next_id = self.next_id.saturating_add(1).max(1);
        }
        invalidation
    }

    pub(crate) fn invalidate_if_not_key(
        &mut self,
        key: Option<&RootQualificationKey>,
    ) -> RootQualificationInvalidation {
        let mut invalidation = RootQualificationInvalidation::default();
        let removed_device = self
            .records
            .keys()
            .find(|record_key| key.is_none_or(|expected| *record_key != expected))
            .map(|record_key| record_key.device_handle.clone());
        if removed_device.is_some() {
            self.records
                .retain(|record_key, _| key.is_some_and(|expected| record_key == expected));
            invalidation.device_handle = removed_device;
        }
        let attempt_matches = self
            .in_flight
            .as_ref()
            .is_some_and(|attempt| key.is_some_and(|expected| &attempt.key == expected));
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
    pub storage: CapabilityAvailabilityDto,
    pub package_manager: CapabilityAvailabilityDto,
    pub activity_manager: CapabilityAvailabilityDto,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObservedQualification<'a> {
    pub opaque_identity: &'a str,
    pub state: ObservedDeviceState,
    pub android_major: Option<u32>,
    pub android_api_level: Option<u32>,
    pub abi: Option<&'a str>,
    pub storage: CapabilityOutcome,
    pub package_manager: CapabilityOutcome,
    pub activity_manager: CapabilityOutcome,
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

/// Classify the complete passive qualification profile. Explicit negative
/// capabilities are unsupported; unknown or incomplete probes remain
/// insufficiently qualified and can never authorize execution.
pub(crate) fn classify_complete(
    compiled: bool,
    runtime_generation: u64,
    qualification_revision: u64,
    observed: &[ObservedQualification<'_>],
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
    if observed.is_empty() {
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
    if observed.len() != 1 {
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
    let device = &observed[0];
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
            let Some(android_major) = device.android_major else {
                return with_capabilities(
                    snapshot(
                        DeviceQualificationState::InsufficientlyQualified,
                        "The connected device could not be fully qualified.",
                        vec!["Refresh discovery before attempting real execution."],
                        runtime_generation,
                        qualification_revision,
                        Some(device.opaque_identity),
                        None,
                        device.android_api_level,
                        normalize_abi(device.abi),
                    ),
                    device,
                );
            };
            let Some(api_level) = device.android_api_level else {
                return with_capabilities(
                    snapshot(
                        DeviceQualificationState::InsufficientlyQualified,
                        "The connected device could not be fully qualified.",
                        vec!["Refresh discovery before attempting real execution."],
                        runtime_generation,
                        qualification_revision,
                        Some(device.opaque_identity),
                        Some(android_major),
                        None,
                        normalize_abi(device.abi),
                    ),
                    device,
                );
            };
            let Some(abi_class) = normalize_abi(device.abi) else {
                return with_capabilities(
                    snapshot(
                        DeviceQualificationState::Unsupported,
                        "The connected device uses an unsupported processor architecture.",
                        vec!["This device cannot begin real execution."],
                        runtime_generation,
                        qualification_revision,
                        Some(device.opaque_identity),
                        Some(android_major),
                        Some(api_level),
                        None,
                    ),
                    device,
                );
            };
            if android_major < 11 || api_level < 30 {
                return with_capabilities(
                    snapshot(
                        DeviceQualificationState::Unsupported,
                        "The connected device uses an unsupported Android version.",
                        vec!["EmuChef requires Android 11 or newer."],
                        runtime_generation,
                        qualification_revision,
                        Some(device.opaque_identity),
                        Some(android_major),
                        Some(api_level),
                        Some(abi_class),
                    ),
                    device,
                );
            }
            let capabilities = [
                device.storage,
                device.package_manager,
                device.activity_manager,
            ];
            let state = if capabilities.contains(&CapabilityOutcome::Unsupported) {
                DeviceQualificationState::Unsupported
            } else if capabilities.contains(&CapabilityOutcome::Unknown) {
                DeviceQualificationState::InsufficientlyQualified
            } else {
                DeviceQualificationState::Supported
            };
            let limitations = match state {
                DeviceQualificationState::Supported => vec!["Root access has not been checked."],
                DeviceQualificationState::Unsupported => {
                    vec!["A required device capability is unavailable."]
                }
                _ => vec!["Refresh discovery before attempting real execution."],
            };
            let mut result = snapshot(
                state,
                match state {
                    DeviceQualificationState::Supported => {
                        "The connected device meets the current qualification requirements."
                    }
                    DeviceQualificationState::Unsupported => {
                        "The connected device does not provide every required capability."
                    }
                    _ => "The connected device could not be fully qualified.",
                },
                limitations,
                runtime_generation,
                qualification_revision,
                Some(device.opaque_identity),
                Some(android_major),
                Some(api_level),
                Some(abi_class),
            );
            result.storage = device.storage.dto();
            result.package_manager = device.package_manager.dto();
            result.activity_manager = device.activity_manager.dto();
            result
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
        storage: CapabilityAvailabilityDto::Unknown,
        package_manager: CapabilityAvailabilityDto::Unknown,
        activity_manager: CapabilityAvailabilityDto::Unknown,
        root: None,
        runtime_generation,
        qualification_revision,
        device_identity: identity.map(ToOwned::to_owned),
    }
}

fn with_capabilities(
    mut snapshot: DeviceQualificationSnapshotDto,
    device: &ObservedQualification<'_>,
) -> DeviceQualificationSnapshotDto {
    snapshot.storage = device.storage.dto();
    snapshot.package_manager = device.package_manager.dto();
    snapshot.activity_manager = device.activity_manager.dto();
    snapshot
}

#[tauri::command]
pub fn get_device_qualification(
    device_handle: Option<String>,
    state: State<'_, AppState>,
) -> Result<DeviceQualificationSnapshotDto, String> {
    refresh_current_qualification(&state, device_handle.as_deref()).map(|result| result.snapshot)
}

/// Re-run the complete passive qualification against the current native
/// session. This is shared by the UI projection, explicit root checks, and the
/// final real-execution preflight so those paths cannot drift apart.
pub(crate) fn refresh_current_qualification(
    state: &AppState,
    requested_handle: Option<&str>,
) -> Result<CurrentQualification, String> {
    let mut request =
        |request_type: &str, payload: Value| state.sidecar.request(request_type, payload);
    refresh_current_qualification_with_runtime(state, requested_handle, &mut request)
}

/// Refresh qualification after reconciling one authoritative inventory. The
/// request function is injectable so the execution preflight can share the
/// exact sidecar request boundary with deterministic tests.
pub(crate) fn refresh_current_qualification_with_runtime<F>(
    state: &AppState,
    requested_handle: Option<&str>,
    request: &mut F,
) -> Result<CurrentQualification, String>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
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
        let mut handles = state.handles.lock().map_err(|_| {
            safe_error("session_state_unavailable", "Session state is unavailable.")
        })?;
        for device in handles.qualification_devices() {
            handles.clear_qualification_context(&device.handle);
            handles.invalidate_reviews_for_device(&device.handle, "device_qualification_changed");
        }
        drop(handles);
        state
            .root_qualification
            .lock()
            .map_err(|_| {
                safe_error(
                    "qualification_state_unavailable",
                    "Device qualification state is unavailable.",
                )
            })?
            .invalidate();
        return Ok(CurrentQualification {
            snapshot: classify(false, runtime_generation, qualification_revision, &[]),
            context: None,
        });
    }

    // The inventory used to resolve the reviewed opaque handle must be fresh
    // and native-authoritative. A targeted qualifyDevice call is deliberately
    // separate; its internal listing is not a continuity authority.
    list_and_reconcile_inventory(state, request)?;

    let adb_path = current_adb_path(state)?;
    qualify_reconciled_current_with_runtime(
        &state.handles,
        &state.root_qualification,
        &adb_path,
        runtime_generation,
        qualification_revision,
        requested_handle,
        request,
    )
}

/// Qualify the single target from an already reconciled native inventory.
/// Keeping this separate from inventory listing makes the final execution
/// gate prove continuity before it performs the targeted capability probe.
pub(crate) fn qualify_reconciled_current_with_runtime<F>(
    handles: &Mutex<SessionHandles>,
    root_qualification: &Mutex<RootQualificationStore>,
    adb_path: &str,
    runtime_generation: u64,
    qualification_revision: u64,
    requested_handle: Option<&str>,
    request: &mut F,
) -> Result<CurrentQualification, String>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let devices = handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?
        .qualification_devices();
    if devices.is_empty() {
        let invalidation = root_qualification
            .lock()
            .map_err(|_| {
                safe_error(
                    "qualification_state_unavailable",
                    "Device qualification state is unavailable.",
                )
            })?
            .invalidate_if_not_key(None);
        if let Some(device_handle) = invalidation.device_handle.as_deref() {
            handles
                .lock()
                .map_err(|_| {
                    safe_error("session_state_unavailable", "Session state is unavailable.")
                })?
                .invalidate_reviews_for_device(device_handle, "root_qualification_changed");
        }
        return Ok(CurrentQualification {
            snapshot: classify_complete(true, runtime_generation, qualification_revision, &[]),
            context: None,
        });
    }
    if devices.len() != 1 {
        let first = ObservedQualification {
            opaque_identity: "qualification-device-1",
            state: ObservedDeviceState::Online,
            android_major: None,
            android_api_level: None,
            abi: None,
            storage: CapabilityOutcome::Unknown,
            package_manager: CapabilityOutcome::Unknown,
            activity_manager: CapabilityOutcome::Unknown,
        };
        let second = ObservedQualification {
            opaque_identity: "qualification-device-2",
            ..first.clone()
        };
        let invalidation = root_qualification
            .lock()
            .map_err(|_| {
                safe_error(
                    "qualification_state_unavailable",
                    "Device qualification state is unavailable.",
                )
            })?
            .invalidate_if_not_key(None);
        if let Some(device_handle) = invalidation.device_handle.as_deref() {
            handles
                .lock()
                .map_err(|_| {
                    safe_error("session_state_unavailable", "Session state is unavailable.")
                })?
                .invalidate_reviews_for_device(device_handle, "root_qualification_changed");
        }
        return Ok(CurrentQualification {
            snapshot: classify_complete(
                true,
                runtime_generation,
                qualification_revision,
                &[first, second],
            ),
            context: None,
        });
    }

    let target = devices.into_iter().next().expect("one device exists");
    if requested_handle.is_some_and(|requested| requested != target.handle) {
        return Ok(CurrentQualification {
            snapshot: classify_complete(true, runtime_generation, qualification_revision, &[]),
            context: None,
        });
    }

    let identity = target.handle.clone();
    if target.state == "unauthorized" || target.state == "offline" {
        let state = if target.state == "unauthorized" {
            ObservedDeviceState::Unauthorized
        } else {
            ObservedDeviceState::Offline
        };
        let observed = [ObservedQualification {
            opaque_identity: &identity,
            state,
            android_major: None,
            android_api_level: None,
            abi: None,
            storage: CapabilityOutcome::Unknown,
            package_manager: CapabilityOutcome::Unknown,
            activity_manager: CapabilityOutcome::Unknown,
        }];
        return Ok(CurrentQualification {
            snapshot: classify_complete(
                true,
                runtime_generation,
                qualification_revision,
                &observed,
            ),
            context: None,
        });
    }
    if target.state != "available" {
        return Ok(CurrentQualification {
            snapshot: classify_complete(true, runtime_generation, qualification_revision, &[]),
            context: None,
        });
    }

    let observed = request(
        "qualifyDevice",
        json!({ "adbPath": adb_path, "serial": target.serial }),
    )
    .map_err(|_| {
        safe_error(
            "device_qualification_failed",
            "Connected-device qualification could not be completed.",
        )
    })?;
    let (mut snapshot, observed_qualification) = classify_observed_complete(
        runtime_generation,
        qualification_revision,
        &observed,
        Some(&identity),
    );
    let context = QualificationContextKey::new(
        &identity,
        target.session_epoch,
        runtime_generation,
        qualification_revision,
        qualification_revision,
        qualification_fingerprint(&observed_qualification),
    );
    let root_key = RootQualificationKey::from_context(&context);
    handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?
        .set_qualification_context(context.clone());
    let root_invalidation = root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .invalidate_if_not_key(Some(&root_key));
    if root_invalidation.device_handle.is_some() {
        handles
            .lock()
            .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?
            .invalidate_reviews_for_device(&identity, "root_qualification_changed");
    }
    snapshot.root = root_qualification
        .lock()
        .map_err(|_| {
            safe_error(
                "qualification_state_unavailable",
                "Device qualification state is unavailable.",
            )
        })?
        .get(&root_key);
    Ok(CurrentQualification {
        snapshot,
        context: Some(context),
    })
}

#[tauri::command]
pub fn check_device_root(
    device_handle: String,
    state: State<'_, AppState>,
) -> Result<RootQualificationCheckDto, String> {
    check_device_root_observation(&device_handle, &state)
}

/// Run the existing authoritative root check for a selected native device.
/// The public command and qualification target capture share this function so
/// root state cannot be inferred or attested through a second authority.
pub(crate) fn check_device_root_observation(
    device_handle: &str,
    state: &AppState,
) -> Result<RootQualificationCheckDto, String> {
    if !cfg!(feature = "real-execution") {
        return Err(safe_error(
            "real_execution_unavailable",
            "Root access checks are unavailable in this development build.",
        ));
    }
    let current = refresh_current_qualification(state, Some(device_handle))?;
    if current.snapshot.state != DeviceQualificationState::Supported {
        return Err(safe_error(
            "device_qualification_incomplete",
            "Complete supported device qualification before checking root access.",
        ));
    }
    let context = current.context.as_ref().ok_or_else(|| {
        safe_error(
            "device_qualification_incomplete",
            "Complete supported device qualification before checking root access.",
        )
    })?;
    let runtime_generation = context.runtime_generation;
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
    let key = RootQualificationKey::from_context(context);
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
            .invalidate_reviews_for_device(device_handle, "root_qualification_changed");
    }
    Ok(RootQualificationCheckDto {
        qualification,
        runtime_generation,
        qualification_revision: context.qualification_revision,
        device_identity: device_handle.to_string(),
    })
}

fn classify_observed_complete<'a>(
    runtime_generation: u64,
    qualification_revision: u64,
    observed: &'a Value,
    identity: Option<&'a str>,
) -> (DeviceQualificationSnapshotDto, ObservedQualification<'a>) {
    let state = observed.get("state").and_then(Value::as_str);
    let identity = identity.unwrap_or("qualification-device");
    let make = |state| ObservedQualification {
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
        storage: capability_outcome(observed.get("storage")),
        package_manager: capability_outcome(observed.get("packageManager")),
        activity_manager: capability_outcome(observed.get("activityManager")),
    };
    let profile = make(match state {
        Some("unauthorized") => ObservedDeviceState::Unauthorized,
        Some("offline") => ObservedDeviceState::Offline,
        _ => ObservedDeviceState::Online,
    });
    let snapshot = match state {
        Some("no_device") => {
            classify_complete(true, runtime_generation, qualification_revision, &[])
        }
        Some("multiple_devices") => {
            let second = ObservedQualification {
                opaque_identity: "qualification-device-2",
                ..profile.clone()
            };
            classify_complete(
                true,
                runtime_generation,
                qualification_revision,
                &[profile.clone(), second],
            )
        }
        _ => classify_complete(
            true,
            runtime_generation,
            qualification_revision,
            std::slice::from_ref(&profile),
        ),
    };
    (snapshot, profile)
}

/// Compatibility projection used by legacy unit fixtures. Production paths use
/// `classify_observed_complete`, which includes all required capability probes.
#[cfg(test)]
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
        _ => vec![observed_device(
            observed,
            ObservedDeviceState::Online,
            identity.unwrap_or("qualification-device"),
        )],
    };
    classify(true, runtime_generation, qualification_revision, &devices)
}

fn capability_outcome(value: Option<&Value>) -> CapabilityOutcome {
    match value.and_then(Value::as_str) {
        Some("available") => CapabilityOutcome::Available,
        Some("unsupported") => CapabilityOutcome::Unsupported,
        _ => CapabilityOutcome::Unknown,
    }
}

fn qualification_fingerprint(observed: &ObservedQualification<'_>) -> String {
    let payload = json!({
        "androidMajor": observed.android_major,
        "androidApiLevel": observed.android_api_level,
        "abi": observed.abi,
        "storage": format!("{:?}", observed.storage),
        "packageManager": format!("{:?}", observed.package_manager),
        "activityManager": format!("{:?}", observed.activity_manager),
    });
    hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).expect("qualification fingerprint is serializable"),
    ))
}

#[cfg(test)]
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

#[cfg(test)]
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
pub(crate) fn test_current_qualification(
    state: DeviceQualificationState,
    context: Option<QualificationContextKey>,
) -> CurrentQualification {
    CurrentQualification {
        snapshot: DeviceQualificationSnapshotDto {
            state,
            summary: "test",
            limitations: Vec::new(),
            android_major: Some(14),
            android_api_level: Some(34),
            abi_class: Some("arm64"),
            storage: CapabilityAvailabilityDto::Available,
            package_manager: CapabilityAvailabilityDto::Available,
            activity_manager: CapabilityAvailabilityDto::Available,
            root: None,
            runtime_generation: 1,
            qualification_revision: 2,
            device_identity: Some("device_one".to_string()),
        },
        context,
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
    fn complete_supported_profile_requires_all_passive_capabilities() {
        let supported = ObservedQualification {
            opaque_identity: "opaque",
            state: ObservedDeviceState::Online,
            android_major: Some(14),
            android_api_level: Some(34),
            abi: Some("arm64-v8a"),
            storage: CapabilityOutcome::Available,
            package_manager: CapabilityOutcome::Available,
            activity_manager: CapabilityOutcome::Available,
        };
        let result = classify_complete(true, 8, 13, std::slice::from_ref(&supported));
        assert_eq!(result.state, DeviceQualificationState::Supported);
        assert_eq!(result.storage, CapabilityAvailabilityDto::Available);
        assert_eq!(result.package_manager, CapabilityAvailabilityDto::Available);
        assert_eq!(
            result.activity_manager,
            CapabilityAvailabilityDto::Available
        );

        for (field, capability) in [
            ("storage", CapabilityOutcome::Unsupported),
            ("package_manager", CapabilityOutcome::Unsupported),
            ("activity_manager", CapabilityOutcome::Unsupported),
        ] {
            let mut candidate = supported.clone();
            match field {
                "storage" => candidate.storage = capability,
                "package_manager" => candidate.package_manager = capability,
                _ => candidate.activity_manager = capability,
            }
            assert_eq!(
                classify_complete(true, 8, 13, &[candidate]).state,
                DeviceQualificationState::Unsupported,
                "{field} negative result must be unsupported"
            );
        }

        let mut unknown = supported;
        unknown.package_manager = CapabilityOutcome::Unknown;
        assert_eq!(
            classify_complete(true, 8, 13, &[unknown]).state,
            DeviceQualificationState::InsufficientlyQualified
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
    fn identity_invalidation_fences_late_root_completion_for_only_the_matching_handle() {
        let mut store = RootQualificationStore::default();
        let matching_key = RootQualificationKey::new("opaque-device-a", 4, 9);
        let completed = store.begin(matching_key.clone()).unwrap();
        assert!(store.complete(completed, RootQualificationState::Granted));
        let late_attempt = store.begin(matching_key.clone()).unwrap();

        let invalidation = store.invalidate_for_device("opaque-device-a");

        assert_eq!(
            invalidation.device_handle.as_deref(),
            Some("opaque-device-a")
        );
        assert!(invalidation.cancelled_in_flight);
        assert_eq!(store.get(&matching_key), None);
        assert!(!store.complete(late_attempt, RootQualificationState::Denied));

        let unrelated_key = RootQualificationKey::new("opaque-device-b", 4, 9);
        let unrelated = store.begin(unrelated_key.clone()).unwrap();
        assert!(store.complete(unrelated, RootQualificationState::Granted));
        let unrelated_attempt = store.begin(unrelated_key.clone()).unwrap();
        assert_eq!(
            store.invalidate_for_device("opaque-device-a"),
            Default::default()
        );
        assert!(store.complete(unrelated_attempt, RootQualificationState::Granted));
        assert_eq!(
            store.get(&unrelated_key),
            Some(RootQualificationState::Granted)
        );
    }

    #[test]
    fn root_key_rejects_a_multiple_device_session_context() {
        let old_context = QualificationContextKey::new("opaque-device", 4, 8, 9, 9, "old");
        let new_context = QualificationContextKey::new("opaque-device", 5, 8, 9, 9, "new");
        let old_key = RootQualificationKey::from_context(&old_context);
        let new_key = RootQualificationKey::from_context(&new_context);
        let mut store = RootQualificationStore::default();
        let attempt = store.begin(old_key.clone()).unwrap();
        assert!(store.complete(attempt, RootQualificationState::Granted));

        let invalidation = store.invalidate_if_not_key(Some(&new_key));

        assert_eq!(invalidation.device_handle.as_deref(), Some("opaque-device"));
        assert_eq!(store.get(&old_key), None);
        assert_eq!(store.get(&new_key), None);
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
