//! Explicit production root authority for device qualification.
//!
//! Root authorization state stays native-owned and generation-bound: the
//! sidecar performs one bounded probe per attempt and this module retains only
//! typed results. Passive device interpretation lives in the device observation
//! module; this module never infers root state from passive device facts.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

use crate::commands::{current_adb_path, safe_error, AppState};
use crate::device_observation::{
    refresh_current_qualification, DeviceQualificationState, QualificationContextKey,
};
use crate::handles::{DeviceDto, SessionHandles};

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
pub struct RootQualificationCheckDto {
    pub qualification: RootQualificationState,
    pub runtime_generation: u64,
    pub qualification_revision: u64,
    pub device_identity: String,
    #[serde(skip)]
    pub(crate) session_epoch: u64,
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
    let _transition = crate::commands::qualification_transition_lock(&state);
    {
        let handles = state.handles.lock().map_err(|_| {
            safe_error("session_state_unavailable", "Session state is unavailable.")
        })?;
        let current_device = handles.device(device_handle).map_err(|_| {
            safe_error(
                "root_check_stale",
                "The device changed while root access was being checked. Try again.",
            )
        })?;
        if current_device.state != "available"
            || current_device.session_epoch != context.session_epoch
            || current_device.serial != target.serial
        {
            return Err(safe_error(
                "root_check_stale",
                "The device changed while root access was being checked. Try again.",
            ));
        }
    }
    let committed = attempt_guard.complete(qualification.clone())?;
    if !committed {
        return Err(safe_error(
            "root_check_stale",
            "The device changed while root access was being checked. Try again.",
        ));
    }
    let mut handles = state
        .handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?;
    let current_device = handles.device(device_handle).map_err(|_| {
        safe_error(
            "root_check_stale",
            "The device changed while root access was being checked. Try again.",
        )
    })?;
    if current_device.state != "available"
        || current_device.session_epoch != context.session_epoch
        || current_device.serial != target.serial
    {
        return Err(safe_error(
            "root_check_stale",
            "The device changed while root access was being checked. Try again.",
        ));
    }
    if changed {
        handles.invalidate_reviews_for_device(device_handle, "root_qualification_changed");
    }
    drop(handles);
    // Feed the committed explicit root-check result to the active attempt. The
    // root check stays the only root authority; qualification only observes the
    // result it committed.
    crate::qualification_session::observe(
        state,
        crate::qualification_session::QualificationLifecycleObservation::RootChecked {
            device_handle: device_handle.to_string(),
            session_epoch: context.session_epoch,
            root_state: qualification.clone(),
        },
    );
    Ok(RootQualificationCheckDto {
        qualification,
        runtime_generation,
        qualification_revision: context.qualification_revision,
        device_identity: device_handle.to_string(),
        session_epoch: context.session_epoch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_observation::QualificationContextKey;
    use serde_json::json;

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
}
