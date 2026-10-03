//! Typed passive device observation for EmuChef.
//!
//! This module owns the complete passive device-observation path: probing
//! device facts through the trusted sidecar boundary, retaining typed facts for
//! the current native session, matching the selected device against authored
//! device profiles, interpreting passive support and capability availability,
//! and projecting sanitized DTOs to the React client. Explicit root checking
//! remains separate production authority owned by the device qualification
//! module; this module may consume already-established root state but never
//! initiates, infers, or duplicates a root check.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use tauri::State;

use crate::commands::{current_adb_path, list_and_reconcile_inventory, safe_error, AppState};
use crate::device_qualification::{
    RootQualificationKey, RootQualificationState, RootQualificationStore,
};
use crate::handles::SessionHandles;

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

impl Default for CapabilityOutcome {
    fn default() -> Self {
        Self::Unknown
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservedDeviceState {
    Unauthorized,
    Offline,
    Online,
    /// The observation did not establish a recognized online/offline state.
    Unverified,
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

/// Typed decoding of one trusted sidecar device-qualification payload.
///
/// This is the production `qualifyDevice` result consumed by passive
/// classification. Every fact is optional because a qualification probe can
/// fail to establish an individual fact, and an absent fact must never be
/// treated as an observed value. Unknown fields are ignored because the sidecar
/// payload is a superset of what passive classification consumes.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualifyDeviceObservation {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    android_major: Option<u32>,
    #[serde(default)]
    android_api_level: Option<u32>,
    #[serde(default)]
    abi: Option<String>,
    #[serde(default, deserialize_with = "deserialize_capability")]
    storage: CapabilityOutcome,
    #[serde(default, deserialize_with = "deserialize_capability")]
    package_manager: CapabilityOutcome,
    #[serde(default, deserialize_with = "deserialize_capability")]
    activity_manager: CapabilityOutcome,
}

impl QualifyDeviceObservation {
    /// Decode the consumed subset of one trusted qualification payload. A
    /// payload that does not decode establishes no passive facts, so callers
    /// must treat `None` as an unverified device rather than inventing values.
    pub(crate) fn decode(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// Decode one capability availability value from a qualification payload. The
/// contract classifies only `available` and `unsupported`; every other
/// spelling, including a missing or null value, is unknown because it
/// establishes no capability.
fn deserialize_capability<'de, D>(deserializer: D) -> Result<CapabilityOutcome, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(match value.as_deref() {
        Some("available") => CapabilityOutcome::Available,
        Some("unsupported") => CapabilityOutcome::Unsupported,
        _ => CapabilityOutcome::Unknown,
    })
}

/// Trusted passive identity facts established by one production device probe.
///
/// This is the typed decoding of the sidecar device-probe payload consumed by
/// qualification. The sidecar payload is a superset of what qualification
/// consumes, so unknown fields are ignored. Every consumed fact is optional
/// because a probe can fail to establish an individual fact; an absent fact is
/// never treated as an observed value. The exact serial is deliberately not a
/// field: it never crosses this boundary.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub(crate) struct DeviceProbeFacts {
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_number")]
    android_version: Option<String>,
    #[serde(default)]
    android_api_level: Option<u64>,
    #[serde(default)]
    firmware_build: Option<String>,
}

impl DeviceProbeFacts {
    /// Decode the consumed subset of one trusted probe payload. A payload that
    /// does not decode establishes no identity facts, so callers must treat
    /// `None` as an unverified device rather than inventing values.
    pub(crate) fn decode(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// Decode one Android release fact, which trusted probes spell as either a
/// string or a number. Values of any other shape establish no version.
fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(Value::String(value)) if !value.is_empty() => Some(value),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    })
}

/// One authored plan candidate resolved by the trusted catalog match.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DevicePlanMatch {
    #[serde(default)]
    pub(crate) plan_id: String,
    #[serde(default)]
    pub(crate) profile_id: Option<String>,
}

/// Typed decoding of the trusted catalog match projection consumed by
/// qualification.
///
/// The public match DTO carries more presentation fields than qualification
/// consumes; this projection keeps only the plan identity facts. Every group is
/// optional so a payload that omits or nulls a group never fails decoding.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceMatchProjection {
    #[serde(default)]
    candidates: Option<Vec<DevicePlanMatch>>,
    #[serde(default)]
    safe_generic_plans: Option<Vec<DevicePlanMatch>>,
    #[serde(default)]
    blank_setup_plans: Option<Vec<DevicePlanMatch>>,
}

