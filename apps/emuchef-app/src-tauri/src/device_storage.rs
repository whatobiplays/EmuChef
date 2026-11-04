//! Trusted device storage volume observation and user selection.
//!
//! The Rust sidecar owns passive ADB observation of an Android device's storage
//! volumes. This module owns the native product consequences of that
//! observation: a short-lived cache keyed by the exact device session, runtime
//! generation, and Platform-Tools revision; opaque volume handles that never
//! reveal a filesystem UUID, mount path, disk identifier, or serial; and one
//! explicit storage selection per device session.
//!
//! Two rules shape the implementation:
//!
//! * No native state lock is held while an ADB command runs. A probe is
//!   prepared under the session lock, executed with every lock released, and
//!   committed only after re-reading the current session.
//! * A selection records user intent and carries no authority. It is discarded whenever the
//!   device session, runtime generation, Platform-Tools revision, or observed
//!   volume evidence no longer proves the same available storage destination.
//!   It never participates in planning, review, or execution.
//!
//! Admission requires the observation token displayed with the chosen
//! projection. The token is compared with the current trusted observation under
//! the same state lock, and a mismatch fails without rebinding an older choice
//! to a replacement card. Explicit clearing needs no handle or token.
//!
//! Cache lookup uses the request-start time. A successful probe records its
//! fetched_at at probe completion, and projection and admission enforce the
//! ten-second limit at their current time.
//!
//! React receives only product labels, fixed status text, opaque handles,
//! presentation keys, and the observation token. Raw volume evidence stays in
//! this process.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use emuchef_rust_backend::storage_targets::{
    validate_relative_location_path, validate_setup_location_name, StorageTargetBinding,
    StorageTargetVolumeKind, MAX_SETUP_LOCATIONS,
};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::commands::{current_adb_runtime_snapshot, safe_error, AppState};
use crate::handles::{DeviceRecord, SessionHandles};

/// How long one observation may be reused without another ADB probe.
const CACHE_TTL: Duration = Duration::from_secs(10);

/// Bound on retained handles from earlier sessions, used to distinguish a
/// superseded handle from one that never belonged to this device.
const MAX_RETIRED_HANDLES: usize = 64;

/// Domain separator for the opaque volume-handle digest.
const HANDLE_DOMAIN: &str = "emuchef.device.volume.handle.v1";

/// Trusted observation of one device's storage volumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VolumeObservation {
    session_epoch: u64,
    runtime_generation: u64,
    platform_tools_revision: u64,
    discovery_status: DiscoveryStatus,
    observation_token: String,
    volumes: Vec<ObservedVolume>,
    fetched_at: Instant,
}

