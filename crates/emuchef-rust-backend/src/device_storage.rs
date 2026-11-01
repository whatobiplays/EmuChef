//! Passive Android storage volume discovery.
//!
//! This module answers one question for an already-selected device: which
//! storage destinations does the device currently report, and which of those
//! destinations may be offered to the user? It performs read-only observation
//! only. It never mounts, unmounts, formats, remounts, or otherwise changes
//! device storage state.
//!
//! Three bounded ADB commands make up one observation:
//!
//! 1. `adb devices -l` confirms that exactly one connected device matches the
//!    requested serial before any device-specific command runs.
//! 2. `adb -s <serial> shell dumpsys mount` reports the storage manager's
//!    current `VolumeInfo` and `DiskInfo` records together with the configured
//!    primary-storage identifier.
//! 3. `adb -s <serial> shell sm list-volumes all` reports the storage manager's
//!    current volume list, which corroborates the state reported by
//!    `dumpsys mount`.
//!
//! Both device reports are untrusted text. Parsing is deterministic and
//! defensive: reports larger than a fixed bound fail closed, unknown or
//! conflicting evidence is projected as unknown, and no raw report text,
//! filesystem UUID, mount path, or disk identifier is placed in an error
//! message or a projected label.
//!
//! Every observation carries a deterministic observation token, and every
//! volume carries a deterministic identity token. Identity is derived from the
//! volume kind, its filesystem UUID when one exists, and its transport class.
//! It deliberately excludes the mount path so that a device which relocates a
//! volume does not appear to the user as a different volume, while a replaced
//! card with a different filesystem UUID does appear as a different volume.
//! Filesystem UUID plus transport class is correlation evidence, not proof of a
//! physical card, so callers treat it as evidence that can lapse rather than as
//! a long-lived hardware identity.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::device_probe::{CommandRunner, ProcessCommandRunner};
use crate::end_user_runtime::{classify_inventory_target, InventoryTarget};
use crate::errors::ApiError;
use crate::owned_process::ProcessOperation;

/// Largest accepted device report. Real `dumpsys mount` output is a few
/// kilobytes; the bound exists so malformed or hostile output cannot consume
/// unbounded memory or parsing time.
const MAX_REPORT_BYTES: usize = 512 * 1024;

/// Largest accepted line count. Guards against newline-flooded reports.
const MAX_REPORT_LINES: usize = 20_000;

/// Domain separator for the 128-bit volume identity digest.
const IDENTITY_DOMAIN: &str = "emuchef.device.volume.identity.v1";

/// Domain separator for the 128-bit observation digest.
const OBSERVATION_DOMAIN: &str = "emuchef.device.volume.observation.v1";

/// Domain separator for the sidecar-only volume continuity digest.
const CONTINUITY_DOMAIN: &str = "emuchef.device.volume.continuity.v1";

/// Storage classes EmuChef can distinguish from passive storage-manager
/// reports. Only `InternalShared` and `RemovableSd` are candidate
/// destinations; the remaining classes exist so unsupported storage is shown
/// truthfully instead of being mistaken for portable storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VolumeKind {
    InternalShared,
    RemovableSd,
    AdoptedPrivate,
    Usb,
    Unknown,
}

impl VolumeKind {
    fn transport_class(self) -> &'static str {
        match self {
            Self::InternalShared => "internal",
            Self::RemovableSd => "sd",
            Self::AdoptedPrivate => "adopted",
            Self::Usb => "usb",
            Self::Unknown => "unknown",
        }
    }

    fn token_name(self) -> &'static str {
        match self {
            Self::InternalShared => "internalShared",
            Self::RemovableSd => "removableSd",
            Self::AdoptedPrivate => "adoptedPrivate",
            Self::Usb => "usb",
            Self::Unknown => "unknown",
        }
    }

    fn sort_rank(self) -> u8 {
        match self {
            Self::InternalShared => 0,
            Self::RemovableSd => 1,
            Self::AdoptedPrivate => 2,
            Self::Usb => 3,
            Self::Unknown => 4,
        }
    }
}

/// Availability of one storage destination.
///
/// `Available` means the trusted observation proves the volume is currently
/// mounted and corroborated. It does not assert writability, application-side
/// access, or execution eligibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VolumeStatus {
    Available,
    Unmounted,
    Ambiguous,
    Unsupported,
    Unknown,
}

impl VolumeStatus {
    fn token_name(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unmounted => "unmounted",
            Self::Ambiguous => "ambiguous",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

/// Mount state reported by the storage manager. `MountedRw` describes the
/// reported mount, not a guaranteed write capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VolumeMountState {
    MountedRw,
    MountedRo,
    Unmounted,
    Unknown,
}

impl VolumeMountState {
    fn token_name(self) -> &'static str {
        match self {
            Self::MountedRw => "mountedRw",
            Self::MountedRo => "mountedRo",
            Self::Unmounted => "unmounted",
            Self::Unknown => "unknown",
        }
    }

    fn is_mounted(self) -> bool {
        matches!(self, Self::MountedRw | Self::MountedRo)
    }
}

/// One classified storage destination. Fields are sidecar-internal: the Tauri
/// boundary replaces identity tokens with opaque handles and never forwards the
/// raw reason keys verbatim.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceVolume {
    pub(crate) identity_token: String,
    pub(crate) continuity_token: String,
    pub(crate) kind: VolumeKind,
    pub(crate) status: VolumeStatus,
    pub(crate) mount_state: VolumeMountState,
    pub(crate) reasons: Vec<String>,
}

/// Failure of one bounded device command or of the report it produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeviceStorageFailure {
    /// The bounded runner could not start the command.
    Unavailable,
    /// The command ran but reported a non-zero status.
    CommandFailed,
    /// The report exceeded the accepted size or ended mid-record.
    Malformed,
}

/// Probe one selected device's current storage volumes.
///
/// The response shape is
/// `{ state, observationToken, volumes }`, where `state` is `observed` when
/// both device reports were read, or a fixed inventory outcome such as
/// `no_device` when the selected device is no longer the single available
/// device. Only `observed` responses carry volume entries.
pub(crate) fn list_device_volumes(adb_path: &str, serial: &str) -> Result<Value, ApiError> {
    list_device_volumes_with_runner(adb_path, serial, &ProcessCommandRunner)
}

/// Probe one selected device using an injectable command runner.
///
/// The inventory check runs first so a stale or ambiguous selection can never
/// produce device-specific output. Both device reports run only after the
/// inventory proves exactly one available device with the requested serial.
pub(crate) fn list_device_volumes_with_runner(
    adb_path: &str,
    requested_serial: &str,
    runner: &impl CommandRunner,
) -> Result<Value, ApiError> {
    let target = classify_inventory_target(
        &crate::end_user_runtime::list_adb_devices_with_runner(adb_path, runner)?,
        requested_serial,
    )?;
    let serial = match target {
        InventoryTarget::NoDevice => return Ok(inventory_state_value("no_device")),
        InventoryTarget::MultipleDevices => {
            return Ok(inventory_state_value("multiple_devices"));
        }
        InventoryTarget::Unavailable { state, .. } => return Ok(inventory_state_value(state)),
        InventoryTarget::Available { serial } => serial,
    };

    let mount = run_report(
        runner,
        &serial_shell_command(adb_path, &serial, &["dumpsys", "mount"]),
    )?;
    let list_volumes = run_report(
        runner,
        &serial_shell_command(adb_path, &serial, &["sm", "list-volumes", "all"]),
    )?;

    let mount = parse_mount_dump(&mount)?;
    let shell_volumes = parse_shell_volumes(&list_volumes)?;
    let volumes = classify_volumes(&mount, &shell_volumes);
    let observation_token = observation_token(&volumes);
    Ok(json!({
        "state": "observed",
        "observationToken": observation_token,
        "volumes": volumes,
    }))
}

fn inventory_state_value(state: &str) -> Value {
    json!({
        "state": state,
        "observationToken": digest128(OBSERVATION_DOMAIN, &[state]),
        "volumes": [],
    })
}

fn serial_shell_command(adb_path: &str, serial: &str, command: &[&str]) -> Vec<String> {
    let mut argv = vec![
        adb_path.to_string(),
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
    ];
    argv.extend(command.iter().map(|part| (*part).to_string()));
    argv
}

fn run_report(runner: &impl CommandRunner, argv: &[String]) -> Result<String, ApiError> {
    let output = runner
        .run_for(argv, ProcessOperation::Probe)
        .map_err(|_| storage_failure(DeviceStorageFailure::Unavailable))?;
    if output.status_code != Some(0) {
        return Err(storage_failure(DeviceStorageFailure::CommandFailed));
    }
    Ok(output.stdout)
}

fn storage_failure(failure: DeviceStorageFailure) -> ApiError {
    match failure {
        DeviceStorageFailure::Malformed => ApiError::command_failed(
            "Device storage volume details could not be read.",
            json!({ "reason": "device_storage_malformed" }),
        ),
        DeviceStorageFailure::Unavailable | DeviceStorageFailure::CommandFailed => {
            ApiError::command_failed(
                "Device storage volume details could not be read.",
                json!({ "reason": "device_storage_probe_failed" }),
            )
        }
    }
}

/// One `VolumeInfo` record from `dumpsys mount`.
#[derive(Clone, Debug, Eq, PartialEq)]
struct MountVolume {
    id: String,
    volume_type: MountVolumeType,
    disk_id: Option<String>,
    state: String,
    fs_uuid: Option<String>,
    path: Option<String>,
    internal_path: Option<String>,
    mount_flags: Option<String>,
    mount_user_id: Option<i64>,
    visible: Option<bool>,
    contradictory: bool,
}

/// Agreement between the two sources of an emulated volume's Android user.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EmulatedUserEvidence {
    /// At least one source names the user, and all present sources agree.
    Associated(i64),
    /// The volume ID and `mountUserId` name different users.
    Conflict,
    /// Neither source names a user.
    Unknown,
}

impl MountVolume {
    fn new(id: String) -> Self {
        Self {
            id,
            volume_type: MountVolumeType::Unknown,
            disk_id: None,
            state: String::new(),
            fs_uuid: None,
            path: None,
            internal_path: None,
            mount_flags: None,
            mount_user_id: None,
            visible: None,
            contradictory: false,
        }
    }

    fn mount_state(&self) -> VolumeMountState {
        mount_state_from_text(&self.state)
    }

    /// User association for emulated volumes. Android encodes the associated
    /// user in the volume id and repeats it in `mountUserId`. The association is
    /// trusted only when both sources agree; a disagreement is reported rather
    /// than choosing one field.
    fn emulated_user_evidence(&self) -> EmulatedUserEvidence {
        match (self.emulated_id_user(), self.mount_user_id) {
            (Some(id_user), Some(mount_user_id)) if id_user != mount_user_id => {
                EmulatedUserEvidence::Conflict
            }
            (Some(user), _) | (None, Some(user)) => EmulatedUserEvidence::Associated(user),
            (None, None) => EmulatedUserEvidence::Unknown,
        }
    }

    /// Android prints built-in emulated volumes as `emulated;<user>` and
    /// stacked adopted volumes as `emulated:<major>,<minor>;<user>`.
    fn emulated_id_user(&self) -> Option<i64> {
        if let Some(value) = self.id.strip_prefix("emulated;") {
            return value.trim().parse::<i64>().ok();
        }
        self.id
            .strip_prefix("emulated:")
            .and_then(|rest| rest.rsplit_once(';'))
            .and_then(|(_, user)| user.trim().parse::<i64>().ok())
    }