impl DeviceMatchProjection {
    /// Decode the consumed subset of one trusted public match projection.
    pub(crate) fn decode(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    /// Every authored plan candidate the match established, in trusted order.
    fn plans(&self) -> impl Iterator<Item = &DevicePlanMatch> {
        self.candidates
            .iter()
            .chain(self.safe_generic_plans.iter())
            .chain(self.blank_setup_plans.iter())
            .flatten()
    }
}

/// One typed authoritative passive observation of the currently selected
/// device.
///
/// Every field except the opaque device handle is optional because the trusted
/// seams that observe a device do not all establish the same facts: a
/// production probe establishes identity facts, a catalog match establishes
/// the authored profile identity, and a support qualification establishes the
/// ABI class and the retained root projection. Consumers such as the
/// qualification session compare only the facts an observation actually
/// carries, so a partial observation can never assert a fact it did not
/// observe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectedDeviceObservation {
    pub(crate) device_handle: String,
    /// Native inventory epoch associated with this observation. It is
    /// process-local continuity metadata and never becomes candidate evidence.
    pub(crate) session_epoch: Option<u64>,
    pub(crate) profile_id: Option<String>,
    pub(crate) manufacturer: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) android_version: Option<String>,
    pub(crate) android_api: Option<u64>,
    pub(crate) abi_soc_class: Option<String>,
    pub(crate) firmware_build: Option<String>,
    /// Committed explicit root-check result for this device, when root
    /// authority already established one. This module never probes for root.
    pub(crate) root_state: Option<RootQualificationState>,
}

impl SelectedDeviceObservation {
    /// Start an observation for one selected device.
    pub(crate) fn new(device_handle: impl Into<String>) -> Self {
        Self {
            device_handle: device_handle.into(),
            session_epoch: None,
            profile_id: None,
            manufacturer: None,
            model: None,
            android_version: None,
            android_api: None,
            abi_soc_class: None,
            firmware_build: None,
            root_state: None,
        }
    }

    /// Attach the trusted identity facts established by one production probe.
    pub(crate) fn with_probe_facts(mut self, facts: &DeviceProbeFacts) -> Self {
        self.manufacturer = facts.manufacturer.clone();
        self.model = facts.model.clone();
        self.android_version = facts.android_version.clone();
        self.android_api = facts.android_api_level;
        self.firmware_build = facts.firmware_build.clone();
        self
    }

    /// Attach the authored profile identity established by a catalog match.
    pub(crate) fn with_profile_id(mut self, profile_id: impl Into<String>) -> Self {
        self.profile_id = Some(profile_id.into());
        self
    }

    /// Attach the native inventory epoch that identifies this device session.
    pub(crate) fn with_session_epoch(mut self, session_epoch: u64) -> Self {
        self.session_epoch = Some(session_epoch);
        self
    }

    /// Attach the passive support interpretation for the selected device.
    pub(crate) fn with_snapshot(mut self, snapshot: &DeviceQualificationSnapshotDto) -> Self {
        self.abi_soc_class = snapshot.abi_class.map(str::to_string);
        if self.android_api.is_none() {
            self.android_api = snapshot.android_api_level.map(u64::from);
        }
        if self.root_state.is_none() {
            self.root_state = snapshot.root.clone();
        }
        self
    }

    /// Attach the committed explicit root-check result for this device.
    pub(crate) fn with_root_state(mut self, root_state: RootQualificationState) -> Self {
        self.root_state = Some(root_state);
        self
    }

    /// Whether this observation establishes every trusted identity fact needed
    /// to reassociate with an immutable registered target.
    ///
    /// The registered target contract treats manufacturer, model, Android
    /// release, Android API level, ABI class, and firmware build as the
    /// material device facts. A trusted catalog match must also supply the
    /// authored profile identity; matching hardware alone cannot associate a
    /// restored session with a possibly changed profile. Connection type is
    /// operator attestation and root state comes from explicit root authority,
    /// so neither is inferred here. Partial observations may accumulate facts,
    /// but absent facts never prove compatibility.
    pub(crate) fn proves_target_compatibility(&self) -> bool {
        self.session_epoch.is_some()
            && self
                .root_state
                .as_ref()
                .and_then(crate::qualification_session::project_root_state)
                .is_some()
            && self.profile_id.is_some()
            && self.manufacturer.is_some()
            && self.model.is_some()
            && self.android_version.is_some()
            && self.android_api.is_some()
            && self.abi_soc_class.is_some()
            && self.firmware_build.is_some()
    }