impl VolumeObservation {
    fn matches_provenance(
        &self,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
    ) -> bool {
        self.session_epoch == session_epoch
            && self.runtime_generation == runtime_generation
            && self.platform_tools_revision == platform_tools_revision
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiscoveryStatus {
    Observed,
    NoDevice,
    MultipleDevices,
    Unauthorized,
    Offline,
    Refreshing,
    Unknown,
}

impl DiscoveryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::NoDevice => "noDevice",
            Self::MultipleDevices => "multipleDevices",
            Self::Unauthorized => "unauthorized",
            Self::Offline => "offline",
            Self::Refreshing => "refreshing",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObservedVolumeKind {
    InternalShared,
    RemovableSd,
    AdoptedPrivate,
    Usb,
    Unknown,
}

impl ObservedVolumeKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "internalShared" => Some(Self::InternalShared),
            "removableSd" => Some(Self::RemovableSd),
            "adoptedPrivate" => Some(Self::AdoptedPrivate),
            "usb" => Some(Self::Usb),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObservedVolumeStatus {
    Available,
    Unmounted,
    Ambiguous,
    Unsupported,
    Unknown,
}

impl ObservedVolumeStatus {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "available" => Some(Self::Available),
            "unmounted" => Some(Self::Unmounted),
            "ambiguous" => Some(Self::Ambiguous),
            "unsupported" => Some(Self::Unsupported),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObservedMountState {
    MountedRw,
    MountedRo,
    Unmounted,
    Unknown,
}

impl ObservedMountState {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "mountedRw" => Some(Self::MountedRw),
            "mountedRo" => Some(Self::MountedRo),
            "unmounted" => Some(Self::Unmounted),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// One volume exactly as the trusted sidecar reported it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObservedVolume {
    identity_token: String,
    continuity_token: String,
    kind: ObservedVolumeKind,
    status: ObservedVolumeStatus,
    mount_state: ObservedMountState,
    setup_location_kind: Option<ObservedVolumeKind>,
    mount_root: Option<String>,
}

impl ObservedVolume {
    fn is_selectable(&self) -> bool {
        self.status == ObservedVolumeStatus::Available
            && self.mount_state == ObservedMountState::MountedRw
            && matches!(
                self.kind,
                ObservedVolumeKind::InternalShared | ObservedVolumeKind::RemovableSd
            )
    }

    fn is_setup_location_available(&self) -> bool {
        self.setup_location_kind.is_some()
            && self.mount_root.is_some()
            && self.status == ObservedVolumeStatus::Available
            && self.mount_state == ObservedMountState::MountedRw
    }
}

/// One explicit storage intent for a device session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VolumeSelection {
    session_epoch: u64,
    volume_handle: String,
    identity_token: String,
    continuity_token: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct SetupLocationRecord {
    binding: StorageTargetBinding,
    runtime_generation: u64,
    platform_tools_revision: u64,
    resolved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetupLocationProjection {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) relative_path: String,
    pub(crate) volume_kind: String,
    pub(crate) volume_label: String,
    pub(crate) ready: bool,
    pub(crate) detail: String,
}

/// Retained handle from an earlier observation, used only to classify a stale
/// handle distinctly from an unknown one.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RetiredHandle {
    device_handle: String,
    volume_handle: String,
}

/// Native storage observation and selection state.
///
/// The value lives with the retained device sessions so inventory
/// reconciliation can invalidate storage authority in the same critical
/// section that advances a device session epoch.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeviceStorageState {
    observations: HashMap<String, VolumeObservation>,
    selections: HashMap<String, VolumeSelection>,
    setup_locations: HashMap<(String, String), Vec<SetupLocationRecord>>,
    in_flight: HashMap<String, u64>,
    sequences: HashMap<String, u64>,
    notices: HashMap<String, String>,
    retired_handles: VecDeque<RetiredHandle>,
    selection_requests: HashMap<String, u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProbeStart {
    Started(u64),
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProbeOutcome {
    Committed,
    Superseded,
}

impl DeviceStorageState {
    /// Drop ephemeral storage authority for one device while retaining every
    /// other device, identity tombstones, and the monotonic probe and selection
    /// counters that fence late responses.
    pub(crate) fn clear_device_after_authority_loss(&mut self, device_handle: &str) {
        self.retire_observation(device_handle);
        self.selections.remove(device_handle);
        self.setup_locations
            .retain(|(device, _), _| device != device_handle);
        self.in_flight.remove(device_handle);
        self.notices.remove(device_handle);
    }

    /// Drop ephemeral storage authority while retaining identity tombstones and
    /// the monotonic probe and selection counters that fence late responses.
    pub(crate) fn clear_all_after_authority_loss(&mut self) {
        let devices = self.observations.keys().cloned().collect::<Vec<_>>();
        for device in devices {
            self.retire_observation(&device);
        }
        self.selections.clear();
        self.setup_locations.clear();
        self.in_flight.clear();
        self.notices.clear();
    }

    /// Drop observations and selections that no longer match the retained
    /// device sessions. Called from inventory reconciliation, where a session
    /// epoch change is the authoritative continuity signal.
    pub(crate) fn reconcile_devices(&mut self, epochs: &HashMap<String, u64>) {
        let stale = self
            .observations
            .iter()
            .filter(|(device, observation)| epochs.get(*device) != Some(&observation.session_epoch))
            .map(|(device, _)| device.clone())
            .collect::<Vec<_>>();
        for device in stale {
            self.retire_observation(&device);
            self.selections.remove(&device);
            self.setup_locations
                .retain(|(setup_device, _), _| setup_device != &device);
            self.in_flight.remove(&device);
        }
        let orphaned = self
            .selections
            .iter()
            .filter(|(device, selection)| epochs.get(*device) != Some(&selection.session_epoch))
            .map(|(device, _)| device.clone())
            .collect::<Vec<_>>();
        for device in orphaned {
            self.selections.remove(&device);
        }
        self.selection_requests
            .retain(|device, _| epochs.contains_key(device));
        self.setup_locations
            .retain(|(device, _), _| epochs.contains_key(device));
    }

    fn mark_setup_locations_unresolved(&mut self, device: &str) {
        for ((location_device, _), locations) in &mut self.setup_locations {
            if location_device == device {
                for location in locations {
                    location.resolved = false;
                }
            }
        }
    }

    fn reconcile_setup_locations(&mut self, device: &str) {
        let Some(observation) = self.observations.get(device) else {
            self.mark_setup_locations_unresolved(device);
            return;
        };
        for ((location_device, _), locations) in &mut self.setup_locations {
            if location_device != device {
                continue;
            }
            for location in locations {
                let matching_volume = observation.volumes.iter().find(|volume| {
                    volume.identity_token == location.binding.volume_identity
                        && volume.continuity_token == location.binding.volume_continuity
                        && volume.mount_root.as_deref()
                            == Some(location.binding.mount_root.as_str())
                        && storage_target_kind(volume.setup_location_kind)
                            == Some(location.binding.volume_kind)
                        && volume.is_setup_location_available()
                });
                location.resolved = location.binding.session_epoch == observation.session_epoch
                    && location.runtime_generation == observation.runtime_generation
                    && location.platform_tools_revision == observation.platform_tools_revision
                    && matching_volume.is_some();
                if location.resolved {
                    location.binding.observation_token = observation.observation_token.clone();
                    location.binding.volume_handle = volume_handle(
                        device,
                        observation.session_epoch,
                        &location.binding.volume_identity,
                    );
                }
            }
        }
    }

    fn save_setup_location(
        &mut self,
        device: &str,
        device_plan: &str,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
        observation_token: &str,
        volume_handle: &str,
        existing_id: Option<&str>,
        name: &str,
        relative_path: &str,
        now: Instant,
    ) -> Result<Vec<SetupLocationProjection>, String> {
        let name = validate_setup_location_name(name)
            .map_err(|_| "Enter a valid name for this Setup Location.".to_string())?;
        let relative_path = validate_relative_location_path(relative_path).map_err(|_| {
            "Enter a safe relative folder path for this Setup Location.".to_string()
        })?;
        let observation = self.observations.get(device).cloned().ok_or_else(|| {
            "Refresh storage options and choose a volume for this Setup Location.".to_string()
        })?;
        if !observation.matches_provenance(
            session_epoch,
            runtime_generation,
            platform_tools_revision,
        ) || observation.discovery_status != DiscoveryStatus::Observed
            || now.saturating_duration_since(observation.fetched_at) >= CACHE_TTL
            || observation.observation_token != observation_token
        {
            return Err("Storage options changed. Refresh and choose a volume again.".to_string());
        }
        let volume = observation
            .volumes
            .iter()
            .find(|volume| {
                volume_handle == volume_handle_for(device, observation.session_epoch, volume)
            })
            .filter(|volume| volume.is_setup_location_available())
            .ok_or_else(|| "That volume cannot be used for a Setup Location.".to_string())?;
        let volume_kind = storage_target_kind(volume.setup_location_kind)
            .ok_or_else(|| "That volume cannot be used for a Setup Location.".to_string())?;
        let mount_root = volume
            .mount_root
            .clone()
            .ok_or_else(|| "That volume does not have a trusted storage root.".to_string())?;
        let scope = (device.to_string(), device_plan.to_string());
        let locations = self.setup_locations.entry(scope).or_default();
        if let Some(existing_id) = existing_id {
            if !locations
                .iter()
                .any(|location| location.binding.id == existing_id)
            {
                return Err("That Setup Location is no longer available.".to_string());
            }
        } else if locations.len() >= MAX_SETUP_LOCATIONS {
            return Err(
                "This setup already has the maximum number of Setup Locations.".to_string(),
            );
        }
        if locations.iter().any(|location| {
            Some(location.binding.id.as_str()) != existing_id
                && location.binding.name.to_lowercase() == name.to_lowercase()
        }) {
            return Err(
                "Choose a different name; Setup Location names must be unique.".to_string(),
            );
        }
        let id = existing_id
            .map(ToString::to_string)
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
        let binding = StorageTargetBinding {
            id: id.clone(),
            device_plan: device_plan.to_string(),
            session_epoch,
            name,
            relative_path,
            volume_handle: volume_handle_for(device, session_epoch, volume),
            volume_kind,
            mount_root,
            volume_identity: volume.identity_token.clone(),
            volume_continuity: volume.continuity_token.clone(),
            observation_token: observation.observation_token.clone(),
        };
        binding
            .validate()
            .map_err(|_| "That volume and path cannot form a safe Setup Location.".to_string())?;
        let record = SetupLocationRecord {
            binding,
            runtime_generation,
            platform_tools_revision,
            resolved: true,
        };
        if let Some(existing_id) = existing_id {
            let location = locations
                .iter_mut()
                .find(|location| location.binding.id == existing_id)
                .expect("existing location was checked above");
            *location = record;
        } else {
            locations.push(record);
        }
        Ok(project_setup_location_records(
            locations,
            Some(&observation),
            now,
        ))
    }

    fn delete_setup_location(&mut self, device: &str, device_plan: &str, id: &str) -> bool {
        let Some(locations) = self
            .setup_locations
            .get_mut(&(device.to_string(), device_plan.to_string()))
        else {
            return false;
        };
        let before = locations.len();
        locations.retain(|location| location.binding.id != id);
        locations.len() != before
    }

    fn project_setup_locations(
        &self,
        device: &str,
        device_plan: &str,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
        now: Instant,
    ) -> Vec<SetupLocationProjection> {
        let Some(locations) = self
            .setup_locations
            .get(&(device.to_string(), device_plan.to_string()))
        else {
            return Vec::new();
        };
        let observation = self.observations.get(device).filter(|observation| {
            observation.matches_provenance(
                session_epoch,
                runtime_generation,
                platform_tools_revision,
            ) && now.saturating_duration_since(observation.fetched_at) < CACHE_TTL
        });
        project_setup_location_records(locations, observation, now)
    }

    fn setup_location_bindings(
        &self,
        device: &str,
        device_plan: &str,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
        ids: &[String],
        now: Instant,
    ) -> Result<Vec<StorageTargetBinding>, String> {
        let observation = self
            .observations
            .get(device)
            .ok_or_else(|| "Refresh storage options before reviewing this setup.".to_string())?;
        if !observation.matches_provenance(
            session_epoch,
            runtime_generation,
            platform_tools_revision,
        ) || observation.discovery_status != DiscoveryStatus::Observed
            || now.saturating_duration_since(observation.fetched_at) >= CACHE_TTL
        {
            return Err(
                "Storage options are stale. Refresh and review this setup again.".to_string(),
            );
        }
        let locations = self
            .setup_locations
            .get(&(device.to_string(), device_plan.to_string()))
            .ok_or_else(|| "Choose a Setup Location for each storage destination.".to_string())?;
        let mut unique_ids = ids.to_vec();
        unique_ids.sort();
        unique_ids.dedup();
        unique_ids
            .iter()
            .map(|id| {
                let location = locations
                    .iter()
                    .find(|location| location.binding.id == *id)
                    .filter(|location| {
                        location.resolved
                            && location.binding.session_epoch == session_epoch
                            && location.runtime_generation == runtime_generation
                            && location.platform_tools_revision == platform_tools_revision
                    })
                    .ok_or_else(|| {
                        "A selected Setup Location needs a refreshed volume choice. Review again after selecting storage.".to_string()
                    })?;
                let volume = observation.volumes.iter().find(|volume| {
                    volume.identity_token == location.binding.volume_identity
                        && volume.continuity_token == location.binding.volume_continuity
                        && volume.mount_root.as_deref()
                            == Some(location.binding.mount_root.as_str())
                        && storage_target_kind(volume.setup_location_kind)
                            == Some(location.binding.volume_kind)
                        && volume.is_setup_location_available()
                });
                if volume.is_none() {
                    return Err("A selected volume changed. Choose storage again and create a new review.".to_string());
                }
                let mut binding = location.binding.clone();
                binding.observation_token = observation.observation_token.clone();
                binding.validate().map_err(|_| {
                    "A selected Setup Location no longer has valid storage evidence.".to_string()
                })?;
                Ok(binding)
            })
            .collect()
    }

    fn next_sequence(&mut self, device: &str) -> u64 {
        let sequence = self.sequences.entry(device.to_string()).or_insert(0);
        *sequence = sequence.saturating_add(1).max(1);
        *sequence
    }

    fn next_selection_request(&mut self, device: &str) -> u64 {
        let sequence = self
            .selection_requests
            .entry(device.to_string())
            .or_insert(0);
        *sequence = sequence.saturating_add(1).max(1);
        *sequence
    }

    fn selection_request_is_current(&self, device: &str, sequence: u64) -> bool {
        self.selection_requests.get(device) == Some(&sequence)
    }

    fn begin_probe(
        &mut self,
        device: &str,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
        now: Instant,
    ) -> ProbeStart {
        self.drop_mismatched_observation(
            device,
            session_epoch,
            runtime_generation,
            platform_tools_revision,
        );
        if self.in_flight.contains_key(device) {
            self.drop_expired_observation(device, now);
            return ProbeStart::Busy;
        }
        let sequence = self.next_sequence(device);
        self.in_flight.insert(device.to_string(), sequence);
        ProbeStart::Started(sequence)
    }

    fn drop_mismatched_observation(
        &mut self,
        device: &str,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
    ) {
        let mismatched = self.observations.get(device).is_some_and(|observation| {
            !observation.matches_provenance(
                session_epoch,
                runtime_generation,
                platform_tools_revision,
            )
        });
        if mismatched {
            self.retire_observation(device);
            self.selections.remove(device);
            self.notices.remove(device);
        }
    }

    fn current_observation(
        &self,
        device: &str,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
        now: Instant,
    ) -> Option<&VolumeObservation> {
        self.observations.get(device).filter(|observation| {
            observation.matches_provenance(
                session_epoch,
                runtime_generation,
                platform_tools_revision,
            ) && now.saturating_duration_since(observation.fetched_at) < CACHE_TTL
        })
    }

    fn drop_expired_observation(&mut self, device: &str, now: Instant) {
        let expired = self.observations.get(device).is_some_and(|observation| {
            now.saturating_duration_since(observation.fetched_at) >= CACHE_TTL
        });
        if expired {
            self.retire_observation(device);
            self.selections.remove(device);
            self.notices.remove(device);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_probe(
        &mut self,
        device: &str,
        sequence: u64,
        session_epoch: u64,
        runtime_generation: u64,
        platform_tools_revision: u64,
        discovery_status: DiscoveryStatus,
        observation_token: String,
        volumes: Vec<ObservedVolume>,
        now: Instant,
    ) -> ProbeOutcome {
        if self.in_flight.get(device) != Some(&sequence) {
            return ProbeOutcome::Superseded;
        }
        self.in_flight.remove(device);
        self.retire_observation(device);
        self.observations.insert(
            device.to_string(),
            VolumeObservation {
                session_epoch,
                runtime_generation,
                platform_tools_revision,
                discovery_status,
                observation_token,
                volumes,
                fetched_at: now,
            },
        );
        self.reconcile_selection(device, now);
        self.reconcile_setup_locations(device);
        ProbeOutcome::Committed
    }

    fn retire_handle(&mut self, device: &str, handle: &str) {
        if self.retired_handles.len() >= MAX_RETIRED_HANDLES {
            self.retired_handles.pop_front();
        }
        self.retired_handles.push_back(RetiredHandle {
            device_handle: device.to_string(),
            volume_handle: handle.to_string(),
        });
    }

    /// Remove one observation and retain its handles so a later attempt to use
    /// them is classified as a stale session rather than an unknown handle.
    fn retire_observation(&mut self, device: &str) {
        if let Some(observation) = self.observations.remove(device) {
            for volume in observation.volumes {
                let handle =
                    volume_handle(device, observation.session_epoch, &volume.identity_token);
                self.retire_handle(device, &handle);
            }
        }
    }

    fn fail_probe(&mut self, device: &str, sequence: u64, notice: &str) -> ProbeOutcome {
        if self.in_flight.get(device) != Some(&sequence) {
            return ProbeOutcome::Superseded;
        }
        self.in_flight.remove(device);
        self.retire_observation(device);
        self.selections.remove(device);
        self.mark_setup_locations_unresolved(device);
        self.notices.insert(device.to_string(), notice.to_string());
        ProbeOutcome::Committed
    }

    fn invalidate_authority(&mut self, device: &str, notice: &str) {
        self.in_flight.remove(device);
        self.retire_observation(device);
        self.selections.remove(device);
        self.mark_setup_locations_unresolved(device);
        self.notices.insert(device.to_string(), notice.to_string());
    }

    fn note_stale_session(&mut self, device: &str) {
        self.invalidate_authority(
            device,
            "This device's storage session changed. Refresh storage options and choose again.",
        );
    }

    fn reconcile_selection(&mut self, device: &str, now: Instant) {
        if self.observations.get(device).is_some_and(|observation| {
            now.saturating_duration_since(observation.fetched_at) >= CACHE_TTL
        }) {
            self.drop_expired_observation(device, now);
            return;
        }
        let Some(selection) = self.selections.get(device).cloned() else {
            return;
        };
        let Some(observation) = self.observations.get(device) else {
            self.selections.remove(device);
            self.note(
                device,
                "The selected storage could not be verified. Choose a storage option again.",
            );
            return;
        };
        if observation.session_epoch != selection.session_epoch {
            self.selections.remove(device);
            self.note(
                device,
                "This device's storage session changed. Choose a storage option again.",
            );
            return;
        }
        let matched = observation
            .volumes
            .iter()
            .find(|volume| volume.identity_token == selection.identity_token);
        match matched {
            None => {
                self.selections.remove(device);
                self.note(
                    device,
                    "The selected storage is no longer present. Choose a storage option again.",
                );
            }
            Some(volume) if volume.status == ObservedVolumeStatus::Ambiguous => {
                self.selections.remove(device);
                self.note(
                    device,
                    "The selected storage can no longer be identified. Choose a storage option again.",
                );
            }
            Some(volume) if !volume.is_selectable() => {
                self.selections.remove(device);
                self.note(
                    device,
                    "The selected storage is no longer available. Choose a storage option again.",
                );
            }
            Some(volume) if volume.continuity_token != selection.continuity_token => {
                self.selections.remove(device);
                self.note(
                    device,
                    "The selected storage changed on the device. Choose a storage option again.",
                );
            }
            Some(_) => {}
        }
    }

    fn note(&mut self, device: &str, notice: &str) {
        self.notices.insert(device.to_string(), notice.to_string());
    }

    fn clear_selection(&mut self, device: &str) {
        self.selections.remove(device);
        self.notices.remove(device);
    }

    fn commit_selection(
        &mut self,
        device: &str,
        handle: &str,
        identity_token: &str,
        continuity_token: &str,
        session_epoch: u64,
    ) {
        self.notices.remove(device);
        self.selections.insert(
            device.to_string(),
            VolumeSelection {
                session_epoch,
                volume_handle: handle.to_string(),
                identity_token: identity_token.to_string(),
                continuity_token: continuity_token.to_string(),
            },
        );
    }

    fn known_retired_handle(&self, device: &str, handle: &str) -> bool {
        self.retired_handles
            .iter()
            .any(|retired| retired.device_handle == device && retired.volume_handle == handle)
    }

    fn project(
        &mut self,
        record: &DeviceRecord,
        runtime_generation: u64,
        platform_tools_revision: u64,
        evidence: &str,
        now: Instant,
    ) -> DeviceVolumesProjection {
        self.reconcile_selection(&record.handle, now);
        let observation = self.current_observation(
            &record.handle,
            record.session_epoch,
            runtime_generation,
            platform_tools_revision,
            now,
        );
        let mut volumes = Vec::new();
        let mut discovery_status = DiscoveryStatus::Unknown;
        let mut observation_token = String::new();
        let has_observation = observation.is_some();
        if let Some(observation) = observation {
            discovery_status = observation.discovery_status;
            observation_token = observation.observation_token.clone();
            let sd_total = observation
                .volumes
                .iter()
                .filter(|volume| volume.kind == ObservedVolumeKind::RemovableSd)
                .count();
            let mut sd_index = 0usize;
            let mut handle_counts = HashMap::<String, usize>::new();
            for volume in &observation.volumes {
                *handle_counts
                    .entry(volume_handle(
                        &record.handle,
                        observation.session_epoch,
                        &volume.identity_token,
                    ))
                    .or_default() += 1;
            }
            let mut handle_ordinals = HashMap::<String, usize>::new();
            for volume in &observation.volumes {
                if volume.kind == ObservedVolumeKind::RemovableSd {
                    sd_index += 1;
                }
                let handle = volume_handle(
                    &record.handle,
                    observation.session_epoch,
                    &volume.identity_token,
                );
                let ordinal = {
                    let entry = handle_ordinals.entry(handle.clone()).or_insert(0);
                    *entry += 1;
                    *entry
                };
                let presentation_key = if handle_counts.get(&handle).copied().unwrap_or(0) > 1 {
                    format!("{handle}:{ordinal}")
                } else {
                    handle.clone()
                };
                volumes.push(ProjectedVolume {
                    presentation_key,
                    volume_handle: handle,
                    kind: kind_name(volume.kind).to_string(),
                    label: volume_label(volume.kind, sd_index, sd_total),
                    status: status_name(volume.status).to_string(),
                    selectable: volume.is_selectable(),
                    detail: volume_detail(volume.kind, volume.status, volume.mount_state),
                });
            }
        } else if self.in_flight.contains_key(&record.handle) {
            discovery_status = DiscoveryStatus::Refreshing;
        }

        DeviceVolumesProjection {
            device_handle: record.handle.clone(),
            session_epoch: record.session_epoch,
            observation_token,
            discovery_status: discovery_status.as_str().to_string(),
            evidence: if has_observation {
                evidence.to_string()
            } else {
                "unavailable".to_string()
            },
            volumes,
            selected_volume_handle: self
                .selections
                .get(&record.handle)
                .map(|selection| selection.volume_handle.clone()),
            selection_notice: self.notices.get(&record.handle).cloned(),
        }
    }
}

/// Selection-safe projection handed to React.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceVolumesProjection {
    device_handle: String,
    session_epoch: u64,
    observation_token: String,
    discovery_status: String,
    evidence: String,
    volumes: Vec<ProjectedVolume>,
    selected_volume_handle: Option<String>,
    selection_notice: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectedVolume {
    presentation_key: String,
    volume_handle: String,
    kind: String,
    label: String,
    status: String,
    selectable: bool,
    detail: String,
}

/// Everything one probe needs, captured before any device work begins.
pub(crate) struct VolumeProbeContext {
    pub(crate) device_handle: String,
    pub(crate) serial: String,
    pub(crate) adb_path: String,
    pub(crate) session_epoch: u64,
    pub(crate) runtime_generation: u64,
    pub(crate) platform_tools_revision: u64,
}

fn unknown_device_error() -> String {
    safe_error(
        "unknown_device_handle",
        "This device is no longer available.",
    )
}

fn stale_device_session_error() -> String {
    safe_error(
        "stale_device_session",
        "This device's storage session changed. Refresh storage options before continuing.",
    )
}

fn sidecar_unavailable_error() -> String {
    safe_error(
        "sidecar_unavailable",
        "Device storage options are temporarily unavailable.",
    )
}

fn volume_not_found_error() -> String {
    safe_error(
        "volume_not_found",
        "That storage option is no longer available.",
    )
}

fn volume_not_selectable_error() -> String {
    safe_error(
        "volume_not_selectable",
        "That storage option cannot be selected.",
    )
}

fn stale_context_error() -> String {
    safe_error(
        "stale_context",
        "Device storage options changed while they were being read. Refresh storage options and choose again.",
    )
}

fn stale_observation_error() -> String {
    safe_error(
        "stale_observation",
        "The storage report changed. Refresh storage options and choose again.",
    )
}

fn lock_handles(state: &AppState) -> Result<MutexGuard<'_, SessionHandles>, String> {
    state
        .handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))
}

/// Resolve one probe context from current native authority.
pub(crate) fn volume_probe_context(
    state: &AppState,
    device_handle: &str,
) -> Result<VolumeProbeContext, String> {
    let adb = current_adb_runtime_snapshot(state)?;
    let runtime_generation = state
        .sidecar
        .try_generation()
        .map_err(|_| sidecar_unavailable_error())?;
    let (serial, session_epoch) = {
        let handles = lock_handles(state)?;
        let record = handles
            .device(device_handle)
            .map_err(|_| unknown_device_error())?;
        (record.serial.clone(), record.session_epoch)
    };
    Ok(VolumeProbeContext {
        device_handle: device_handle.to_string(),
        serial,
        adb_path: adb.adb_path,
        session_epoch,
        runtime_generation,
        platform_tools_revision: adb.revision,
    })
}

/// Re-read the native provenance that authorizes one storage observation.
///
/// The runtime generation identifies the sidecar instance. The Platform-Tools
/// revision identifies the ADB installation whose path was captured before the
/// request started.
pub(crate) fn current_volume_provenance(state: &AppState) -> Result<(u64, u64), String> {
    let runtime_generation = state
        .sidecar
        .try_generation()
        .map_err(|_| sidecar_unavailable_error())?;
    let adb = current_adb_runtime_snapshot(state)?;
    Ok((runtime_generation, adb.revision))
}

/// Observe storage volumes for one device session.
///
/// Returns true when this call committed a fresh observation, so the caller can
/// label the projection as live evidence.
#[cfg(test)]
pub(crate) fn observe_device_volumes<F>(
    handles: &Mutex<SessionHandles>,
    context: &VolumeProbeContext,
    refresh: bool,
    request: &mut F,
    now: Instant,
) -> Result<bool, String>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
{
    let runtime_generation = context.runtime_generation;
    let platform_tools_revision = context.platform_tools_revision;
    let mut clock = move || now;
    observe_device_volumes_with_provenance(
        handles,
        context,
        refresh,
        request,
        &mut || Ok((runtime_generation, platform_tools_revision)),
        now,
        &mut clock,
    )
}

/// Observe storage volumes and re-read native provenance before committing.
///
/// The provenance callback runs after the sidecar request returns and supplies
/// the current runtime generation and Platform-Tools revision. A changed value
/// discards the observation, invalidates the retained selection, and surfaces a
/// stale-context error. The handles lock is never held while the request or the
/// provenance re-read runs.
pub(crate) fn observe_device_volumes_with_provenance<F, P, C>(
    handles: &Mutex<SessionHandles>,
    context: &VolumeProbeContext,
    refresh: bool,
    request: &mut F,
    reread_provenance: &mut P,
    request_start: Instant,
    clock: &mut C,
) -> Result<bool, String>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
    P: FnMut() -> Result<(u64, u64), String>,
    C: FnMut() -> Instant,
{
    let sequence = {
        let mut guard = handles.lock().map_err(|_| {
            safe_error("session_state_unavailable", "Session state is unavailable.")
        })?;
        let store = guard.device_storage_mut();
        if !refresh {
            if let Some(observation) = store.current_observation(
                &context.device_handle,
                context.session_epoch,
                context.runtime_generation,
                context.platform_tools_revision,
                request_start,
            ) {
                if request_start.saturating_duration_since(observation.fetched_at) < CACHE_TTL {
                    return Ok(false);
                }
            }
        }
        match store.begin_probe(
            &context.device_handle,
            context.session_epoch,
            context.runtime_generation,
            context.platform_tools_revision,
            request_start,
        ) {
            ProbeStart::Busy => return Ok(false),
            ProbeStart::Started(sequence) => sequence,
        }
    };

    let decoded = match request(
        "listDeviceVolumes",
        json!({ "adbPath": context.adb_path, "serial": context.serial }),
    ) {
        Ok(value) => decode_observation(&value),
        Err(_) => None,
    };
    let provenance = reread_provenance();
    let completed_at = clock();

    let mut guard = handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?;
    let current_epoch = guard
        .device(&context.device_handle)
        .map(|record| record.session_epoch)
        .unwrap_or(u64::MAX);
    let store = guard.device_storage_mut();
    if current_epoch != context.session_epoch {
        if store.in_flight.get(&context.device_handle) != Some(&sequence) {
            return Ok(false);
        }
        store.note_stale_session(&context.device_handle);
        return Err(stale_device_session_error());
    }
    match provenance {
        Ok((runtime_generation, platform_tools_revision))
            if runtime_generation == context.runtime_generation
                && platform_tools_revision == context.platform_tools_revision => {}
        Ok(_) => {
            if store.in_flight.get(&context.device_handle) != Some(&sequence) {
                return Ok(false);
            }
            store.invalidate_authority(
                &context.device_handle,
                "Device storage options changed while they were being read. Your selection was cleared. Refresh storage options and choose again.",
            );
            return Err(stale_context_error());
        }
        Err(_) => {
            let outcome = store.fail_probe(
                &context.device_handle,
                sequence,
                "Storage options could not be read. Your selection was cleared. Choose a storage option again.",
            );
            return match outcome {
                ProbeOutcome::Superseded => Ok(false),
                ProbeOutcome::Committed => Err(sidecar_unavailable_error()),
            };
        }
    }
    let Some(decoded) = decoded else {
        let outcome = store.fail_probe(
            &context.device_handle,
            sequence,
            "Storage options could not be read. Your selection was cleared. Choose a storage option again.",
        );
        return match outcome {
            ProbeOutcome::Superseded => Ok(false),
            ProbeOutcome::Committed => Err(sidecar_unavailable_error()),
        };
    };
    match store.commit_probe(
        &context.device_handle,
        sequence,
        context.session_epoch,
        context.runtime_generation,
        context.platform_tools_revision,
        decoded.discovery_status,
        decoded.observation_token,
        decoded.volumes,
        completed_at,
    ) {
        ProbeOutcome::Committed => Ok(true),
        ProbeOutcome::Superseded => Ok(false),
    }
}

/// Invalidate any retained observation and selection after an error that
/// happened before a probe could establish fresh authority.
pub(crate) fn invalidate_device_storage_authority(
    handles: &Mutex<SessionHandles>,
    device_handle: &str,
) {
    if let Ok(mut guard) = handles.lock() {
        guard.device_storage_mut().invalidate_authority(
            device_handle,
            "Storage options could not be read. Your selection was cleared. Choose a storage option again.",
        );
    }
}

/// Fail a storage command when the captured native provenance no longer
/// matches the current runtime generation or Platform-Tools revision.
pub(crate) fn ensure_volume_context_current(
    state: &AppState,
    context: &VolumeProbeContext,
) -> Result<(), String> {
    let (runtime_generation, platform_tools_revision) = match current_volume_provenance(state) {
        Ok(values) => values,
        Err(error) => {
            invalidate_device_storage_authority(&state.handles, &context.device_handle);
            return Err(error);
        }
    };
    if runtime_generation != context.runtime_generation
        || platform_tools_revision != context.platform_tools_revision
    {
        invalidate_device_storage_authority(&state.handles, &context.device_handle);
        return Err(stale_context_error());
    }
    Ok(())
}

/// Allocate the per-device request sequence used to keep an older selection
/// response from overwriting a newer one.
pub(crate) fn begin_selection_request(
    handles: &Mutex<SessionHandles>,
    device_handle: &str,
) -> Result<u64, String> {
    let mut guard = handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?;
    guard
        .device(device_handle)
        .map_err(|_| unknown_device_error())?;
    Ok(guard
        .device_storage_mut()
        .next_selection_request(device_handle))
}

/// Build the current projection for one device.
pub(crate) fn project_device_volumes(
    handles: &Mutex<SessionHandles>,
    context: &VolumeProbeContext,
    evidence: &str,
    now: Instant,
) -> Result<DeviceVolumesProjection, String> {
    let mut guard = handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?;
    let record = guard
        .device(&context.device_handle)
        .map_err(|_| unknown_device_error())?
        .clone();
    if record.session_epoch != context.session_epoch {
        guard
            .device_storage_mut()
            .note_stale_session(&context.device_handle);
        return Err(stale_device_session_error());
    }
    let store = guard.device_storage_mut();
    Ok(store.project(
        &record,
        context.runtime_generation,
        context.platform_tools_revision,
        evidence,
        now,
    ))
}

#[tauri::command]
pub(crate) fn list_setup_locations(
    state: tauri::State<'_, AppState>,
    device_handle: &str,
    device_plan: &str,
) -> Result<Vec<SetupLocationProjection>, String> {
    let context = volume_probe_context(&state, device_handle)?;
    let mut guard = lock_handles(&state)?;
    let record = guard
        .device(device_handle)
        .map_err(|_| unknown_device_error())?;
    if record.session_epoch != context.session_epoch {
        return Err(stale_device_session_error());
    }
    Ok(guard.device_storage_mut().project_setup_locations(
        device_handle,
        device_plan,
        context.session_epoch,
        context.runtime_generation,
        context.platform_tools_revision,
        Instant::now(),
    ))
}

#[tauri::command]
pub(crate) fn save_setup_location(
    state: tauri::State<'_, AppState>,
    device_handle: &str,
    device_plan: &str,
    observation_token: &str,
    volume_handle: &str,
    existing_id: Option<&str>,
    name: &str,
    relative_path: &str,
) -> Result<Vec<SetupLocationProjection>, String> {
    let context = volume_probe_context(&state, device_handle)?;
    let mut guard = lock_handles(&state)?;
    let record = guard
        .device(device_handle)
        .map_err(|_| unknown_device_error())?;
    if record.session_epoch != context.session_epoch {
        return Err(stale_device_session_error());
    }
    guard.device_storage_mut().save_setup_location(
        device_handle,
        device_plan,
        context.session_epoch,
        context.runtime_generation,
        context.platform_tools_revision,
        observation_token,
        volume_handle,
        existing_id,
        name,
        relative_path,
        Instant::now(),
    )
}

#[tauri::command]
pub(crate) fn remove_setup_location(
    state: tauri::State<'_, AppState>,
    device_handle: &str,
    device_plan: &str,
    id: &str,
) -> Result<Vec<SetupLocationProjection>, String> {
    let context = volume_probe_context(&state, device_handle)?;
    let mut guard = lock_handles(&state)?;
    let record = guard
        .device(device_handle)
        .map_err(|_| unknown_device_error())?;
    if record.session_epoch != context.session_epoch {
        return Err(stale_device_session_error());
    }
    let storage = guard.device_storage_mut();
    if !storage.delete_setup_location(device_handle, device_plan, id) {
        return Err("That Setup Location is no longer available.".to_string());
    }
    Ok(storage.project_setup_locations(
        device_handle,
        device_plan,
        context.session_epoch,
        context.runtime_generation,
        context.platform_tools_revision,
        Instant::now(),
    ))
}

pub(crate) fn setup_location_bindings(
    state: &AppState,
    device_handle: &str,
    device_plan: &str,
    ids: &[String],
) -> Result<Vec<StorageTargetBinding>, String> {
    let context = volume_probe_context(state, device_handle)?;
    let mut guard = lock_handles(state)?;
    let record = guard
        .device(device_handle)
        .map_err(|_| unknown_device_error())?;
    if record.session_epoch != context.session_epoch {
        return Err(stale_device_session_error());
    }
    guard.device_storage_mut().setup_location_bindings(
        device_handle,
        device_plan,
        context.session_epoch,
        context.runtime_generation,
        context.platform_tools_revision,
        ids,
        Instant::now(),
    )
}

fn kind_name(kind: ObservedVolumeKind) -> &'static str {
    match kind {
        ObservedVolumeKind::InternalShared => "internalShared",
        ObservedVolumeKind::RemovableSd => "removableSd",
        ObservedVolumeKind::AdoptedPrivate => "adoptedPrivate",
        ObservedVolumeKind::Usb => "usb",
        ObservedVolumeKind::Unknown => "unknown",
    }
}

fn status_name(status: ObservedVolumeStatus) -> &'static str {
    match status {
        ObservedVolumeStatus::Available => "available",
        ObservedVolumeStatus::Unmounted => "unmounted",
        ObservedVolumeStatus::Ambiguous => "ambiguous",
        ObservedVolumeStatus::Unsupported => "unsupported",
        ObservedVolumeStatus::Unknown => "unknown",
    }
}