    fn normalized_fs_uuid(&self) -> Option<String> {
        self.fs_uuid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("null"))
            .map(|value| value.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MountVolumeType {
    Private,
    Emulated,
    Public,
    Unknown,
}

/// One `DiskInfo` record from `dumpsys mount`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MountDisk {
    disk_id: Option<String>,
    flags: DiskFlags,
    flags_observed: bool,
    contradictory: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DiskFlags {
    sd: bool,
    usb: bool,
    adoptable: bool,
    default_primary: bool,
}

/// Parsed `dumpsys mount` evidence.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MountDump {
    primary_storage_uuid: Option<String>,
    primary_storage_conflict: bool,
    volumes: Vec<MountVolume>,
    disks: Vec<MountDisk>,
}

fn parse_mount_dump(stdout: &str) -> Result<MountDump, ApiError> {
    let lines = bounded_lines(stdout)?;
    let mut dump = MountDump::default();
    let mut section = DumpSection::Other;
    let mut current_volume: Option<usize> = None;
    let mut current_disk: Option<usize> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(value) = aosp_primary_storage_uuid(trimmed) {
            section = DumpSection::Other;
            current_volume = None;
            current_disk = None;
            record_primary_storage_uuid(&mut dump, value);
            continue;
        }
        if section == DumpSection::Records {
            if !starts_storage_section(trimmed) {
                continue;
            }
            section = DumpSection::Other;
        }
        if trimmed == "Disks:" {
            section = DumpSection::Disks;
            current_volume = None;
            current_disk = None;
            continue;
        }
        if trimmed == "Volumes:" {
            section = DumpSection::Volumes;
            current_volume = None;
            current_disk = None;
            continue;
        }
        if is_records_header(trimmed) {
            section = DumpSection::Records;
            current_volume = None;
            current_disk = None;
            continue;
        }
        if trimmed.starts_with("mVolumes") {
            section = DumpSection::Volumes;
            current_volume = None;
            current_disk = None;
            continue;
        }
        if trimmed.starts_with("mDisks") {
            section = DumpSection::Disks;
            current_volume = None;
            current_disk = None;
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("mPrimaryStorageUuid") {
            if let Some(value) = value.trim_start().strip_prefix('=') {
                record_primary_storage_uuid(&mut dump, value);
                continue;
            }
        }
        match section {
            DumpSection::Volumes => {
                if trimmed.contains("VolumeInfo{") {
                    let Some((id, _)) = trimmed
                        .split_once("VolumeInfo{")
                        .and_then(|(_, rest)| rest.split_once('}'))
                    else {
                        return Err(storage_failure(DeviceStorageFailure::Malformed));
                    };
                    let id = id.trim();
                    if id.is_empty() {
                        return Err(storage_failure(DeviceStorageFailure::Malformed));
                    }
                    current_disk = None;
                    current_volume = Some(upsert_volume(&mut dump.volumes, id));
                    continue;
                }
                if let Some(index) = current_volume {
                    apply_volume_tokens(&mut dump.volumes[index], trimmed);
                }
            }
            DumpSection::Disks => {
                if trimmed.contains("DiskInfo{") {
                    let Some((inner, _)) = trimmed
                        .split_once("DiskInfo{")
                        .and_then(|(_, rest)| rest.split_once('}'))
                    else {
                        return Err(storage_failure(DeviceStorageFailure::Malformed));
                    };
                    let disk_id = normalize_disk_id(inner);
                    current_volume = None;
                    current_disk = Some(upsert_disk(&mut dump.disks, disk_id));
                    continue;
                }
                if let Some(index) = current_disk {
                    apply_disk_tokens(&mut dump.disks[index], trimmed);
                }
            }
            DumpSection::Other | DumpSection::Records => {}
        }
    }
    if dump.primary_storage_conflict
        || dump.volumes.iter().any(|volume| volume.contradictory)
        || dump.disks.iter().any(|disk| disk.contradictory)
    {
        return Err(storage_failure(DeviceStorageFailure::Malformed));
    }
    Ok(dump)
}

/// Record one primary-storage declaration, failing closed when two
/// declarations name different identifiers. Repeated identical declarations
/// are consistent. Empty and null values carry no identifier and do not
/// conflict with a named value.
fn record_primary_storage_uuid(dump: &mut MountDump, raw: &str) {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return;
    }
    match dump.primary_storage_uuid.as_deref() {
        Some(existing) if !existing.eq_ignore_ascii_case(value) => {
            dump.primary_storage_conflict = true;
        }
        Some(_) => {}
        None => dump.primary_storage_uuid = Some(value.to_string()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DumpSection {
    Other,
    Volumes,
    Disks,
    Records,
}

fn is_records_header(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    lower == "records:"
        || lower.starts_with("records:")
        || lower == "mrecords:"
        || lower.starts_with("mrecords:")
}

/// Extract the top-level primary-storage UUID fact printed after the
/// historical records section by current AOSP storage-manager dumps.
fn aosp_primary_storage_uuid(trimmed: &str) -> Option<&str> {
    trimmed.strip_prefix("Primary storage UUID:").map(str::trim)
}

fn starts_storage_section(trimmed: &str) -> bool {
    trimmed == "Disks:"
        || trimmed == "Volumes:"
        || is_records_header(trimmed)
        || aosp_primary_storage_uuid(trimmed).is_some()
        || trimmed.starts_with("mVolumes")
        || trimmed.starts_with("mDisks")
        || trimmed.starts_with("mPrimaryStorageUuid")
        || trimmed.starts_with("System build date")
}

/// Normalize a storage-manager disk identifier to its bare value.
///
/// Current AOSP prints a disk identifier as `disk:<major>,<minor>` in both a
/// `DiskInfo{...}` header and a volume's `diskId=` field. Older reports print
/// only the bare value. Both forms normalize to the same string so a volume
/// and its disk compare exactly.
fn normalize_disk_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return None;
    }
    let without_prefix = if value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("disk:"))
    {
        &value[5..]
    } else {
        value
    };
    let stripped = without_prefix.trim();
    if stripped.is_empty() || stripped.eq_ignore_ascii_case("null") {
        return None;
    }
    Some(stripped.to_string())
}

fn upsert_volume(volumes: &mut Vec<MountVolume>, id: &str) -> usize {
    if let Some(index) = volumes.iter().position(|volume| volume.id == id) {
        return index;
    }
    volumes.push(MountVolume::new(id.to_string()));
    volumes.len() - 1
}

fn upsert_disk(disks: &mut Vec<MountDisk>, disk_id: Option<String>) -> usize {
    if let Some(index) = disks
        .iter()
        .position(|disk| disk.disk_id.is_some() && disk.disk_id == disk_id)
    {
        return index;
    }
    disks.push(MountDisk {
        disk_id,
        flags: DiskFlags::default(),
        flags_observed: false,
        contradictory: false,
    });
    disks.len() - 1
}

fn apply_volume_tokens(volume: &mut MountVolume, line: &str) {
    for_each_key_value(line, |key, value| match key {
        "type" => {
            let next = match value.to_ascii_uppercase().as_str() {
                "PRIVATE" => MountVolumeType::Private,
                "EMULATED" => MountVolumeType::Emulated,
                "PUBLIC" => MountVolumeType::Public,
                _ => MountVolumeType::Unknown,
            };
            if volume.volume_type == MountVolumeType::Unknown {
                volume.volume_type = next;
            } else if next != MountVolumeType::Unknown && next != volume.volume_type {
                volume.contradictory = true;
            }
        }
        "diskid" => merge_volume_disk_id(volume, value),
        "mountuserid" => merge_volume_mount_user(volume, value),
        "mountflags" => merge_volume_mount_flags(volume, value),
        "state" => merge_volume_state(volume, value),
        "fsuuid" => {
            merge_volume_option(&mut volume.fs_uuid, value, &mut volume.contradictory, true)
        }
        "path" => merge_volume_option(&mut volume.path, value, &mut volume.contradictory, false),
        "internalpath" => merge_volume_option(
            &mut volume.internal_path,
            value,
            &mut volume.contradictory,
            false,
        ),
        _ => {}
    });
}

fn merge_volume_state(volume: &mut MountVolume, value: &str) {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return;
    }
    if volume.state.is_empty() {
        volume.state = value.to_ascii_uppercase();
    } else if !volume.state.eq_ignore_ascii_case(value) {
        volume.contradictory = true;
    }
}

fn merge_volume_mount_flags(volume: &mut MountVolume, value: &str) {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return;
    }
    match volume.mount_flags.as_deref() {
        Some(existing) if existing != value => volume.contradictory = true,
        Some(_) => {}
        None => {
            volume.mount_flags = Some(value.to_string());
            volume.visible = Some(parse_volume_visibility(value));
        }
    }
}

fn merge_volume_disk_id(volume: &mut MountVolume, value: &str) {
    let next = normalize_disk_id(value);
    match (&volume.disk_id, next) {
        (None, Some(next)) => volume.disk_id = Some(next),
        (Some(existing), Some(next)) if existing != &next => volume.contradictory = true,
        _ => {}
    }
}

fn merge_volume_mount_user(volume: &mut MountVolume, value: &str) {
    let Ok(next) = value.parse::<i64>() else {
        return;
    };
    match volume.mount_user_id {
        None => volume.mount_user_id = Some(next),
        Some(existing) if existing != next => volume.contradictory = true,
        Some(_) => {}
    }
}

fn merge_volume_option(
    field: &mut Option<String>,
    value: &str,
    contradictory: &mut bool,
    case_insensitive: bool,
) {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return;
    }
    match field.as_deref() {
        Some(existing) => {
            let matches = if case_insensitive {
                existing.eq_ignore_ascii_case(value)
            } else {
                existing == value
            };
            if !matches {
                *contradictory = true;
            }
        }
        None => *field = Some(value.to_string()),
    }
}

fn apply_disk_tokens(disk: &mut MountDisk, line: &str) {
    let mut observed = None;
    for_each_key_value(line, |key, value| {
        if key == "flags" {
            observed = Some(parse_disk_flags(value));
        }
    });
    let flags = observed.or_else(|| {
        let flags = parse_disk_flags(line);
        (flags.sd || flags.usb || flags.adoptable || flags.default_primary).then_some(flags)
    });
    let Some(flags) = flags else {
        return;
    };
    if disk.flags_observed {
        if disk.flags != flags {
            disk.contradictory = true;
        }
    } else {
        disk.flags = flags;
        disk.flags_observed = true;
    }
}

fn for_each_key_value(line: &str, mut visit: impl FnMut(&str, &str)) {
    for token in line.split_whitespace() {
        let Some(index) = token.find(['=', ':']) else {
            continue;
        };
        let key = &token[..index];
        let value = &token[index + 1..];
        if key.is_empty() {
            continue;
        }
        let normalized = key.to_ascii_lowercase();
        visit(&normalized, value);
    }
}

fn parse_disk_flags(value: &str) -> DiskFlags {
    let mut flags = DiskFlags::default();
    for raw in value.split(['|', ',', '+']) {
        let token = raw.trim().to_ascii_uppercase();
        if token.is_empty() {
            continue;
        }
        let token = token.strip_prefix("FLAG_").unwrap_or(token.as_str());
        match token {
            "SD" => flags.sd = true,
            "USB" => flags.usb = true,
            "ADOPTABLE" => flags.adoptable = true,
            "DEFAULT_PRIMARY" => flags.default_primary = true,
            _ => {}
        }
    }
    flags
}

/// Whether a reported `VolumeInfo.mountFlags` value makes the volume visible
/// to other applications. AOSP joins flag names with `|` and prints `0` when
/// no flags are set.
fn parse_volume_visibility(value: &str) -> bool {
    value.split('|').any(|raw| {
        let token = raw.trim().to_ascii_uppercase();
        let token = token.strip_prefix("MOUNT_FLAG_").unwrap_or(token.as_str());
        matches!(token, "VISIBLE" | "VISIBLE_FOR_READ" | "VISIBLE_FOR_WRITE")
    })
}

fn mount_state_from_text(raw: &str) -> VolumeMountState {
    match raw.trim().to_ascii_uppercase().as_str() {
        "MOUNTED" => VolumeMountState::MountedRw,
        "MOUNTED_RO" | "MOUNTEDRO" | "MOUNTED_READ_ONLY" => VolumeMountState::MountedRo,
        "UNMOUNTED" | "REMOVED" | "EJECTING" | "BAD_REMOVAL" | "UNMOUNTABLE" | "CHECKING"
        | "FORMATTING" => VolumeMountState::Unmounted,
        _ => VolumeMountState::Unknown,
    }
}

/// State reported by `sm list-volumes all` for one volume identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellVolumeState {
    Mounted,
    MountedRo,
    NotMounted,
    Unknown,
}

/// Current volume list from `sm list-volumes all`.
///
/// `None` means the same identifier appeared with two different states, which
/// makes the report internally inconsistent.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ShellVolumeList {
    entries: BTreeMap<String, Option<ShellVolumeState>>,
}

