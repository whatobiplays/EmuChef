//! Trusted support diagnostics and app-owned artifact-cache management.
//!
//! React receives only aggregate diagnostics and opaque, generation-scoped
//! logical-entry handles. Paths, filenames, metadata identities, raw bundle
//! bytes, and deletion authority remain inside this module.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::commands::AppState;
use crate::support_codes::SupportCode;

const MAX_CACHE_ENTRIES: usize = 4_096;
const MAX_METADATA_BYTES: u64 = 16 * 1_024;
const MAX_DIAGNOSTICS_BYTES: usize = 2 * 1_024 * 1_024;
const METADATA_SUFFIX: &str = ".emuchef-cache.json";
const MAX_RESET_AUTHORIZATIONS: usize = 64;
const DIAGNOSTICS_MEMBER_NAMES: [&str; 7] = [
    "manifest.json",
    "runtime.json",
    "catalog.json",
    "configuration-summary.json",
    "execution-summaries.json",
    "cache-summary.json",
    "support-status.json",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheMetadata {
    schema_version: u64,
    payload_file_name: String,
    artifact_label: String,
    source_kind: String,
    source_fingerprint: String,
    payload_size_bytes: u64,
    payload_modified_nanos: Option<u128>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileFingerprint {
    len: u64,
    modified_nanos: Option<u128>,
}

#[derive(Clone, Debug)]
struct CacheEntryRecord {
    handle: String,
    payload: PathBuf,
    metadata: Option<PathBuf>,
    payload_fingerprint: FileFingerprint,
    metadata_fingerprint: Option<FileFingerprint>,
    category: &'static str,
    label: String,
    source_kind: String,
    integrity: &'static str,
    size_bytes: u64,
    age_bucket: &'static str,
    in_use: bool,
    removable: bool,
}

/// Bounded, restart-volatile mapping for one trusted cache root.
pub struct SupportStore {
    cache_root: PathBuf,
    generation: u64,
    presentation_revision: u64,
    entries: HashMap<String, CacheEntryRecord>,
    reset_authorizations: HashMap<String, ResetAuthorization>,
    reset_order: VecDeque<String>,
}

impl SupportStore {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            generation: 0,
            presentation_revision: 0,
            entries: HashMap::new(),
            reset_authorizations: HashMap::new(),
            reset_order: VecDeque::new(),
        }
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
    }

    fn issue_reset_authorization(&mut self, authorization: ResetAuthorization) -> String {
        let handle = format!("reset_{}", Uuid::new_v4().simple());
        self.reset_authorizations
            .insert(handle.clone(), authorization);
        self.reset_order.push_back(handle.clone());
        while self.reset_order.len() > MAX_RESET_AUTHORIZATIONS {
            if let Some(expired) = self.reset_order.pop_front() {
                self.reset_authorizations.remove(&expired);
            }
        }
        handle
    }

    fn take_reset_authorization(&mut self, handle: &str) -> Result<ResetAuthorization, String> {
        self.reset_order.retain(|candidate| candidate != handle);
        self.reset_authorizations.remove(handle).ok_or_else(|| {
            support_error(
                "reset_category_stale",
                "This reset option is outdated. Review the current local-state categories.",
            )
        })
    }

    fn refresh(&mut self, execution_in_flight: bool) -> Result<Value, String> {
        let canonical_root = canonical_cache_root(&self.cache_root)?;
        let mut paths = fs::read_dir(&canonical_root)
            .map_err(|_| {
                support_error(
                    "cache_inventory_failed",
                    "The artifact cache could not be inspected.",
                )
            })?
            .take(MAX_CACHE_ENTRIES + 1)
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                support_error(
                    "cache_inventory_failed",
                    "The artifact cache could not be inspected.",
                )
            })?;
        if paths.len() > MAX_CACHE_ENTRIES {
            self.invalidate();
            return Err(support_error(
                "cache_inventory_limit_exceeded",
                "The artifact cache contains more entries than this app can safely manage at once.",
            ));
        }
        paths.sort();
        let path_set = paths.iter().cloned().collect::<HashSet<_>>();
        let mut next = HashMap::new();
        let mut unmanaged_count = 0_u64;
        let mut unmanaged_size = 0_u64;

        for path in paths {
            let file_name = match path.file_name().and_then(|name| name.to_str()) {
                Some(name) => name,
                None => {
                    unmanaged_count += 1;
                    continue;
                }
            };
            if is_metadata_name(file_name) {
                let associated = metadata_payload_name(file_name)
                    .map(|name| canonical_root.join(name))
                    .is_some_and(|payload| path_set.contains(&payload));
                if !associated {
                    unmanaged_count += 1;
                    unmanaged_size = unmanaged_size.saturating_add(file_len_if_regular(&path));
                }
                continue;
            }
            let category = if is_canonical_payload_name(file_name) {
                "artifact"
            } else if is_partial_name(file_name) {
                "partial"
            } else {
                unmanaged_count += 1;
                unmanaged_size = unmanaged_size.saturating_add(file_len_if_regular(&path));
                continue;
            };
            let payload_fingerprint = match fingerprint(&path) {
                Ok(value) => value,
                Err(_) => {
                    unmanaged_count += 1;
                    continue;
                }
            };
            let metadata_path = metadata_path(&path);
            let (metadata, metadata_fingerprint) = if metadata_path.is_file() {
                let fingerprint = fingerprint(&metadata_path).ok();
                let metadata = read_metadata(&metadata_path);
                (metadata, fingerprint)
            } else {
                (None, None)
            };
            let metadata_consistent = metadata.as_ref().is_some_and(|metadata| {
                metadata.schema_version == 1
                    && metadata.payload_file_name == file_name
                    && metadata.payload_size_bytes == payload_fingerprint.len
                    && metadata.payload_modified_nanos == payload_fingerprint.modified_nanos
                    && file_name.get(..64) == Some(metadata.source_fingerprint.as_str())
                    && matches!(metadata.source_kind.as_str(), "file" | "http" | "https")
                    && is_safe_identifier(&metadata.artifact_label)
                    && is_sha256(&metadata.source_fingerprint)
            });
            let metadata_bytes = metadata_fingerprint.as_ref().map_or(0, |value| value.len);
            let record = CacheEntryRecord {
                handle: format!("cache_{}", Uuid::new_v4().simple()),
                payload: path,
                metadata: metadata_fingerprint.as_ref().map(|_| metadata_path),
                payload_fingerprint: payload_fingerprint.clone(),
                metadata_fingerprint,
                category,
                label: if category == "partial" {
                    "Incomplete artifact download".to_string()
                } else if metadata_consistent {
                    metadata
                        .as_ref()
                        .map(|value| value.artifact_label.clone())
                        .unwrap_or_else(|| "Cached artifact".to_string())
                } else {
                    "Cached artifact".to_string()
                },
                source_kind: if metadata_consistent {
                    metadata
                        .as_ref()
                        .map(|value| value.source_kind.clone())
                        .unwrap_or_else(|| "unknown".to_string())
                } else {
                    "unknown".to_string()
                },
                integrity: if category == "partial" {
                    "incomplete"
                } else if metadata_consistent {
                    "complete"
                } else if metadata.is_some() {
                    "metadata_mismatch"
                } else {
                    "unindexed"
                },
                size_bytes: payload_fingerprint.len.saturating_add(metadata_bytes),
                age_bucket: age_bucket(&payload_fingerprint),
                in_use: execution_in_flight,
                removable: true,
            };
            next.insert(record.handle.clone(), record);
        }

        self.generation = self.generation.wrapping_add(1);
        self.entries = next;
        Ok(self.inventory_value(unmanaged_count, unmanaged_size))
    }

    fn inventory_value(&self, unmanaged_count: u64, unmanaged_size: u64) -> Value {
        let mut entries = self.entries.values().collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then(left.handle.cmp(&right.handle))
        });
        let total_size = entries
            .iter()
            .fold(0_u64, |total, entry| total.saturating_add(entry.size_bytes));
        let removable = self
            .entries
            .values()
            .filter(|entry| entry.removable && !entry.in_use)
            .collect::<Vec<_>>();
        let removable_size = removable
            .iter()
            .fold(0_u64, |total, entry| total.saturating_add(entry.size_bytes));
        json!({
            "generation": self.generation.to_string(),
            "entries": entries.into_iter().map(project_entry).collect::<Vec<_>>(),
            "summary": {
                "entryCount": self.entries.len(),
                "totalSizeBytes": total_size,
                "inUseCount": self.entries.values().filter(|entry| entry.in_use).count(),
                "removableCount": removable.len(),
                "removableSizeBytes": removable_size,
                "unusedRemovableCount": removable.len(),
                "unusedRemovableSizeBytes": removable_size,
                "unmanagedCount": unmanaged_count,
                "unmanagedSizeBytes": unmanaged_size,
            },
            "categories": [
                {
                    "id": "artifact",
                    "label": "Reusable setup downloads",
                    "description": "App-owned copies of files used to prepare a setup.",
                    "deletionConsequence": "Removed files are downloaded or copied again when a later setup needs them. Saved setups and original user files are preserved."
                },
                {
                    "id": "partial",
                    "label": "Incomplete downloads",
                    "description": "App-owned temporary files left by an incomplete transfer.",
                    "deletionConsequence": "A later setup starts the download again. Saved setups and original user files are preserved."
                }
            ],
        })
    }
}

