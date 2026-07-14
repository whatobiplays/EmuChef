//! Trusted support diagnostics and app-owned artifact-cache management.
//!
//! React receives only aggregate diagnostics and opaque, generation-scoped
//! logical-entry handles. Paths, filenames, metadata identities, raw bundle
//! bytes, and deletion authority remain inside this module.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::commands::AppState;

const MAX_CACHE_ENTRIES: usize = 4_096;
const MAX_METADATA_BYTES: u64 = 16 * 1_024;
const MAX_DIAGNOSTICS_BYTES: usize = 2 * 1_024 * 1_024;
const METADATA_SUFFIX: &str = ".emuchef-cache.json";

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
    entries: HashMap<String, CacheEntryRecord>,
}

impl SupportStore {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            generation: 0,
            entries: HashMap::new(),
        }
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
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
        json!({
            "generation": self.generation.to_string(),
            "entries": entries.into_iter().map(project_entry).collect::<Vec<_>>(),
            "summary": {
                "entryCount": self.entries.len(),
                "totalSizeBytes": total_size,
                "inUseCount": self.entries.values().filter(|entry| entry.in_use).count(),
                "unmanagedCount": unmanaged_count,
                "unmanagedSizeBytes": unmanaged_size,
            },
        })
    }
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
pub fn cleanup_cache(
    request: CacheCleanupRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
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
    let cache_inventory = state
        .support
        .lock()
        .map_err(|_| support_error("support_state_unavailable", "Support state is unavailable."))?
        .refresh(in_flight)?;
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
    let runtime = state.sidecar.diagnostics();
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
            "manifest.json",
            json!({ "schema": "emuchef.support-diagnostics", "schemaVersion": 1 }),
        ),
        (
            "runtime.json",
            json!({
                "appVersion": env!("CARGO_PKG_VERSION"),
                "runtime": runtime,
                "platform": platform_class(),
                "featureGates": { "realExecution": cfg!(feature = "real-execution") },
            }),
        ),
        ("catalog.json", catalog),
        ("configuration-summary.json", configuration_summary),
        ("execution-summaries.json", execution_summary),
        ("cache-summary.json", cache_summary),
    ];
    let bytes = build_diagnostics_zip(members)?;
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
    json!({ "entryHandle": record.handle, "outcome": outcome, "code": code, "message": message })
}

fn project_entry(entry: &CacheEntryRecord) -> Value {
    json!({
        "cacheEntryHandle": entry.handle,
        "category": entry.category,
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
        assert_eq!(outcome["code"], "cache_entry_partial_failure");
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
        assert_eq!(outcome["code"], "cache_entry_delete_failed");
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
        let bytes = build_diagnostics_zip(vec![
            ("manifest.json", json!({ "schemaVersion": 1 })),
            ("runtime.json", json!({ "status": "ready" })),
        ])
        .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 2);
        assert_eq!(archive.by_index(0).unwrap().name(), "manifest.json");
        assert_eq!(archive.by_index(1).unwrap().name(), "runtime.json");
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