fn volume_label(kind: ObservedVolumeKind, sd_index: usize, sd_total: usize) -> String {
    match kind {
        ObservedVolumeKind::InternalShared => "Internal storage".to_string(),
        ObservedVolumeKind::RemovableSd => {
            if sd_total <= 1 {
                "SD card".to_string()
            } else {
                format!("SD card {sd_index}")
            }
        }
        ObservedVolumeKind::AdoptedPrivate => "Adopted storage".to_string(),
        ObservedVolumeKind::Usb => "USB storage".to_string(),
        ObservedVolumeKind::Unknown => "Unknown storage".to_string(),
    }
}

fn storage_target_kind(kind: Option<ObservedVolumeKind>) -> Option<StorageTargetVolumeKind> {
    match kind? {
        ObservedVolumeKind::InternalShared => Some(StorageTargetVolumeKind::InternalShared),
        ObservedVolumeKind::RemovableSd => Some(StorageTargetVolumeKind::RemovableSd),
        _ => None,
    }
}

fn volume_handle_for(device: &str, session_epoch: u64, volume: &ObservedVolume) -> String {
    volume_handle(device, session_epoch, &volume.identity_token)
}

fn project_setup_location_records(
    locations: &[SetupLocationRecord],
    observation: Option<&VolumeObservation>,
    now: Instant,
) -> Vec<SetupLocationProjection> {
    locations
        .iter()
        .map(|location| {
            let observation_is_fresh = observation.is_some_and(|observation| {
                now.saturating_duration_since(observation.fetched_at) < CACHE_TTL
                    && observation.session_epoch == location.binding.session_epoch
                    && observation.observation_token == location.binding.observation_token
            });
            let volume_is_current = observation.is_some_and(|observation| {
                observation.volumes.iter().any(|volume| {
                    volume.identity_token == location.binding.volume_identity
                        && volume.continuity_token == location.binding.volume_continuity
                        && volume.mount_root.as_deref()
                            == Some(location.binding.mount_root.as_str())
                        && storage_target_kind(volume.setup_location_kind)
                            == Some(location.binding.volume_kind)
                        && volume.is_setup_location_available()
                })
            });
            let ready = location.resolved && observation_is_fresh && volume_is_current;
            let (volume_kind, volume_label) = match location.binding.volume_kind {
                StorageTargetVolumeKind::InternalShared => ("internalShared", "Internal storage"),
                StorageTargetVolumeKind::RemovableSd => ("removableSd", "SD card"),
            };
            SetupLocationProjection {
                id: location.binding.id.clone(),
                name: location.binding.name.clone(),
                relative_path: location.binding.relative_path.clone(),
                volume_kind: volume_kind.to_string(),
                volume_label: volume_label.to_string(),
                ready,
                detail: if ready {
                    "Available for this setup.".to_string()
                } else {
                    "Refresh storage and choose this volume again before review.".to_string()
                },
            }
        })
        .collect()
}

fn volume_detail(
    kind: ObservedVolumeKind,
    status: ObservedVolumeStatus,
    mount_state: ObservedMountState,
) -> String {
    match status {
        ObservedVolumeStatus::Available => match mount_state {
            ObservedMountState::MountedRo => {
                "Mounted read-only. EmuChef cannot use read-only storage as a destination."
                    .to_string()
            }
            _ => "Mounted. EmuChef has not verified write access.".to_string(),
        },
        ObservedVolumeStatus::Unmounted => {
            "Not currently mounted. Check the storage on your device, then refresh.".to_string()
        }
        ObservedVolumeStatus::Ambiguous => {
            "EmuChef cannot tell these storage options apart. Resolve the storage state on your device, then refresh."
                .to_string()
        }
        ObservedVolumeStatus::Unsupported => match kind {
            ObservedVolumeKind::Usb => "USB storage is not a supported destination.".to_string(),
            ObservedVolumeKind::AdoptedPrivate => {
                "Adopted storage is managed by Android and is not a supported destination."
                    .to_string()
            }
            ObservedVolumeKind::RemovableSd => {
                match mount_state {
                    ObservedMountState::MountedRw | ObservedMountState::MountedRo => {
                        "This SD card is mounted but not visible to apps. Eject and reinsert it, then refresh."
                            .to_string()
                    }
                    ObservedMountState::Unmounted => {
                        "This SD card is not currently mounted and is not visible to apps. Check the storage on your device, then refresh."
                            .to_string()
                    }
                    ObservedMountState::Unknown => {
                        "EmuChef could not verify this SD card's mount state or visibility. Refresh to try again."
                            .to_string()
                    }
                }
            }
            _ => "This storage type is not a supported destination.".to_string(),
        },
        ObservedVolumeStatus::Unknown => {
            "EmuChef could not verify this storage option. Refresh to try again.".to_string()
        }
    }
}

fn volume_handle(device_handle: &str, session_epoch: u64, identity_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(HANDLE_DOMAIN.as_bytes());
    hasher.update([0u8]);
    hasher.update(device_handle.as_bytes());
    hasher.update([0u8]);
    hasher.update(session_epoch.to_string().as_bytes());
    hasher.update([0u8]);
    hasher.update(identity_token.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct DecodedObservation {
    discovery_status: DiscoveryStatus,
    observation_token: String,
    volumes: Vec<ObservedVolume>,
}

/// Decode one sidecar payload.
///
/// Anything that does not match the exact expected shape is rejected rather
/// than projected, so unexpected text can never reach the user interface.
fn decode_observation(value: &Value) -> Option<DecodedObservation> {
    let state = value.get("state")?.as_str()?;
    let discovery_status = match state {
        "observed" => DiscoveryStatus::Observed,
        "no_device" => DiscoveryStatus::NoDevice,
        "multiple_devices" => DiscoveryStatus::MultipleDevices,
        "unauthorized" => DiscoveryStatus::Unauthorized,
        "offline" => DiscoveryStatus::Offline,
        _ => return None,
    };
    let evidence_token = value.get("observationToken")?.as_str()?;
    if !is_opaque_token(evidence_token) {
        return None;
    }
    let raw_volumes = value.get("volumes")?.as_array()?;
    let mut volumes = Vec::with_capacity(raw_volumes.len());
    for raw in raw_volumes {
        let identity_token = raw.get("identityToken")?.as_str()?;
        if !is_opaque_token(identity_token) {
            return None;
        }
        let continuity_token = raw.get("continuityToken")?.as_str()?;
        if !is_opaque_token(continuity_token) {
            return None;
        }
        let kind = ObservedVolumeKind::parse(raw.get("kind")?.as_str()?)?;
        let status = ObservedVolumeStatus::parse(raw.get("status")?.as_str()?)?;
        let mount_state = ObservedMountState::parse(raw.get("mountState")?.as_str()?)?;
        let reasons = raw.get("reasons")?.as_array()?;
        if !reasons
            .iter()
            .all(|reason| reason.as_str().is_some_and(is_reason_key))
        {
            return None;
        }
        let setup_location_kind = match raw.get("setupLocationKind") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(ObservedVolumeKind::parse(value)?),
            _ => return None,
        };
        let mount_root = match raw.get("mountRoot") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            _ => return None,
        };
        if !valid_setup_location_evidence(
            kind,
            status,
            mount_state,
            setup_location_kind,
            mount_root.as_deref(),
            reasons.iter().filter_map(Value::as_str),
        ) {
            return None;
        }
        volumes.push(ObservedVolume {
            identity_token: identity_token.to_string(),
            continuity_token: continuity_token.to_string(),
            kind,
            status,
            mount_state,
            setup_location_kind,
            mount_root,
        });
    }
    if discovery_status != DiscoveryStatus::Observed && !volumes.is_empty() {
        return None;
    }
    Some(DecodedObservation {
        discovery_status,
        observation_token: uuid::Uuid::new_v4().simple().to_string(),
        volumes,
    })
}