#[derive(Clone, Debug)]
enum ResetAuthorization {
    Recents {
        revision: u64,
        item_count: usize,
    },
    Cache {
        revision: u64,
        item_count: usize,
        total_size_bytes: u64,
    },
    Recovery {
        revision: u64,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum SupportSeverity {
    Healthy,
    Neutral,
    Warning,
    Failure,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum CorrectiveActionDto {
    RestartService { service_generation: u64 },
    ImportManagedPlatformTools { platform_tools_revision: u64 },
    ReplaceManagedPlatformTools { platform_tools_revision: u64 },
    RemoveManagedPlatformTools { platform_tools_revision: u64 },
    RefreshDevices { device_generation: u64 },
    RefreshCache { cache_generation: String },
    OpenUpdates,
    OpenSavedSetupRepair,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SupportActionDto {
    label: &'static str,
    consequence: &'static str,
    available: bool,
    unavailable_reason: Option<&'static str>,
    destructive: bool,
    action: CorrectiveActionDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubsystemStatusDto {
    id: &'static str,
    label: &'static str,
    severity: SupportSeverity,
    summary: String,
    consequence: String,
    support_code: Option<&'static str>,
    actions: Vec<SupportActionDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsDisclosureDto {
    included_categories: Vec<&'static str>,
    excluded_categories: Vec<&'static str>,
    local_until_shared: bool,
    uploads_automatically: bool,
    maximum_size_bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetCategoryDto {
    reset_handle: Option<String>,
    id: &'static str,
    label: &'static str,
    description: &'static str,
    consequence: &'static str,
    affected_scope: &'static str,
    available: bool,
    unavailable_reason: Option<&'static str>,
    confirmation_required: bool,
    restart_required: bool,
    item_count: usize,
    total_size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SupportSnapshotDto {
    presentation_revision: u64,
    overall_severity: SupportSeverity,
    overall_summary: String,
    subsystems: Vec<SubsystemStatusDto>,
    cache_inventory: Option<Value>,
    diagnostics_disclosure: DiagnosticsDisclosureDto,
    reset_categories: Vec<ResetCategoryDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetLocalAppStateRequest {
    reset_handle: String,
    confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CacheCleanupRequest {
    mode: CacheCleanupMode,
    inventory_generation: String,
    #[serde(default)]
    entry_handles: Vec<String>,
    confirmation: CacheCleanupConfirmation,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CacheCleanupMode {
    Selected,
    Unused,
    AllRemovable,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheCleanupConfirmation {
    confirmed: bool,
    entry_count: usize,
    total_size_bytes: u64,
}

#[tauri::command]
pub fn get_cache_inventory(state: State<'_, AppState>) -> Result<Value, String> {
    let in_flight = state
        .executions
        .lock()
        .map_err(|_| {
            support_error(
                "execution_state_unavailable",
                "Execution state is unavailable.",
            )
        })?
        .has_in_flight();
    state
        .support
        .lock()
        .map_err(|_| support_error("support_state_unavailable", "Support state is unavailable."))?
        .refresh(in_flight)
}

#[tauri::command]
pub fn get_support_snapshot(state: State<'_, AppState>) -> Result<Value, String> {
    serde_json::to_value(build_support_snapshot(&state)?).map_err(|_| {
        support_error(
            "support_snapshot_failed",
            "Troubleshooting status could not be prepared.",
        )
    })
}

fn build_support_snapshot(state: &AppState) -> Result<SupportSnapshotDto, String> {
    let in_flight = state
        .executions
        .lock()
        .map(|executions| executions.has_in_flight())
        .unwrap_or(true);
    let execution_summary = state
        .executions
        .lock()
        .map(|executions| executions.support_summary(&state.sidecar))
        .unwrap_or_else(|_| json!({ "unavailable": true }));
    let runtime_value = serde_json::to_value(state.sidecar.status())
        .unwrap_or_else(|_| json!({ "status": "failed" }));
    let runtime_status = runtime_value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    let service_generation = state.sidecar.generation();

    let (adb_status, platform_tools_revision, app_managed) = state
        .adb
        .lock()
        .map(|adb| (adb.status(), adb.revision(), adb.is_app_managed()))
        .unwrap_or_else(|_| {
            (
                crate::adb::AdbSetupStatusDto {
                    status: "invalid",
                    version: None,
                    warning: None,
                    error: None,
                    can_import: false,
                    can_replace: false,
                    can_remove: false,
                },
                0,
                false,
            )
        });
    let device_summary = state
        .handles
        .lock()
        .map(|handles| handles.support_summary())
        .unwrap_or_else(|_| json!({ "generation": 0, "deviceCount": 0 }));
    let device_generation = device_summary
        .get("generation")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let device_count = device_summary
        .get("deviceCount")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let available_devices = device_summary
        .get("availableCount")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    let saved_summary = state
        .saved_configurations
        .lock()
        .map(|saved| saved.support_summary())
        .unwrap_or_else(|_| json!({ "recentCount": 0, "recentsRevision": 0 }));
    let recents_revision = saved_summary
        .get("recentsRevision")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let recent_count = saved_summary
        .get("recentCount")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    let recovery_summary = state
        .recovery
        .lock()
        .map(|recovery| recovery.support_summary())
        .unwrap_or_else(|_| json!({ "draftAvailable": false, "loadIssue": true }));
    let recovery_generation = recovery_summary
        .get("draftGeneration")
        .and_then(Value::as_u64);

    let update_value = state
        .updates
        .status()
        .ok()
        .and_then(|status| serde_json::to_value(status).ok())
        .unwrap_or_else(|| json!({ "state": "failed" }));
    let update_status = update_value
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("failed");

    let mut support_store = state.support.lock().map_err(|_| {
        support_error(
            "support_state_unavailable",
            SupportCode::SupportUnavailable.entry().message,
        )
    })?;
    let cache_inventory = support_store.refresh(in_flight).ok();
    support_store.presentation_revision =
        support_store.presentation_revision.saturating_add(1).max(1);
    let presentation_revision = support_store.presentation_revision;
    let cache_generation = cache_inventory
        .as_ref()
        .and_then(|inventory| inventory.get("generation"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let cache_removable_count = cache_inventory
        .as_ref()
        .and_then(|inventory| inventory.pointer("/summary/removableCount"))
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    let cache_removable_size = cache_inventory
        .as_ref()
        .and_then(|inventory| inventory.pointer("/summary/removableSizeBytes"))
        .and_then(Value::as_u64)
        .unwrap_or_default();

    let mut subsystems = Vec::new();
    let mut attention = false;

    let service = match runtime_status {
        "ready" => SubsystemStatusDto {
            id: "service",
            label: "App service",
            severity: SupportSeverity::Healthy,
            summary: "The local app service is ready.".to_string(),
            consequence: "Setup validation and planning are available.".to_string(),
            support_code: None,
            actions: Vec::new(),
        },
        "unsupported" => {
            attention = true;
            SubsystemStatusDto {
                id: "service",
                label: "App service",
                severity: SupportSeverity::Failure,
                summary: SupportCode::ServiceUnsupported.entry().message.to_string(),
                consequence: "Device setup cannot continue in this build.".to_string(),
                support_code: Some(SupportCode::ServiceUnsupported.code()),
                actions: Vec::new(),
            }
        }
        _ => {
            attention = true;
            SubsystemStatusDto {
                id: "service",
                label: "App service",
                severity: SupportSeverity::Failure,
                summary: SupportCode::ServiceStartFailed.entry().message.to_string(),
                consequence: "Catalog access, planning, and execution are unavailable.".to_string(),
                support_code: Some(SupportCode::ServiceStartFailed.code()),
                actions: vec![SupportActionDto {
                    label: "Restart app service",
                    consequence: "Portable setup intent is preserved, but device, review, and execution authority must be refreshed.",
                    available: !in_flight,
                    unavailable_reason: in_flight.then_some("An execution must finish before the service can restart."),
                    destructive: false,
                    action: CorrectiveActionDto::RestartService { service_generation },
                }],
            }
        }
    };
    subsystems.push(service);

    let platform = match adb_status.status {
        "ready" => {
            let platform_warning = adb_status.warning.is_some();
            if platform_warning {
                attention = true;
            }
            let mut actions = Vec::new();
            if app_managed {
                actions.push(SupportActionDto {
                    label: "Replace managed Platform-Tools",
                    consequence: "Device and review authority will be refreshed after the replacement is validated.",
                    available: !in_flight,
                    unavailable_reason: in_flight.then_some("An execution must finish before Platform-Tools can change."),
                    destructive: false,
                    action: CorrectiveActionDto::ReplaceManagedPlatformTools { platform_tools_revision },
                });
                actions.push(SupportActionDto {
                    label: "Remove managed Platform-Tools",
                    consequence: "Device detection stops until Platform-Tools are installed again. External installations are never deleted.",
                    available: !in_flight,
                    unavailable_reason: in_flight.then_some("An execution must finish before Platform-Tools can change."),
                    destructive: true,
                    action: CorrectiveActionDto::RemoveManagedPlatformTools { platform_tools_revision },
                });
            }
            SubsystemStatusDto {
                id: "platform_tools",
                label: "Android Platform-Tools",
                severity: if platform_warning {
                    SupportSeverity::Warning
                } else {
                    SupportSeverity::Healthy
                },
                summary: adb_status
                    .warning
                    .clone()
                    .unwrap_or_else(|| "Platform-Tools are ready.".to_string()),
                consequence: "Connected Android devices can be detected.".to_string(),
                support_code: platform_warning.then_some(SupportCode::PlatformToolsLimited.code()),
                actions,
            }
        }
        "missing" => {
            attention = true;
            SubsystemStatusDto {
                id: "platform_tools",
                label: "Android Platform-Tools",
                severity: SupportSeverity::Warning,
                summary: SupportCode::PlatformToolsMissing
                    .entry()
                    .message
                    .to_string(),
                consequence:
                    "Device detection is unavailable; offline setup work remains available."
                        .to_string(),
                support_code: Some(SupportCode::PlatformToolsMissing.code()),
                actions: vec![SupportActionDto {
                    label: "Install managed Platform-Tools",
                    consequence:
                        "EmuChef imports a validated ZIP into its private managed location.",
                    available: adb_status.can_import,
                    unavailable_reason: (!adb_status.can_import)
                        .then_some("Installation is temporarily unavailable."),
                    destructive: false,
                    action: CorrectiveActionDto::ImportManagedPlatformTools {
                        platform_tools_revision,
                    },
                }],
            }
        }
        _ => {
            attention = true;
            SubsystemStatusDto {
                id: "platform_tools",
                label: "Android Platform-Tools",
                severity: SupportSeverity::Failure,
                summary: SupportCode::PlatformToolsInvalid
                    .entry()
                    .message
                    .to_string(),
                consequence:
                    "Device detection is unavailable until a validated installation is selected."
                        .to_string(),
                support_code: Some(SupportCode::PlatformToolsInvalid.code()),
                actions: vec![SupportActionDto {
                    label: "Install managed Platform-Tools",
                    consequence:
                        "EmuChef imports a validated ZIP into its private managed location.",
                    available: adb_status.can_import,
                    unavailable_reason: (!adb_status.can_import)
                        .then_some("Installation is temporarily unavailable."),
                    destructive: false,
                    action: CorrectiveActionDto::ImportManagedPlatformTools {
                        platform_tools_revision,
                    },
                }],
            }
        }
    };
    subsystems.push(platform);

    let device_needs_attention = device_count > 0 && available_devices == 0;
    if device_needs_attention {
        attention = true;
    }
    subsystems.push(SubsystemStatusDto {
        id: "device",
        label: "Connected device",
        severity: if available_devices > 0 {
            SupportSeverity::Healthy
        } else if device_needs_attention {
            SupportSeverity::Warning
        } else {
            SupportSeverity::Neutral
        },
        summary: if available_devices > 0 {
            format!("{available_devices} available device{} detected.", if available_devices == 1 { "" } else { "s" })
        } else if device_count > 0 {
            "A connected device needs attention before use.".to_string()
        } else {
            "No Android device is currently detected.".to_string()
        },
        consequence: "Refreshing device status may invalidate a review if the selected device changed.".to_string(),
        support_code: device_needs_attention.then_some(SupportCode::DeviceUnavailable.code()),
        actions: vec![SupportActionDto {
            label: "Refresh connected devices",
            consequence: "Current device facts are re-read and stale review authority is invalidated when necessary.",
            available: adb_status.status == "ready" && !in_flight,
            unavailable_reason: (adb_status.status != "ready").then_some("Platform-Tools must be ready first."),
            destructive: false,
            action: CorrectiveActionDto::RefreshDevices { device_generation },
        }],
    });

    if state.catalog.is_ok() {
        subsystems.push(SubsystemStatusDto {
            id: "catalog",
            label: "Setup catalog",
            severity: SupportSeverity::Healthy,
            summary: "The bundled setup catalog is available.".to_string(),
            consequence: "Supported setups can be described and reviewed.".to_string(),
            support_code: None,
            actions: Vec::new(),
        });
    } else {
        attention = true;
        subsystems.push(SubsystemStatusDto {
            id: "catalog",
            label: "Setup catalog",
            severity: SupportSeverity::Failure,
            summary: SupportCode::CatalogUnavailable.entry().message.to_string(),
            consequence: "Setups cannot be described until the application resources are repaired."
                .to_string(),
            support_code: Some(SupportCode::CatalogUnavailable.code()),
            actions: Vec::new(),
        });
    }

    if cache_inventory.is_some() {
        subsystems.push(SubsystemStatusDto {
            id: "cache",
            label: "App-owned storage",
            severity: SupportSeverity::Healthy,
            summary: "App-owned cache inventory is available.".to_string(),
            consequence: "Only validated removable entries can be cleared.".to_string(),
            support_code: None,
            actions: vec![SupportActionDto {
                label: "Refresh storage inventory",
                consequence: "Current app-owned cache entries are inspected again.",
                available: !in_flight,
                unavailable_reason: in_flight
                    .then_some("Storage inventory is protected while execution is active."),
                destructive: false,
                action: CorrectiveActionDto::RefreshCache {
                    cache_generation: cache_generation.clone(),
                },
            }],
        });
    } else {
        attention = true;
        subsystems.push(SubsystemStatusDto {
            id: "cache",
            label: "App-owned storage",
            severity: SupportSeverity::Failure,
            summary: SupportCode::CacheInspectionFailed
                .entry()
                .message
                .to_string(),
            consequence: "Cache cleanup remains unavailable until a safe inventory can be built."
                .to_string(),
            support_code: Some(SupportCode::CacheInspectionFailed.code()),
            actions: Vec::new(),
        });
    }

    subsystems.push(match update_status {
        "failed" => {
            attention = true;
            SubsystemStatusDto {
                id: "updates",
                label: "Updates",
                severity: SupportSeverity::Warning,
                summary: SupportCode::UpdateCheckFailed.entry().message.to_string(),
                consequence: "Local use is unaffected; update availability is unknown.".to_string(),
                support_code: Some(SupportCode::UpdateCheckFailed.code()),
                actions: vec![SupportActionDto {
                    label: "Open Updates",
                    consequence: "Review current update status and retry when available.",
                    available: true,
                    unavailable_reason: None,
                    destructive: false,
                    action: CorrectiveActionDto::OpenUpdates,
                }],
            }
        }
        "unconfigured" => SubsystemStatusDto {
            id: "updates",
            label: "Updates",
            severity: SupportSeverity::Neutral,
            summary: "Update checking is not configured for this build.".to_string(),
            consequence: "Local use is unaffected.".to_string(),
            support_code: None,
            actions: Vec::new(),
        },
        _ => SubsystemStatusDto {
            id: "updates",
            label: "Updates",
            severity: SupportSeverity::Healthy,
            summary: "Update status is available.".to_string(),
            consequence: "Validated update information can be reviewed in Updates.".to_string(),
            support_code: None,
            actions: Vec::new(),
        },
    });

    let recovery_available = recovery_generation.is_some();
    let recovery_load_issue = recovery_summary
        .get("loadIssue")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if recovery_load_issue {
        attention = true;
    }
    subsystems.push(SubsystemStatusDto {
        id: "recovery",
        label: "Recovery data",
        severity: if recovery_load_issue { SupportSeverity::Warning } else { SupportSeverity::Neutral },
        summary: if recovery_load_issue {
            SupportCode::RecoveryDataInvalid.entry().message.to_string()
        } else if recovery_available {
            "A recovery draft is available for workflow review.".to_string()
        } else {
            "No recovery draft is waiting.".to_string()
        },
        consequence: if recovery_available {
            "A draft may restore portable setup choices, but never device identity, review plans, execution progress, or secrets.".to_string()
        } else {
            "Saved setup files and current workflow state are unaffected.".to_string()
        },
        support_code: recovery_load_issue.then_some(SupportCode::RecoveryDataInvalid.code()),
        actions: Vec::new(),
    });

    let missing_recents = saved_summary
        .get("missingRecentCount")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if missing_recents > 0 {
        attention = true;
    }
    subsystems.push(SubsystemStatusDto {
        id: "saved_configuration",
        label: "Saved setups",
        severity: if missing_recents > 0 {
            SupportSeverity::Warning
        } else {
            SupportSeverity::Healthy
        },
        summary: if missing_recents > 0 {
            format!(
                "{missing_recents} Recent entr{} need relinking or removal.",
                if missing_recents == 1 { "y" } else { "ies" }
            )
        } else {
            "Saved setup references are available.".to_string()
        },
        consequence: "Resetting Recents never deletes saved setup files.".to_string(),
        support_code: (missing_recents > 0)
            .then_some(SupportCode::SavedConfigurationReferenceMissing.code()),
        actions: if missing_recents > 0 {
            vec![SupportActionDto {
                label: "Open saved setup repair",
                consequence:
                    "Review missing Recent entries without changing saved files automatically.",
                available: true,
                unavailable_reason: None,
                destructive: false,
                action: CorrectiveActionDto::OpenSavedSetupRepair,
            }]
        } else {
            Vec::new()
        },
    });

    let execution_unavailable = execution_summary.get("unavailable").is_some();
    if execution_unavailable {
        attention = true;
    }
    subsystems.push(SubsystemStatusDto {
        id: "execution",
        label: "Execution retention",
        severity: if execution_unavailable {
            SupportSeverity::Warning
        } else {
            SupportSeverity::Neutral
        },
        summary: if in_flight {
            "An execution is starting or active."
        } else {
            "No execution is currently active."
        }
        .to_string(),
        consequence: if execution_unavailable {
            "Retained execution status could not be inspected.".to_string()
        } else {
            "Support inspection does not change retained execution state.".to_string()
        },
        support_code: execution_unavailable
            .then_some(SupportCode::ExecutionStateUnavailable.code()),
        actions: Vec::new(),
    });

    let recents_handle = (recent_count > 0).then(|| {
        support_store.issue_reset_authorization(ResetAuthorization::Recents {
            revision: recents_revision,
            item_count: recent_count,
        })
    });
    let cache_revision = support_store.generation;
    let cache_handle = (cache_removable_count > 0).then(|| {
        support_store.issue_reset_authorization(ResetAuthorization::Cache {
            revision: cache_revision,
            item_count: cache_removable_count,
            total_size_bytes: cache_removable_size,
        })
    });
    let recovery_handle = recovery_generation.map(|revision| {
        support_store.issue_reset_authorization(ResetAuthorization::Recovery { revision })
    });

    Ok(SupportSnapshotDto {
        presentation_revision,
        overall_severity: if attention { SupportSeverity::Warning } else { SupportSeverity::Healthy },
        overall_summary: if attention {
            "One or more systems need attention.".to_string()
        } else {
            "EmuChef is ready. No troubleshooting issues were found.".to_string()
        },
        subsystems,
        cache_inventory,
        diagnostics_disclosure: DiagnosticsDisclosureDto {
            included_categories: vec![
                "App and local-service readiness",
                "Operating-system class and feature gates",
                "Public catalog identity",
                "Aggregate saved-setup and execution state",
                "Aggregate cache counts and current public support codes",
            ],
            excluded_categories: vec![
                "Paths, serials, credentials, and environment values",
                "Raw logs, process output, and internal errors",
                "Configuration contents, input values, plans, and authority handles",
            ],
            local_until_shared: true,
            uploads_automatically: false,
            maximum_size_bytes: MAX_DIAGNOSTICS_BYTES,
        },
        reset_categories: vec![
            ResetCategoryDto {
                reset_handle: recents_handle,
                id: "recents",
                label: "Clear Recents",
                description: "Remove the app-owned list of recently opened setup files.",
                consequence: "Saved setup files, the active setup, and portable intent remain unchanged.",
                affected_scope: "Recent setup index only",
                available: recent_count > 0,
                unavailable_reason: (recent_count == 0).then_some("There are no Recent entries to clear."),
                confirmation_required: true,
                restart_required: false,
                item_count: recent_count,
                total_size_bytes: None,
            },
            ResetCategoryDto {
                reset_handle: cache_handle,
                id: "cache",
                label: "Clear app-owned cache",
                description: "Remove currently approved app-owned cache entries that are not in use.",
                consequence: "Later setup runs may download or rebuild files. Saved setups and original user files remain unchanged.",
                affected_scope: "Canonical app-owned cache root only",
                available: cache_removable_count > 0,
                unavailable_reason: (cache_removable_count == 0).then_some("There are no removable cache entries."),
                confirmation_required: true,
                restart_required: false,
                item_count: cache_removable_count,
                total_size_bytes: Some(cache_removable_size),
            },
            ResetCategoryDto {
                reset_handle: recovery_handle,
                id: "recovery",
                label: "Reset recovery data",
                description: "Remove the app-owned recovery draft as a maintenance action.",
                consequence: "The active process marker, current workflow, saved files, portable intent, and review authority remain unchanged.",
                affected_scope: "Recovery draft only",
                available: recovery_available,
                unavailable_reason: (!recovery_available).then_some("There is no recovery draft to reset."),
                confirmation_required: true,
                restart_required: false,
                item_count: usize::from(recovery_available),
                total_size_bytes: None,
            },
        ],
    })
}

#[tauri::command]
pub fn reset_local_app_state(
    request: ResetLocalAppStateRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if !request.confirmed {
        return Err(support_error(
            "reset_confirmation_required",
            "Confirm this reset category before continuing.",
        ));
    }
    let authorization = state
        .support
        .lock()
        .map_err(|_| support_error("support_state_unavailable", "Support state is unavailable."))?
        .take_reset_authorization(&request.reset_handle)?;
    let summary = match authorization {
        ResetAuthorization::Recents {
            revision,
            item_count,
        } => {
            let mut saved = state.saved_configurations.lock().map_err(|_| {
                support_error(
                    "configuration_state_unavailable",
                    "Saved-setup state is unavailable.",
                )
            })?;
            if saved.recent_count() != item_count {
                return Err(support_error(
                    "reset_category_stale",
                    "Recent setups changed. Review the reset category again.",
                ));
            }
            saved.reset_recents(revision)?;
            "Recent setup history was cleared. Saved setup files were preserved."
        }
        ResetAuthorization::Recovery { revision } => {
            state
                .recovery
                .lock()
                .map_err(|_| {
                    support_error(
                        "recovery_state_unavailable",
                        "Recovery state is unavailable.",
                    )
                })?
                .reset_recovery_data(revision)?;
            "Recovery data was reset. The active application session and current workflow were preserved."
        }
        ResetAuthorization::Cache {
            revision,
            item_count,
            total_size_bytes,
        } => {
            if state
                .executions
                .lock()
                .map_err(|_| {
                    support_error(
                        "execution_state_unavailable",
                        "Execution state is unavailable.",
                    )
                })?
                .has_in_flight()
            {
                return Err(support_error(
                    "cache_cleanup_execution_active",
                    "App-owned storage cannot be cleared while an execution is active.",
                ));
            }
            let mut support = state.support.lock().map_err(|_| {
                support_error("support_state_unavailable", "Support state is unavailable.")
            })?;
            if support.generation != revision {
                return Err(support_error(
                    "reset_category_stale",
                    "App-owned storage changed. Review the reset category again.",
                ));
            }
            let selected = support
                .entries
                .values()
                .filter(|entry| entry.removable && !entry.in_use)
                .cloned()
                .collect::<Vec<_>>();
            let selected_size = selected
                .iter()
                .fold(0_u64, |total, entry| total.saturating_add(entry.size_bytes));
            if selected.len() != item_count || selected_size != total_size_bytes {
                return Err(support_error(
                    "reset_category_stale",
                    "App-owned storage changed. Review the reset category again.",
                ));
            }
            let root = canonical_cache_root(&support.cache_root)?;
            let outcomes = selected
                .iter()
                .map(|record| delete_logical_entry(&root, record))
                .collect::<Vec<_>>();
            support.refresh(false)?;
            if outcomes
                .iter()
                .any(|outcome| outcome["outcome"] == "failed")
            {
                "Cache cleanup completed with a partial failure. Review the current inventory before retrying."
            } else {
                "Approved app-owned cache entries were cleared. Saved setups and external files were preserved."
            }
        }
    };
    let snapshot = build_support_snapshot(&state)?;
    Ok(json!({ "outcome": { "summary": summary }, "snapshot": snapshot }))
}

#[tauri::command]
pub fn cleanup_cache(
    request: CacheCleanupRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    // Reserve destructive work before reading execution state so navigation,
    // cleanup, and execution-start paths share one lock order.
    let _activity = state.update_activity.reserve_cleanup()?;
    if state
        .executions
        .lock()
        .map_err(|_| {
            support_error(
                "execution_state_unavailable",
                "Execution state is unavailable.",
            )
        })?
        .has_in_flight()
    {
        return Err(support_error(
            "cache_cleanup_execution_active",
            "Artifact cleanup is unavailable while an execution is starting or active.",
        ));
    }
    let mut store = state
        .support
        .lock()
        .map_err(|_| support_error("support_state_unavailable", "Support state is unavailable."))?;
    if request.inventory_generation != store.generation.to_string() {
        return Err(support_error(
            "cache_inventory_stale",
            "The cache changed. Refresh the inventory and confirm the cleanup again.",
        ));
    }
    let selected = selected_records(&store, &request)?;
    let selected_size = selected
        .iter()
        .fold(0_u64, |total, entry| total.saturating_add(entry.size_bytes));
    if !request.confirmation.confirmed
        || request.confirmation.entry_count != selected.len()
        || request.confirmation.total_size_bytes != selected_size
    {
        return Err(support_error(
            "cache_confirmation_invalid",
            "The cleanup confirmation no longer matches the selected cache entries.",
        ));
    }
    let root = canonical_cache_root(&store.cache_root)?;
    let outcomes = selected
        .iter()
        .map(|record| delete_logical_entry(&root, record))
        .collect::<Vec<_>>();
    let inventory = store.refresh(false)?;
    Ok(json!({ "outcomes": outcomes, "inventory": inventory }))
}

#[tauri::command]
pub async fn export_support_diagnostics(app: AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let snapshot = build_support_snapshot(&state)?;
    let cache_inventory = snapshot
        .cache_inventory
        .clone()
        .unwrap_or_else(|| json!({}));
    let configuration_summary = state
        .saved_configurations
        .lock()
        .map_err(|_| {
            support_error(
                "configuration_state_unavailable",
                "Configuration state is unavailable.",
            )
        })?
        .support_summary();
    let execution_summary = state
        .executions
        .lock()
        .map_err(|_| {
            support_error(
                "execution_state_unavailable",
                "Execution state is unavailable.",
            )
        })?
        .support_summary(&state.sidecar);
    let support_status = snapshot
        .subsystems
        .iter()
        .map(|subsystem| {
            json!({
                "subsystem": subsystem.id,
                "status": subsystem.severity,
                "supportCode": subsystem.support_code,
            })
        })
        .collect::<Vec<_>>();
    let service_status = support_status
        .iter()
        .find(|status| status["subsystem"] == "service")
        .cloned()
        .unwrap_or_else(|| json!({ "subsystem": "service", "status": "failure" }));
    let catalog = state
        .catalog
        .as_ref()
        .ok()
        .and_then(|catalog| serde_json::to_value(catalog.public_identity()).ok())
        .unwrap_or_else(|| json!({ "status": "unavailable" }));
    let cache_summary = cache_inventory
        .get("summary")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let members = vec![
        (
            DIAGNOSTICS_MEMBER_NAMES[0],
            json!({ "schema": "emuchef.support-diagnostics", "schemaVersion": 2 }),
        ),
        (
            DIAGNOSTICS_MEMBER_NAMES[1],
            json!({
                "appVersion": env!("CARGO_PKG_VERSION"),
                "service": service_status,
                "platform": platform_class(),
                "featureGates": { "realExecution": cfg!(feature = "real-execution") },
            }),
        ),
        (DIAGNOSTICS_MEMBER_NAMES[2], catalog),
        (DIAGNOSTICS_MEMBER_NAMES[3], configuration_summary),
        (DIAGNOSTICS_MEMBER_NAMES[4], execution_summary),
        (DIAGNOSTICS_MEMBER_NAMES[5], cache_summary),
        (
            DIAGNOSTICS_MEMBER_NAMES[6],
            json!({ "subsystems": support_status }),
        ),
    ];
    let bytes = build_diagnostics_zip(members)?;
    let _dialog_activity = app
        .state::<AppState>()
        .update_activity
        .reserve_native_dialog()?;
    let picker = app
        .dialog()
        .file()
        .set_file_name("emuchef-support-diagnostics.zip")
        .add_filter("EmuChef support diagnostics", &["zip"]);
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    picker.save_file(move |selection| {
        let _ = sender.try_send(selection);
    });
    let selected: Option<FilePath> = receiver.recv().await.ok_or_else(|| {
        support_error(
            "diagnostics_picker_failed",
            "The diagnostics save dialog could not be opened.",
        )
    })?;
    let Some(path) = diagnostics_destination(selected)? else {
        return Ok(json!({ "outcome": "cancelled" }));
    };
    tauri::async_runtime::spawn_blocking(move || fs::write(path, bytes))
        .await
        .map_err(|_| {
            support_error(
                "diagnostics_write_failed",
                "Support diagnostics could not be saved.",
            )
        })?
        .map_err(|_| {
            support_error(
                "diagnostics_write_failed",
                "Support diagnostics could not be saved.",
            )
        })?;
    Ok(json!({ "outcome": "saved" }))
}

fn diagnostics_destination(selection: Option<FilePath>) -> Result<Option<PathBuf>, String> {
    selection
        .map(|selection| {
            selection.into_path().map_err(|_| {
                support_error(
                    "diagnostics_destination_unavailable",
                    "The selected diagnostics destination is unavailable.",
                )
            })
        })
        .transpose()
}

fn selected_records(
    store: &SupportStore,
    request: &CacheCleanupRequest,
) -> Result<Vec<CacheEntryRecord>, String> {
    let records = match request.mode {
        CacheCleanupMode::Selected => request
            .entry_handles
            .iter()
            .map(|handle| {
                store.entries.get(handle).cloned().ok_or_else(|| {
                    support_error(
                        "cache_entry_invalidated",
                        "A selected cache entry is no longer available.",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        CacheCleanupMode::Unused => store
            .entries
            .values()
            .filter(|entry| entry.removable && !entry.in_use)
            .cloned()
            .collect(),
        CacheCleanupMode::AllRemovable => store
            .entries
            .values()
            .filter(|entry| entry.removable)
            .cloned()
            .collect(),
    };
    Ok(records)
}

fn delete_logical_entry(root: &Path, record: &CacheEntryRecord) -> Value {
    delete_logical_entry_with(root, record, |path| fs::remove_file(path))
}

/// Delete a logical cache entry with an injectable file remover.
///
/// Production uses [`fs::remove_file`]. The indirection keeps the ordering and
/// partial-failure contract directly testable without depending on platform-
/// specific filesystem permission behavior.
fn delete_logical_entry_with<F>(root: &Path, record: &CacheEntryRecord, mut remove_file: F) -> Value
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    if record.in_use {
        return cleanup_outcome(
            record,
            "skipped_in_use",
            "cache_entry_in_use",
            "This cache entry is currently in use.",
        );
    }
    if !record.removable || !matches!(record.category, "artifact" | "partial") {
        return cleanup_outcome(
            record,
            "invalidated",
            "cache_entry_invalidated",
            "This cache entry is no longer approved for removal.",
        );
    }
    if !direct_child(root, &record.payload) {
        return cleanup_outcome(
            record,
            "invalidated",
            "cache_entry_invalidated",
            "This cache entry no longer matches the approved cache root.",
        );
    }
    let current_payload = match fingerprint(&record.payload) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return cleanup_outcome(
                record,
                "already_missing",
                "cache_entry_missing",
                "This cache entry was already absent.",
            )
        }
        Err(_) => {
            return cleanup_outcome(
                record,
                "failed",
                "cache_entry_delete_failed",
                "This cache entry could not be removed.",
            )
        }
    };
    if current_payload != record.payload_fingerprint {
        return cleanup_outcome(
            record,
            "invalidated",
            "cache_entry_invalidated",
            "This cache entry changed after inventory.",
        );
    }
    if let (Some(path), Some(expected)) = (&record.metadata, &record.metadata_fingerprint) {
        if !direct_child(root, path) || fingerprint(path).ok().as_ref() != Some(expected) {
            return cleanup_outcome(
                record,
                "invalidated",
                "cache_entry_invalidated",
                "This cache entry metadata changed after inventory.",
            );
        }
        if remove_file(path).is_err() {
            return cleanup_outcome(
                record,
                "failed",
                "cache_entry_delete_failed",
                "This cache entry could not be removed.",
            );
        }
    }
    if remove_file(&record.payload).is_err() {
        return cleanup_outcome(record, "failed", "cache_entry_partial_failure", "Only part of this cache entry could be removed. Refresh the inventory before trying again.");
    }
    cleanup_outcome(
        record,
        "removed",
        "cache_entry_removed",
        "The cache entry was removed.",
    )
}

fn cleanup_outcome(record: &CacheEntryRecord, outcome: &str, code: &str, message: &str) -> Value {
    let support_code = match code {
        "cache_entry_partial_failure" => Some(SupportCode::CacheCleanupPartial.code()),
        "cache_entry_delete_failed" | "cache_entry_invalidated" => {
            Some(SupportCode::CacheCleanupFailed.code())
        }
        _ => None,
    };
    json!({
        "outcome": outcome,
        "message": message,
        "supportCode": support_code,
        "entryCategory": record.category,
    })
}

fn project_entry(entry: &CacheEntryRecord) -> Value {
    let (category_label, description, deletion_consequence) = match entry.category {
        "partial" => (
            "Incomplete download",
            "A temporary app-owned file from an incomplete transfer.",
            "Removing it makes a later setup restart the download.",
        ),
        _ => (
            "Reusable setup download",
            "An app-owned copy retained so later setup runs can reuse it.",
            "Removing it makes a later setup download or copy the file again.",
        ),
    };
    json!({
        "cacheEntryHandle": entry.handle,
        "category": entry.category,
        "categoryLabel": category_label,
        "description": description,
        "deletionConsequence": deletion_consequence,
        "artifactLabel": entry.label,
        "sourceKind": entry.source_kind,
        "integrityState": entry.integrity,
        "sizeBytes": entry.size_bytes,
        "ageBucket": entry.age_bucket,
        "inUse": entry.in_use,
        "removable": entry.removable,
    })
}

fn canonical_cache_root(root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|_| {
        support_error(
            "cache_root_unavailable",
            "The app-owned artifact cache is unavailable.",
        )
    })?;
    let metadata = fs::symlink_metadata(root).map_err(|_| {
        support_error(
            "cache_root_unavailable",
            "The app-owned artifact cache is unavailable.",
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(support_error(
            "cache_root_invalid",
            "The app-owned artifact cache failed its safety check.",
        ));
    }
    root.canonicalize().map_err(|_| {
        support_error(
            "cache_root_unavailable",
            "The app-owned artifact cache is unavailable.",
        )
    })
}

fn fingerprint(path: &Path) -> std::io::Result<FileFingerprint> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::other("not a regular cache file"));
    }
    Ok(FileFingerprint {
        len: metadata.len(),
        modified_nanos: metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos()),
    })
}

fn direct_child(root: &Path, path: &Path) -> bool {
    path.parent() == Some(root) && path.file_name().is_some()
}

fn metadata_path(payload: &Path) -> PathBuf {
    let file_name = payload
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    payload.with_file_name(format!(".{file_name}{METADATA_SUFFIX}"))
}

fn is_metadata_name(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(METADATA_SUFFIX)
}

fn metadata_payload_name(name: &str) -> Option<&str> {
    name.strip_prefix('.')?.strip_suffix(METADATA_SUFFIX)
}

fn is_canonical_payload_name(name: &str) -> bool {
    name.len() > 65
        && name.as_bytes().get(64) == Some(&b'-')
        && name.as_bytes()[..64].iter().all(u8::is_ascii_hexdigit)
}

fn is_partial_name(name: &str) -> bool {
    name.starts_with(".emuchef-artifact-") && name.ends_with(".partial")
}

fn read_metadata(path: &Path) -> Option<CacheMetadata> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_METADATA_BYTES {
        return None;
    }
    serde_json::from_reader(file).ok()
}

fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '/')
        })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn file_len_if_regular(path: &Path) -> u64 {
    fingerprint(path).map_or(0, |value| value.len)
}

fn age_bucket(fingerprint: &FileFingerprint) -> &'static str {
    let Some(modified) = fingerprint.modified_nanos else {
        return "unknown";
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(modified);
    let days = now.saturating_sub(modified) / 86_400_000_000_000;
    match days {
        0 => "under_1_day",
        1..=7 => "1_to_7_days",
        8..=30 => "8_to_30_days",
        _ => "over_30_days",
    }
}

fn build_diagnostics_zip(members: Vec<(&str, Value)>) -> Result<Vec<u8>, String> {
    let mut output = Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o600);
        let mut uncompressed = 0_usize;
        for (name, value) in members {
            let sanitized = sanitize_value(value);
            let mut bytes = serde_json::to_vec_pretty(&sanitized).map_err(|_| {
                support_error(
                    "diagnostics_build_failed",
                    "Support diagnostics could not be prepared.",
                )
            })?;
            bytes.push(b'\n');
            uncompressed = uncompressed.saturating_add(bytes.len());
            if uncompressed > MAX_DIAGNOSTICS_BYTES {
                return Err(support_error(
                    "diagnostics_size_limit_exceeded",
                    "Support diagnostics exceeded the safe export size limit.",
                ));
            }
            archive.start_file(name, options).map_err(|_| {
                support_error(
                    "diagnostics_build_failed",
                    "Support diagnostics could not be prepared.",
                )
            })?;
            archive.write_all(&bytes).map_err(|_| {
                support_error(
                    "diagnostics_build_failed",
                    "Support diagnostics could not be prepared.",
                )
            })?;
        }
        archive.finish().map_err(|_| {
            support_error(
                "diagnostics_build_failed",
                "Support diagnostics could not be prepared.",
            )
        })?;
    }
    let bytes = output.into_inner();
    if bytes.len() > MAX_DIAGNOSTICS_BYTES {
        return Err(support_error(
            "diagnostics_size_limit_exceeded",
            "Support diagnostics exceeded the safe export size limit.",
        ));
    }
    Ok(bytes)
}

fn sanitize_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, sanitize_value(value)))
                .collect(),
        ),
        Value::String(value) => Value::String(sanitize_string(&value)),
        other => other,
    }
}

fn sanitize_string(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let unsafe_value = value.contains('\n')
        || value.contains('\r')
        || value.contains("/Users/")
        || value.contains("/home/")
        || value.contains("\\Users\\")
        || lower.contains("bearer ")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("token=")
        || looks_like_device_serial(value)
        || (lower.contains("://")
            && (value.contains('@') || value.contains('?') || value.contains('#')));
    if unsafe_value || value.len() > 512 {
        "[redacted]".to_string()
    } else {
        value.to_string()
    }
}

fn looks_like_device_serial(value: &str) -> bool {
    (10..=64).contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        && value.bytes().any(|byte| byte.is_ascii_alphabetic())
        && value.bytes().any(|byte| byte.is_ascii_digit())
        && !(value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn platform_class() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        _ => "other",
    }
}

fn support_error(code: &str, message: &str) -> String {
    json!({ "code": code, "message": message }).to_string()
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    fn store(root: &Path) -> SupportStore {
        SupportStore::new(root.to_path_buf())
    }

    fn create_payload(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(format!("{}-{name}", "a".repeat(64)));
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn inventory_projects_one_logical_entry_and_combines_metadata_size() {
        let temp = tempfile::tempdir().unwrap();
        let payload = create_payload(temp.path(), "artifact.bin", b"payload");
        let metadata = json!({
            "schemaVersion": 1,
            "payloadFileName": payload.file_name().unwrap().to_str().unwrap(),
            "artifactLabel": "recipe/artifact",
            "sourceKind": "https",
            "sourceFingerprint": "a".repeat(64),
            "payloadSizeBytes": 7,
            "payloadModifiedNanos": fingerprint(&payload).unwrap().modified_nanos,
        });
        let metadata_path = metadata_path(&payload);
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let mut store = store(temp.path());
        let inventory = store.refresh(false).unwrap();
        assert_eq!(inventory["summary"]["entryCount"], 1);
        assert_eq!(
            inventory["summary"]["totalSizeBytes"],
            7 + fs::metadata(metadata_path).unwrap().len()
        );
        assert_eq!(inventory["entries"][0]["integrityState"], "complete");
        assert_eq!(inventory["entries"][0]["sourceKind"], "https");
        let projected = inventory.to_string();
        assert!(!projected.contains(&temp.path().display().to_string()));
        assert!(!projected.contains("artifact.bin"));
    }

    #[test]
    fn orphan_metadata_and_unmanaged_files_are_not_selectable_entries() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".missing.emuchef-cache.json"), b"{}").unwrap();
        fs::write(temp.path().join("unmanaged.txt"), b"x").unwrap();
        let mut store = store(temp.path());
        let inventory = store.refresh(false).unwrap();
        assert_eq!(inventory["summary"]["entryCount"], 0);
        assert_eq!(inventory["summary"]["unmanagedCount"], 2);
    }