    /// Merge one newer trusted observation of the same process-local device
    /// into this one. Observed facts win field by field and unobserved facts
    /// are retained, so a sequence of partial observations accumulates the
    /// trusted observation state without ever inventing a value.
    pub(crate) fn merged_with(
        &self,
        newer: &SelectedDeviceObservation,
    ) -> SelectedDeviceObservation {
        debug_assert_eq!(self.device_handle, newer.device_handle);
        if self.session_epoch != newer.session_epoch {
            return newer.clone();
        }
        let mut merged = self.clone();
        if newer.profile_id.is_some() {
            merged.profile_id = newer.profile_id.clone();
        }
        if newer.manufacturer.is_some() {
            merged.manufacturer = newer.manufacturer.clone();
        }
        if newer.model.is_some() {
            merged.model = newer.model.clone();
        }
        if newer.android_version.is_some() {
            merged.android_version = newer.android_version.clone();
        }
        if newer.android_api.is_some() {
            merged.android_api = newer.android_api;
        }
        if newer.abi_soc_class.is_some() {
            merged.abi_soc_class = newer.abi_soc_class.clone();
        }
        if newer.firmware_build.is_some() {
            merged.firmware_build = newer.firmware_build.clone();
        }
        if newer.root_state.is_some() {
            merged.root_state = newer.root_state.clone();
        }
        merged
    }
}

/// One trusted typed capture of the currently selected device, together with
/// the passive support interpretation the capture established.
pub(crate) struct SelectedDeviceCapture {
    pub(crate) observation: SelectedDeviceObservation,
    pub(crate) capabilities: Vec<String>,
}

/// The trusted observation boundary consumed by qualification orchestration.
///
/// Production orchestration uses the concrete application-state
/// implementation below; alternative implementations exist only so that
/// deterministic tests can prove registration and session start without a
/// device or a sidecar process. Every implementation must establish the same
/// typed result the production seams establish.
pub(crate) trait SelectedDeviceObservationSource {
    fn capture_selected_device(
        &mut self,
        device_handle: &str,
        device_plan: &str,
    ) -> Result<SelectedDeviceCapture, String>;
}

/// Production observation source composed from the shared passive device and
/// explicit root-check seams.
pub(crate) struct AppStateObservationSource<'a> {
    pub(crate) state: &'a AppState,
}

impl SelectedDeviceObservationSource for AppStateObservationSource<'_> {
    fn capture_selected_device(
        &mut self,
        device_handle: &str,
        device_plan: &str,
    ) -> Result<SelectedDeviceCapture, String> {
        let state = self.state;
        let probe = crate::commands::probe_device_facts(device_handle, state)?;
        let Some(facts) = probe.typed else {
            return Err(unverified_device_error());
        };
        let matched = crate::commands::match_device_projection(device_handle, state)?;
        let profile_id = matched_profile_id(&matched, device_plan).ok_or_else(|| {
            safe_error(
                "device_plan_unmatched",
                "The selected device plan is not a trusted match for this device.",
            )
        })?;
        let current = refresh_current_qualification(state, Some(device_handle))?;
        let snapshot = &current.snapshot;
        if snapshot.state != DeviceQualificationState::Supported
            || snapshot.device_identity.as_deref() != Some(device_handle)
            || snapshot.android_api_level.is_none()
            || snapshot.abi_class.is_none()
        {
            return Err(unverified_device_error());
        }
        let current_epoch = current
            .context
            .as_ref()
            .ok_or_else(unverified_device_error)?
            .session_epoch;
        if current_epoch != probe.session_epoch {
            return Err(safe_error(
                "device_changed",
                "The selected device changed. Refresh device discovery and try again.",
            ));
        }
        // This seam composes passive observation only. Explicit root authority
        // is composed by the qualification-specific target setup, which
        // attaches its committed typed result to the returned capture.
        let observation = SelectedDeviceObservation::new(device_handle)
            .with_probe_facts(&facts)
            .with_profile_id(profile_id)
            .with_snapshot(&snapshot)
            .with_session_epoch(probe.session_epoch);
        if observation.android_api != snapshot.android_api_level.map(u64::from) {
            return Err(unverified_device_error());
        }
        let capabilities = capabilities_from_snapshot(&snapshot);
        commit_selected_observation(state, observation.clone())?;
        Ok(SelectedDeviceCapture {
            observation,
            capabilities,
        })
    }
}

pub(crate) fn unverified_device_error() -> String {
    safe_error(
        "device_target_unverified",
        "The connected device target could not be verified from trusted observations.",
    )
}

/// Resolve the authored profile identity of one device plan from the trusted
/// catalog match projection. A plan that is not an exact match establishes no
/// profile identity.
pub(crate) fn matched_profile_id(
    matches: &DeviceMatchProjection,
    device_plan: &str,
) -> Option<String> {
    matches
        .plans()
        .find(|candidate| candidate.plan_id == device_plan)
        .and_then(|candidate| candidate.profile_id.clone())
        .filter(|profile_id| !profile_id.is_empty())
}