fn valid_setup_location_evidence<'a>(
    kind: ObservedVolumeKind,
    status: ObservedVolumeStatus,
    mount_state: ObservedMountState,
    setup_kind: Option<ObservedVolumeKind>,
    mount_root: Option<&str>,
    reasons: impl Iterator<Item = &'a str>,
) -> bool {
    let reasons = reasons.collect::<Vec<_>>();
    match (setup_kind, mount_root) {
        (None, None) => true,
        (Some(ObservedVolumeKind::InternalShared), Some("/storage/emulated/0")) => {
            kind == ObservedVolumeKind::InternalShared
                && status == ObservedVolumeStatus::Available
                && mount_state == ObservedMountState::MountedRw
                && reasons.contains(&"emulated_user_associated")
                && !reasons.contains(&"primary_storage_adopted")
        }
        (Some(ObservedVolumeKind::RemovableSd), Some(root)) => {
            let supported_root = root.strip_prefix("/storage/").is_some_and(|part| {
                !part.is_empty()
                    && !part.contains('/')
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            });
            supported_root
                && status == ObservedVolumeStatus::Available
                && mount_state == ObservedMountState::MountedRw
                && ((kind == ObservedVolumeKind::RemovableSd
                    && reasons.contains(&"disk_removable_sd"))
                    || (kind == ObservedVolumeKind::InternalShared
                        && reasons.contains(&"primary_storage_physical")))
        }
        _ => false,
    }
}