impl ShellVolumeList {
    fn state(&self, id: &str) -> ShellStateLookup {
        match self.entries.get(id) {
            None => ShellStateLookup::Missing,
            Some(None) => ShellStateLookup::Conflicting,
            Some(Some(state)) => ShellStateLookup::Known(*state),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellStateLookup {
    Missing,
    Conflicting,
    Known(ShellVolumeState),
}

fn parse_shell_volumes(stdout: &str) -> Result<ShellVolumeList, ApiError> {
    let lines = bounded_lines(stdout)?;
    let mut list = ShellVolumeList::default();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.ends_with(':') {
            continue;
        }
        let mut fields = trimmed.split_whitespace();
        let Some(id) = fields.next() else {
            continue;
        };
        let Some(raw_state) = fields.next() else {
            continue;
        };
        // AOSP prints "id environmentState fsUuid". The optional third field
        // is deliberately read and discarded so the filesystem UUID never
        // leaves this parser. Reports with more fields are not trusted.
        if fields.next().is_some() && fields.next().is_some() {
            continue;
        }
        let state = match raw_state.to_ascii_lowercase().as_str() {
            "mounted" => ShellVolumeState::Mounted,
            "mounted_ro" | "mountedro" | "mounted_read_only" => ShellVolumeState::MountedRo,
            "unmounted" | "ejecting" | "removed" | "bad_removal" | "checking" | "unmountable"
            | "formatting" => ShellVolumeState::NotMounted,
            _ => ShellVolumeState::Unknown,
        };
        match list.entries.get(id) {
            None => {
                list.entries.insert(id.to_string(), Some(state));
            }
            Some(Some(previous)) if *previous == state => {}
            Some(_) => {
                list.entries.insert(id.to_string(), None);
            }
        }
    }
    Ok(list)
}

fn bounded_lines(stdout: &str) -> Result<Vec<&str>, ApiError> {
    if stdout.len() > MAX_REPORT_BYTES {
        return Err(storage_failure(DeviceStorageFailure::Malformed));
    }
    let mut lines = Vec::new();
    for line in stdout.lines() {
        if lines.len() >= MAX_REPORT_LINES {
            return Err(storage_failure(DeviceStorageFailure::Malformed));
        }
        lines.push(line);
    }
    Ok(lines)
}

/// Primary-storage identifier the storage manager currently uses.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PrimaryStorage {
    /// Built-in storage backs the primary volume. On AOSP this is the
    /// per-user emulated volume. Null, empty, and the legacy `private`
    /// spelling all describe this case because `UUID_PRIVATE_INTERNAL` is
    /// null in Android 10 and later.
    BuiltInInternal,
    /// An adopted private volume backs the primary volume. The payload is the
    /// private volume's filesystem UUID.
    AdoptedBacking(String),
    /// The default physical disk backs the primary volume.
    PhysicalPrimary,
    /// An explicitly named public volume backs the primary volume.
    NamedFsUuid(String),
    /// An explicit `emulated;<user>` primary declaration. Only user 0 remains
    /// accepted as compatibility evidence; another user fails closed.
    EmulatedUser(i64),
    /// No usable primary-storage identifier was reported.
    Unreported,
}

fn primary_storage(dump: &MountDump) -> PrimaryStorage {
    let Some(raw) = dump.primary_storage_uuid.as_deref() else {
        return PrimaryStorage::Unreported;
    };
    let value = raw.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("null")
        || value.eq_ignore_ascii_case("private")
    {
        return PrimaryStorage::BuiltInInternal;
    }
    if value.eq_ignore_ascii_case("primary_physical") {
        return PrimaryStorage::PhysicalPrimary;
    }
    if let Some(user) = value
        .strip_prefix("emulated;")
        .and_then(|value| value.parse::<i64>().ok())
    {
        return PrimaryStorage::EmulatedUser(user);
    }
    if let Some(uuid) = value.strip_prefix("public:") {
        return PrimaryStorage::NamedFsUuid(uuid.to_string());
    }
    PrimaryStorage::AdoptedBacking(value.to_string())
}

fn classify_volumes(dump: &MountDump, shell: &ShellVolumeList) -> Vec<DeviceVolume> {
    let mut volumes = Vec::new();
    let primary = primary_storage(dump);
    let internal_source = internal_source(dump, shell, &primary, &mut volumes);
    for volume in &dump.volumes {
        if Some(volume.id.as_str()) == internal_source.as_deref() {
            continue;
        }
        match volume.volume_type {
            MountVolumeType::Public => {
                volumes.push(classify_public_volume(dump, shell, volume));
            }
            MountVolumeType::Private => {
                // A private volume that is not backing primary storage is
                // only meaningful adopted media once it has mounted or
                // carries a filesystem UUID. The always-present removed
                // placeholder must not be shown as an unavailable
                // destination.
                if volume.mount_state().is_mounted() || volume.normalized_fs_uuid().is_some() {
                    volumes.push(unsupported_volume(
                        volume,
                        VolumeKind::AdoptedPrivate,
                        "volume_type_private",
                    ));
                }
            }
            MountVolumeType::Unknown => {
                // An unrecognized volume type is reported as unknown rather
                // than dropped, so unexpected OEM storage is visible without
                // being mistaken for a supported destination.
                if !volume.state.is_empty() && !volume.state.eq_ignore_ascii_case("REMOVED") {
                    volumes.push(unknown_volume(volume));
                }
            }
            MountVolumeType::Emulated => {}
        }
    }

    remove_ambiguous_duplicate_identities(&mut volumes);
    volumes.sort_by(|left, right| {
        left.kind
            .sort_rank()
            .cmp(&right.kind.sort_rank())
            .then_with(|| left.identity_token.cmp(&right.identity_token))
    });
    volumes
}

/// Select the internal-shared volume, pushing its classified entry.
fn internal_source(
    dump: &MountDump,
    shell: &ShellVolumeList,
    primary: &PrimaryStorage,
    volumes: &mut Vec<DeviceVolume>,
) -> Option<String> {
    match primary {
        PrimaryStorage::BuiltInInternal | PrimaryStorage::Unreported => {
            resolve_emulated_primary(dump, volumes)
        }
        // Android normally reports the private internal UUID rather than
        // `emulated;<user>`. Keep the system-user spelling as a compatibility
        // allowance, but do not treat another user's volume as the selected
        // device user's internal storage.
        PrimaryStorage::EmulatedUser(0) => resolve_named_emulated(dump, 0, volumes),
        PrimaryStorage::EmulatedUser(_) => {
            volumes.push(ambiguous_internal("emulated_user_not_system"));
            None
        }
        PrimaryStorage::PhysicalPrimary => resolve_physical_primary(dump, shell, volumes),
        PrimaryStorage::AdoptedBacking(uuid) => resolve_adopted_backing(dump, shell, uuid, volumes),
        PrimaryStorage::NamedFsUuid(uuid) => resolve_public_by_uuid(dump, shell, uuid, volumes),
    }
}

/// Resolve built-in primary storage to its per-user emulated volume. The
/// system user is preferred when several emulated volumes are present. A sole
/// volume with no positive user evidence keeps the assumed-user compatibility
/// fallback; positive evidence for another user is ambiguous.
fn resolve_emulated_primary(dump: &MountDump, volumes: &mut Vec<DeviceVolume>) -> Option<String> {
    let emulated = dump
        .volumes
        .iter()
        .filter(|volume| volume.volume_type == MountVolumeType::Emulated)
        .collect::<Vec<_>>();
    if emulated.is_empty() {
        volumes.push(unavailable_internal("primary_storage_unresolved"));
        return None;
    }
    let system_user_conflict = emulated.iter().any(|volume| {
        volume.emulated_user_evidence() == EmulatedUserEvidence::Conflict
            && (volume.emulated_id_user() == Some(0) || volume.mount_user_id == Some(0))
    });
    if system_user_conflict
        || (emulated.len() == 1
            && emulated[0].emulated_user_evidence() == EmulatedUserEvidence::Conflict)
    {
        volumes.push(ambiguous_internal("emulated_user_conflict"));
        return None;
    }
    let system_user = emulated
        .iter()
        .copied()
        .find(|volume| volume.emulated_user_evidence() == EmulatedUserEvidence::Associated(0));
    match (system_user, emulated.len()) {
        (Some(volume), _) => {
            volumes.push(internal_volume(
                volume,
                &["primary_storage_emulated", "emulated_user_associated"],
            ));
            Some(volume.id.clone())
        }
        (None, 1) => {
            let volume = emulated[0];
            let reasons = match volume.emulated_user_evidence() {
                // Older reports may omit both the user suffix and mountUserId.
                // Preserve their existing single-device-user compatibility
                // fallback, but never infer another user's volume.
                EmulatedUserEvidence::Unknown => {
                    vec!["primary_storage_emulated", "emulated_user_assumed"]
                }
                _ => {
                    volumes.push(ambiguous_internal("emulated_user_not_system"));
                    return None;
                }
            };
            volumes.push(internal_volume(volume, &reasons));
            Some(volume.id.clone())
        }
        (None, _) => {
            volumes.push(ambiguous_internal("emulated_users_ambiguous"));
            None
        }
    }
}

fn resolve_named_emulated(
    dump: &MountDump,
    user: i64,
    volumes: &mut Vec<DeviceVolume>,
) -> Option<String> {
    resolve_emulated_for_user(
        dump,
        user,
        &["primary_storage_emulated", "emulated_user_associated"],
        volumes,
    )
}

fn resolve_emulated_for_user(
    dump: &MountDump,
    user: i64,
    reasons: &[&str],
    volumes: &mut Vec<DeviceVolume>,
) -> Option<String> {
    if dump.volumes.iter().any(|volume| {
        volume.volume_type == MountVolumeType::Emulated
            && volume.emulated_user_evidence() == EmulatedUserEvidence::Conflict
            && (volume.emulated_id_user() == Some(user) || volume.mount_user_id == Some(user))
    }) {
        volumes.push(ambiguous_internal("emulated_user_conflict"));
        return None;
    }
    let matches = dump
        .volumes
        .iter()
        .filter(|volume| {
            volume.volume_type == MountVolumeType::Emulated
                && volume.emulated_user_evidence() == EmulatedUserEvidence::Associated(user)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [volume] => {
            volumes.push(internal_volume(volume, reasons));
            Some(volume.id.clone())
        }
        [] => {
            volumes.push(unavailable_internal("primary_storage_missing"));
            None
        }
        _ => {
            volumes.push(ambiguous_internal("emulated_users_ambiguous"));
            None
        }
    }
}

/// Resolve a raw primary-storage UUID. Current AOSP stores an adopted
/// volume's filesystem UUID in `mPrimaryStorageUuid`, so the UUID first
/// matches a private volume and then resolves to that user's emulated volume.
/// A raw UUID that matches a public volume is retained as a legacy physical
/// primary form.
fn resolve_adopted_backing(
    dump: &MountDump,
    shell: &ShellVolumeList,
    fs_uuid: &str,
    volumes: &mut Vec<DeviceVolume>,
) -> Option<String> {
    let private_matches = dump
        .volumes
        .iter()
        .filter(|volume| {
            volume.volume_type == MountVolumeType::Private
                && volume
                    .normalized_fs_uuid()
                    .is_some_and(|value| value.eq_ignore_ascii_case(fs_uuid))
        })
        .collect::<Vec<_>>();
    match private_matches.as_slice() {
        [private] => match emulated_stacked_volumes(dump, private).as_slice() {
            [emulated] => {
                volumes.push(internal_volume(
                    emulated,
                    &["primary_storage_adopted", "emulated_user_associated"],
                ));
                Some(emulated.id.clone())
            }
            [] => {
                volumes.push(unavailable_internal("primary_storage_missing"));
                None
            }
            _ => {
                volumes.push(ambiguous_internal("primary_storage_ambiguous"));
                None
            }
        },
        [] => resolve_public_by_uuid(dump, shell, fs_uuid, volumes),
        _ => {
            volumes.push(ambiguous_internal("primary_storage_ambiguous"));
            None
        }
    }
}

/// Find the emulated volume stacked above an adopted private volume.
///
/// AOSP derives the stacked volume id by replacing the `private` prefix with
/// `emulated` and appending the user id, for example `private:179,64` becomes
/// `emulated:179,64;<user>`. The private volume itself carries `USER_UNKNOWN`,
/// so its mount user is not evidence. Disk identity is used as a fallback for
/// reports that omit the private id suffix.
fn emulated_stacked_volumes<'a>(
    dump: &'a MountDump,
    private: &MountVolume,
) -> Vec<&'a MountVolume> {
    let backing_key = private
        .id
        .strip_prefix("private:")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let disk_match = private.disk_id.as_deref();
    let mut by_id = Vec::new();
    let mut by_disk = Vec::new();
    for volume in dump
        .volumes
        .iter()
        .filter(|volume| volume.volume_type == MountVolumeType::Emulated)
    {
        if let Some(key) = backing_key {
            if volume
                .id
                .strip_prefix("emulated:")
                .and_then(|rest| rest.split_once(';'))
                .is_some_and(|(candidate, _)| candidate == key)
            {
                by_id.push(volume);
                continue;
            }
        }
        if disk_match.is_some() && volume.disk_id.as_deref() == disk_match {
            by_disk.push(volume);
        }
    }
    if by_id.is_empty() {
        by_disk
    } else {
        by_id
    }
}