/// Passive capabilities a workflow may require, derived only from observed
/// capability availability.
pub(crate) fn capabilities_from_snapshot(snapshot: &DeviceQualificationSnapshotDto) -> Vec<String> {
    let mut capabilities = Vec::new();
    if snapshot.package_manager == CapabilityAvailabilityDto::Available {
        capabilities.push("apk_install".to_string());
    }
    if snapshot.storage == CapabilityAvailabilityDto::Available {
        capabilities.push("shared_storage_write".to_string());
    }
    capabilities
}

/// Commit one newly established authoritative device observation.
///
/// The typed observation is fed to the active qualification session before the
/// originating product operation returns and before its result is published to
/// React. Qualification failures are contained by the session module and can
/// never change the committed product result. When no attempt is active this is
/// a no-persistence no-op.
pub(crate) fn commit_selected_observation(
    state: &AppState,
    observation: SelectedDeviceObservation,
) -> Result<(), String> {
    let session_epoch = observation
        .session_epoch
        .ok_or_else(unverified_device_error)?;
    let handles = match state.handles.lock() {
        Ok(handles) => handles,
        Err(poisoned) => {
            let handles = poisoned.into_inner();
            state.handles.clear_poison();
            handles
        }
    };
    let current = handles
        .qualification_devices()
        .into_iter()
        .find(|device| device.handle == observation.device_handle);
    if !current
        .is_some_and(|device| device.state == "available" && device.session_epoch == session_epoch)
    {
        return Err(safe_error(
            "device_changed",
            "The selected device changed. Refresh device discovery and try again.",
        ));
    }
    crate::qualification_session::observe(
        state,
        crate::qualification_session::QualificationLifecycleObservation::DeviceObserved(Box::new(
            observation,
        )),
    );
    drop(handles);
    Ok(())
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
        ObservedDeviceState::Unverified => {
            incomplete(runtime_generation, qualification_revision, device)
        }
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
        ObservedDeviceState::Unverified => with_capabilities(
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
            ),
            device,
        ),
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
            if device.abi.is_none() {
                return with_capabilities(
                    snapshot(
                        DeviceQualificationState::InsufficientlyQualified,
                        "The connected device could not be fully qualified.",
                        vec!["Refresh discovery before attempting real execution."],
                        runtime_generation,
                        qualification_revision,
                        Some(device.opaque_identity),
                        Some(android_major),
                        Some(api_level),
                        None,
                    ),
                    device,
                );
            }
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
    if device.abi.is_none() {
        return incomplete(runtime_generation, qualification_revision, device);
    }
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
    let inventory_failure_target = current_observation_failure_target(state, requested_handle);
    if let Err(error) = list_and_reconcile_inventory(state, request) {
        return fail_qualification_refresh(state, inventory_failure_target, error);
    }

    // Inventory reconciliation has already published its generation to the
    // active attempt. Capture the target for the subsequent asynchronous
    // qualification request from that reconciled device session.
    let observation_failure_target = current_observation_failure_target(state, requested_handle);
    let adb_path = match current_adb_path(state) {
        Ok(path) => path,
        Err(error) => {
            return fail_qualification_refresh(state, observation_failure_target, error);
        }
    };
    let current = match qualify_reconciled_current_with_runtime(
        &state.handles,
        &state.root_qualification,
        &adb_path,
        runtime_generation,
        qualification_revision,
        requested_handle,
        request,
    ) {
        Ok(current) => current,
        Err(error) => {
            return fail_qualification_refresh(state, observation_failure_target, error);
        }
    };
    commit_snapshot_observation(state, &current, observation_failure_target)?;
    Ok(current)
}

fn fail_qualification_refresh<T>(
    state: &AppState,
    target: Option<crate::qualification_session::DeviceObservationFailureTarget>,
    error: String,
) -> Result<T, String> {
    crate::qualification_session::observe_device_observation_failure(state, target);
    Err(error)
}

/// Capture only an already-associated attempt for the exact device session this
/// refresh is about to observe. The candidate and epoch fence delayed results.
fn current_observation_failure_target(
    state: &AppState,
    requested_handle: Option<&str>,
) -> Option<crate::qualification_session::DeviceObservationFailureTarget> {
    let (device_handle, session_epoch) = {
        let handles = match state.handles.lock() {
            Ok(handles) => handles,
            Err(poisoned) => {
                let handles = poisoned.into_inner();
                state.handles.clear_poison();
                handles
            }
        };
        let devices = handles.qualification_devices();
        if devices.len() != 1 {
            return None;
        }
        let device = devices.into_iter().next()?;
        if device.state != "available"
            || requested_handle.is_some_and(|requested| requested != device.handle)
        {
            return None;
        }
        (device.handle, device.session_epoch)
    };
    crate::qualification_session::capture_device_observation_failure_target(
        state,
        &device_handle,
        session_epoch,
    )
}