    #[test]
    fn stale_metadata_never_marks_a_replaced_payload_complete() {
        let temp = tempfile::tempdir().unwrap();
        let payload = create_payload(temp.path(), "artifact.bin", b"new");
        fs::write(
            metadata_path(&payload),
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "payloadFileName": payload.file_name().unwrap().to_str().unwrap(),
                "artifactLabel": "recipe/artifact",
                "sourceKind": "file",
                "sourceFingerprint": "c".repeat(64),
                "payloadSizeBytes": 999,
            }))
            .unwrap(),
        )
        .unwrap();
        let inventory = store(temp.path()).refresh(false).unwrap();
        assert_eq!(
            inventory["entries"][0]["integrityState"],
            "metadata_mismatch"
        );
    }

    #[test]
    fn cleanup_removes_payload_and_metadata_as_one_entry() {
        let temp = tempfile::tempdir().unwrap();
        let payload = create_payload(temp.path(), "artifact.bin", b"payload");
        let sidecar = metadata_path(&payload);
        fs::write(&sidecar, b"{}").unwrap();
        let mut store = store(temp.path());
        store.refresh(false).unwrap();
        let record = store.entries.values().next().unwrap().clone();
        let outcome = delete_logical_entry(&temp.path().canonicalize().unwrap(), &record);
        assert_eq!(outcome["outcome"], "removed");
        assert!(!payload.exists());
        assert!(!sidecar.exists());
    }

    #[test]
    fn cleanup_reports_partial_failure_after_metadata_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let payload = create_payload(temp.path(), "artifact.bin", b"payload");
        let sidecar = metadata_path(&payload);
        fs::write(&sidecar, b"{}").unwrap();
        let mut store = store(temp.path());
        store.refresh(false).unwrap();
        let record = store.entries.values().next().unwrap().clone();
        let failing_payload = record.payload.clone();
        let outcome =
            delete_logical_entry_with(&temp.path().canonicalize().unwrap(), &record, |path| {
                if path == failing_payload {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "injected payload deletion failure",
                    ))
                } else {
                    fs::remove_file(path)
                }
            });
        assert_eq!(outcome["outcome"], "failed");
        assert_eq!(
            outcome["supportCode"],
            SupportCode::CacheCleanupPartial.code()
        );
        assert!(!outcome.to_string().contains("cache_entry_partial_failure"));
        assert!(payload.exists());
        assert!(!sidecar.exists());

        let refreshed = SupportStore::new(temp.path().to_path_buf())
            .refresh(false)
            .unwrap();
        assert_eq!(refreshed["entries"][0]["integrityState"], "unindexed");
    }

    #[test]
    fn cleanup_keeps_payload_when_metadata_deletion_fails() {
        let temp = tempfile::tempdir().unwrap();
        let payload = create_payload(temp.path(), "artifact.bin", b"payload");
        let sidecar = metadata_path(&payload);
        fs::write(&sidecar, b"{}").unwrap();
        let mut store = store(temp.path());
        store.refresh(false).unwrap();
        let record = store.entries.values().next().unwrap().clone();
        let failing_sidecar = record.metadata.clone().unwrap();
        let outcome =
            delete_logical_entry_with(&temp.path().canonicalize().unwrap(), &record, |path| {
                if path == failing_sidecar {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "injected metadata deletion failure",
                    ))
                } else {
                    fs::remove_file(path)
                }
            });
        assert_eq!(outcome["outcome"], "failed");
        assert_eq!(
            outcome["supportCode"],
            SupportCode::CacheCleanupFailed.code()
        );
        assert!(!outcome.to_string().contains("cache_entry_delete_failed"));
        assert!(payload.exists());
        assert!(sidecar.exists());
    }

    #[test]
    fn cleanup_rejects_symlinks_and_stale_mappings() {
        let temp = tempfile::tempdir().unwrap();
        let payload = create_payload(temp.path(), "artifact.bin", b"payload");
        let mut store = store(temp.path());
        store.refresh(false).unwrap();
        let record = store.entries.values().next().unwrap().clone();
        fs::write(&payload, b"changed-size").unwrap();
        let outcome = delete_logical_entry(&temp.path().canonicalize().unwrap(), &record);
        assert_eq!(outcome["outcome"], "invalidated");
        #[cfg(unix)]
        {
            fs::remove_file(&payload).unwrap();
            std::os::unix::fs::symlink("outside", &payload).unwrap();
            assert!(fingerprint(&payload).is_err());
        }
    }

    #[test]
    fn cleanup_revalidates_removable_status_and_category_before_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let payload = create_payload(temp.path(), "artifact.bin", b"payload");
        let mut support = store(temp.path());
        support.refresh(false).unwrap();
        let mut record = support.entries.values().next().unwrap().clone();
        record.removable = false;
        let protected = delete_logical_entry(&temp.path().canonicalize().unwrap(), &record);
        assert_eq!(protected["outcome"], "invalidated");
        assert!(payload.exists());

        record.removable = true;
        record.category = "unexpected";
        let wrong_category = delete_logical_entry(&temp.path().canonicalize().unwrap(), &record);
        assert_eq!(wrong_category["outcome"], "invalidated");
        assert!(payload.exists());
    }

    #[test]
    fn active_use_blocks_both_logical_entry_components() {
        let temp = tempfile::tempdir().unwrap();
        let payload = create_payload(temp.path(), "artifact.bin", b"payload");
        let sidecar = metadata_path(&payload);
        fs::write(&sidecar, b"{}").unwrap();
        let mut store = store(temp.path());
        let inventory = store.refresh(true).unwrap();
        assert_eq!(inventory["summary"]["inUseCount"], 1);
        let record = store.entries.values().next().unwrap().clone();
        let outcome = delete_logical_entry(&temp.path().canonicalize().unwrap(), &record);
        assert_eq!(outcome["outcome"], "skipped_in_use");
        assert!(payload.exists());
        assert!(sidecar.exists());
    }

    #[test]
    fn cleanup_request_rejects_unknown_fields() {
        let request = json!({
            "mode": "selected",
            "inventoryGeneration": "1",
            "entryHandles": [],
            "confirmation": {
                "confirmed": true,
                "entryCount": 0,
                "totalSizeBytes": 0,
            },
            "cacheRoot": "/untrusted",
        });
        assert!(serde_json::from_value::<CacheCleanupRequest>(request).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cache_root_rejects_symlink_indirection() {
        let temp = tempfile::tempdir().unwrap();
        let actual = temp.path().join("actual");
        fs::create_dir(&actual).unwrap();
        let linked = temp.path().join("linked");
        std::os::unix::fs::symlink(&actual, &linked).unwrap();
        assert!(canonical_cache_root(&linked).is_err());
    }

    #[test]
    fn diagnostics_redactor_blocks_adversarial_values() {
        for value in [
            "/Users/alice/private/file",
            "/home/alice/private",
            r"C:\\Users\\alice\\secret",
            "https://user:pass@example.invalid/a?token=secret",
            "Bearer abc.def.ghi",
            "api_key=secret",
            "R58M12345AB",
            "serial\nraw adb output",
        ] {
            assert_eq!(sanitize_string(value), "[redacted]");
        }
    }

    #[test]
    fn diagnostics_zip_is_bounded_and_contains_only_fixed_members() {
        let bytes = build_diagnostics_zip(
            DIAGNOSTICS_MEMBER_NAMES
                .iter()
                .map(|name| (*name, json!({ "status": "ready" })))
                .collect(),
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), DIAGNOSTICS_MEMBER_NAMES.len());
        for (index, expected) in DIAGNOSTICS_MEMBER_NAMES.iter().enumerate() {
            assert_eq!(archive.by_index(index).unwrap().name(), *expected);
        }
    }

    #[test]
    fn diagnostics_archive_redacts_hostile_values_and_excludes_ui_authority() {
        let hostile = "/Users/alice/private R58M12345AB token=secret";
        let bytes = build_diagnostics_zip(vec![(
            DIAGNOSTICS_MEMBER_NAMES[6],
            json!({
                "subsystems": [{ "subsystem": "service", "status": "failure", "supportCode": SupportCode::ServiceStartFailed.code() }],
                "summary": hostile,
            }),
        )])
        .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut contents = String::new();
        archive
            .by_index(0)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert!(!contents.contains(hostile));
        assert!(!contents.contains("R58M12345AB"));
        assert!(!contents.contains("token=secret"));
        assert!(contents.contains(SupportCode::ServiceStartFailed.code()));
        for forbidden in [
            "actionHandle",
            "resetHandle",
            "presentationRevision",
            "modalOutcome",
            "exportOutcome",
            "rawError",
        ] {
            assert!(!contents.contains(forbidden));
        }
    }

    #[test]
    fn healthy_and_neutral_statuses_serialize_without_failure_codes() {
        for severity in [SupportSeverity::Healthy, SupportSeverity::Neutral] {
            let status = SubsystemStatusDto {
                id: "optional",
                label: "Optional service",
                severity,
                summary: "This optional service is not configured.".to_string(),
                consequence: "Local use is unaffected.".to_string(),
                support_code: None,
                actions: Vec::new(),
            };
            let projected = serde_json::to_value(status).unwrap();
            assert!(projected["supportCode"].is_null());
            assert!(!projected.to_string().contains("EMUCHEF-"));
        }
    }

    #[test]
    fn diagnostics_zip_rejects_oversized_allowlisted_content() {
        let values = vec![Value::String("a".repeat(512)); 4_097];
        let error =
            build_diagnostics_zip(vec![("oversized.json", Value::Array(values))]).unwrap_err();
        assert!(error.contains("diagnostics_size_limit_exceeded"));
    }

    #[test]
    fn native_dialog_cancellation_has_no_destination_projection() {
        assert_eq!(diagnostics_destination(None).unwrap(), None);
    }
}