fn resolve_public_by_uuid(
    dump: &MountDump,
    shell: &ShellVolumeList,
    fs_uuid: &str,
    volumes: &mut Vec<DeviceVolume>,
) -> Option<String> {
    let matches = dump
        .volumes
        .iter()
        .filter(|volume| {
            volume.volume_type == MountVolumeType::Public
                && volume
                    .normalized_fs_uuid()
                    .is_some_and(|value| value.eq_ignore_ascii_case(fs_uuid))
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [volume] => promote_supported_public_primary(dump, shell, volume, volumes),
        [] => {
            volumes.push(unavailable_internal("primary_storage_missing"));
            None
        }
        _ => {
            volumes.push(ambiguous_internal("primary_storage_ambiguous"));
            None
        }
    }
}

/// Resolve `primary_physical` the way AOSP does: the primary physical volume
/// is the public volume whose disk reports `FLAG_DEFAULT_PRIMARY`.
fn resolve_physical_primary(
    dump: &MountDump,
    shell: &ShellVolumeList,
    volumes: &mut Vec<DeviceVolume>,
) -> Option<String> {
    let matches = dump
        .volumes
        .iter()
        .filter(|volume| {
            volume.volume_type == MountVolumeType::Public
                && dump.disks.iter().any(|disk| {
                    disk.disk_id.is_some()
                        && disk.disk_id == volume.disk_id
                        && disk.flags.default_primary
                })
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [volume] => promote_supported_public_primary(dump, shell, volume, volumes),
        [] => {
            volumes.push(unavailable_internal("primary_storage_missing"));
            None
        }
        _ => {
            volumes.push(ambiguous_internal("primary_storage_ambiguous"));
            None
        }
    }
}

/// Promote a public volume to the logical primary-storage source only when
/// its regular removable-SD classification proves it currently supported,
/// visible, and corroborated. When these gates do not pass, return no source
/// so the normal volume walk reports the volume's truthful unsupported,
/// unknown, unmounted, or ambiguous classification.
fn promote_supported_public_primary(
    dump: &MountDump,
    shell: &ShellVolumeList,
    volume: &MountVolume,
    volumes: &mut Vec<DeviceVolume>,
) -> Option<String> {
    if classify_public_volume(dump, shell, volume).status != VolumeStatus::Available {
        return None;
    }
    volumes.push(internal_volume(volume, &["primary_storage_physical"]));
    Some(volume.id.clone())
}

fn internal_volume(volume: &MountVolume, reasons: &[&str]) -> DeviceVolume {
    let mount_state = volume.mount_state();
    let status = match mount_state {
        VolumeMountState::MountedRw | VolumeMountState::MountedRo => VolumeStatus::Available,
        VolumeMountState::Unmounted => VolumeStatus::Unmounted,
        VolumeMountState::Unknown => VolumeStatus::Unknown,
    };
    let mut reasons = reasons
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    reasons.push(mount_reason(mount_state));
    DeviceVolume {
        identity_token: identity_token(
            VolumeKind::InternalShared,
            volume.normalized_fs_uuid().as_deref(),
        ),
        continuity_token: continuity_token(volume),
        kind: VolumeKind::InternalShared,
        status,
        mount_state,
        reasons,
    }
}

fn unavailable_internal(reason: &str) -> DeviceVolume {
    DeviceVolume {
        identity_token: identity_token(VolumeKind::InternalShared, None),
        continuity_token: placeholder_continuity_token(),
        kind: VolumeKind::InternalShared,
        status: VolumeStatus::Unknown,
        mount_state: VolumeMountState::Unknown,
        reasons: vec![reason.to_string(), mount_reason(VolumeMountState::Unknown)],
    }
}

fn ambiguous_internal(reason: &str) -> DeviceVolume {
    DeviceVolume {
        identity_token: identity_token(VolumeKind::InternalShared, None),
        continuity_token: placeholder_continuity_token(),
        kind: VolumeKind::InternalShared,
        status: VolumeStatus::Ambiguous,
        mount_state: VolumeMountState::Unknown,
        reasons: vec![reason.to_string()],
    }
}

fn mount_reason(mount_state: VolumeMountState) -> String {
    match mount_state {
        VolumeMountState::MountedRw => "volume_mounted_rw",
        VolumeMountState::MountedRo => "volume_mounted_ro",
        VolumeMountState::Unmounted => "volume_unmounted",
        VolumeMountState::Unknown => "mount_state_unknown",
    }
    .to_string()
}

fn classify_public_volume(
    dump: &MountDump,
    shell: &ShellVolumeList,
    volume: &MountVolume,
) -> DeviceVolume {
    let disk = volume.disk_id.as_ref().and_then(|disk_id| {
        dump.disks
            .iter()
            .find(|disk| disk.disk_id.as_ref() == Some(disk_id))
    });
    let mount_state = volume.mount_state();
    let fs_uuid = volume.normalized_fs_uuid();
    let fs_uuid_ref = fs_uuid.as_deref();
    let identity = |kind: VolumeKind| identity_token(kind, fs_uuid_ref);
    let classified = |kind: VolumeKind,
                      status: VolumeStatus,
                      mount_state: VolumeMountState,
                      reasons: Vec<String>| {
        classified_public(volume, identity(kind), kind, status, mount_state, reasons)
    };
    if disk.is_some_and(|disk| disk.flags.usb) {
        return classified(
            VolumeKind::Usb,
            VolumeStatus::Unsupported,
            mount_state,
            reasons_with("disk_usb", mount_state),
        );
    }
    let Some(disk) = disk else {
        return classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Unknown,
            mount_state,
            reasons_with("missing_disk_evidence", mount_state),
        );
    };
    if !disk.flags.sd {
        return classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Unknown,
            mount_state,
            reasons_with("disk_not_removable", mount_state),
        );
    }
    let Some(fs_uuid) = fs_uuid_ref else {
        return classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Unknown,
            mount_state,
            reasons_with("missing_fs_uuid", mount_state),
        );
    };
    let Some(path_form) = public_volume_path_form(volume.path.as_deref(), fs_uuid) else {
        return classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Unknown,
            mount_state,
            reasons_with("missing_path_evidence", mount_state),
        );
    };
    if path_form == PublicVolumePath::Mismatch {
        return classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Unknown,
            mount_state,
            reasons_with("path_mismatch", mount_state),
        );
    }
    let Some(flags_visible) = volume.visible else {
        return classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Unknown,
            mount_state,
            reasons_with("missing_mount_flags", mount_state),
        );
    };
    match public_volume_visibility(flags_visible, path_form) {
        PublicVolumeVisibility::Conflict => {
            return classified(
                VolumeKind::RemovableSd,
                VolumeStatus::Unknown,
                mount_state,
                reasons_with("visibility_conflict", mount_state),
            );
        }
        PublicVolumeVisibility::NotVisible => {
            return classified(
                VolumeKind::RemovableSd,
                VolumeStatus::Unsupported,
                mount_state,
                public_reasons(disk, "public_volume_not_visible", mount_state),
            );
        }
        PublicVolumeVisibility::Visible => {}
    }
    let corroboration = corroborate(mount_state, shell.state(&volume.id));
    match corroboration {
        Corroboration::AgreeMounted if mount_state.is_mounted() => classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Available,
            mount_state,
            public_reasons(disk, "disk_removable_sd", mount_state),
        ),
        Corroboration::AgreeUnmounted => classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Unmounted,
            mount_state,
            public_reasons(disk, "disk_removable_sd", mount_state),
        ),
        Corroboration::Conflicting => classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Ambiguous,
            mount_state,
            reasons_with("corroboration_conflict", mount_state),
        ),
        Corroboration::Missing | Corroboration::AgreeMounted => classified(
            VolumeKind::RemovableSd,
            VolumeStatus::Unknown,
            mount_state,
            reasons_with("corroboration_missing", mount_state),
        ),
    }
}

/// Where a public volume's mount path places it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicVolumePath {
    /// `/storage/<fsUuid>`: mounted into the shared storage namespace.
    Visible,
    /// `/mnt/media_rw/<fsUuid>`: mounted only for the media provider.
    NotVisible,
    /// A path that does not name this volume.
    Mismatch,
}