fn is_opaque_token(value: &str) -> bool {
    value.len() == 32 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn is_reason_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

#[tauri::command(async)]
pub fn get_device_volumes(
    device_handle: String,
    refresh: bool,
    state: State<'_, AppState>,
) -> Result<DeviceVolumesProjection, String> {
    let mut request =
        |request_type: &str, payload: Value| state.sidecar.request(request_type, payload);
    let mut clock = Instant::now;
    get_device_volumes_with_runtime(&state, &device_handle, refresh, &mut request, &mut clock)
}

pub(crate) fn get_device_volumes_with_runtime<F, C>(
    state: &AppState,
    device_handle: &str,
    refresh: bool,
    request: &mut F,
    clock: &mut C,
) -> Result<DeviceVolumesProjection, String>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
    C: FnMut() -> Instant,
{
    let request_start = clock();
    let mut provenance = || current_volume_provenance(state);
    let context = match volume_probe_context(state, device_handle) {
        Ok(context) => context,
        Err(error) => {
            invalidate_device_storage_authority(&state.handles, device_handle);
            return Err(error);
        }
    };
    let fresh = observe_device_volumes_with_provenance(
        &state.handles,
        &context,
        refresh,
        request,
        &mut provenance,
        request_start,
        clock,
    )?;
    ensure_volume_context_current(state, &context)?;
    let projected_at = clock();
    project_device_volumes(
        &state.handles,
        &context,
        if fresh { "live" } else { "cached" },
        projected_at,
    )
}

#[tauri::command(async)]
pub fn select_device_volume(
    device_handle: String,
    volume_handle: Option<String>,
    observation_token: Option<String>,
    state: State<'_, AppState>,
) -> Result<DeviceVolumesProjection, String> {
    let mut request =
        |request_type: &str, payload: Value| state.sidecar.request(request_type, payload);
    let mut clock = Instant::now;
    select_device_volume_with_runtime(
        &state,
        &device_handle,
        volume_handle.as_deref(),
        observation_token.as_deref(),
        &mut request,
        &mut clock,
    )
}

pub(crate) fn select_device_volume_with_runtime<F, C>(
    state: &AppState,
    device_handle: &str,
    volume_handle: Option<&str>,
    observation_token: Option<&str>,
    request: &mut F,
    clock: &mut C,
) -> Result<DeviceVolumesProjection, String>
where
    F: FnMut(&str, Value) -> Result<Value, String>,
    C: FnMut() -> Instant,
{
    let request_start = clock();
    let mut provenance = || current_volume_provenance(state);
    let selection_sequence = match begin_selection_request(&state.handles, device_handle) {
        Ok(sequence) => sequence,
        Err(error) => {
            invalidate_device_storage_authority(&state.handles, device_handle);
            return Err(error);
        }
    };
    let context = match volume_probe_context(state, device_handle) {
        Ok(context) => context,
        Err(error) => {
            invalidate_device_storage_authority(&state.handles, device_handle);
            return Err(error);
        }
    };
    let fresh = observe_device_volumes_with_provenance(
        &state.handles,
        &context,
        false,
        request,
        &mut provenance,
        request_start,
        clock,
    )?;
    ensure_volume_context_current(state, &context)?;
    let admitted_at = clock();
    admit_device_volume_selection_at_sequence(
        &state.handles,
        &context,
        volume_handle,
        observation_token,
        selection_sequence,
        if fresh { "live" } else { "cached" },
        admitted_at,
    )
}

/// Admit or clear one storage selection against the current trusted
/// observation.
///
/// The function re-reads the retained device session after the probe
/// finishes, so a handle minted for an earlier session or an earlier
/// observation can never be admitted as current intent.
#[cfg(test)]
pub(crate) fn admit_device_volume_selection(
    handles: &Mutex<SessionHandles>,
    context: &VolumeProbeContext,
    requested_handle: Option<&str>,
    now: Instant,
) -> Result<DeviceVolumesProjection, String> {
    let sequence = begin_selection_request(handles, &context.device_handle)?;
    let expected_observation_token = if requested_handle.is_some() {
        let mut guard = handles.lock().map_err(|_| {
            safe_error("session_state_unavailable", "Session state is unavailable.")
        })?;
        guard
            .device_storage_mut()
            .current_observation(
                &context.device_handle,
                context.session_epoch,
                context.runtime_generation,
                context.platform_tools_revision,
                now,
            )
            .map(|observation| observation.observation_token.clone())
    } else {
        None
    };
    admit_device_volume_selection_at_sequence(
        handles,
        context,
        requested_handle,
        expected_observation_token.as_deref(),
        sequence,
        "cached",
        now,
    )
}

/// Admit or clear one selection against an already allocated request sequence.
///
/// A superseded response returns the current projection without committing
/// the older intent. A selection whose displayed observation token no longer
/// matches the current trusted observation is rejected with a fixed
/// stale-observation error and never rebinds the earlier choice.
pub(crate) fn admit_device_volume_selection_at_sequence(
    handles: &Mutex<SessionHandles>,
    context: &VolumeProbeContext,
    requested_handle: Option<&str>,
    expected_observation_token: Option<&str>,
    selection_sequence: u64,
    evidence: &str,
    now: Instant,
) -> Result<DeviceVolumesProjection, String> {
    let mut guard = handles
        .lock()
        .map_err(|_| safe_error("session_state_unavailable", "Session state is unavailable."))?;
    let record = guard
        .device(&context.device_handle)
        .map_err(|_| unknown_device_error())?
        .clone();
    let store = guard.device_storage_mut();
    if !store.selection_request_is_current(&context.device_handle, selection_sequence) {
        // A superseded request carries stale authority. Project the retained
        // observation using its provenance so the response cannot mix epochs.
        let (runtime_generation, platform_tools_revision) = store
            .observations
            .get(&record.handle)
            .filter(|observation| observation.session_epoch == record.session_epoch)
            .map(|observation| {
                (
                    observation.runtime_generation,
                    observation.platform_tools_revision,
                )
            })
            .unwrap_or((context.runtime_generation, context.platform_tools_revision));
        return Ok(store.project(
            &record,
            runtime_generation,
            platform_tools_revision,
            evidence,
            now,
        ));
    }
    if record.session_epoch != context.session_epoch {
        store.note_stale_session(&context.device_handle);
        return Err(stale_device_session_error());
    }
    match requested_handle {
        None => store.clear_selection(&context.device_handle),
        Some(handle) => {
            let observation = store
                .current_observation(
                    &context.device_handle,
                    context.session_epoch,
                    context.runtime_generation,
                    context.platform_tools_revision,
                    now,
                )
                .cloned()
                .ok_or_else(stale_device_session_error)?;
            if expected_observation_token != Some(observation.observation_token.as_str()) {
                return Err(stale_observation_error());
            }
            let found = observation
                .volumes
                .iter()
                .find(|volume| {
                    volume_handle(
                        &context.device_handle,
                        observation.session_epoch,
                        &volume.identity_token,
                    ) == handle
                })
                .cloned();
            let Some(volume) = found else {
                if store.known_retired_handle(&context.device_handle, handle) {
                    return Err(stale_device_session_error());
                }
                return Err(volume_not_found_error());
            };
            if !volume.is_selectable() {
                return Err(volume_not_selectable_error());
            }
            store.commit_selection(
                &context.device_handle,
                handle,
                &volume.identity_token,
                &volume.continuity_token,
                observation.session_epoch,
            );
        }
    }
    Ok(store.project(
        &record,
        context.runtime_generation,
        context.platform_tools_revision,
        evidence,
        now,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::adb::AdbManager;
    use crate::commands::{
        InputContractSnapshot, PlatformToolsSelectionStore, QualificationTransitionGate,
    };
    use crate::device_qualification::RootQualificationStore;
    use crate::execution::ExecutionHandleStore;
    use crate::qualification_repository::QualificationRepositoryProvider;
    use crate::qualification_session::QualificationSessionStore;
    use crate::recovery::RecoveryStore;
    use crate::saved_configurations::SavedConfigurationStore;
    use crate::sidecar::SidecarState;
    use crate::support::SupportStore;
    use crate::updates::{ActivityGate, UpdateService};

    use std::collections::VecDeque;

    const DEVICE: &str = "device-1";
    const ADB: &str = "/opt/emuchef/platform-tools/adb";
    const INTERNAL_IDENTITY: &str = "11111111111111111111111111111111";
    const SD_IDENTITY: &str = "22222222222222222222222222222222";
    const USB_IDENTITY: &str = "33333333333333333333333333333333";

    #[derive(Default)]
    struct ScriptedRequest {
        responses: VecDeque<Result<Value, String>>,
        calls: Vec<(String, Value)>,
    }

    impl ScriptedRequest {
        fn with(responses: Vec<Result<Value, String>>) -> Self {
            Self {
                responses: responses.into(),
                calls: Vec::new(),
            }
        }

        fn call(&mut self, request_type: &str, payload: Value) -> Result<Value, String> {
            self.calls.push((request_type.to_string(), payload));
            self.responses
                .pop_front()
                .expect("scripted sidecar request without a response")
        }
    }

    fn volume(identity: &str, kind: &str, status: &str, mount_state: &str) -> Value {
        volume_with_continuity(
            identity,
            "dddddddddddddddddddddddddddddddd",
            kind,
            status,
            mount_state,
        )
    }

    fn volume_with_continuity(
        identity: &str,
        continuity: &str,
        kind: &str,
        status: &str,
        mount_state: &str,
    ) -> Value {
        json!({
            "identityToken": identity,
            "continuityToken": continuity,
            "kind": kind,
            "status": status,
            "mountState": mount_state,
            "reasons": ["stub_reason"],
        })
    }

    fn observed_payload(observations: &[(&str, &str, &str, &str)]) -> Value {
        json!({
            "state": "observed",
            "observationToken": "aaaaaaaabbbbbbbbccccccccdddddddd",
            "volumes": observations
                .iter()
                .map(|(identity, kind, status, mount_state)| volume(identity, kind, status, mount_state))
                .collect::<Vec<_>>(),
        })
    }

    fn internal_and_sd() -> Value {
        observed_payload(&[
            (
                INTERNAL_IDENTITY,
                "internalShared",
                "available",
                "mountedRw",
            ),
            (SD_IDENTITY, "removableSd", "available", "mountedRw"),
        ])
    }

    fn sd_payload(continuity: &str, token: &str) -> Value {
        let mut payload = internal_and_sd();
        payload["observationToken"] = json!(token);
        for volume in payload["volumes"].as_array_mut().expect("volumes array") {
            if volume["kind"] == "removableSd" {
                volume["continuityToken"] = json!(continuity);
            }
        }
        payload
    }

    fn duplicate_sd_payload() -> Value {
        observed_payload(&[
            (
                INTERNAL_IDENTITY,
                "internalShared",
                "available",
                "mountedRw",
            ),
            (SD_IDENTITY, "removableSd", "ambiguous", "mountedRw"),
            (SD_IDENTITY, "removableSd", "ambiguous", "mountedRw"),
        ])
    }

    fn handles_at_epoch(epoch: u64) -> Mutex<SessionHandles> {
        let mut handles = SessionHandles::default();
        handles.retain_available_device_for_test(DEVICE, epoch);
        Mutex::new(handles)
    }

    fn update_inventory(handles: &mut SessionHandles, transport: &str) {
        handles
            .update_devices(&json!({
                "devices": [{
                    "serial": "T04-SERIAL",
                    "state": "available",
                    "model": "T04 Device",
                    "transportId": transport
                }]
            }))
            .expect("inventory should reconcile");
    }

    fn handles_with_inventory(transport: &str) -> (Mutex<SessionHandles>, String) {
        let mut handles = SessionHandles::default();
        update_inventory(&mut handles, transport);
        let handle = handles
            .qualification_devices()
            .into_iter()
            .next()
            .expect("retained device")
            .handle;
        (Mutex::new(handles), handle)
    }

    fn device_context(handle: &str, epoch: u64) -> VolumeProbeContext {
        VolumeProbeContext {
            device_handle: handle.to_string(),
            serial: "T04-SERIAL".to_string(),
            adb_path: ADB.to_string(),
            session_epoch: epoch,
            runtime_generation: 1,
            platform_tools_revision: 1,
        }
    }

    fn context(epoch: u64) -> VolumeProbeContext {
        VolumeProbeContext {
            device_handle: DEVICE.to_string(),
            serial: "test-device-1".to_string(),
            adb_path: ADB.to_string(),
            session_epoch: epoch,
            runtime_generation: 1,
            platform_tools_revision: 1,
        }
    }

    fn observe(
        handles: &Mutex<SessionHandles>,
        context: &VolumeProbeContext,
        payload: Value,
        now: Instant,
    ) -> DeviceVolumesProjection {
        let mut request = ScriptedRequest::with(vec![Ok(payload)]);
        observe_device_volumes(
            handles,
            context,
            true,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            now,
        )
        .expect("observation should commit");
        project_device_volumes(handles, context, "live", now).expect("projection")
    }

    fn handle_for(projection: &DeviceVolumesProjection, kind: &str) -> String {
        projection
            .volumes
            .iter()
            .find(|volume| volume.kind == kind)
            .unwrap_or_else(|| panic!("missing {kind} volume"))
            .volume_handle
            .clone()
    }

    #[test]
    fn frontend_freshness_token_is_not_the_sidecar_volume_evidence_digest() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let projection = observe(&handles, &context, internal_and_sd(), Instant::now());

        assert_ne!(
            projection.observation_token,
            "aaaaaaaabbbbbbbbccccccccdddddddd",
        );
    }

    fn clock_at(now: Instant) -> impl FnMut() -> Instant {
        move || now
    }

    fn volumes_at<F>(
        state: &AppState,
        device: &str,
        refresh: bool,
        request: &mut F,
        now: Instant,
    ) -> Result<DeviceVolumesProjection, String>
    where
        F: FnMut(&str, Value) -> Result<Value, String>,
    {
        let mut clock = clock_at(now);
        get_device_volumes_with_runtime(state, device, refresh, request, &mut clock)
    }

    fn selection_at<F>(
        state: &AppState,
        device: &str,
        volume: Option<&str>,
        token: Option<&str>,
        request: &mut F,
        now: Instant,
    ) -> Result<DeviceVolumesProjection, String>
    where
        F: FnMut(&str, Value) -> Result<Value, String>,
    {
        let mut clock = clock_at(now);
        select_device_volume_with_runtime(state, device, volume, token, request, &mut clock)
    }

    #[test]
    fn volume_handles_are_stable_for_the_same_session_and_identity() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let first = observe(&handles, &context, internal_and_sd(), base);
        let second = observe(
            &handles,
            &context,
            internal_and_sd(),
            base + Duration::from_secs(11),
        );

        assert_eq!(
            handle_for(&first, "removableSd"),
            handle_for(&second, "removableSd")
        );
        assert_eq!(
            handle_for(&first, "internalShared"),
            handle_for(&second, "internalShared")
        );
        assert_ne!(
            handle_for(&first, "removableSd"),
            handle_for(&first, "internalShared")
        );
    }

    #[test]
    fn volume_handles_change_when_the_device_session_changes() {
        let base = Instant::now();
        let first = observe(&handles_at_epoch(1), &context(1), internal_and_sd(), base);
        let second = observe(&handles_at_epoch(2), &context(2), internal_and_sd(), base);

        assert_ne!(
            handle_for(&first, "removableSd"),
            handle_for(&second, "removableSd")
        );
    }

    #[test]
    fn cached_observation_avoids_a_second_device_probe() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let mut request = ScriptedRequest::with(vec![Ok(internal_and_sd())]);

        assert!(observe_device_volumes(
            &handles,
            &context,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            base
        )
        .expect("first observation"));
        assert!(!observe_device_volumes(
            &handles,
            &context,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            base + Duration::from_secs(2)
        )
        .expect("cached observation"));

        assert_eq!(request.calls.len(), 1);
        assert_eq!(request.calls[0].0, "listDeviceVolumes");
        assert_eq!(request.calls[0].1["adbPath"], ADB);
        let projection =
            project_device_volumes(&handles, &context, "cached", base + Duration::from_secs(2))
                .expect("projection");
        assert_eq!(projection.evidence, "cached");
        assert_eq!(projection.discovery_status, "observed");
    }

    #[test]
    fn cache_expires_after_ten_seconds() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let mut request = ScriptedRequest::with(vec![Ok(internal_and_sd()), Ok(internal_and_sd())]);

        observe_device_volumes(
            &handles,
            &context,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            base,
        )
        .expect("first observation");
        assert!(observe_device_volumes(
            &handles,
            &context,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            base + Duration::from_secs(11)
        )
        .expect("expired observation should probe again"));
        assert_eq!(request.calls.len(), 2);
    }

    #[test]
    fn aged_observation_is_not_served_or_selected_while_a_probe_is_in_flight() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        {
            let mut guard = handles.lock().expect("handles");
            guard
                .device_storage_mut()
                .in_flight
                .insert(DEVICE.to_string(), 99);
        }
        let mut request = ScriptedRequest::with(vec![]);
        let committed = observe_device_volumes(
            &handles,
            &context,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            base + Duration::from_secs(11),
        )
        .expect("busy probe is not an error");
        assert!(!committed);

        let after =
            project_device_volumes(&handles, &context, "cached", base + Duration::from_secs(11))
                .expect("projection");
        assert_eq!(after.discovery_status, "refreshing");
        assert_eq!(after.evidence, "unavailable");
        assert!(after.volumes.is_empty());
        assert!(after.selected_volume_handle.is_none());

        let error = admit_device_volume_selection(
            &handles,
            &context,
            Some(&sd),
            base + Duration::from_secs(11),
        )
        .expect_err("an aged observation cannot admit a selection");
        assert!(error.contains("stale_device_session"), "error was {error}");
    }

    #[test]
    fn superseded_selection_response_cannot_revive_an_aged_selection() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        let selected =
            admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");
        assert_eq!(
            selected.selected_volume_handle.as_deref(),
            Some(sd.as_str())
        );

        // Supersede the selection request and keep a probe in flight while the
        // retained observation ages past the cache lifetime.
        let older = begin_selection_request(&handles, &context.device_handle)
            .expect("older selection sequence");
        let newer = begin_selection_request(&handles, &context.device_handle)
            .expect("newer selection sequence");
        assert!(newer > older);
        {
            let mut guard = handles.lock().expect("handles");
            guard
                .device_storage_mut()
                .in_flight
                .insert(DEVICE.to_string(), 99);
        }

        let late = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(&projection.observation_token),
            older,
            "live",
            base + Duration::from_secs(11),
        )
        .expect("superseded response should project the current state");

        assert!(late.selected_volume_handle.is_none());
        assert!(late.volumes.is_empty());
        assert_eq!(late.discovery_status, "refreshing");
        assert_eq!(late.evidence, "unavailable");
    }

    #[test]
    fn older_epoch_superseded_selection_response_preserves_newer_selection() {
        let base = Instant::now();
        let (handles, device) = handles_with_inventory("transport-1");
        let epoch_one = device_context(&device, 1);
        let first = observe(&handles, &epoch_one, internal_and_sd(), base);
        let old_sd = handle_for(&first, "removableSd");
        let old_token = first.observation_token.clone();
        let older_sequence =
            begin_selection_request(&handles, &device).expect("older selection sequence");

        {
            let mut guard = handles.lock().expect("handles");
            guard.invalidate_identity_authority(&device);
            update_inventory(&mut guard, "transport-1");
        }

        let epoch_two = {
            let guard = handles.lock().expect("handles");
            let epoch = guard
                .device(&device)
                .expect("redetected device")
                .session_epoch;
            device_context(&device, epoch)
        };
        let second = observe(&handles, &epoch_two, internal_and_sd(), base);
        let new_sd = handle_for(&second, "removableSd");
        let new_token = second.observation_token.clone();
        let newer_sequence =
            begin_selection_request(&handles, &device).expect("newer selection sequence");
        assert!(newer_sequence > older_sequence);

        admit_device_volume_selection_at_sequence(
            &handles,
            &epoch_two,
            Some(&new_sd),
            Some(&new_token),
            newer_sequence,
            "cached",
            base,
        )
        .expect("newer selection commits");

        let preserved = admit_device_volume_selection_at_sequence(
            &handles,
            &epoch_one,
            Some(&old_sd),
            Some(&old_token),
            older_sequence,
            "cached",
            base,
        )
        .expect("older superseded response returns the current projection");
        assert_eq!(
            preserved.selected_volume_handle.as_deref(),
            Some(new_sd.as_str())
        );

        let current = project_device_volumes(&handles, &epoch_two, "cached", base)
            .expect("current projection");
        assert_eq!(
            current.selected_volume_handle.as_deref(),
            Some(new_sd.as_str())
        );
        assert_eq!(current.observation_token, new_token);
    }

    #[test]
    fn superseded_stale_epoch_projection_uses_current_observation_provenance() {
        let base = Instant::now();
        let (handles, device) = handles_with_inventory("transport-1");
        let epoch_one = device_context(&device, 1);
        let first = observe(&handles, &epoch_one, internal_and_sd(), base);
        let old_sd = handle_for(&first, "removableSd");
        let old_token = first.observation_token.clone();
        let older_sequence =
            begin_selection_request(&handles, &device).expect("older selection sequence");

        {
            let mut guard = handles.lock().expect("handles");
            guard.invalidate_identity_authority(&device);
            update_inventory(&mut guard, "transport-1");
        }

        let mut epoch_two = {
            let guard = handles.lock().expect("handles");
            let epoch = guard
                .device(&device)
                .expect("redetected device")
                .session_epoch;
            device_context(&device, epoch)
        };
        epoch_two.runtime_generation = 2;
        epoch_two.platform_tools_revision = 2;
        let second = observe(&handles, &epoch_two, internal_and_sd(), base);
        let new_sd = handle_for(&second, "removableSd");
        let new_token = second.observation_token.clone();
        let newer_sequence =
            begin_selection_request(&handles, &device).expect("newer selection sequence");

        admit_device_volume_selection_at_sequence(
            &handles,
            &epoch_two,
            Some(&new_sd),
            Some(&new_token),
            newer_sequence,
            "cached",
            base,
        )
        .expect("newer selection commits");

        let preserved = admit_device_volume_selection_at_sequence(
            &handles,
            &epoch_one,
            Some(&old_sd),
            Some(&old_token),
            older_sequence,
            "cached",
            base,
        )
        .expect("older superseded response returns the current projection");
        assert_eq!(preserved.session_epoch, epoch_two.session_epoch);
        assert_eq!(preserved.observation_token, new_token);
        assert_eq!(
            preserved.selected_volume_handle.as_deref(),
            Some(new_sd.as_str())
        );
        assert!(preserved
            .volumes
            .iter()
            .any(|volume| volume.volume_handle == new_sd));
    }

    #[test]
    fn explicit_refresh_bypasses_the_cache() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let mut request = ScriptedRequest::with(vec![Ok(internal_and_sd()), Ok(internal_and_sd())]);

        observe_device_volumes(
            &handles,
            &context,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            base,
        )
        .expect("first observation");
        assert!(observe_device_volumes(
            &handles,
            &context,
            true,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            base + Duration::from_secs(1)
        )
        .expect("explicit refresh"));
        assert_eq!(request.calls.len(), 2);
    }

    #[test]
    fn selection_is_recorded_and_cleared_explicitly() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let now = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), now);
        let sd = handle_for(&projection, "removableSd");
        assert!(
            projection.selected_volume_handle.is_none(),
            "no default selection"
        );

        let selected =
            admit_device_volume_selection(&handles, &context, Some(&sd), now).expect("select");
        assert_eq!(
            selected.selected_volume_handle.as_deref(),
            Some(sd.as_str())
        );

        let cleared = admit_device_volume_selection(&handles, &context, None, now).expect("clear");
        assert!(cleared.selected_volume_handle.is_none());
        assert!(cleared.selection_notice.is_none());
    }

    #[test]
    fn stale_handle_from_a_previous_session_is_rejected() {
        let base = Instant::now();
        let (handles, device) = handles_with_inventory("transport-1");
        let context = device_context(&device, 1);
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        {
            let mut guard = handles.lock().expect("handles");
            update_inventory(&mut guard, "transport-2");
        }
        let renewed_context = device_context(&device, 2);
        observe(&handles, &renewed_context, internal_and_sd(), base);

        let error = admit_device_volume_selection(&handles, &renewed_context, Some(&sd), base)
            .expect_err("stale handle must be rejected");
        assert!(error.contains("stale_device_session"), "error was {error}");
    }

    #[test]
    fn unknown_handle_is_reported_as_not_found() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let now = Instant::now();
        observe(&handles, &context, internal_and_sd(), now);

        let error = admit_device_volume_selection(
            &handles,
            &context,
            Some("ffffffffffffffffffffffffffffffff"),
            now,
        )
        .expect_err("unknown handle must be rejected");
        assert!(error.contains("volume_not_found"), "error was {error}");
    }

    #[test]
    fn unsupported_volume_cannot_be_selected() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let now = Instant::now();
        let projection = observe(
            &handles,
            &context,
            observed_payload(&[
                (
                    INTERNAL_IDENTITY,
                    "internalShared",
                    "available",
                    "mountedRw",
                ),
                (USB_IDENTITY, "usb", "unsupported", "mountedRw"),
            ]),
            now,
        );
        let usb = handle_for(&projection, "usb");

        let error = admit_device_volume_selection(&handles, &context, Some(&usb), now)
            .expect_err("unsupported volume must not be selectable");
        assert!(error.contains("volume_not_selectable"), "error was {error}");
    }

    #[test]
    fn ambiguous_internal_volume_is_listed_but_not_selectable() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let now = Instant::now();
        let projection = observe(
            &handles,
            &context,
            observed_payload(&[
                (
                    INTERNAL_IDENTITY,
                    "internalShared",
                    "ambiguous",
                    "mountedRw",
                ),
                (SD_IDENTITY, "removableSd", "available", "mountedRw"),
            ]),
            now,
        );

        let internal = handle_for(&projection, "internalShared");
        let choice = projection
            .volumes
            .iter()
            .find(|volume| volume.volume_handle == internal)
            .expect("ambiguous internal volume");
        assert_eq!(choice.status, "ambiguous");
        assert!(!choice.selectable);
        let error = admit_device_volume_selection(&handles, &context, Some(&internal), now)
            .expect_err("ambiguous internal storage must not be selectable");
        assert!(error.contains("volume_not_selectable"), "error was {error}");
    }

    #[test]
    fn read_only_volume_is_listed_but_not_selectable() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let now = Instant::now();
        let projection = observe(
            &handles,
            &context,
            observed_payload(&[
                (
                    INTERNAL_IDENTITY,
                    "internalShared",
                    "available",
                    "mountedRw",
                ),
                (SD_IDENTITY, "removableSd", "available", "mountedRo"),
            ]),
            now,
        );
        let sd = handle_for(&projection, "removableSd");
        let choice = projection
            .volumes
            .iter()
            .find(|volume| volume.volume_handle == sd)
            .expect("read-only SD volume");
        assert_eq!(choice.status, "available");
        assert!(!choice.selectable);
        assert!(choice.detail.contains("read-only"));

        let error = admit_device_volume_selection(&handles, &context, Some(&sd), now)
            .expect_err("read-only storage is not a destination");
        assert!(error.contains("volume_not_selectable"), "error was {error}");
    }

    #[test]
    fn unmounted_unsupported_sd_detail_does_not_claim_it_is_mounted() {
        let detail = volume_detail(
            ObservedVolumeKind::RemovableSd,
            ObservedVolumeStatus::Unsupported,
            ObservedMountState::Unmounted,
        );
        assert!(detail.contains("not currently mounted"));
        assert!(!detail.contains("is mounted but"));
    }

    #[test]
    fn changed_card_identity_invalidates_the_selection() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        let replaced = observe(
            &handles,
            &context,
            observed_payload(&[
                (
                    INTERNAL_IDENTITY,
                    "internalShared",
                    "available",
                    "mountedRw",
                ),
                (
                    "44444444444444444444444444444444",
                    "removableSd",
                    "available",
                    "mountedRw",
                ),
            ]),
            base + Duration::from_secs(1),
        );

        assert!(replaced.selected_volume_handle.is_none());
        assert!(replaced
            .selection_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("no longer present")));
    }

    #[test]
    fn same_filesystem_uuid_with_different_disk_evidence_invalidates_the_selection() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        let changed = json!({
            "state": "observed",
            "observationToken": "aaaaaaaabbbbbbbbccccccccdddddddd",
            "volumes": [
                volume(INTERNAL_IDENTITY, "internalShared", "available", "mountedRw"),
                volume_with_continuity(
                    SD_IDENTITY,
                    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "removableSd",
                    "available",
                    "mountedRw"
                )
            ]
        });
        let after = observe(&handles, &context, changed, base + Duration::from_secs(1));

        assert!(after.selected_volume_handle.is_none());
        assert!(after
            .selection_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("changed")));
    }

    #[test]
    fn unmounted_selected_volume_invalidates_the_selection() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        let unmounted = observe(
            &handles,
            &context,
            observed_payload(&[
                (
                    INTERNAL_IDENTITY,
                    "internalShared",
                    "available",
                    "mountedRw",
                ),
                (SD_IDENTITY, "removableSd", "unmounted", "unmounted"),
            ]),
            base + Duration::from_secs(1),
        );

        assert!(unmounted.selected_volume_handle.is_none());
        assert!(unmounted
            .selection_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("no longer available")));
    }

    #[test]
    fn ambiguous_selected_volume_invalidates_the_selection() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        let ambiguous = observe(
            &handles,
            &context,
            observed_payload(&[
                (
                    INTERNAL_IDENTITY,
                    "internalShared",
                    "available",
                    "mountedRw",
                ),
                (SD_IDENTITY, "removableSd", "ambiguous", "mountedRw"),
            ]),
            base + Duration::from_secs(1),
        );

        assert!(ambiguous.selected_volume_handle.is_none());
        assert!(ambiguous
            .selection_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("can no longer be identified")));
    }

    #[test]
    fn probe_failure_clears_the_selection_and_reports_a_fixed_error() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        let mut request = ScriptedRequest::with(vec![Err(safe_error(
            "runtime_session_lost",
            "the runtime session is gone",
        ))]);
        let error = observe_device_volumes(
            &handles,
            &context,
            true,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            base + Duration::from_secs(1),
        )
        .expect_err("probe failure must surface");
        assert!(error.contains("sidecar_unavailable"), "error was {error}");

        let after =
            project_device_volumes(&handles, &context, "cached", base + Duration::from_secs(1))
                .expect("projection");
        assert!(after.selected_volume_handle.is_none());
        assert!(after
            .selection_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("selection was cleared")));
    }

    #[test]
    fn runtime_generation_change_invalidates_the_cached_observation() {
        let handles = handles_at_epoch(1);
        let base = Instant::now();
        let first = context(1);
        observe(&handles, &first, internal_and_sd(), base);
        let sd = handle_for(
            &project_device_volumes(&handles, &first, "live", base).expect("projection"),
            "removableSd",
        );
        admit_device_volume_selection(&handles, &first, Some(&sd), base).expect("select");

        let mut restarted = context(1);
        restarted.runtime_generation = 2;
        let mut request = ScriptedRequest::with(vec![Ok(internal_and_sd())]);
        assert!(
            observe_device_volumes(
                &handles,
                &restarted,
                false,
                &mut |request_type: &str, payload: Value| request.call(request_type, payload),
                base
            )
            .expect("probe after restart"),
            "a runtime generation change must force a fresh probe"
        );
        let projection =
            project_device_volumes(&handles, &restarted, "live", base).expect("projection");
        assert!(projection.selected_volume_handle.is_none());
    }

    #[test]
    fn platform_tools_revision_change_invalidates_the_cached_observation() {
        let handles = handles_at_epoch(1);
        let base = Instant::now();
        let first = context(1);
        observe(&handles, &first, internal_and_sd(), base);

        let mut replaced = context(1);
        replaced.platform_tools_revision = 2;
        let mut request = ScriptedRequest::with(vec![Ok(internal_and_sd())]);
        assert!(
            observe_device_volumes(
                &handles,
                &replaced,
                false,
                &mut |request_type: &str, payload: Value| request.call(request_type, payload),
                base
            )
            .expect("probe after replacement"),
            "a Platform-Tools revision change must force a fresh probe"
        );
    }

    #[test]
    fn superseded_probe_responses_are_discarded() {
        let mut store = DeviceStorageState::default();
        let ProbeStart::Started(sequence) = store.begin_probe(DEVICE, 1, 1, 1, Instant::now())
        else {
            panic!("first probe should start");
        };
        let outcome = store.commit_probe(
            DEVICE,
            sequence + 1,
            1,
            1,
            1,
            DiscoveryStatus::Observed,
            "aaaaaaaabbbbbbbbccccccccdddddddd".to_string(),
            Vec::new(),
            Instant::now(),
        );
        assert_eq!(outcome, ProbeOutcome::Superseded);
        assert!(!store.observations.contains_key(DEVICE));
        assert_eq!(store.in_flight.get(DEVICE), Some(&sequence));
    }

    #[test]
    fn late_probe_response_cannot_restore_an_invalidated_selection() {
        let (handles, device) = handles_with_inventory("transport-1");
        let context = device_context(&device, 1);
        let now = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), now);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), now).expect("select");

        let sequence = {
            let mut guard = handles.lock().expect("handles");
            match guard
                .device_storage_mut()
                .begin_probe(&context.device_handle, 1, 1, 1, now)
            {
                ProbeStart::Started(sequence) => sequence,
                ProbeStart::Busy => panic!("probe should start"),
            }
        };

        let mut guard = handles.lock().expect("handles");
        update_inventory(&mut guard, "transport-2");
        let store = guard.device_storage_mut();
        assert!(store.selections.is_empty());
        let outcome = store.commit_probe(
            &context.device_handle,
            sequence,
            1,
            1,
            1,
            DiscoveryStatus::Observed,
            "bbbbbbbbccccccccddddddddeeeeeeee".to_string(),
            Vec::new(),
            now,
        );
        assert_eq!(outcome, ProbeOutcome::Superseded);
        assert!(store.selections.is_empty());
        assert!(store.observations.is_empty());
    }

    #[test]
    fn runtime_authority_invalidation_discards_inflight_storage_probe() {
        let (handles, device) = handles_with_inventory("transport-1");
        let context = device_context(&device, 1);
        let now = Instant::now();
        let sequence = {
            let mut guard = handles.lock().expect("handles");
            let store = guard.device_storage_mut();
            store
                .notices
                .insert(DEVICE.to_string(), "old notice".to_string());
            match store.begin_probe(&context.device_handle, 1, 1, 1, now) {
                ProbeStart::Started(sequence) => sequence,
                ProbeStart::Busy => panic!("probe should start"),
            }
        };

        handles
            .lock()
            .expect("handles")
            .invalidate_runtime_authority_preserving_identities();

        let mut guard = handles.lock().expect("handles");
        let store = guard.device_storage_mut();
        assert!(!store.in_flight.contains_key(DEVICE));
        assert!(!store.notices.contains_key(DEVICE));
        assert_eq!(
            store.commit_probe(
                &context.device_handle,
                sequence,
                1,
                1,
                1,
                DiscoveryStatus::Observed,
                "bbbbbbbbccccccccddddddddeeeeeeee".to_string(),
                Vec::new(),
                now,
            ),
            ProbeOutcome::Superseded
        );
    }

    #[test]
    fn late_probe_after_runtime_invalidation_does_not_restore_a_notice() {
        let (handles, device) = handles_with_inventory("transport-1");
        let context = device_context(&device, 1);
        let now = Instant::now();
        let mut request = |_: &str, _: Value| {
            handles
                .lock()
                .expect("handles")
                .invalidate_runtime_authority_preserving_identities();
            Ok(internal_and_sd())
        };

        let committed = observe_device_volumes(&handles, &context, true, &mut request, now)
            .expect("a discarded late response is not a user-facing failure");

        assert!(!committed);
        let mut guard = handles.lock().expect("handles");
        assert!(!guard.device_storage_mut().notices.contains_key(DEVICE));
    }

    #[test]
    fn single_flight_probe_does_not_start_a_second_device_probe() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let now = Instant::now();
        {
            let mut guard = handles.lock().expect("handles");
            let store = guard.device_storage_mut();
            assert!(matches!(
                store.begin_probe(DEVICE, 1, 1, 1, now),
                ProbeStart::Started(_)
            ));
        }
        let mut request = ScriptedRequest::with(vec![Ok(internal_and_sd())]);
        assert!(
            !observe_device_volumes(
                &handles,
                &context,
                true,
                &mut |request_type: &str, payload: Value| request.call(request_type, payload),
                now
            )
            .expect("busy probe is not an error"),
            "a busy probe must be served from current state"
        );
        assert!(request.calls.is_empty());
        let projection =
            project_device_volumes(&handles, &context, "cached", now).expect("projection");
        assert_eq!(projection.discovery_status, "refreshing");
    }

    #[test]
    fn malformed_sidecar_payload_is_rejected_instead_of_projected() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let mut request = ScriptedRequest::with(vec![Ok(json!({
            "state": "observed",
            "observationToken": "/storage/8C4B-1A2F",
            "volumes": [],
        }))]);

        let error = observe_device_volumes(
            &handles,
            &context,
            true,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            Instant::now(),
        )
        .expect_err("malformed payload must fail closed");
        assert!(error.contains("sidecar_unavailable"), "error was {error}");
    }

    #[test]
    fn projection_contains_no_raw_device_identifiers() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let mut raw_observation = internal_and_sd();
        for volume in raw_observation["volumes"]
            .as_array_mut()
            .expect("raw volume array")
        {
            if volume["kind"] == "removableSd" {
                volume["setupLocationKind"] = json!("removableSd");
                volume["mountRoot"] = json!("/storage/8C4B-1A2F");
                volume["reasons"] = json!(["disk_removable_sd"]);
            }
        }
        let projection = observe(&handles, &context, raw_observation, Instant::now());
        let rendered = serde_json::to_string(&projection).expect("serialize projection");

        for forbidden in [
            "test-device-1",
            ADB,
            "/storage/",
            "8C4B",
            INTERNAL_IDENTITY,
            SD_IDENTITY,
            "identityToken",
            "mountState",
            "continuityToken",
            "dddddddddddddddddddddddddddddddd",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "projection {rendered} must not contain {forbidden}"
            );
        }
        assert!(rendered.contains("SD card"));
        assert!(rendered.contains("Internal storage"));
    }

    #[test]
    fn inventory_reconciliation_invalidates_storage_authority() {
        let (handles, device) = handles_with_inventory("transport-1");
        let context = device_context(&device, 1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        {
            let mut guard = handles.lock().expect("handles");
            update_inventory(&mut guard, "transport-2");
        }

        let error = project_device_volumes(&handles, &context, "cached", base)
            .expect_err("a continuity loss advances the session epoch");
        assert!(error.contains("stale_device_session"), "error was {error}");
        let mut guard = handles.lock().expect("handles");
        let store = guard.device_storage_mut();
        assert!(store.observations.is_empty());
        assert!(store.selections.is_empty());
    }

    #[test]
    fn vertical_card_change_requires_reselection_and_reconnect_clears_state() {
        let (handles, device) = handles_with_inventory("transport-1");
        let context = device_context(&device, 1);
        let base = Instant::now();

        let initial = observe(&handles, &context, internal_and_sd(), base);
        let original_sd = handle_for(&initial, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&original_sd), base)
            .expect("select");

        let replaced = observe(
            &handles,
            &context,
            observed_payload(&[
                (
                    INTERNAL_IDENTITY,
                    "internalShared",
                    "available",
                    "mountedRw",
                ),
                (
                    "55555555555555555555555555555555",
                    "removableSd",
                    "available",
                    "mountedRw",
                ),
            ]),
            base + Duration::from_secs(1),
        );
        assert!(replaced.selected_volume_handle.is_none());
        let new_sd = handle_for(&replaced, "removableSd");
        assert_ne!(new_sd, original_sd);
        let reselected = admit_device_volume_selection(&handles, &context, Some(&new_sd), base)
            .expect("reselect");
        assert_eq!(
            reselected.selected_volume_handle.as_deref(),
            Some(new_sd.as_str())
        );

        {
            let mut guard = handles.lock().expect("handles");
            assert!(guard.update_devices(&json!({ "devices": [] })).is_ok());
        }
        let error =
            project_device_volumes(&handles, &context, "cached", base + Duration::from_secs(1))
                .expect_err("a disconnected device has no projection");
        assert!(error.contains("unknown_device_handle"), "error was {error}");
        let mut guard = handles.lock().expect("handles");
        let store = guard.device_storage_mut();
        assert!(store.observations.is_empty());
        assert!(store.selections.is_empty());
        assert!(store.in_flight.is_empty());
    }

    /// Native application state with an installed fake Platform-Tools tree.
    ///
    /// Only the storage command seam reads these fields: the fake installation
    /// supplies a deterministic ADB path and revision, while the sidecar state
    /// supplies a stable runtime generation. No catalog or qualification data
    /// participates in storage discovery.
    fn test_state(root: &std::path::Path) -> AppState {
        AppState {
            sidecar: SidecarState::new(root.join("sidecar-cache")),
            catalog: Err("catalog is not used by storage discovery".to_string()),
            qualification_repository: QualificationRepositoryProvider::default(),
            qualification_transition_gate: QualificationTransitionGate::new(),
            adb: Mutex::new(AdbManager::with_test_installation(
                root.join("platform-tools"),
            )),
            platform_tools_selections: Mutex::new(PlatformToolsSelectionStore::default()),
            input_contracts: Mutex::new(InputContractSnapshot::default()),
            handles: Mutex::new(SessionHandles::default()),
            qualification_sessions: Mutex::new(QualificationSessionStore::default()),
            root_qualification: Mutex::new(RootQualificationStore::default()),
            executions: Mutex::new(ExecutionHandleStore::default()),
            saved_configurations: Mutex::new(SavedConfigurationStore::load(
                root.join("recent-configurations.json"),
            )),
            recovery: Mutex::new(RecoveryStore::load(
                root.join("recovery-draft.json"),
                root.join("session-active.marker"),
            )),
            support: Mutex::new(SupportStore::new(root.join("support-cache"))),
            updates: UpdateService::from_production_document()
                .expect("production update document should be valid in tests"),
            update_activity: ActivityGate::default(),
        }
    }

    fn retain_inventory_device(state: &AppState) -> String {
        let mut handles = state.handles.lock().expect("handles");
        update_inventory(&mut handles, "transport-1");
        handles
            .qualification_devices()
            .into_iter()
            .next()
            .expect("retained device")
            .handle
    }

    #[test]
    fn pre_probe_context_error_clears_an_orphaned_selection() {
        let temp = tempfile::tempdir().expect("test directory");
        let state = test_state(temp.path());
        let orphan = "orphan-device";
        {
            let mut guard = state.handles.lock().expect("handles");
            let store = guard.device_storage_mut();
            store.observations.insert(
                orphan.to_string(),
                VolumeObservation {
                    session_epoch: 1,
                    runtime_generation: 1,
                    platform_tools_revision: 1,
                    discovery_status: DiscoveryStatus::Observed,
                    observation_token: "aaaaaaaabbbbbbbbccccccccdddddddd".to_string(),
                    volumes: Vec::new(),
                    fetched_at: Instant::now(),
                },
            );
            store.selections.insert(
                orphan.to_string(),
                VolumeSelection {
                    session_epoch: 1,
                    volume_handle: "ffffffffffffffffffffffffffffffff".to_string(),
                    identity_token: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
                    continuity_token: "ffffffffffffffffffffffffffffffff".to_string(),
                },
            );
        }
        let mut request = ScriptedRequest::with(Vec::new());
        let error = volumes_at(
            &state,
            orphan,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            Instant::now(),
        )
        .expect_err("unknown device handle must fail");
        assert!(error.contains("unknown_device_handle"), "error was {error}");
        let mut guard = state.handles.lock().expect("handles");
        let store = guard.device_storage_mut();
        assert!(!store.observations.contains_key(orphan));
        assert!(!store.selections.contains_key(orphan));
        assert!(store
            .notices
            .get(orphan)
            .is_some_and(|notice| notice.contains("selection was cleared")));
    }

    #[test]
    fn older_selection_request_cannot_overwrite_a_newer_choice() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        let internal = handle_for(&projection, "internalShared");
        let older = begin_selection_request(&handles, &context.device_handle)
            .expect("older request sequence");
        let newer = begin_selection_request(&handles, &context.device_handle)
            .expect("newer request sequence");
        admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(&projection.observation_token),
            newer,
            "live",
            base,
        )
        .expect("newer selection commits");
        let late = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&internal),
            Some(&projection.observation_token),
            older,
            "live",
            base + Duration::from_secs(1),
        )
        .expect("superseded request returns current state");
        assert_eq!(late.selected_volume_handle.as_deref(), Some(sd.as_str()));
        let current =
            project_device_volumes(&handles, &context, "live", base + Duration::from_secs(1))
                .expect("projection");
        assert_eq!(current.selected_volume_handle.as_deref(), Some(sd.as_str()));
    }

    #[test]
    fn platform_tools_revision_change_during_probe_discards_late_commit() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        let revision = std::cell::Cell::new(context.platform_tools_revision);
        let mut reply = ScriptedRequest::with(vec![Ok(internal_and_sd())]);
        let mut request = |request_type: &str, payload: Value| {
            revision.set(context.platform_tools_revision + 1);
            reply.call(request_type, payload)
        };
        let mut provenance = || Ok((context.runtime_generation, revision.get()));
        let error = observe_device_volumes_with_provenance(
            &handles,
            &context,
            true,
            &mut request,
            &mut provenance,
            base + Duration::from_secs(1),
            &mut clock_at(base + Duration::from_secs(1)),
        )
        .expect_err("a changed Platform-Tools revision must be stale");
        assert!(error.contains("stale_context"), "error was {error}");

        let after =
            project_device_volumes(&handles, &context, "cached", base + Duration::from_secs(1))
                .expect("projection");
        assert!(after.selected_volume_handle.is_none());
        assert_eq!(after.discovery_status, "unknown");
    }

    #[test]
    fn runtime_generation_change_during_probe_discards_late_commit() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, internal_and_sd(), base);
        let sd = handle_for(&projection, "removableSd");
        admit_device_volume_selection(&handles, &context, Some(&sd), base).expect("select");

        let generation = std::cell::Cell::new(context.runtime_generation);
        let mut reply = ScriptedRequest::with(vec![Ok(internal_and_sd())]);
        let mut request = |request_type: &str, payload: Value| {
            generation.set(context.runtime_generation + 1);
            reply.call(request_type, payload)
        };
        let mut provenance = || Ok((generation.get(), context.platform_tools_revision));
        let error = observe_device_volumes_with_provenance(
            &handles,
            &context,
            true,
            &mut request,
            &mut provenance,
            base + Duration::from_secs(1),
            &mut clock_at(base + Duration::from_secs(1)),
        )
        .expect_err("a changed runtime generation must be stale");
        assert!(error.contains("stale_context"), "error was {error}");

        let after =
            project_device_volumes(&handles, &context, "cached", base + Duration::from_secs(1))
                .expect("projection");
        assert!(after.selected_volume_handle.is_none());
    }

    #[test]
    fn command_projection_labels_live_and_cached_evidence() {
        let temp = tempfile::tempdir().expect("test directory");
        let state = test_state(temp.path());
        let device = retain_inventory_device(&state);
        let mut request = ScriptedRequest::with(vec![Ok(internal_and_sd()), Ok(internal_and_sd())]);
        let now = Instant::now();

        let first = volumes_at(
            &state,
            &device,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            now,
        )
        .expect("first projection");
        assert_eq!(first.evidence, "live");
        assert_eq!(request.calls.len(), 1);
        assert_eq!(request.calls[0].0, "listDeviceVolumes");
        assert_eq!(request.calls[0].1["serial"], "T04-SERIAL");
        assert!(request.calls[0].1["adbPath"]
            .as_str()
            .expect("adb path")
            .ends_with("platform-tools/adb"));

        let cached = volumes_at(
            &state,
            &device,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            now + Duration::from_secs(1),
        )
        .expect("cached projection");
        assert_eq!(cached.evidence, "cached");
        assert_eq!(request.calls.len(), 1);

        let refreshed = volumes_at(
            &state,
            &device,
            true,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            now + Duration::from_secs(2),
        )
        .expect("refreshed projection");
        assert_eq!(refreshed.evidence, "live");
        assert_eq!(request.calls.len(), 2);
    }

    #[test]
    fn command_selection_reuses_the_trusted_observation() {
        let temp = tempfile::tempdir().expect("test directory");
        let state = test_state(temp.path());
        let device = retain_inventory_device(&state);
        let mut request = ScriptedRequest::with(vec![Ok(internal_and_sd())]);
        let now = Instant::now();

        let projection = volumes_at(
            &state,
            &device,
            false,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            now,
        )
        .expect("projection");
        let sd = handle_for(&projection, "removableSd");

        let selected = selection_at(
            &state,
            &device,
            Some(&sd),
            Some(&projection.observation_token),
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            now,
        )
        .expect("selection");
        assert_eq!(
            selected.selected_volume_handle.as_deref(),
            Some(sd.as_str())
        );
        assert_eq!(selected.evidence, "cached");
        assert_eq!(
            request.calls.len(),
            1,
            "an admitted selection reuses the trusted observation"
        );

        let unknown = selection_at(
            &state,
            &device,
            Some("ffffffffffffffffffffffffffffffff"),
            Some(&projection.observation_token),
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            now,
        )
        .expect_err("unknown handles are rejected");
        assert!(unknown.contains("volume_not_found"), "error was {unknown}");

        let cleared = selection_at(
            &state,
            &device,
            None,
            None,
            &mut |request_type: &str, payload: Value| request.call(request_type, payload),
            now,
        )
        .expect("clear selection");
        assert!(cleared.selected_volume_handle.is_none());
    }

    #[cfg(unix)]
    fn write_fake_adb(root: &std::path::Path, fs_uuid: &str, dump_state: &str, shell_state: &str) {
        use std::os::unix::fs::PermissionsExt;

        let tools = root.join("platform-tools");
        std::fs::create_dir_all(&tools).expect("platform-tools directory");
        let adb = tools.join("adb");
        std::fs::write(
            &adb,
            "#!/bin/sh\n\
             dir=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\n\
             if [ \"$1\" = \"devices\" ]; then\n\
               printf 'List of devices attached\\nT04-SERIAL\\tdevice product:emu model:T04_Device transport_id:7\\n'\n\
               exit 0\n\
             fi\n\
             if [ \"$1\" = \"-s\" ] && [ \"$2\" = \"T04-SERIAL\" ] && [ \"$3\" = \"shell\" ]; then\n\
               case \"$4\" in\n\
                 dumpsys) cat \"$dir/dump.txt\" ;;\n\
                 sm) cat \"$dir/list.txt\" ;;\n\
               esac\n\
               exit 0\n\
             fi\n\
             exit 1\n",
        )
        .expect("fake adb script");
        let mut permissions = std::fs::metadata(&adb)
            .expect("fake adb metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&adb, permissions).expect("fake adb permissions");

        let dump = "Disks:
 DiskInfo{disk:179,0}:
  flags=DEFAULT_PRIMARY size=32000000000 label=null
  sysPath=/sys/block/sda
 DiskInfo{disk:179,64}:
  flags=SD size=127000000000 label=SDCARD
  sysPath=/sys/block/mmcblk0

Volumes:
 VolumeInfo{emulated;0}:
  type=EMULATED diskId=disk:179,0 partGuid=null mountFlags=PRIMARY|VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED
  fsType=null fsUuid=null fsLabel=null
  path=/storage/emulated/0 internalPath=/data/media/0
 VolumeInfo{public:179,65}:
  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=__STATE__
  fsType=vfat fsUuid=__UUID__ fsLabel=SDCARD
  path=/storage/__UUID__ internalPath=/mnt/media_rw/__UUID__

Records:
 VolumeRecord:
  type=PUBLIC fsUuid=__UUID__ partGuid=null
  nickname=SDCARD userFlags=0
  createdMillis=2020-01-01T00:00:00.000Z lastSeenMillis=2020-01-01T00:00:00.000Z lastTrimMillis=2020-01-01T00:00:00.000Z lastBenchMillis=2020-01-01T00:00:00.000Z

Primary storage UUID: null

Internal storage (/storage/emulated) total size: 32000000000 (30517 MiB)
"
    .replace("__UUID__", fs_uuid)
    .replace("__STATE__", dump_state);
        std::fs::write(tools.join("dump.txt"), dump).expect("mount report");
        let list = format!(
            "emulated;0 mounted null
public:179,65 {shell_state} {fs_uuid}
"
        );
        std::fs::write(tools.join("list.txt"), list).expect("shell volume report");
    }

    #[cfg(unix)]
    fn sidecar_request(
        state: &AppState,
        request_type: &str,
        payload: Value,
    ) -> Result<Value, String> {
        state.sidecar.request(request_type, payload)
    }

    #[cfg(unix)]
    #[test]
    fn fake_adb_vertical_selection_invalidates_on_change_unmount_and_reconnect() {
        let temp = tempfile::tempdir().expect("test directory");
        write_fake_adb(temp.path(), "8C4B-1A2F", "MOUNTED", "mounted");
        let mut state = test_state(temp.path());
        state.sidecar = SidecarState::with_test_backend(
            temp.path().join("sidecar-cache"),
            emuchef_rust_backend::execution_session::SidecarRuntimeConfig {
                runtime_root: temp.path().join("runtime"),
                cache_root: temp.path().join("cache"),
                adb_path: temp
                    .path()
                    .join("platform-tools")
                    .join("adb")
                    .to_string_lossy()
                    .into_owned(),
            },
        );
        let device = retain_inventory_device(&state);
        let now = Instant::now();

        let projection = volumes_at(
            &state,
            &device,
            false,
            &mut |request_type: &str, payload: Value| {
                sidecar_request(&state, request_type, payload)
            },
            now,
        )
        .expect("fake-adb projection");
        assert_eq!(projection.evidence, "live");
        assert_eq!(projection.discovery_status, "observed");
        let original_sd = handle_for(&projection, "removableSd");

        let selected = selection_at(
            &state,
            &device,
            Some(&original_sd),
            Some(&projection.observation_token),
            &mut |request_type: &str, payload: Value| {
                sidecar_request(&state, request_type, payload)
            },
            now,
        )
        .expect("selection through the real sidecar parser");
        assert_eq!(
            selected.selected_volume_handle.as_deref(),
            Some(original_sd.as_str())
        );

        write_fake_adb(temp.path(), "5E6F-7A8B", "MOUNTED", "mounted");
        let changed = volumes_at(
            &state,
            &device,
            true,
            &mut |request_type: &str, payload: Value| {
                sidecar_request(&state, request_type, payload)
            },
            now + Duration::from_secs(11),
        )
        .expect("post-change projection");
        assert!(changed.selected_volume_handle.is_none());
        assert_eq!(
            changed.selection_notice.as_deref(),
            Some("The selected storage is no longer present. Choose a storage option again.")
        );
        let replacement_sd = handle_for(&changed, "removableSd");
        assert_ne!(replacement_sd, original_sd);
        selection_at(
            &state,
            &device,
            Some(&replacement_sd),
            Some(&changed.observation_token),
            &mut |request_type: &str, payload: Value| {
                sidecar_request(&state, request_type, payload)
            },
            now + Duration::from_secs(11),
        )
        .expect("reselect the replacement card");

        write_fake_adb(temp.path(), "5E6F-7A8B", "UNMOUNTED", "unmounted");
        let unmounted = volumes_at(
            &state,
            &device,
            true,
            &mut |request_type: &str, payload: Value| {
                sidecar_request(&state, request_type, payload)
            },
            now + Duration::from_secs(12),
        )
        .expect("post-eject projection");
        assert!(unmounted.selected_volume_handle.is_none());
        assert_eq!(
            unmounted.selection_notice.as_deref(),
            Some("The selected storage is no longer available. Choose a storage option again.")
        );
        assert!(unmounted
            .volumes
            .iter()
            .filter(|volume| volume.kind == "removableSd")
            .all(|volume| !volume.selectable));

        {
            let mut guard = state.handles.lock().expect("handles");
            guard
                .update_devices(&json!({ "devices": [] }))
                .expect("disconnect inventory");
        }
        let disconnected = volumes_at(
            &state,
            &device,
            false,
            &mut |request_type: &str, payload: Value| {
                sidecar_request(&state, request_type, payload)
            },
            now + Duration::from_secs(12),
        )
        .expect_err("a disconnected device has no storage projection");
        assert!(disconnected.contains("unknown_device_handle"));

        {
            let mut guard = state.handles.lock().expect("handles");
            update_inventory(&mut guard, "transport-2");
            let store = guard.device_storage_mut();
            assert!(store.observations.is_empty());
            assert!(store.selections.is_empty());
        }
        write_fake_adb(temp.path(), "5E6F-7A8B", "MOUNTED", "mounted");
        let reconnected = volumes_at(
            &state,
            &device,
            false,
            &mut |request_type: &str, payload: Value| {
                sidecar_request(&state, request_type, payload)
            },
            now + Duration::from_secs(13),
        )
        .expect("reconnected projection");
        assert!(reconnected.selected_volume_handle.is_none());
        assert_eq!(reconnected.discovery_status, "observed");
    }

    const OBSERVATION_ONE: &str = "11111111111111111111111111111111";
    const OBSERVATION_TWO: &str = "22222222222222222222222222222222";
    const CONTINUITY_ONE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CONTINUITY_TWO: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn stale_observation_token_cannot_rebind_an_older_choice_to_a_new_card() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let first = observe(
            &handles,
            &context,
            sd_payload(CONTINUITY_ONE, OBSERVATION_ONE),
            base,
        );
        let sd = handle_for(&first, "removableSd");
        let second = observe(
            &handles,
            &context,
            sd_payload(CONTINUITY_TWO, OBSERVATION_TWO),
            base + Duration::from_secs(1),
        );
        assert_eq!(sd, handle_for(&second, "removableSd"));
        assert_ne!(second.observation_token, OBSERVATION_TWO);

        let sequence = begin_selection_request(&handles, &context.device_handle)
            .expect("stale selection sequence");
        let error = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(&first.observation_token),
            sequence,
            "cached",
            base + Duration::from_secs(1),
        )
        .expect_err("the displayed observation token is no longer current");
        assert!(error.contains("stale_observation"), "error was {error}");

        let after =
            project_device_volumes(&handles, &context, "cached", base + Duration::from_secs(1))
                .expect("projection");
        assert!(after.selected_volume_handle.is_none());

        let sequence = begin_selection_request(&handles, &context.device_handle)
            .expect("current selection sequence");
        let selected = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(&second.observation_token),
            sequence,
            "cached",
            base + Duration::from_secs(1),
        )
        .expect("the current observation token admits the selection");
        assert_eq!(
            selected.selected_volume_handle.as_deref(),
            Some(sd.as_str())
        );
    }

    #[test]
    fn a_superseded_stale_token_cannot_disturb_a_newer_selection() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(
            &handles,
            &context,
            sd_payload(CONTINUITY_TWO, OBSERVATION_TWO),
            base,
        );
        let sd = handle_for(&projection, "removableSd");
        let older = begin_selection_request(&handles, &context.device_handle)
            .expect("older selection sequence");
        let newer = begin_selection_request(&handles, &context.device_handle)
            .expect("newer selection sequence");
        let selected = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(&projection.observation_token),
            newer,
            "cached",
            base,
        )
        .expect("newer selection commits");
        assert_eq!(
            selected.selected_volume_handle.as_deref(),
            Some(sd.as_str())
        );

        let late = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(OBSERVATION_ONE),
            older,
            "cached",
            base + Duration::from_secs(1),
        )
        .expect("superseded request returns the current projection");
        assert_eq!(late.selected_volume_handle.as_deref(), Some(sd.as_str()));
    }

    #[test]
    fn identical_continuity_evidence_refresh_mints_a_fresh_frontend_token() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let first = observe(
            &handles,
            &context,
            sd_payload(CONTINUITY_ONE, OBSERVATION_ONE),
            base,
        );
        let second = observe(
            &handles,
            &context,
            sd_payload(CONTINUITY_ONE, OBSERVATION_ONE),
            base + Duration::from_secs(1),
        );
        // A refreshed observation gets a new request token even when device
        // evidence is unchanged. The stable volume handle still identifies the
        // same logical choice within this session.
        assert_ne!(first.observation_token, second.observation_token);
        let sd = handle_for(&first, "removableSd");
        let sequence =
            begin_selection_request(&handles, &context.device_handle).expect("selection sequence");
        let stale = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(&first.observation_token),
            sequence,
            "cached",
            base + Duration::from_secs(1),
        )
        .expect_err("a refreshed observation invalidates the displayed token");
        assert!(stale.contains("stale_observation"), "error was {stale}");

        let sequence =
            begin_selection_request(&handles, &context.device_handle).expect("selection sequence");
        let selected = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(&second.observation_token),
            sequence,
            "cached",
            base + Duration::from_secs(1),
        )
        .expect("the current freshness token admits the stable volume choice");
        assert_eq!(
            selected.selected_volume_handle.as_deref(),
            Some(sd.as_str())
        );
    }

    #[test]
    fn slow_probe_commits_at_completion_and_remains_available() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let mut reply =
            ScriptedRequest::with(vec![Ok(sd_payload(CONTINUITY_ONE, OBSERVATION_ONE))]);
        let mut request = |request_type: &str, payload: Value| reply.call(request_type, payload);
        let mut provenance = || Ok((context.runtime_generation, context.platform_tools_revision));
        let mut completion_times = vec![base + Duration::from_secs(11)].into_iter();
        let mut clock = || completion_times.next().expect("completion clock");

        let fresh = observe_device_volumes_with_provenance(
            &handles,
            &context,
            true,
            &mut request,
            &mut provenance,
            base,
            &mut clock,
        )
        .expect("a slow successful probe commits");
        assert!(fresh);

        let immediate =
            project_device_volumes(&handles, &context, "live", base + Duration::from_secs(11))
                .expect("immediate projection");
        assert_eq!(immediate.discovery_status, "observed");
        assert_eq!(
            immediate
                .volumes
                .iter()
                .find(|volume| volume.kind == "removableSd")
                .expect("removable SD")
                .status,
            "available"
        );

        let sd = handle_for(&immediate, "removableSd");
        let sequence =
            begin_selection_request(&handles, &context.device_handle).expect("selection sequence");
        admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&sd),
            Some(&immediate.observation_token),
            sequence,
            "live",
            base + Duration::from_secs(11),
        )
        .expect("selection admitted at probe completion");

        let mut cached_reply = ScriptedRequest::with(vec![]);
        let mut cached_request =
            |request_type: &str, payload: Value| cached_reply.call(request_type, payload);
        let mut cached_completion = vec![base + Duration::from_secs(15)].into_iter();
        let mut cached_clock = || cached_completion.next().expect("cache clock");
        let fetched_again = observe_device_volumes_with_provenance(
            &handles,
            &context,
            false,
            &mut cached_request,
            &mut provenance,
            base + Duration::from_secs(15),
            &mut cached_clock,
        )
        .expect("subsequent cache lookup");
        assert!(!fetched_again);
        assert!(cached_reply.calls.is_empty());

        let expired = admit_device_volume_selection(
            &handles,
            &context,
            Some(&sd),
            base + Duration::from_secs(22),
        )
        .expect_err("the observation expires ten seconds after completion");
        assert!(
            expired.contains("stale_device_session"),
            "error was {expired}"
        );
    }

    #[test]
    fn duplicate_non_selectable_volumes_get_distinct_presentation_keys() {
        let handles = handles_at_epoch(1);
        let context = context(1);
        let base = Instant::now();
        let projection = observe(&handles, &context, duplicate_sd_payload(), base);
        let duplicates = projection
            .volumes
            .iter()
            .filter(|volume| volume.kind == "removableSd")
            .collect::<Vec<_>>();
        assert_eq!(duplicates.len(), 2);
        assert_eq!(duplicates[0].volume_handle, duplicates[1].volume_handle);
        assert_ne!(
            duplicates[0].presentation_key,
            duplicates[1].presentation_key
        );
        assert!(duplicates.iter().all(|volume| !volume.selectable));

        let sequence =
            begin_selection_request(&handles, &context.device_handle).expect("selection sequence");
        let error = admit_device_volume_selection_at_sequence(
            &handles,
            &context,
            Some(&duplicates[0].volume_handle),
            Some(&projection.observation_token),
            sequence,
            "cached",
            base,
        )
        .expect_err("an ambiguous duplicate cannot be selected");
        assert!(error.contains("volume_not_selectable"), "error was {error}");
    }

    #[test]
    fn authority_loss_clears_transient_storage_state() {
        let mut state = DeviceStorageState::default();
        state.observations.insert(
            "device".to_string(),
            VolumeObservation {
                session_epoch: 1,
                runtime_generation: 1,
                platform_tools_revision: 1,
                discovery_status: DiscoveryStatus::Observed,
                observation_token: "observation".to_string(),
                volumes: Vec::new(),
                fetched_at: Instant::now(),
            },
        );
        state.selections.insert(
            "device".to_string(),
            VolumeSelection {
                session_epoch: 1,
                volume_handle: "volume".to_string(),
                identity_token: "identity".to_string(),
                continuity_token: "continuity".to_string(),
            },
        );
        state.in_flight.insert("device".to_string(), 1);
        state
            .notices
            .insert("device".to_string(), "notice".to_string());
        state.selection_requests.insert("device".to_string(), 1);
        state.sequences.insert("device".to_string(), 7);

        state.clear_all_after_authority_loss();

        assert!(state.observations.is_empty());
        assert!(state.selections.is_empty());
        assert!(state.in_flight.is_empty());
        assert!(state.notices.is_empty());
        assert_eq!(state.selection_requests.get("device"), Some(&1));
        assert_eq!(state.sequences.get("device"), Some(&7));
    }

    #[test]
    fn authority_loss_retires_volume_handles_for_stale_session_rejection() {
        for invalidate_runtime in [true, false] {
            let base = Instant::now();
            let (handles, device) = handles_with_inventory("transport-1");
            let context = device_context(&device, 1);
            let projection = observe(&handles, &context, internal_and_sd(), base);
            let sd = handle_for(&projection, "removableSd");

            {
                let mut guard = handles.lock().expect("handles");
                if invalidate_runtime {
                    guard.invalidate_runtime_authority_preserving_identities();
                } else {
                    guard.invalidate_identity_authority(&device);
                }
                update_inventory(&mut guard, "transport-1");
            }

            let renewed_context = {
                let guard = handles.lock().expect("handles");
                let epoch = guard
                    .device(&device)
                    .expect("redetected device")
                    .session_epoch;
                device_context(&device, epoch)
            };
            observe(&handles, &renewed_context, internal_and_sd(), base);

            let error = admit_device_volume_selection(&handles, &renewed_context, Some(&sd), base)
                .expect_err("a retired handle must be rejected");
            assert!(
                error.contains("stale_device_session"),
                "runtime_invalidation={invalidate_runtime}, error was {error}"
            );
        }
    }

    #[test]
    fn mismatched_observation_retirement_reports_stale_session() {
        let base = Instant::now();
        let (handles, device) = handles_with_inventory("transport-1");
        let first_context = device_context(&device, 1);
        let first = observe(&handles, &first_context, internal_and_sd(), base);
        let old_sd = handle_for(&first, "removableSd");

        {
            let mut guard = handles.lock().expect("handles");
            assert!(matches!(
                guard
                    .device_storage_mut()
                    .begin_probe(&device, 2, 1, 1, base),
                ProbeStart::Started(_)
            ));
        }

        {
            let mut guard = handles.lock().expect("handles");
            guard.invalidate_identity_authority(&device);
            update_inventory(&mut guard, "transport-1");
        }

        let renewed_context = {
            let guard = handles.lock().expect("handles");
            let epoch = guard
                .device(&device)
                .expect("redetected device")
                .session_epoch;
            device_context(&device, epoch)
        };
        let renewed = observe(&handles, &renewed_context, internal_and_sd(), base);
        let sequence = begin_selection_request(&handles, &device).expect("selection sequence");

        let error = admit_device_volume_selection_at_sequence(
            &handles,
            &renewed_context,
            Some(&old_sd),
            Some(&renewed.observation_token),
            sequence,
            "cached",
            base,
        )
        .expect_err("a retired handle must be rejected");
        assert!(error.contains("stale_device_session"), "error was {error}");
    }

    #[test]
    fn per_device_authority_loss_keeps_unrelated_storage_state() {
        let mut handles = SessionHandles::default();
        handles.retain_available_device_for_test("device-a", 1);
        handles.retain_available_device_for_test("device-b", 1);
        let now = Instant::now();
        {
            let store = handles.device_storage_mut();
            for device in ["device-a", "device-b"] {
                store.observations.insert(
                    device.to_string(),
                    VolumeObservation {
                        session_epoch: 1,
                        runtime_generation: 1,
                        platform_tools_revision: 1,
                        discovery_status: DiscoveryStatus::Observed,
                        observation_token: format!("{device}-observation"),
                        volumes: Vec::new(),
                        fetched_at: now,
                    },
                );
                store.selections.insert(
                    device.to_string(),
                    VolumeSelection {
                        session_epoch: 1,
                        volume_handle: format!("{device}-volume"),
                        identity_token: "identity".to_string(),
                        continuity_token: "continuity".to_string(),
                    },
                );
                store.in_flight.insert(device.to_string(), 1);
                store
                    .notices
                    .insert(device.to_string(), "notice".to_string());
                store.selection_requests.insert(device.to_string(), 1);
                store.sequences.insert(device.to_string(), 1);
            }
        }

        handles.invalidate_identity_authority("device-a");

        let store = handles.device_storage_mut();
        assert!(!store.observations.contains_key("device-a"));
        assert!(!store.selections.contains_key("device-a"));
        assert!(!store.in_flight.contains_key("device-a"));
        assert!(!store.notices.contains_key("device-a"));
        assert_eq!(store.selection_requests.get("device-a"), Some(&1));
        assert_eq!(store.sequences.get("device-a"), Some(&1));
        assert_eq!(
            store
                .observations
                .get("device-b")
                .map(|observation| observation.observation_token.as_str()),
            Some("device-b-observation")
        );
        assert!(store.selections.contains_key("device-b"));
        assert!(store.in_flight.contains_key("device-b"));
        assert!(store.notices.contains_key("device-b"));
        assert!(store.selection_requests.contains_key("device-b"));
        assert!(store.sequences.contains_key("device-b"));
    }
    #[test]
    fn setup_locations_keep_independent_volumes_and_invalidate_only_changed_continuity() {
        let now = Instant::now();
        let internal = ObservedVolume {
            identity_token: INTERNAL_IDENTITY.to_string(),
            continuity_token: "dddddddddddddddddddddddddddddddd".to_string(),
            kind: ObservedVolumeKind::InternalShared,
            status: ObservedVolumeStatus::Available,
            mount_state: ObservedMountState::MountedRw,
            setup_location_kind: Some(ObservedVolumeKind::InternalShared),
            mount_root: Some("/storage/emulated/0".to_string()),
        };
        let sd = ObservedVolume {
            identity_token: SD_IDENTITY.to_string(),
            continuity_token: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
            kind: ObservedVolumeKind::RemovableSd,
            status: ObservedVolumeStatus::Available,
            mount_state: ObservedMountState::MountedRw,
            setup_location_kind: Some(ObservedVolumeKind::RemovableSd),
            mount_root: Some("/storage/1234-ABCD".to_string()),
        };
        let observation = |token: &str, volumes: Vec<ObservedVolume>| VolumeObservation {
            session_epoch: 1,
            runtime_generation: 2,
            platform_tools_revision: 3,
            discovery_status: DiscoveryStatus::Observed,
            observation_token: token.to_string(),
            volumes,
            fetched_at: now,
        };
        let mut store = DeviceStorageState::default();
        store.observations.insert(
            DEVICE.to_string(),
            observation(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                vec![internal.clone(), sd.clone()],
            ),
        );
        let internal_handle = volume_handle_for(DEVICE, 1, &internal);
        let sd_handle = volume_handle_for(DEVICE, 1, &sd);
        let internal_id = store
            .save_setup_location(
                DEVICE,
                "plan-a",
                1,
                2,
                3,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &internal_handle,
                None,
                "Internal ROMs",
                "Games/ROMs",
                now,
            )
            .unwrap()[0]
            .id
            .clone();
        let sd_id = store
            .save_setup_location(
                DEVICE,
                "plan-a",
                1,
                2,
                3,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &sd_handle,
                None,
                "SD BIOS",
                "EmuChef/BIOS",
                now,
            )
            .unwrap()
            .into_iter()
            .find(|location| location.name == "SD BIOS")
            .unwrap()
            .id;
        let other_plan_id = store
            .save_setup_location(
                DEVICE,
                "plan-b",
                1,
                2,
                3,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &internal_handle,
                None,
                "Other ROMs",
                "Other/Games",
                now,
            )
            .unwrap()[0]
            .id
            .clone();

        let bindings = store
            .setup_location_bindings(
                DEVICE,
                "plan-a",
                1,
                2,
                3,
                &[internal_id.clone(), sd_id.clone()],
                now,
            )
            .unwrap();
        assert_eq!(bindings.len(), 2);
        assert_ne!(bindings[0].mount_root, bindings[1].mount_root);
        let public = store.project_setup_locations(DEVICE, "plan-a", 1, 2, 3, now);
        let serialized = serde_json::to_string(&public).unwrap();
        assert!(!serialized.contains("/storage/"));
        assert!(!serialized.contains(INTERNAL_IDENTITY));
        assert!(!serialized.contains(SD_IDENTITY));

        let refreshed = observation(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            vec![internal.clone(), sd.clone()],
        );
        store.observations.insert(DEVICE.to_string(), refreshed);
        store.reconcile_setup_locations(DEVICE);
        assert!(store
            .project_setup_locations(DEVICE, "plan-a", 1, 2, 3, now)
            .iter()
            .all(|location| location.ready));

        let replaced_sd = ObservedVolume {
            identity_token: "ffffffffffffffffffffffffffffffff".to_string(),
            continuity_token: "99999999999999999999999999999999".to_string(),
            ..sd
        };
        store.observations.insert(
            DEVICE.to_string(),
            observation(
                "cccccccccccccccccccccccccccccccc",
                vec![internal, replaced_sd],
            ),
        );
        store.reconcile_setup_locations(DEVICE);
        let plan_a = store.project_setup_locations(DEVICE, "plan-a", 1, 2, 3, now);
        assert!(
            plan_a
                .iter()
                .find(|location| location.id == internal_id)
                .unwrap()
                .ready
        );
        assert!(
            !plan_a
                .iter()
                .find(|location| location.id == sd_id)
                .unwrap()
                .ready
        );
        assert!(store
            .setup_location_bindings(DEVICE, "plan-b", 1, 2, 3, &[other_plan_id], now)
            .is_ok());
    }
}