/// Commit the typed facts from one targeted qualification before applying its
/// support result. Conflicts are therefore visible to qualification even when
/// the product correctly reports an unsupported or incomplete device.
pub(crate) fn commit_snapshot_observation(
    state: &AppState,
    current: &CurrentQualification,
    failure_target: Option<crate::qualification_session::DeviceObservationFailureTarget>,
) -> Result<(), String> {
    let Some(identity) = current.snapshot.device_identity.as_deref() else {
        crate::qualification_session::observe_device_observation_failure(state, failure_target);
        return Ok(());
    };
    let Some(context) = current.context.as_ref() else {
        crate::qualification_session::observe_device_observation_failure(state, failure_target);
        return Ok(());
    };
    if let Err(error) = commit_selected_observation(
        state,
        SelectedDeviceObservation::new(identity)
            .with_snapshot(&current.snapshot)
            .with_session_epoch(context.session_epoch),
    ) {
        return fail_qualification_refresh(state, failure_target, error);
    }
    if current.snapshot.state != DeviceQualificationState::Supported {
        crate::qualification_session::observe_device_observation_failure(state, failure_target);
    }
    Ok(())
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

    let response = request(
        "qualifyDevice",
        json!({ "adbPath": adb_path, "serial": &target.serial }),
    );
    ensure_target_session_current(handles, &target)?;
    let response = match response {
        Ok(response) => response,
        Err(_) => {
            clear_unverified_device_authority(
                handles,
                root_qualification,
                &identity,
                target.session_epoch,
            )?;
            return Err(qualification_probe_error());
        }
    };
    let Some(observed) = QualifyDeviceObservation::decode(&response) else {
        clear_unverified_device_authority(
            handles,
            root_qualification,
            &identity,
            target.session_epoch,
        )?;
        return Err(qualification_probe_error());
    };
    let (mut snapshot, observed_qualification) = classify_observed_complete(
        runtime_generation,
        qualification_revision,
        &observed,
        Some(&identity),
    );
    if observed_qualification.state == ObservedDeviceState::Unverified {
        clear_unverified_device_authority(
            handles,
            root_qualification,
            &identity,
            target.session_epoch,
        )?;
        return Ok(CurrentQualification {
            snapshot,
            context: None,
        });
    }
    let context = QualificationContextKey::new(
        &identity,
        target.session_epoch,
        runtime_generation,
        qualification_revision,
        qualification_revision,
        qualification_fingerprint(&observed_qualification),
    );
    let mut handles_guard = handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?;
    let current_device = handles_guard.device(&identity).map_err(|_| {
        safe_error(
            "device_changed",
            "The selected device changed. Refresh device discovery and try again.",
        )
    })?;
    if current_device.state != "available"
        || current_device.session_epoch != target.session_epoch
        || current_device.serial != target.serial
    {
        return Err(safe_error(
            "device_changed",
            "The selected device changed. Refresh device discovery and try again.",
        ));
    }
    handles_guard.set_qualification_context(context.clone());
    drop(handles_guard);
    let root_key = RootQualificationKey::from_context(&context);
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

fn qualification_probe_error() -> String {
    safe_error(
        "device_qualification_failed",
        "Connected-device qualification could not be completed.",
    )
}

/// Reject a delayed qualification response from a replaced native session.
fn ensure_target_session_current(
    handles: &Mutex<SessionHandles>,
    target: &crate::handles::DeviceRecord,
) -> Result<(), String> {
    let handles = handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?;
    let current = handles.device(&target.handle).map_err(|_| {
        safe_error(
            "device_changed",
            "The selected device changed. Refresh device discovery and try again.",
        )
    })?;
    if current.state != "available"
        || current.serial != target.serial
        || current.session_epoch != target.session_epoch
    {
        return Err(safe_error(
            "device_changed",
            "The selected device changed. Refresh device discovery and try again.",
        ));
    }
    Ok(())
}

/// Drop product qualification and root authority after an observation cannot
/// establish trustworthy facts for the still-current device session.
fn clear_unverified_device_authority(
    handles: &Mutex<SessionHandles>,
    root_qualification: &Mutex<RootQualificationStore>,
    device_handle: &str,
    expected_session_epoch: u64,
) -> Result<(), String> {
    let mut handles_guard = match handles.lock() {
        Ok(handles_guard) => handles_guard,
        Err(poisoned) => {
            let handles_guard = poisoned.into_inner();
            handles.clear_poison();
            handles_guard
        }
    };
    let current = handles_guard.device(device_handle).map_err(|_| {
        safe_error(
            "device_changed",
            "The selected device changed. Refresh device discovery and try again.",
        )
    })?;
    if current.state != "available" || current.session_epoch != expected_session_epoch {
        return Err(safe_error(
            "device_changed",
            "The selected device changed. Refresh device discovery and try again.",
        ));
    }
    handles_guard.clear_qualification_context(device_handle);
    handles_guard.invalidate_reviews_for_device(device_handle, "device_qualification_changed");

    let mut root_guard = match root_qualification.lock() {
        Ok(root_guard) => root_guard,
        Err(poisoned) => {
            let root_guard = poisoned.into_inner();
            root_qualification.clear_poison();
            root_guard
        }
    };
    root_guard.invalidate_for_device(device_handle);
    Ok(())
}

fn classify_observed_complete<'a>(
    runtime_generation: u64,
    qualification_revision: u64,
    observed: &'a QualifyDeviceObservation,
    identity: Option<&'a str>,
) -> (DeviceQualificationSnapshotDto, ObservedQualification<'a>) {
    let state = observed.state.as_deref();
    let identity = identity.unwrap_or("qualification-device");
    let profile = ObservedQualification {
        opaque_identity: identity,
        state: match state {
            Some("unauthorized") => ObservedDeviceState::Unauthorized,
            Some("offline") => ObservedDeviceState::Offline,
            Some("online") => ObservedDeviceState::Online,
            _ => ObservedDeviceState::Unverified,
        },
        android_major: observed.android_major,
        android_api_level: observed.android_api_level,
        abi: observed.abi.as_deref(),
        storage: observed.storage,
        package_manager: observed.package_manager,
        activity_manager: observed.activity_manager,
    };
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
        Some("online") => vec![observed_device(
            observed,
            ObservedDeviceState::Online,
            identity.unwrap_or("qualification-device"),
        )],
        _ => vec![observed_device(
            observed,
            ObservedDeviceState::Unverified,
            identity.unwrap_or("qualification-device"),
        )],
    };
    classify(true, runtime_generation, qualification_revision, &devices)
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

    /// Decode one fixture probe payload exactly like the production seam does.
    fn probe_facts(value: Value) -> DeviceProbeFacts {
        DeviceProbeFacts::decode(&value).expect("test probe facts should decode")
    }

    fn qualification_snapshot_with_state(state: Option<&str>) -> DeviceQualificationSnapshotDto {
        let mut payload = json!({
            "androidMajor": 14,
            "androidApiLevel": 34,
            "abi": "arm64-v8a",
            "storage": "available",
            "packageManager": "available",
            "activityManager": "available"
        });
        if let Some(state) = state {
            payload["state"] = json!(state);
        }
        let observed = QualifyDeviceObservation::decode(&payload)
            .expect("well-shaped qualification facts should decode");
        classify_observed_complete(1, 2, &observed, Some("opaque-device")).0
    }

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
    fn qualification_command_errors_are_fixed_safe_ipc_messages() {
        let mut native_handles = crate::handles::SessionHandles::default();
        native_handles
            .update_devices(&json!({
                "devices": [{
                    "serial": "private-device-serial",
                    "state": "available"
                }]
            }))
            .unwrap();
        let handles = std::sync::Mutex::new(native_handles);
        let roots =
            std::sync::Mutex::new(crate::device_qualification::RootQualificationStore::default());
        let mut request = |_: &str, _: Value| {
            Err("/Users/example/private/tool adb output private-device-serial qualification_repository_unavailable".to_string())
        };

        let error = qualify_reconciled_current_with_runtime(
            &handles,
            &roots,
            "/Users/example/private/tool",
            1,
            1,
            None,
            &mut request,
        )
        .expect_err("a backend failure should become a sanitized IPC error");
        let payload: Value = serde_json::from_str(&error).unwrap();
        assert_eq!(
            payload,
            json!({
                "code": "device_qualification_failed",
                "message": "Connected-device qualification could not be completed."
            })
        );
        for private_value in [
            "/Users/example/private/tool",
            "private-device-serial",
            "adb output",
            "qualification_repository_unavailable",
        ] {
            assert!(!error.contains(private_value), "IPC leaked {private_value}");
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
                &[online("missing-abi", Some(14), Some(34), None)]
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
    fn missing_qualification_state_is_not_treated_as_online() {
        let result = qualification_snapshot_with_state(None);

        assert_eq!(
            result.state,
            DeviceQualificationState::InsufficientlyQualified
        );
    }

    #[test]
    fn unknown_qualification_state_is_not_treated_as_online() {
        let result = qualification_snapshot_with_state(Some("connected-ish"));

        assert_eq!(
            result.state,
            DeviceQualificationState::InsufficientlyQualified
        );
    }

    #[test]
    fn unverified_qualification_clears_retained_context_and_root_authority() {
        let mut native_handles = SessionHandles::default();
        native_handles
            .update_devices(&json!({
                "devices": [{ "serial": "private-serial", "state": "available" }]
            }))
            .unwrap();
        let device_handle = native_handles
            .single_available_device_handle()
            .expect("one available device has an opaque handle");
        let context = QualificationContextKey::new(
            &device_handle,
            1,
            2,
            3,
            3,
            "previously-supported-capabilities",
        );
        native_handles.set_qualification_context(context.clone());
        let handles = Mutex::new(native_handles);
        let root_key = RootQualificationKey::from_context(&context);
        let mut roots = RootQualificationStore::default();
        let root_attempt = roots.begin(root_key.clone()).unwrap();
        assert!(roots.complete(root_attempt, RootQualificationState::Granted));
        let roots = Mutex::new(roots);
        let mut request = |request_type: &str, _: Value| {
            assert_eq!(request_type, "qualifyDevice");
            Ok(json!({
                "androidMajor": 14,
                "androidApiLevel": 34,
                "abi": "arm64-v8a",
                "storage": "available",
                "packageManager": "available",
                "activityManager": "available"
            }))
        };

        let current = qualify_reconciled_current_with_runtime(
            &handles,
            &roots,
            "/adb",
            2,
            3,
            Some(&device_handle),
            &mut request,
        )
        .unwrap();

        assert_eq!(
            current.snapshot.state,
            DeviceQualificationState::InsufficientlyQualified
        );
        assert!(current.context.is_none());
        assert!(handles
            .lock()
            .unwrap()
            .qualification_context(&device_handle)
            .is_none());
        assert_eq!(roots.lock().unwrap().get(&root_key), None);
    }

    #[test]
    fn delayed_unverified_result_cannot_clear_replacement_session_authority() {
        let mut native_handles = SessionHandles::default();
        native_handles
            .update_devices(&json!({
                "devices": [{
                    "serial": "private-serial",
                    "state": "available",
                    "transportId": "transport-old"
                }]
            }))
            .unwrap();
        let device_handle = native_handles
            .single_available_device_handle()
            .expect("one available device has an opaque handle");
        let handles = Mutex::new(native_handles);
        let roots = Mutex::new(RootQualificationStore::default());
        let mut replacement_context = None;
        let mut request = |request_type: &str, _: Value| {
            assert_eq!(request_type, "qualifyDevice");
            let context = {
                let mut handles = handles.lock().unwrap();
                handles
                    .update_devices(&json!({
                        "devices": [{
                            "serial": "private-serial",
                            "state": "available",
                            "transportId": "transport-new"
                        }]
                    }))
                    .unwrap();
                let epoch = handles.device_session_epoch(&device_handle).unwrap();
                let context = QualificationContextKey::new(
                    &device_handle,
                    epoch,
                    2,
                    3,
                    3,
                    "replacement-capabilities",
                );
                handles.set_qualification_context(context.clone());
                context
            };
            let root_key = RootQualificationKey::from_context(&context);
            let mut roots = roots.lock().unwrap();
            let attempt = roots.begin(root_key).unwrap();
            assert!(roots.complete(attempt, RootQualificationState::Granted));
            replacement_context = Some(context);
            Ok(json!({
                "androidMajor": 14,
                "androidApiLevel": 34,
                "abi": "arm64-v8a",
                "storage": "available",
                "packageManager": "available",
                "activityManager": "available"
            }))
        };

        let error = qualify_reconciled_current_with_runtime(
            &handles,
            &roots,
            "/adb",
            2,
            3,
            Some(&device_handle),
            &mut request,
        )
        .expect_err("a response from the replaced device session must be rejected");

        assert!(error.contains("device_changed"));
        let replacement_context = replacement_context.unwrap();
        let stale_clear = clear_unverified_device_authority(&handles, &roots, &device_handle, 1)
            .expect_err("an old epoch cannot clear authority for the replacement session");
        assert!(stale_clear.contains("device_changed"));
        assert_eq!(
            handles
                .lock()
                .unwrap()
                .qualification_context(&device_handle),
            Some(replacement_context.clone())
        );
        assert_eq!(
            roots
                .lock()
                .unwrap()
                .get(&RootQualificationKey::from_context(&replacement_context)),
            Some(RootQualificationState::Granted)
        );
    }

    #[test]
    fn production_qualification_classifier_preserves_recognized_device_states() {
        assert_eq!(
            qualification_snapshot_with_state(Some("online")).state,
            DeviceQualificationState::Supported
        );
        assert_eq!(
            qualification_snapshot_with_state(Some("unauthorized")).state,
            DeviceQualificationState::Unauthorized
        );
        assert_eq!(
            qualification_snapshot_with_state(Some("offline")).state,
            DeviceQualificationState::Offline
        );
        assert_eq!(
            qualification_snapshot_with_state(Some("no_device")).state,
            DeviceQualificationState::NoDevice
        );
        assert_eq!(
            qualification_snapshot_with_state(Some("multiple_devices")).state,
            DeviceQualificationState::InsufficientlyQualified
        );
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

    #[test]
    fn typed_observation_retains_only_normalized_trusted_facts() {
        let observation =
            SelectedDeviceObservation::new("device-one").with_probe_facts(&probe_facts(json!({
                "manufacturer": "AYANEO",
                "model": "Pocket S2",
                "android_version": 15,
                "android_api_level": 35,
                "firmware_build": "vendor/build",
                "serial": "exact-sensitive-serial",
            })));
        assert_eq!(observation.device_handle, "device-one");
        assert_eq!(observation.manufacturer.as_deref(), Some("AYANEO"));
        assert_eq!(observation.model.as_deref(), Some("Pocket S2"));
        assert_eq!(observation.android_version.as_deref(), Some("15"));
        assert_eq!(observation.android_api, Some(35));
        assert_eq!(observation.firmware_build.as_deref(), Some("vendor/build"));
        assert_eq!(observation.profile_id, None);
        assert_eq!(observation.abi_soc_class, None);
        assert_eq!(observation.root_state, None);
        assert!(!format!("{observation:?}").contains("exact-sensitive-serial"));

        let empty = SelectedDeviceObservation::new("device-one").with_probe_facts(&probe_facts(
            json!({ "model": null, "android_version": true }),
        ));
        assert_eq!(empty.model, None);
        assert_eq!(empty.android_version, None);
    }

    #[test]
    fn typed_observation_derives_profile_support_and_root_from_their_own_seams() {
        let snapshot = DeviceQualificationSnapshotDto {
            state: DeviceQualificationState::Supported,
            summary: "supported",
            limitations: Vec::new(),
            android_major: Some(15),
            android_api_level: Some(35),
            abi_class: Some("arm64"),
            storage: CapabilityAvailabilityDto::Available,
            package_manager: CapabilityAvailabilityDto::Available,
            activity_manager: CapabilityAvailabilityDto::Unavailable,
            root: Some(RootQualificationState::Denied),
            runtime_generation: 1,
            qualification_revision: 1,
            device_identity: Some("device-one".to_string()),
        };
        let observation = SelectedDeviceObservation::new("device-one")
            .with_profile_id("ayaneo.pocket_s2")
            .with_snapshot(&snapshot);
        assert_eq!(observation.profile_id.as_deref(), Some("ayaneo.pocket_s2"));
        assert_eq!(observation.abi_soc_class.as_deref(), Some("arm64"));
        assert_eq!(observation.android_api, Some(35));
        assert_eq!(observation.root_state, Some(RootQualificationState::Denied));

        let rooted = observation
            .clone()
            .with_root_state(RootQualificationState::Granted);
        assert_eq!(rooted.root_state, Some(RootQualificationState::Granted));
    }

    #[test]
    fn capability_projection_uses_only_observed_availability() {
        let snapshot = DeviceQualificationSnapshotDto {
            state: DeviceQualificationState::Supported,
            summary: "supported",
            limitations: Vec::new(),
            android_major: Some(15),
            android_api_level: Some(35),
            abi_class: Some("arm64"),
            storage: CapabilityAvailabilityDto::Available,
            package_manager: CapabilityAvailabilityDto::Available,
            activity_manager: CapabilityAvailabilityDto::Unavailable,
            root: None,
            runtime_generation: 1,
            qualification_revision: 1,
            device_identity: Some("device-one".to_string()),
        };
        assert_eq!(
            capabilities_from_snapshot(&snapshot),
            vec!["apk_install", "shared_storage_write"]
        );

        let unknown = DeviceQualificationSnapshotDto {
            storage: CapabilityAvailabilityDto::Unknown,
            package_manager: CapabilityAvailabilityDto::Unknown,
            ..snapshot
        };
        assert!(capabilities_from_snapshot(&unknown).is_empty());
    }

    #[test]
    fn selected_plan_resolves_only_from_the_trusted_match_projection() {
        let result = DeviceMatchProjection::decode(&json!({
            "candidates": [{ "planId": "wrong", "profileId": "wrong.profile" }],
            "safeGenericPlans": [{ "planId": "selected", "profileId": "selected.profile" }]
        }))
        .expect("test match projection should decode");
        assert_eq!(
            matched_profile_id(&result, "selected"),
            Some("selected.profile".to_string())
        );
        assert_eq!(matched_profile_id(&result, "missing"), None);

        let empty_profile = DeviceMatchProjection::decode(&json!({
            "candidates": [{ "planId": "selected", "profileId": "" }]
        }))
        .expect("test match projection should decode");
        assert_eq!(matched_profile_id(&empty_profile, "selected"), None);
    }
}