fn public_volume_path_form(path: Option<&str>, fs_uuid: &str) -> Option<PublicVolumePath> {
    let path = path?;
    if path == format!("/storage/{fs_uuid}") {
        Some(PublicVolumePath::Visible)
    } else if path == format!("/mnt/media_rw/{fs_uuid}") {
        Some(PublicVolumePath::NotVisible)
    } else {
        Some(PublicVolumePath::Mismatch)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicVolumeVisibility {
    Visible,
    NotVisible,
    Conflict,
}

/// Combine the reported mount flags with the mount path. Both sources must
/// agree before a card can be offered; disagreement fails closed.
fn public_volume_visibility(
    flags_visible: bool,
    path_form: PublicVolumePath,
) -> PublicVolumeVisibility {
    let path_visible = match path_form {
        PublicVolumePath::Visible => true,
        PublicVolumePath::NotVisible => false,
        PublicVolumePath::Mismatch => return PublicVolumeVisibility::Conflict,
    };
    if flags_visible != path_visible {
        PublicVolumeVisibility::Conflict
    } else if path_visible {
        PublicVolumeVisibility::Visible
    } else {
        PublicVolumeVisibility::NotVisible
    }
}

/// Build removable-SD reasons, retaining an adoptable-disk capability note.
/// `FLAG_ADOPTABLE` describes what the disk supports; it does not make the
/// public volume already adopted.
fn public_reasons(disk: &MountDisk, primary: &str, mount_state: VolumeMountState) -> Vec<String> {
    let mut reasons = reasons_with(primary, mount_state);
    if disk.flags.adoptable {
        reasons.push("disk_adoptable".to_string());
    }
    reasons
}

/// Build a reason list from one classification key and the reported mount
/// state. Keys are fixed product-safe identifiers with no device detail.
fn reasons_with(primary: &str, mount_state: VolumeMountState) -> Vec<String> {
    vec![primary.to_string(), mount_reason(mount_state)]
}

fn classified_public(
    volume: &MountVolume,
    identity_token: String,
    kind: VolumeKind,
    status: VolumeStatus,
    mount_state: VolumeMountState,
    reasons: Vec<String>,
) -> DeviceVolume {
    DeviceVolume {
        identity_token,
        continuity_token: continuity_token(volume),
        kind,
        status,
        mount_state,
        reasons,
    }
}

fn unsupported_volume(volume: &MountVolume, kind: VolumeKind, reason: &str) -> DeviceVolume {
    let mount_state = volume.mount_state();
    DeviceVolume {
        identity_token: identity_token(kind, volume.normalized_fs_uuid().as_deref()),
        continuity_token: continuity_token(volume),
        kind,
        status: VolumeStatus::Unsupported,
        mount_state,
        reasons: vec![reason.to_string(), mount_reason(mount_state)],
    }
}

fn unknown_volume(volume: &MountVolume) -> DeviceVolume {
    let mount_state = volume.mount_state();
    DeviceVolume {
        identity_token: identity_token(VolumeKind::Unknown, volume.normalized_fs_uuid().as_deref()),
        continuity_token: continuity_token(volume),
        kind: VolumeKind::Unknown,
        status: VolumeStatus::Unknown,
        mount_state,
        reasons: vec!["volume_type_unknown".to_string(), mount_reason(mount_state)],
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Corroboration {
    Missing,
    Conflicting,
    AgreeMounted,
    AgreeUnmounted,
}

fn corroborate(mount_state: VolumeMountState, shell: ShellStateLookup) -> Corroboration {
    match shell {
        ShellStateLookup::Missing => Corroboration::Missing,
        ShellStateLookup::Conflicting => Corroboration::Conflicting,
        ShellStateLookup::Known(state) => match (mount_state, state) {
            (VolumeMountState::MountedRw, ShellVolumeState::Mounted)
            | (VolumeMountState::MountedRo, ShellVolumeState::MountedRo) => {
                Corroboration::AgreeMounted
            }
            (VolumeMountState::Unmounted, ShellVolumeState::NotMounted) => {
                Corroboration::AgreeUnmounted
            }
            (VolumeMountState::MountedRw | VolumeMountState::MountedRo, _)
            | (VolumeMountState::Unmounted, _) => Corroboration::Conflicting,
            (VolumeMountState::Unknown, _) => Corroboration::Missing,
        },
    }
}

/// Two current volumes that resolve to the same identity cannot be told apart,
/// so neither may be offered as an available destination.
fn remove_ambiguous_duplicate_identities(volumes: &mut [DeviceVolume]) {
    let mut counts = BTreeMap::<String, usize>::new();
    for volume in volumes.iter() {
        *counts.entry(volume.identity_token.clone()).or_default() += 1;
    }
    for volume in volumes.iter_mut() {
        if counts.get(&volume.identity_token).copied().unwrap_or(0) > 1 {
            volume.status = VolumeStatus::Ambiguous;
            if !volume
                .reasons
                .iter()
                .any(|reason| reason == "duplicate_identity")
            {
                volume.reasons.push("duplicate_identity".to_string());
            }
        }
    }
}

/// Sidecar-only evidence that distinguishes two observations with the same
/// filesystem UUID. The digest covers the disk, mount and backing paths, mount
/// flags, mount state, and filesystem UUID. It never crosses the Tauri
/// boundary into React.
fn continuity_token(volume: &MountVolume) -> String {
    digest128(
        CONTINUITY_DOMAIN,
        &[
            volume.disk_id.as_deref().unwrap_or(""),
            volume.path.as_deref().unwrap_or(""),
            volume.internal_path.as_deref().unwrap_or(""),
            volume.mount_flags.as_deref().unwrap_or(""),
            &volume.state,
            volume.fs_uuid.as_deref().unwrap_or(""),
        ],
    )
}

fn placeholder_continuity_token() -> String {
    digest128(CONTINUITY_DOMAIN, &[""; 6])
}

fn identity_token(kind: VolumeKind, fs_uuid: Option<&str>) -> String {
    digest128(
        IDENTITY_DOMAIN,
        &[
            kind.token_name(),
            fs_uuid.unwrap_or(""),
            kind.transport_class(),
        ],
    )
}

fn observation_token(volumes: &[DeviceVolume]) -> String {
    let mut parts = Vec::with_capacity(volumes.len());
    for volume in volumes {
        parts.push(format!(
            "{}|{}|{}|{}|{}",
            volume.identity_token,
            volume.continuity_token,
            volume.kind.token_name(),
            volume.status.token_name(),
            volume.mount_state.token_name()
        ));
    }
    parts.sort();
    let borrowed = parts.iter().map(String::as_str).collect::<Vec<_>>();
    digest128(OBSERVATION_DOMAIN, &borrowed)
}

/// Deterministic opaque 128-bit digest rendered as 32 lowercase hex characters.
fn digest128(domain: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update([0u8]);
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_probe::CommandOutput;
    use std::cell::RefCell;

    const SERIAL: &str = "EMU-T04-SERIAL";

    #[derive(Debug)]
    struct ScriptedRunner {
        calls: RefCell<Vec<Vec<String>>>,
        operations: RefCell<Vec<ProcessOperation>>,
        responses: RefCell<Vec<Result<CommandOutput, crate::device_probe::DeviceProbeError>>>,
    }

    impl ScriptedRunner {
        fn new(
            responses: Vec<Result<CommandOutput, crate::device_probe::DeviceProbeError>>,
        ) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                operations: RefCell::new(Vec::new()),
                responses: RefCell::new(responses),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(
            &self,
            _argv: &[String],
        ) -> Result<CommandOutput, crate::device_probe::DeviceProbeError> {
            panic!("device storage probes must use the bounded runner path")
        }

        fn run_for(
            &self,
            argv: &[String],
            operation: ProcessOperation,
        ) -> Result<CommandOutput, crate::device_probe::DeviceProbeError> {
            self.calls.borrow_mut().push(argv.to_vec());
            self.operations.borrow_mut().push(operation);
            self.responses.borrow_mut().remove(0)
        }
    }

    fn completed(
        status: i32,
        stdout: &str,
        stderr: &str,
    ) -> Result<CommandOutput, crate::device_probe::DeviceProbeError> {
        Ok(CommandOutput {
            status_code: Some(status),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        })
    }

    fn one_available_device() -> Result<CommandOutput, crate::device_probe::DeviceProbeError> {
        completed(
            0,
            &format!("List of devices attached\n{SERIAL}\tdevice product:emu model:EMU_Device transport_id:1\n"),
            "",
        )
    }

    fn runner_with(mount: &str, list_volumes: &str) -> ScriptedRunner {
        ScriptedRunner::new(vec![
            one_available_device(),
            completed(0, mount, ""),
            completed(0, list_volumes, ""),
        ])
    }

    fn volume<'a>(value: &'a Value, kind: &str) -> &'a Value {
        value["volumes"]
            .as_array()
            .expect("volumes array")
            .iter()
            .find(|volume| volume["kind"] == kind)
            .unwrap_or_else(|| panic!("expected {kind} volume in {value}"))
    }

    fn identity(value: &Value, kind: &str) -> String {
        volume(value, kind)["identityToken"]
            .as_str()
            .expect("identity token")
            .to_string()
    }

    /// AOSP API 30-shaped storage-manager dump with an emulated primary
    /// volume and one removable SD card. The section order and field lines
    /// match the reports printed by StorageManagerService.dump and the
    /// VolumeInfo and DiskInfo dump methods.
    const API30_DUMP: &str = "Disks:
 DiskInfo{disk:179,0}:
  flags=DEFAULT_PRIMARY size=16000000000 label=null
  sysPath=/sys/block/sda
 DiskInfo{disk:179,64}:
  flags=SD size=32000000000 label=SDCARD
  sysPath=/sys/block/mmcblk0

Volumes:
 VolumeInfo{emulated;0}:
  type=EMULATED diskId=disk:179,0 partGuid=null mountFlags=PRIMARY|VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED
  fsType=null fsUuid=null fsLabel=null
  path=/storage/emulated/0 internalPath=/data/media/0
 VolumeInfo{public:179,65}:
  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED
  fsType=vfat fsUuid=1A2B-3C4D fsLabel=SDCARD
  path=/storage/1A2B-3C4D internalPath=/mnt/media_rw/1A2B-3C4D

Records:
 VolumeRecord:
  type=PUBLIC fsUuid=DEAD-BEEF partGuid=null
  nickname=OLD userFlags=0
  createdMillis=2020-01-01T00:00:00.000Z lastSeenMillis=2020-01-01T00:00:00.000Z lastTrimMillis=2020-01-01T00:00:00.000Z lastBenchMillis=2020-01-01T00:00:00.000Z

Primary storage UUID: null

Internal storage (/storage/emulated/0) total size: 16000000000 (15258 MiB)
";
    const API30_LIST: &str = "emulated;0 mounted null
public:179,65 mounted 1A2B-3C4D
";

    /// AOSP API 33-shaped storage-manager dump with an emulated primary
    /// volume and one removable SD card.
    const API33_DUMP: &str = "Disks:
 DiskInfo{disk:179,0}:
  flags=DEFAULT_PRIMARY size=32000000000 label=null
  sysPath=/sys/block/sda
 DiskInfo{disk:179,64}:
  flags=SD size=32000000000 label=SDCARD
  sysPath=/sys/block/mmcblk0

Volumes:
 VolumeInfo{emulated;0}:
  type=EMULATED diskId=disk:179,0 partGuid=null mountFlags=PRIMARY|VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED
  fsType=null fsUuid=null fsLabel=null
  path=/storage/emulated/0 internalPath=/data/media/0
 VolumeInfo{public:179,65}:
  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED
  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD
  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F

Records:
 VolumeRecord:
  type=PUBLIC fsUuid=DEAD-BEEF partGuid=null
  nickname=OLD userFlags=0
  createdMillis=2020-01-01T00:00:00.000Z lastSeenMillis=2020-01-01T00:00:00.000Z lastTrimMillis=2020-01-01T00:00:00.000Z lastBenchMillis=2020-01-01T00:00:00.000Z

Primary storage UUID: null

Internal storage (/storage/emulated/0) total size: 32000000000 (30517 MiB)
";
    const API33_LIST: &str = "emulated;0 mounted null
public:179,65 mounted 8C4B-1A2F
";

    /// AOSP API 34-shaped storage-manager dump with an exFAT card on a later
    /// disk identifier.
    const API34_DUMP: &str = "Disks:
 DiskInfo{disk:179,0}:
  flags=DEFAULT_PRIMARY size=64000000000 label=null
  sysPath=/sys/block/sda
 DiskInfo{disk:179,64}:
  flags=SD size=32000000000 label=SDCARD
  sysPath=/sys/block/mmcblk0

Volumes:
 VolumeInfo{emulated;0}:
  type=EMULATED diskId=disk:179,0 partGuid=null mountFlags=PRIMARY|VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED
  fsType=null fsUuid=null fsLabel=null
  path=/storage/emulated/0 internalPath=/data/media/0
 VolumeInfo{public:179,65}:
  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED
  fsType=vfat fsUuid=5E6F-7A8B fsLabel=SDCARD
  path=/storage/5E6F-7A8B internalPath=/mnt/media_rw/5E6F-7A8B

Records:
 VolumeRecord:
  type=PUBLIC fsUuid=DEAD-BEEF partGuid=null
  nickname=OLD userFlags=0
  createdMillis=2020-01-01T00:00:00.000Z lastSeenMillis=2020-01-01T00:00:00.000Z lastTrimMillis=2020-01-01T00:00:00.000Z lastBenchMillis=2020-01-01T00:00:00.000Z

Primary storage UUID: null

Internal storage (/storage/emulated/0) total size: 64000000000 (61035 MiB)
";
    const API34_LIST: &str = "emulated;0 mounted null
public:179,65 mounted 5E6F-7A8B
";

    /// Legacy pre-section storage-manager report retained for compatibility
    /// with older OEM builds. Its two-field shell listing is also legacy.
    const LEGACY_DUMP: &str = "Current StorageManager state:
mBootCompleted=true
mDaemonConnected=true
mPrimaryStorageUuid=null
mVolumes:
- VolumeInfo{emulated;0}:
type=EMULATED diskId=1 partGuid= mountFlags=0 mountUserId=0 state=MOUNTED
fsType=null fsUuid=null fsLabel=null
path=/storage/emulated id=emulated;0
internalPath=/data/media
- VolumeInfo{public:1A2B-3C4D}:
type=PUBLIC diskId=1 partGuid= mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED
fsType=vfat fsUuid=1A2B-3C4D fsLabel=SDCARD
path=/storage/1A2B-3C4D
mDisks:
- DiskInfo{disk:1}:
flags=FLAG_SD
";
    const LEGACY_LIST: &str = "emulated;0 mounted
public:1A2B-3C4D mounted
";

    #[test]
    fn legacy_storage_manager_sections_still_parse() {
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(LEGACY_DUMP, LEGACY_LIST))
                .expect("legacy report should parse");
        assert_eq!(
            volume(&observation, "internalShared")["status"],
            "available"
        );
        assert_eq!(volume(&observation, "removableSd")["status"], "available");
        assert_eq!(observation["volumes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn probe_issues_inventory_mount_and_shell_volume_commands_as_probes() {
        let runner = runner_with(API33_DUMP, API33_LIST);

        let observation = list_device_volumes_with_runner("adb", SERIAL, &runner)
            .expect("observation should succeed");

        assert_eq!(observation["state"], "observed");
        assert_eq!(
            runner.calls(),
            vec![
                vec!["adb".to_string(), "devices".to_string(), "-l".to_string()],
                vec![
                    "adb".to_string(),
                    "-s".to_string(),
                    SERIAL.to_string(),
                    "shell".to_string(),
                    "dumpsys".to_string(),
                    "mount".to_string(),
                ],
                vec![
                    "adb".to_string(),
                    "-s".to_string(),
                    SERIAL.to_string(),
                    "shell".to_string(),
                    "sm".to_string(),
                    "list-volumes".to_string(),
                    "all".to_string(),
                ],
            ]
        );
        assert_eq!(
            *runner.operations.borrow(),
            [
                ProcessOperation::Probe,
                ProcessOperation::Probe,
                ProcessOperation::Probe
            ]
        );
    }

    #[test]
    fn aosp_api33_report_classifies_emulated_internal_storage_and_removable_sd() {
        let runner = runner_with(API33_DUMP, API33_LIST);

        let observation = list_device_volumes_with_runner("adb", SERIAL, &runner)
            .expect("observation should succeed");

        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "available");
        assert_eq!(internal["mountState"], "mountedRw");
        assert_eq!(
            internal["reasons"],
            json!([
                "primary_storage_emulated",
                "emulated_user_associated",
                "volume_mounted_rw"
            ])
        );

        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "available");
        assert_eq!(sd["mountState"], "mountedRw");
        assert_eq!(
            sd["reasons"],
            json!(["disk_removable_sd", "volume_mounted_rw"])
        );

        assert_eq!(observation["volumes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn aosp_api30_and_api34_reports_classify_like_api33() {
        for (dump, list) in [(API30_DUMP, API30_LIST), (API34_DUMP, API34_LIST)] {
            let observation =
                list_device_volumes_with_runner("adb", SERIAL, &runner_with(dump, list))
                    .expect("observation should succeed");

            let internal = volume(&observation, "internalShared");
            assert_eq!(internal["status"], "available");
            assert_eq!(internal["mountState"], "mountedRw");

            let sd = volume(&observation, "removableSd");
            assert_eq!(sd["status"], "available");
            assert_eq!(sd["mountState"], "mountedRw");

            assert_eq!(observation["volumes"].as_array().unwrap().len(), 2);
        }
    }

    #[test]
    fn observation_and_identity_tokens_are_stable_and_128_bit() {
        let first =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(API33_DUMP, API33_LIST))
                .expect("first observation");
        let second =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(API33_DUMP, API33_LIST))
                .expect("second observation");

        assert_eq!(first["observationToken"], second["observationToken"]);
        assert_eq!(
            identity(&first, "removableSd"),
            identity(&second, "removableSd")
        );
        let token = identity(&first, "removableSd");
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn same_card_is_identified_even_when_the_mount_path_changes() {
        let relocated = API33_DUMP
            .replace(
                "mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat",
                "mountFlags=0 mountUserId=0 state=MOUNTED\n  fsType=vfat",
            )
            .replace(
                "path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
                "path=/mnt/media_rw/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            );
        let original =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(API33_DUMP, API33_LIST))
                .expect("original");
        let relocated =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&relocated, API33_LIST))
                .expect("relocated");
        assert_eq!(
            identity(&original, "removableSd"),
            identity(&relocated, "removableSd")
        );
        assert_ne!(
            volume(&original, "removableSd")["continuityToken"],
            volume(&relocated, "removableSd")["continuityToken"]
        );
        let sd = volume(&relocated, "removableSd");
        assert_eq!(sd["status"], "unsupported");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "public_volume_not_visible"));
    }

    #[test]
    fn same_filesystem_uuid_on_another_disk_changes_continuity_not_identity() {
        let moved = API33_DUMP
            .replace("DiskInfo{disk:179,64}", "DiskInfo{disk:179,80}")
            .replace("diskId=disk:179,64", "diskId=disk:179,80")
            .replace("public:179,65", "public:179,81");
        let moved_list = API33_LIST.replace("public:179,65", "public:179,81");
        let original =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(API33_DUMP, API33_LIST))
                .expect("original");
        let moved =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&moved, &moved_list))
                .expect("moved");

        assert_eq!(
            identity(&original, "removableSd"),
            identity(&moved, "removableSd")
        );
        assert_ne!(
            volume(&original, "removableSd")["continuityToken"],
            volume(&moved, "removableSd")["continuityToken"]
        );
        assert_eq!(volume(&moved, "removableSd")["status"], "available");
    }

    #[test]
    fn a_different_card_is_a_different_identity() {
        let replaced = API33_DUMP.replace("8C4B-1A2F", "1111-2222");
        let replaced_list = API33_LIST.replace("8C4B-1A2F", "1111-2222");
        let original =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(API33_DUMP, API33_LIST))
                .expect("original");
        let replaced =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&replaced, &replaced_list))
                .expect("replaced");

        assert_ne!(
            identity(&original, "removableSd"),
            identity(&replaced, "removableSd")
        );
    }

    #[test]
    fn multiple_distinct_cards_are_separate_candidates() {
        let dump = API33_DUMP
            .replace(
                "  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
                "  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F\n VolumeInfo{public:179,66}:\n  type=PUBLIC diskId=disk:179,96 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=9999-AAAA fsLabel=EMMC\n  path=/storage/9999-AAAA internalPath=/mnt/media_rw/9999-AAAA",
            )
            .replace(
                "  sysPath=/sys/block/mmcblk0",
                "  sysPath=/sys/block/mmcblk0\n DiskInfo{disk:179,96}:\n  flags=SD size=64000000000 label=EMMC\n  sysPath=/sys/block/mmcblk1",
            );
        let list = format!("{API33_LIST}public:179,66 mounted 9999-AAAA\n");
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, &list))
                .expect("observation");
        let candidates = observation["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|volume| volume["kind"] == "removableSd")
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 2);
        assert_ne!(
            candidates[0]["identityToken"],
            candidates[1]["identityToken"]
        );
        assert!(candidates
            .iter()
            .all(|volume| volume["status"] == "available"));
    }

    #[test]
    fn duplicate_filesystem_uuids_are_ambiguous_and_never_available() {
        let dump = API33_DUMP
            .replace(
                "  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
                "  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F\n VolumeInfo{public:179,66}:\n  type=PUBLIC diskId=disk:179,96 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SECOND\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            )
            .replace(
                "  sysPath=/sys/block/mmcblk0",
                "  sysPath=/sys/block/mmcblk0\n DiskInfo{disk:179,96}:\n  flags=SD size=64000000000 label=EMMC\n  sysPath=/sys/block/mmcblk1",
            );
        let list = format!("{API33_LIST}public:179,66 mounted 8C4B-1A2F\n");
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, &list))
                .expect("observation");
        let candidates = observation["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|volume| volume["kind"] == "removableSd")
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 2);
        for candidate in candidates {
            assert_eq!(candidate["status"], "ambiguous");
            assert!(candidate["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "duplicate_identity"));
        }
    }

    #[test]
    fn legacy_two_field_shell_listing_is_accepted() {
        let observation = list_device_volumes_with_runner(
            "adb",
            SERIAL,
            &runner_with(API33_DUMP, "emulated;0 mounted\npublic:179,65 mounted\n"),
        )
        .expect("two-field legacy listing should parse");
        assert_eq!(volume(&observation, "removableSd")["status"], "available");
    }

    #[test]
    fn missing_shell_corroboration_is_unknown_rather_than_available() {
        let runner = runner_with(API33_DUMP, "Emulated storage:\nemulated;0 mounted\n");

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("observation");

        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unknown");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "corroboration_missing"));
    }

    #[test]
    fn shell_filesystem_uuid_is_accepted_but_not_forwarded() {
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(API33_DUMP, API33_LIST))
                .expect("observation");
        assert_eq!(volume(&observation, "removableSd")["status"], "available");
        let serialized = serde_json::to_string(&observation).expect("serialized observation");
        for forbidden in ["8C4B-1A2F", "/storage/", "mmcblk0", "DEAD-BEEF"] {
            assert!(
                !serialized.contains(forbidden),
                "observation must not contain raw device evidence"
            );
        }
    }

    #[test]
    fn shell_listing_with_extra_fields_is_not_trusted() {
        let observation = list_device_volumes_with_runner(
            "adb",
            SERIAL,
            &runner_with(
                API33_DUMP,
                "emulated;0 mounted null\npublic:179,65 mounted 8C4B-1A2F EXTRA\n",
            ),
        )
        .expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unknown");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "corroboration_missing"));
    }

    #[test]
    fn disagreeing_shell_state_is_ambiguous() {
        let runner = runner_with(API33_DUMP, "public:179,65 unmounted\n");

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("observation");

        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "ambiguous");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "corroboration_conflict"));
    }

    #[test]
    fn unmounted_and_ejecting_volumes_are_not_available() {
        let unmounted = API33_DUMP.replace(
            "state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F",
            "state=UNMOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F",
        );
        let runner = runner_with(&unmounted, "public:179,65 unmounted 8C4B-1A2F\n");
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("observation");
        assert_eq!(volume(&observation, "removableSd")["status"], "unmounted");
        assert_eq!(
            volume(&observation, "removableSd")["mountState"],
            "unmounted"
        );

        let ejecting = API33_DUMP.replace(
            "state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F",
            "state=EJECTING\n  fsType=vfat fsUuid=8C4B-1A2F",
        );
        let runner = runner_with(&ejecting, "public:179,65 ejecting 8C4B-1A2F\n");
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("observation");
        assert_eq!(volume(&observation, "removableSd")["status"], "unmounted");
    }

    #[test]
    fn read_only_mount_is_available_with_mounted_ro_state() {
        let ro_dump = API33_DUMP.replace(
            "state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F",
            "state=MOUNTED_READ_ONLY\n  fsType=vfat fsUuid=8C4B-1A2F",
        );
        let runner = runner_with(&ro_dump, "public:179,65 mounted_ro 8C4B-1A2F\n");
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "available");
        assert_eq!(sd["mountState"], "mountedRo");
    }

    #[test]
    fn usb_volume_is_unsupported_and_never_a_removable_candidate() {
        let dump = API33_DUMP.replace(
            " flags=SD size=32000000000",
            " flags=SD|USB size=32000000000",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let usb = volume(&observation, "usb");
        assert_eq!(usb["status"], "unsupported");
        assert!(observation["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|volume| volume["kind"] != "removableSd"));
    }

    #[test]
    fn adoptable_disk_keeps_a_public_volume_as_a_removable_candidate() {
        let dump = API33_DUMP.replace(
            " flags=SD size=32000000000",
            " flags=ADOPTABLE|SD size=32000000000",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "available");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "disk_adoptable"));
        assert!(observation["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|volume| volume["kind"] != "adoptedPrivate"));
    }

    #[test]
    fn mounted_adopted_volume_is_unsupported_rather_than_portable() {
        let dump = API33_DUMP.replace(
            " VolumeInfo{emulated;0}:",
            " VolumeInfo{private:179,64}:\n  type=PRIVATE diskId=disk:179,64 partGuid=null mountFlags=0 mountUserId=-1 state=MOUNTED\n  fsType=f2fs fsUuid=ADOPTED-PRIVATE fsLabel=Adopted\n  path=/mnt/expand/ADOPTED-PRIVATE internalPath=null\n VolumeInfo{emulated;0}:",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let adopted = volume(&observation, "adoptedPrivate");
        assert_eq!(adopted["status"], "unsupported");
        assert!(adopted["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "volume_type_private"));
        assert_eq!(
            volume(&observation, "internalShared")["status"],
            "available"
        );
    }

    #[test]
    fn unrecognized_volume_type_is_reported_as_unknown_not_dropped() {
        let dump = API33_DUMP.replace(
            "\n\nRecords:",
            "\n VolumeInfo{vendor:0A0A}:\n  type=VENDOR_ONLY diskId=5 partGuid=null mountFlags=0 mountUserId=0 state=MOUNTED\n  fsType=ext4 fsUuid=VENDOR-UUID fsLabel=Vendor\n  path=/storage/VENDOR-UUID internalPath=null\n\nRecords:",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let unknown = volume(&observation, "unknown");
        assert_eq!(unknown["status"], "unknown");
        assert_eq!(unknown["mountState"], "mountedRw");
        assert!(unknown["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "volume_type_unknown"));
    }

    #[test]
    fn missing_disk_evidence_never_produces_an_available_card() {
        let dump = API33_DUMP.replace(
            "type=PUBLIC diskId=disk:179,64 partGuid=null",
            "type=PUBLIC diskId=disk:179,97 partGuid=null",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unknown");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "missing_disk_evidence"));
    }

    #[test]
    fn missing_filesystem_uuid_never_produces_an_available_card() {
        let dump = API33_DUMP.replace("fsUuid=8C4B-1A2F", "fsUuid=null");

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unknown");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "missing_fs_uuid"));
    }

    #[test]
    fn missing_mount_path_never_produces_an_available_card() {
        let dump = API33_DUMP.replace(
            "path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            "path=null internalPath=/mnt/media_rw/8C4B-1A2F",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unknown");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "missing_path_evidence"));
    }

    #[test]
    fn truncated_public_volume_without_mount_flags_is_unknown() {
        let dump = API33_DUMP.replace(
            " mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat",
            " mountUserId=0 state=MOUNTED\n  fsType=vfat",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unknown");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "missing_mount_flags"));
    }

    #[test]
    fn usb_and_sd_with_the_same_filesystem_uuid_have_separate_identities() {
        let dump = API33_DUMP
            .replace(
                " DiskInfo{disk:179,64}:\n  flags=SD size=32000000000 label=SDCARD\n  sysPath=/sys/block/mmcblk0",
                " DiskInfo{disk:179,64}:\n  flags=SD size=32000000000 label=SDCARD\n  sysPath=/sys/block/mmcblk0\n DiskInfo{disk:179,96}:\n  flags=USB size=16000000000 label=USB\n  sysPath=/sys/block/sdb",
            )
            .replace(
                " VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
                " VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F\n VolumeInfo{public:179,66}:\n  type=PUBLIC diskId=disk:179,96 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=USB\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            );
        let list = format!("{API33_LIST}public:179,66 mounted 8C4B-1A2F\n");
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, &list))
                .expect("observation");
        let sd = volume(&observation, "removableSd");
        let usb = volume(&observation, "usb");
        assert_eq!(sd["status"], "available");
        assert_eq!(usb["status"], "unsupported");
        assert_ne!(sd["identityToken"], usb["identityToken"]);
    }

    #[test]
    fn unmounted_non_visible_public_volume_is_unsupported() {
        let dump = API33_DUMP
            .replace(
                "type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED",
                "type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=0 mountUserId=0 state=UNMOUNTED",
            )
            .replace(
                "path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
                "path=/mnt/media_rw/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            );
        let list = API33_LIST.replace(
            "public:179,65 mounted 8C4B-1A2F",
            "public:179,65 unmounted 8C4B-1A2F",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, &list))
                .expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unsupported");
        assert_eq!(sd["mountState"], "unmounted");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "public_volume_not_visible"));
    }

    #[test]
    fn adopted_primary_resolves_the_matching_emulated_volume() {
        let dump = API33_DUMP
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: ADOPTED-UUID",
            )
            .replace(
                " VolumeInfo{emulated;0}:",
                " VolumeInfo{private:179,64}:\n  type=PRIVATE diskId=disk:179,64 partGuid=null mountFlags=0 mountUserId=-1 state=MOUNTED\n  fsType=f2fs fsUuid=ADOPTED-UUID fsLabel=Adopted\n  path=/mnt/expand/ADOPTED-UUID internalPath=null\n VolumeInfo{emulated:179,64;10}:\n  type=EMULATED diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=10 state=MOUNTED\n  fsType=null fsUuid=null fsLabel=null\n  path=/storage/ADOPTED-UUID internalPath=/mnt/expand/ADOPTED-UUID/media\n VolumeInfo{emulated;0}:",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "available");
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "primary_storage_adopted"));
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "emulated_user_associated"));
        assert_eq!(
            observation["volumes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|volume| volume["kind"] == "removableSd")
                .count(),
            1
        );
    }

    #[test]
    fn adopted_primary_without_a_unique_emulated_volume_fails_closed() {
        let dump = API33_DUMP
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: ADOPTED-UUID",
            )
            .replace(
                " VolumeInfo{emulated;0}:",
                " VolumeInfo{private:179,64}:\n  type=PRIVATE diskId=disk:179,64 partGuid=null mountFlags=0 mountUserId=-1 state=MOUNTED\n  fsType=f2fs fsUuid=ADOPTED-UUID fsLabel=Adopted\n  path=/mnt/expand/ADOPTED-UUID internalPath=null\n VolumeInfo{emulated;0}:",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "unknown");
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "primary_storage_missing"));
        assert_eq!(
            volume(&observation, "adoptedPrivate")["status"],
            "unsupported"
        );
    }

    #[test]
    fn multiple_private_volumes_do_not_displace_built_in_internal_storage() {
        let dump = API33_DUMP.replace(
            " VolumeInfo{emulated;0}:",
            " VolumeInfo{private:179,64}:\n  type=PRIVATE diskId=disk:179,64 partGuid=null mountFlags=0 mountUserId=-1 state=MOUNTED\n  fsType=f2fs fsUuid=ADOPTED-ONE fsLabel=Adopted\n  path=/mnt/expand/ADOPTED-ONE internalPath=null\n VolumeInfo{private:179,80}:\n  type=PRIVATE diskId=disk:179,80 partGuid=null mountFlags=0 mountUserId=-1 state=MOUNTED\n  fsType=f2fs fsUuid=ADOPTED-TWO fsLabel=Adopted\n  path=/mnt/expand/ADOPTED-TWO internalPath=null\n VolumeInfo{emulated;0}:",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "available");
        assert_eq!(
            observation["volumes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|volume| volume["kind"] == "adoptedPrivate")
                .count(),
            2
        );
    }

    #[test]
    fn duplicate_primary_private_uuid_is_ambiguous() {
        let dump = API33_DUMP
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: ADOPTED-SHARED",
            )
            .replace(
                " VolumeInfo{emulated;0}:",
                " VolumeInfo{private:179,64}:\n  type=PRIVATE diskId=disk:179,64 partGuid=null mountFlags=0 mountUserId=-1 state=MOUNTED\n  fsType=f2fs fsUuid=ADOPTED-SHARED fsLabel=Adopted\n  path=/mnt/expand/ADOPTED-SHARED internalPath=null\n VolumeInfo{private:179,80}:\n  type=PRIVATE diskId=disk:179,80 partGuid=null mountFlags=0 mountUserId=-1 state=MOUNTED\n  fsType=f2fs fsUuid=ADOPTED-SHARED fsLabel=Adopted\n  path=/mnt/expand/ADOPTED-SHARED internalPath=null\n VolumeInfo{emulated;0}:",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "ambiguous");
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "primary_storage_ambiguous"));
    }

    #[test]
    fn legacy_private_primary_resolves_built_in_emulated_storage() {
        let dump = API33_DUMP.replace(
            "Primary storage UUID: null",
            "Primary storage UUID: private",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "available");
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "primary_storage_emulated"));
        assert!(observation["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|volume| volume["kind"] != "adoptedPrivate"));
    }

    #[test]
    fn physical_primary_selects_the_default_primary_public_volume() {
        let dump = API33_DUMP
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: primary_physical",
            )
            .replace(
                " flags=SD size=32000000000 label=SDCARD",
                " flags=DEFAULT_PRIMARY|SD size=32000000000 label=SDCARD",
            );
        let baseline =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(API33_DUMP, API33_LIST))
                .expect("baseline");
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "available");
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "primary_storage_physical"));
        assert_ne!(
            internal["identityToken"],
            volume(&baseline, "internalShared")["identityToken"],
            "primary_physical must not fall back to the emulated volume"
        );
        assert!(observation["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|volume| volume["kind"] != "removableSd"));
    }

    #[test]
    fn physical_primary_without_a_unique_default_primary_volume_fails_closed() {
        let dump = API33_DUMP.replace(
            "Primary storage UUID: null",
            "Primary storage UUID: primary_physical",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "unknown");
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "primary_storage_missing"));
        assert_eq!(volume(&observation, "removableSd")["status"], "available");
    }

    #[test]
    fn physical_primary_usb_candidate_does_not_become_internal_storage() {
        let dump = API33_DUMP
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: primary_physical",
            )
            .replace(
                " flags=SD size=32000000000 label=SDCARD",
                " flags=DEFAULT_PRIMARY|SD|USB size=32000000000 label=SDCARD",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        let usb = volume(&observation, "usb");
        assert_eq!(usb["status"], "unsupported");
        assert!(usb["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "disk_usb"));
        assert!(observation["volumes"]
            .as_array()
            .expect("volumes")
            .iter()
            .all(|volume| volume["kind"] != "internalShared" || volume["status"] != "available"));
    }

    #[test]
    fn physical_primary_without_visibility_evidence_is_not_internal_storage() {
        let dump = API33_DUMP
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: primary_physical",
            )
            .replace(
                " flags=SD size=32000000000 label=SDCARD",
                " flags=DEFAULT_PRIMARY|SD size=32000000000 label=SDCARD",
            )
            .replace(
                " mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat",
                " mountUserId=0 state=MOUNTED\n  fsType=vfat",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unknown");
        assert!(sd["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "missing_mount_flags"));
        assert!(observation["volumes"]
            .as_array()
            .expect("volumes")
            .iter()
            .all(|volume| volume["kind"] != "internalShared" || volume["status"] != "available"));
    }

    #[test]
    fn physical_primary_with_conflicting_shell_state_is_not_internal_storage() {
        let dump = API33_DUMP
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: primary_physical",
            )
            .replace(
                " flags=SD size=32000000000 label=SDCARD",
                " flags=DEFAULT_PRIMARY|SD size=32000000000 label=SDCARD",
            );
        let list = "public:179,65 unmounted 8C4B-1A2F\n";
        let observation = list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, list))
            .expect("observation");

        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "ambiguous");
        assert!(sd["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "corroboration_conflict"));
        assert!(observation["volumes"]
            .as_array()
            .expect("volumes")
            .iter()
            .all(|volume| volume["kind"] != "internalShared" || volume["status"] != "available"));
    }

    #[test]
    fn explicit_public_primary_does_not_resolve_a_private_volume() {
        let dump = API33_DUMP
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: public:ADOPTED-PRIVATE",
            )
            .replace(
                " VolumeInfo{emulated;0}:",
                " VolumeInfo{private:179,64}:\n  type=PRIVATE diskId=disk:179,64 partGuid=null mountFlags=0 mountUserId=-1 state=MOUNTED\n  fsType=f2fs fsUuid=ADOPTED-PRIVATE fsLabel=Adopted\n  path=/mnt/expand/ADOPTED-PRIVATE internalPath=null\n VolumeInfo{emulated;0}:",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        assert_eq!(volume(&observation, "internalShared")["status"], "unknown");
        let adopted = volume(&observation, "adoptedPrivate");
        assert_eq!(adopted["status"], "unsupported");
        assert!(adopted["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "volume_type_private"));
        assert_eq!(
            observation["volumes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|volume| volume["kind"] == "removableSd")
                .count(),
            1
        );
    }

    #[test]
    fn mounted_public_volume_that_is_not_visible_is_unsupported() {
        let dump = API33_DUMP
            .replace(
                "mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat",
                "mountFlags=0 mountUserId=0 state=MOUNTED\n  fsType=vfat",
            )
            .replace(
                "path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
                "path=/mnt/media_rw/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["mountState"], "mountedRw");
        assert_eq!(sd["status"], "unsupported");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "public_volume_not_visible"));
    }

    #[test]
    fn visibility_disagreement_between_flags_and_path_is_unknown() {
        let dump = API33_DUMP.replace(
            "path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            "path=/mnt/media_rw/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");
        let sd = volume(&observation, "removableSd");
        assert_eq!(sd["status"], "unknown");
        assert!(sd["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "visibility_conflict"));
    }

    #[test]
    fn emulated_volume_without_a_path_field_still_classifies_internal_storage() {
        let dump = API33_DUMP.replace(
            "  path=/storage/emulated/0 internalPath=/data/media/0\n",
            "",
        );

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        assert_eq!(
            volume(&observation, "internalShared")["status"],
            "available"
        );
    }

    #[test]
    fn multiple_emulated_users_without_the_system_user_are_ambiguous() {
        let dump = API33_DUMP
            .replace(
                "VolumeInfo{emulated;0}:",
                "VolumeInfo{emulated;10}:",
            )
            .replace(
                "mountUserId=0 state=MOUNTED\n  fsType=null fsUuid=null fsLabel=null",
                "mountUserId=10 state=MOUNTED\n  fsType=null fsUuid=null fsLabel=null",
            )
            .replace(
                "path=/storage/emulated/0 internalPath=/data/media/0",
                "path=/storage/emulated/10 internalPath=/data/media/10",
            )
            .replace(
                " VolumeInfo{public:179,65}:",
                " VolumeInfo{emulated;11}:\n  type=EMULATED diskId=disk:179,0 partGuid=null mountFlags=0 mountUserId=11 state=MOUNTED\n  fsType=null fsUuid=null fsLabel=null\n  path=/storage/emulated/11 internalPath=/data/media/11\n VolumeInfo{public:179,65}:",
            );
        let list = "public:179,65 mounted 8C4B-1A2F\n";
        let observation = list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, list))
            .expect("observation");
        assert_eq!(
            volume(&observation, "internalShared")["status"],
            "ambiguous"
        );
    }

    #[test]
    fn contradictory_emulated_user_evidence_is_ambiguous() {
        let dump = API33_DUMP.replace("VolumeInfo{emulated;0}:", "VolumeInfo{emulated;10}:");

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "ambiguous");
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "emulated_user_conflict"));
    }

    #[test]
    fn sole_emulated_volume_for_another_user_is_ambiguous() {
        let dump = API33_DUMP
            .replace("VolumeInfo{emulated;0}:", "VolumeInfo{emulated;10}:")
            .replace(
                "mountUserId=0 state=MOUNTED\n  fsType=null fsUuid=null fsLabel=null",
                "mountUserId=10 state=MOUNTED\n  fsType=null fsUuid=null fsLabel=null",
            )
            .replace(
                "path=/storage/emulated/0 internalPath=/data/media/0",
                "path=/storage/emulated/10 internalPath=/data/media/10",
            );
        let list = "emulated;10 mounted null\npublic:179,65 mounted 8C4B-1A2F\n";
        let observation = list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, list))
            .expect("observation");

        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "ambiguous");
        let reasons = internal["reasons"].as_array().expect("reasons");
        assert!(reasons
            .iter()
            .any(|reason| reason == "emulated_user_not_system"));
        assert!(!reasons
            .iter()
            .any(|reason| reason == "emulated_user_associated"));
    }

    #[test]
    fn sole_emulated_volume_without_user_evidence_uses_assumed_user_compatibility() {
        let dump = API33_DUMP
            .replace("VolumeInfo{emulated;0}:", "VolumeInfo{emulated}:")
            .replace(
                " mountFlags=PRIMARY|VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=null",
                " mountFlags=PRIMARY|VISIBLE_FOR_READ|VISIBLE_FOR_WRITE state=MOUNTED\n  fsType=null",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "available");
        assert_eq!(
            internal["reasons"],
            json!([
                "primary_storage_emulated",
                "emulated_user_assumed",
                "volume_mounted_rw"
            ])
        );
    }

    #[test]
    fn explicit_non_system_emulated_primary_is_ambiguous() {
        let dump = API33_DUMP
            .replace("VolumeInfo{emulated;0}:", "VolumeInfo{emulated;10}:")
            .replace(
                "mountUserId=0 state=MOUNTED\n  fsType=null fsUuid=null fsLabel=null",
                "mountUserId=10 state=MOUNTED\n  fsType=null fsUuid=null fsLabel=null",
            )
            .replace(
                "path=/storage/emulated/0 internalPath=/data/media/0",
                "path=/storage/emulated/10 internalPath=/data/media/10",
            )
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: emulated;10",
            );
        let list = "emulated;10 mounted null\npublic:179,65 mounted 8C4B-1A2F\n";
        let observation = list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, list))
            .expect("observation");

        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "ambiguous");
        let reasons = internal["reasons"].as_array().expect("reasons");
        assert!(reasons
            .iter()
            .any(|reason| reason == "emulated_user_not_system"));
        assert!(!reasons
            .iter()
            .any(|reason| reason == "emulated_user_associated"));
    }

    #[test]
    fn explicit_system_emulated_primary_retains_compatibility() {
        let dump = API33_DUMP.replace(
            "Primary storage UUID: null",
            "Primary storage UUID: emulated;0",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "available");
        assert!(internal["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "emulated_user_associated"));
    }

    #[test]
    fn explicit_system_emulated_primary_does_not_assume_an_unknown_volume() {
        let dump = API33_DUMP
            .replace("VolumeInfo{emulated;0}:", "VolumeInfo{emulated}:")
            .replace(
                " mountFlags=PRIMARY|VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=null",
                " mountFlags=PRIMARY|VISIBLE_FOR_READ|VISIBLE_FOR_WRITE state=MOUNTED\n  fsType=null",
            )
            .replace(
                "Primary storage UUID: null",
                "Primary storage UUID: emulated;0",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "unknown");
        assert!(internal["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "primary_storage_missing"));
    }

    #[test]
    fn identical_repeated_volume_and_disk_records_are_accepted() {
        let dump = API33_DUMP
            .replace(
                " DiskInfo{disk:179,64}:\n  flags=SD size=32000000000 label=SDCARD\n  sysPath=/sys/block/mmcblk0",
                " DiskInfo{disk:179,64}:\n  flags=SD size=32000000000 label=SDCARD\n  sysPath=/sys/block/mmcblk0\n DiskInfo{disk:179,64}:\n  flags=SD size=32000000000 label=SDCARD\n  sysPath=/sys/block/mmcblk0",
            )
            .replace(
                " VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
                " VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F\n VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("identical repeated records should remain supported");
        assert_eq!(volume(&observation, "removableSd")["status"], "available");
        assert_eq!(
            observation["volumes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|volume| volume["kind"] == "removableSd")
                .count(),
            1
        );
    }

    #[test]
    fn repeated_records_differing_only_in_presentation_fields_are_accepted() {
        // `partGuid`, `fsType`, and `fsLabel` appear in platform dumps but
        // never feed classification. A repeat that changes only those values
        // must stay accepted instead of failing the report closed.
        let dump = API33_DUMP.replace(
            " VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            " VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F\n VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=8f3a1c77-4d2e-4c55-9b6f-0d1e2f3a4b5c mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=exfat fsUuid=8C4B-1A2F fsLabel=PROBE-CARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
        );
        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("presentation-only differences should remain supported");
        assert_eq!(volume(&observation, "removableSd")["status"], "available");
        assert_eq!(
            observation["volumes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|volume| volume["kind"] == "removableSd")
                .count(),
            1
        );
    }

    #[test]
    fn contradictory_repeated_volume_records_fail_closed() {
        let dump = API33_DUMP.replace(
            " VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
            " VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=UNMOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F\n VolumeInfo{public:179,65}:\n  type=PUBLIC diskId=disk:179,64 partGuid=null mountFlags=VISIBLE_FOR_READ|VISIBLE_FOR_WRITE mountUserId=0 state=MOUNTED\n  fsType=vfat fsUuid=8C4B-1A2F fsLabel=SDCARD\n  path=/storage/8C4B-1A2F internalPath=/mnt/media_rw/8C4B-1A2F",
        );
        let error = list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
            .expect_err("contradictory repeated volume records must fail closed");
        assert_eq!(error.details["reason"], "device_storage_malformed");
    }

    #[test]
    fn contradictory_repeated_disk_records_fail_closed() {
        let dump = API33_DUMP.replace(
            " DiskInfo{disk:179,64}:\n  flags=SD size=32000000000 label=SDCARD\n  sysPath=/sys/block/mmcblk0",
            " DiskInfo{disk:179,64}:\n  flags=USB size=32000000000 label=SDCARD\n  sysPath=/sys/block/mmcblk0\n DiskInfo{disk:179,64}:\n  flags=SD size=32000000000 label=SDCARD\n  sysPath=/sys/block/mmcblk0",
        );
        let error = list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
            .expect_err("contradictory repeated disk records must fail closed");
        assert_eq!(error.details["reason"], "device_storage_malformed");
    }

    #[test]
    fn conflicting_primary_storage_declarations_fail_closed() {
        let dump = API33_DUMP.replace(
            "Primary storage UUID: null",
            "mPrimaryStorageUuid=ADOPTED-A
Primary storage UUID: ADOPTED-B",
        );
        let error = list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
            .expect_err("conflicting primary-storage declarations must fail closed");
        assert_eq!(error.details["reason"], "device_storage_malformed");
    }

    #[test]
    fn records_section_is_ignored() {
        // Deliberately synthetic: record-shaped lines after Records: must not
        // create current volumes.
        let dump = format!(
            "{API33_DUMP}  Records:\n    - VolumeInfo{{public:DEAD-BEEF}}:\n        type=PUBLIC diskId:9 partGuid: mountFlags=0 mountUserId=0 state=MOUNTED\n        fsType=vfat fsUuid=DEAD-BEEF fsLabel=OLD\n    - DiskInfo{{disk:179,99}}:\n        flags=SD\n"
        );

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner_with(&dump, API33_LIST))
                .expect("observation");

        assert!(observation["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|volume| volume["identityToken"].as_str().is_some()));
        assert_eq!(observation["volumes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn oversized_report_fails_closed_without_echoing_device_text() {
        let oversized = "x".repeat(MAX_REPORT_BYTES + 1);
        let runner = ScriptedRunner::new(vec![
            one_available_device(),
            completed(0, &oversized, ""),
            completed(0, API33_LIST, ""),
        ]);

        let error = list_device_volumes_with_runner("adb", SERIAL, &runner)
            .expect_err("oversized report must fail");

        assert_eq!(error.code, crate::errors::ApiErrorCode::CommandFailed);
        assert_eq!(
            error.message,
            "Device storage volume details could not be read."
        );
        assert_eq!(error.details["reason"], "device_storage_malformed");
    }

    #[test]
    fn truncated_record_fails_closed() {
        let truncated =
            API33_DUMP.replace("VolumeInfo{public:179,65}:", "VolumeInfo{public:179,65");
        let runner = ScriptedRunner::new(vec![
            one_available_device(),
            completed(0, &truncated, ""),
            completed(0, API33_LIST, ""),
        ]);

        let error = list_device_volumes_with_runner("adb", SERIAL, &runner)
            .expect_err("unbalanced record must fail");

        assert_eq!(error.details["reason"], "device_storage_malformed");
    }

    #[test]
    fn oem_divergent_report_yields_an_unknown_internal_volume_instead_of_guessing() {
        let runner = runner_with("this device does not implement storage dumps\n", "");

        let observation = list_device_volumes_with_runner("adb", SERIAL, &runner)
            .expect("a divergent report is still an observation");

        assert_eq!(observation["state"], "observed");
        let internal = volume(&observation, "internalShared");
        assert_eq!(internal["status"], "unknown");
        assert!(internal["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "primary_storage_unresolved"));
    }

    #[test]
    fn errors_never_echo_raw_report_text_or_identifiers() {
        let hostile =
            "Volumes:\n VolumeInfo{public:SECRET-SERIAL 8C4B-1A2F /data/media/0 device exploded\n";
        let runner = ScriptedRunner::new(vec![
            one_available_device(),
            completed(0, hostile, "device exploded"),
            completed(0, API33_LIST, ""),
        ]);

        let error = list_device_volumes_with_runner("adb", SERIAL, &runner)
            .expect_err("malformed record must fail");
        let rendered = error.to_value().to_string();

        for needle in [
            "8C4B-1A2F",
            "SECRET-SERIAL",
            "/data/media",
            "device exploded",
        ] {
            assert!(
                !rendered.contains(needle),
                "error {rendered} must not echo {needle}"
            );
        }
    }

    #[test]
    fn non_zero_device_report_fails_with_a_fixed_error() {
        let runner = ScriptedRunner::new(vec![
            one_available_device(),
            completed(1, "VolumeInfo{public:179,65}: state=MOUNTED", "denied"),
            completed(0, API33_LIST, ""),
        ]);

        let error = list_device_volumes_with_runner("adb", SERIAL, &runner)
            .expect_err("failed report must fail");

        assert_eq!(error.details["reason"], "device_storage_probe_failed");
    }

    #[test]
    fn multiple_devices_skip_device_specific_probes() {
        let runner = ScriptedRunner::new(vec![completed(
            0,
            "List of devices attached\nAAA\tdevice transport_id:1\nBBB\tdevice transport_id:2\n",
            "",
        )]);

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("inventory outcome");

        assert_eq!(observation["state"], "multiple_devices");
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn a_different_connected_serial_reports_no_device_without_probing() {
        let runner = ScriptedRunner::new(vec![completed(
            0,
            "List of devices attached\nOTHER\tdevice transport_id:1\n",
            "",
        )]);

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("inventory outcome");

        assert_eq!(observation["state"], "no_device");
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn unauthorized_device_reports_unauthorized_without_probing() {
        let runner = ScriptedRunner::new(vec![completed(
            0,
            &format!("List of devices attached\n{SERIAL}\tunauthorized\n"),
            "",
        )]);

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("inventory outcome");

        assert_eq!(observation["state"], "unauthorized");
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn unrecognized_inventory_rows_prevent_a_single_device_probe() {
        let runner = ScriptedRunner::new(vec![completed(
            0,
            &format!(
                "List of devices attached\n{SERIAL}\tdevice transport_id:1\nOTHER\tno permissions (missing udev rules)\n"
            ),
            "",
        )]);

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("inventory outcome");

        assert_eq!(observation["state"], "multiple_devices");
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn state_less_inventory_rows_prevent_a_single_device_probe() {
        let runner = ScriptedRunner::new(vec![completed(
            0,
            &format!("List of devices attached\n{SERIAL}\tdevice transport_id:1\nTRUNCATED\n"),
            "",
        )]);

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("inventory outcome");

        assert_eq!(observation["state"], "multiple_devices");
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn diagnostic_looking_serial_prevents_a_single_device_probe() {
        let runner = ScriptedRunner::new(vec![
            completed(
                0,
                &format!(
                    "List of devices attached\n{SERIAL}\tdevice transport_id:1\ncannot device\n"
                ),
                "",
            ),
            completed(0, API33_DUMP, ""),
            completed(0, API33_LIST, ""),
        ]);

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("inventory outcome");

        assert_eq!(observation["state"], "multiple_devices");
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn daemon_chatter_does_not_prevent_single_device_storage_discovery() {
        let runner = ScriptedRunner::new(vec![
            completed(
                0,
                &format!(
                    "List of devices attached\n* daemon started successfully\n* daemon started successfully *\nadb server version (41) doesn't match this client (40); killing...\ndaemon not running; starting now at tcp:5037\nerror: more than one device/emulator\nadb: no devices/emulators found\nadb: insufficient permissions for device: user in plugdev group; are your udev rules wrong?\nerror: device 'missing' not found\n{SERIAL}\tdevice transport_id:1\n"
                ),
                "",
            ),
            completed(0, API33_DUMP, ""),
            completed(0, API33_LIST, ""),
        ]);

        let observation =
            list_device_volumes_with_runner("adb", SERIAL, &runner).expect("inventory outcome");

        assert_eq!(observation["state"], "observed");
        assert_eq!(runner.calls().len(), 3);
        assert_eq!(
            runner.calls()[1],
            vec!["adb", "-s", SERIAL, "shell", "dumpsys", "mount"]
        );
    }

    #[test]
    fn storage_discovery_counts_device_rows_outside_the_common_shape() {
        for extra in [
            "my phone device transport_id:2",
            "serial device unexpected",
            "serial no permissions usb:1-1",
            "*serial device",
            "List of devices attached extra",
            "adb: no devices/emulators found extra",
            "error: device 'missing'",
            "adb server version (41)\tdevice",
            "* daemon not running\tdevice",
        ] {
            let runner = ScriptedRunner::new(vec![
                completed(
                    0,
                    &format!(
                        "List of devices attached\n{SERIAL}\tdevice transport_id:1\n{extra}\n"
                    ),
                    "",
                ),
                completed(0, API33_DUMP, ""),
                completed(0, API33_LIST, ""),
            ]);

            let observation =
                list_device_volumes_with_runner("adb", SERIAL, &runner).expect("inventory outcome");

            assert_eq!(
                observation["state"], "multiple_devices",
                "extra row: {extra}"
            );
            assert_eq!(runner.calls().len(), 1, "extra row: {extra}");
        }
    }
}
