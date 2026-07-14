//! Crash-safe persistence for portable configuration intent.
//!
//! The recovery file is fixed beneath application data and contains no device,
//! plan, execution, dialog, or sidecar authority. Binding sensitivity comes
//! only from trusted authored input metadata observed in sidecar descriptions.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::State;
use uuid::Uuid;

use crate::commands::{safe_error, AppState};
use crate::saved_configurations::{
    recovery_source_identity, restore_recovery_source, RecoveryPortableIntent,
};

const RECOVERY_SCHEMA_VERSION: u64 = 1;
const MAX_RECOVERY_BYTES: u64 = 64 * 1024;
const MAX_RECIPES: usize = 256;
const MAX_BINDINGS: usize = 512;
const MAX_STRING_CHARS: usize = 4096;
const MAX_VALUE_DEPTH: usize = 8;
const MAX_ARRAY_ITEMS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryRecord {
    schema_version: u64,
    generation: u64,
    saved_at_epoch_ms: u64,
    dirty: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_configuration_id: Option<String>,
    device_plan: String,
    selected_recipes: Vec<String>,
    bindings: Map<String, Value>,
    omitted_bindings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DraftDisposition {
    Deferred,
    Restored,
    CurrentSession,
}

/// Process-local coordination around one restart-stable recovery record.
pub struct RecoveryStore {
    path: PathBuf,
    marker_path: PathBuf,
    record: Option<RecoveryRecord>,
    load_notice: Option<&'static str>,
    session_generation: u64,
    latest_request_generation: u64,
    latest_draft_generation: u64,
    latest_record_generation: u64,
    disposition: Option<DraftDisposition>,
    sensitivity: HashMap<String, bool>,
    required_reentry: HashSet<String>,
}

impl RecoveryStore {
    pub fn load(path: PathBuf, marker_path: PathBuf) -> Self {
        let (record, load_notice) = load_record(&path);
        let latest_record_generation = record.as_ref().map_or(0, |record| record.generation);
        Self {
            path,
            marker_path,
            record,
            load_notice,
            session_generation: 0,
            latest_request_generation: 0,
            latest_draft_generation: 0,
            latest_record_generation,
            disposition: None,
            sensitivity: HashMap::new(),
            required_reentry: HashSet::new(),
        }
    }

    pub fn begin_session(&mut self) -> Result<Value, String> {
        let interrupted_session = self.marker_path.is_file();
        atomic_write(&self.marker_path, b"1", "recovery_session_marker_failed")?;
        self.session_generation = self.session_generation.saturating_add(1).max(1);
        self.latest_request_generation = 0;
        self.latest_draft_generation = 0;
        self.disposition = None;
        self.sensitivity.clear();
        self.required_reentry.clear();
        let recovery = if let Some(record) = &self.record {
            json!({
                "state": "available",
                "draftGeneration": record.generation,
                "displayName": record.display_name,
                "savedAtEpochMs": record.saved_at_epoch_ms,
                "sourceSavedConfiguration": record.source_configuration_id.is_some(),
            })
        } else if let Some(reason) = self.load_notice.take() {
            json!({ "state": "invalid_removed", "reason": reason })
        } else {
            json!({ "state": "none" })
        };
        Ok(json!({
            "sessionGeneration": self.session_generation,
            "interruptedSession": interrupted_session,
            "recovery": recovery,
        }))
    }

    pub fn record_schema(&mut self, description: &Value) {
        for input in description
            .get("inputs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(key), Some(sensitive)) = (
                input.get("key").and_then(Value::as_str),
                input.get("sensitive").and_then(Value::as_bool),
            ) {
                self.sensitivity.insert(key.to_string(), sensitive);
            }
        }
    }

    pub fn required_reentry(&self) -> Vec<String> {
        let mut keys = self.required_reentry.iter().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    pub fn note_current_binding_keys(&mut self, bindings: &HashSet<String>) {
        self.required_reentry.retain(|key| !bindings.contains(key));
    }

    fn require_session(&self, generation: u64) -> Result<(), String> {
        if generation != self.session_generation {
            return Err(safe_error(
                "recovery_session_stale",
                "The recovery request belongs to an older application session.",
            ));
        }
        Ok(())
    }

    fn stage(
        &mut self,
        request: StageRecoveryDraftRequest,
        source: Option<(String, String)>,
    ) -> Result<Value, String> {
        self.require_session(request.session_generation)?;
        if request.request_generation <= self.latest_request_generation
            || request.draft_generation <= self.latest_draft_generation
        {
            return Err(safe_error(
                "recovery_generation_stale",
                "A newer recovery draft request has already been accepted.",
            ));
        }
        validate_identifier(&request.device_plan, "device plan")?;
        if request.selected_recipes.len() > MAX_RECIPES {
            return Err(invalid_draft());
        }
        for recipe in &request.selected_recipes {
            validate_identifier(recipe, "recipe")?;
        }
        if request.bindings.len() > MAX_BINDINGS {
            return Err(invalid_draft());
        }
        self.note_current_binding_keys(&request.bindings.keys().cloned().collect());

        let mut bindings = Map::new();
        let mut omitted = Vec::new();
        let mut entries = request.bindings.into_iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        for (key, value) in entries {
            validate_binding_key(&key)?;
            validate_value(&value, 0)?;
            // Unknown schema metadata is conservative: retain only the key as a
            // missing-value marker, never the unclassified value.
            if self.sensitivity.get(&key).copied() == Some(false) {
                bindings.insert(key, value);
            } else {
                omitted.push(key);
            }
        }
        omitted.sort();
        omitted.dedup();

        let next_generation = self.latest_record_generation.saturating_add(1).max(1);
        let (source_configuration_id, source_name) = source
            .map(|(id, name)| (Some(id), Some(name)))
            .unwrap_or((None, None));
        let display_name = source_name.or_else(|| safe_optional_name(request.display_name));
        let record = RecoveryRecord {
            schema_version: RECOVERY_SCHEMA_VERSION,
            generation: next_generation,
            saved_at_epoch_ms: now_epoch_ms(),
            dirty: true,
            display_name,
            source_configuration_id,
            device_plan: request.device_plan,
            selected_recipes: request.selected_recipes,
            bindings,
            omitted_bindings: omitted,
        };
        validate_record(&record)?;
        persist_record(&self.path, &record)?;
        self.record = Some(record);
        self.latest_request_generation = request.request_generation;
        self.latest_draft_generation = request.draft_generation;
        self.latest_record_generation = next_generation;
        self.disposition = Some(DraftDisposition::CurrentSession);
        Ok(json!({
            "requestGeneration": request.request_generation,
            "draftGeneration": request.draft_generation,
            "recordGeneration": next_generation,
        }))
    }

    fn defer(&mut self, request: RecoveryRecordRequest) -> Result<(), String> {
        self.require_session(request.session_generation)?;
        self.require_record_generation(request.record_generation)?;
        self.disposition = Some(DraftDisposition::Deferred);
        Ok(())
    }

    fn prepare_restore(
        &mut self,
        request: RecoveryRecordRequest,
    ) -> Result<RecoveryRecord, String> {
        self.require_session(request.session_generation)?;
        self.require_record_generation(request.record_generation)?;
        let record = self.record.clone().ok_or_else(recovery_unavailable)?;
        self.required_reentry = record.omitted_bindings.iter().cloned().collect();
        self.disposition = Some(DraftDisposition::Restored);
        Ok(record)
    }

    fn require_record_generation(&self, generation: u64) -> Result<(), String> {
        if self.record.as_ref().map(|record| record.generation) != Some(generation) {
            return Err(safe_error(
                "recovery_generation_conflict",
                "The recovery draft changed before this action completed.",
            ));
        }
        Ok(())
    }

    fn clear(&mut self, request: RecoveryRecordRequest) -> Result<(), String> {
        self.require_session(request.session_generation)?;
        self.require_record_generation(request.record_generation)?;
        self.clear_unchecked()
    }

    pub fn clear_after_save(&mut self) -> Result<(), String> {
        self.clear_unchecked()
    }

    fn clear_unchecked(&mut self) -> Result<(), String> {
        remove_if_present(&self.path, "recovery_clear_failed")?;
        self.record = None;
        self.disposition = None;
        self.required_reentry.clear();
        Ok(())
    }

    fn finish(&mut self, request: FinishAppSessionRequest) -> Result<(), String> {
        self.require_session(request.session_generation)?;
        if !request.current_session_dirty
            && self.disposition == Some(DraftDisposition::CurrentSession)
        {
            self.clear_unchecked()?;
        }
        // Deferred and restored generations survive clean shutdown until an
        // explicit discard/save or a newer valid dirty draft supersedes them.
        remove_if_present(&self.marker_path, "recovery_session_marker_failed")
    }
}

pub type RecoveryState = Mutex<RecoveryStore>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageRecoveryDraftRequest {
    session_generation: u64,
    request_generation: u64,
    draft_generation: u64,
    display_name: Option<String>,
    source_configuration_handle: Option<String>,
    device_plan: String,
    selected_recipes: Vec<String>,
    bindings: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryRecordRequest {
    session_generation: u64,
    record_generation: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreRecoveryDraftRequest {
    session_generation: u64,
    record_generation: u64,
    request_generation: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinishAppSessionRequest {
    session_generation: u64,
    current_session_dirty: bool,
}

#[tauri::command]
pub fn stage_recovery_draft(
    request: StageRecoveryDraftRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let source = request
        .source_configuration_handle
        .as_deref()
        .map(|handle| recovery_source_identity(&state, handle))
        .transpose()?;
    state
        .recovery
        .lock()
        .map_err(|_| recovery_state_error())?
        .stage(request, source)
}

#[tauri::command]
pub fn defer_recovery_draft(
    request: RecoveryRecordRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .recovery
        .lock()
        .map_err(|_| recovery_state_error())?
        .defer(request)
}

#[tauri::command]
pub fn restore_recovery_draft(
    request: RestoreRecoveryDraftRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let record = state
        .recovery
        .lock()
        .map_err(|_| recovery_state_error())?
        .prepare_restore(RecoveryRecordRequest {
            session_generation: request.session_generation,
            record_generation: request.record_generation,
        })?;
    let intent = RecoveryPortableIntent {
        device_plan: record.device_plan.clone(),
        selected_recipes: record.selected_recipes.clone(),
        bindings: record.bindings.clone(),
        omitted_bindings: record.omitted_bindings.clone(),
    };
    let mut document = record
        .source_configuration_id
        .as_deref()
        .map(|identity| restore_recovery_source(&state, identity, &intent))
        .transpose()?
        .flatten();
    if !record.omitted_bindings.is_empty() {
        if let Some(document) = document.as_mut() {
            if let Some(validation) = document.get_mut("validation") {
                validation["state"] = json!("requires_attention");
                if let Some(diagnostics) = validation
                    .get_mut("diagnostics")
                    .and_then(Value::as_array_mut)
                {
                    diagnostics.extend(record.omitted_bindings.iter().map(|key| {
                    json!({
                        "key": key,
                        "code": "recovery_sensitive_input_required",
                        "message": "A sensitive value was omitted from recovery and must be re-entered.",
                        "severity": "error",
                    })
                }));
                }
            }
        }
    }
    Ok(json!({
        "requestGeneration": request.request_generation,
        "draftGeneration": record.generation,
        "displayName": record.display_name,
        "sourceStatus": if document.is_some() { "available" } else if record.source_configuration_id.is_some() { "missing" } else { "unsaved" },
        "document": document,
        "intent": {
            "devicePlan": record.device_plan,
            "selectedRecipes": record.selected_recipes,
            "bindings": record.bindings,
            "requiredReentryBindings": record.omitted_bindings,
        },
    }))
}

#[tauri::command]
pub fn discard_recovery_draft(
    request: RecoveryRecordRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .recovery
        .lock()
        .map_err(|_| recovery_state_error())?
        .clear(request)
}

#[tauri::command]
pub fn finish_app_session(
    request: FinishAppSessionRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .recovery
        .lock()
        .map_err(|_| recovery_state_error())?
        .finish(request)
}

pub fn clear_recovery_after_save(state: &AppState) -> Result<(), String> {
    state
        .recovery
        .lock()
        .map_err(|_| recovery_state_error())?
        .clear_after_save()
}

fn load_record(path: &Path) -> (Option<RecoveryRecord>, Option<&'static str>) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return (None, None);
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        let _ = fs::remove_file(path);
        return (None, Some("privacy_invalid"));
    }
    if metadata.len() > MAX_RECOVERY_BYTES {
        let _ = fs::remove_file(path);
        return (None, Some("oversized"));
    }
    let Ok(bytes) = fs::read(path) else {
        let _ = fs::remove_file(path);
        return (None, Some("invalid"));
    };
    if serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.get("schemaVersion").and_then(Value::as_u64))
        .is_some_and(|version| version != RECOVERY_SCHEMA_VERSION)
    {
        let _ = fs::remove_file(path);
        return (None, Some("unsupported_version"));
    }
    let result = serde_json::from_slice::<RecoveryRecord>(&bytes).ok();
    match result {
        Some(record) if validate_record(&record).is_ok() => (Some(record), None),
        _ => {
            let _ = fs::remove_file(path);
            (None, Some("invalid"))
        }
    }
}

fn validate_record(record: &RecoveryRecord) -> Result<(), String> {
    if record.schema_version != RECOVERY_SCHEMA_VERSION
        || !record.dirty
        || record.generation == 0
        || record.saved_at_epoch_ms == 0
        || record.selected_recipes.len() > MAX_RECIPES
        || record.bindings.len() > MAX_BINDINGS
        || record.omitted_bindings.len() > MAX_BINDINGS
    {
        return Err(invalid_draft());
    }
    validate_identifier(&record.device_plan, "device plan")?;
    for recipe in &record.selected_recipes {
        validate_identifier(recipe, "recipe")?;
    }
    if let Some(name) = &record.display_name {
        if name.trim().is_empty() || name.chars().count() > 120 {
            return Err(invalid_draft());
        }
    }
    if let Some(identity) = &record.source_configuration_id {
        validate_identifier(identity, "configuration identity")?;
    }
    for (key, value) in &record.bindings {
        validate_binding_key(key)?;
        validate_value(value, 0)?;
    }
    let mut omitted = HashSet::new();
    for key in &record.omitted_bindings {
        validate_binding_key(key)?;
        if record.bindings.contains_key(key) || !omitted.insert(key) {
            return Err(invalid_draft());
        }
    }
    let bytes = serde_json::to_vec(record).map_err(|_| invalid_draft())?;
    if bytes.len() as u64 > MAX_RECOVERY_BYTES {
        return Err(invalid_draft());
    }
    Ok(())
}

fn validate_value(value: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_VALUE_DEPTH {
        return Err(invalid_draft());
    }
    match value {
        Value::String(value) if value.chars().count() > MAX_STRING_CHARS => Err(invalid_draft()),
        Value::Array(values) if values.len() > MAX_ARRAY_ITEMS => Err(invalid_draft()),
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| validate_value(value, depth + 1)),
        Value::Object(values) if values.len() > MAX_ARRAY_ITEMS => Err(invalid_draft()),
        Value::Object(values) => values.iter().try_for_each(|(key, value)| {
            if key.chars().count() > 256 {
                return Err(invalid_draft());
            }
            validate_value(value, depth + 1)
        }),
        _ => Ok(()),
    }
}

fn validate_binding_key(value: &str) -> Result<(), String> {
    let Some((recipe, input)) = value.split_once('/') else {
        return Err(invalid_draft());
    };
    if input.contains('/') {
        return Err(invalid_draft());
    }
    validate_identifier(recipe, "binding recipe")?;
    validate_identifier(input, "binding input")
}

fn validate_identifier(value: &str, _label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.chars().count() > 160
        || !value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(invalid_draft());
    }
    Ok(())
}

fn safe_optional_name(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.chars().take(120).collect())
    })
}

fn persist_record(path: &Path, record: &RecoveryRecord) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(record).map_err(|_| invalid_draft())?;
    atomic_write(path, &bytes, "recovery_write_failed")
}

fn atomic_write(path: &Path, bytes: &[u8], code: &'static str) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| recovery_io_error(code))?;
    fs::create_dir_all(parent).map_err(|_| recovery_io_error(code))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("recovery"),
        Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| recovery_io_error(code))?;
        file.write_all(bytes).map_err(|_| recovery_io_error(code))?;
        file.sync_all().map_err(|_| recovery_io_error(code))?;
        fs::rename(&temporary, path).map_err(|_| recovery_io_error(code))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|_| recovery_io_error(code))?;
        }
        let _ = File::open(parent).and_then(|directory| directory.sync_all());
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn remove_if_present(path: &Path, code: &'static str) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(recovery_io_error(code)),
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn invalid_draft() -> String {
    safe_error(
        "recovery_draft_invalid",
        "The portable recovery draft is invalid and was not saved.",
    )
}

fn recovery_unavailable() -> String {
    safe_error(
        "recovery_unavailable",
        "The recovery draft is no longer available.",
    )
}

fn recovery_state_error() -> String {
    safe_error(
        "recovery_state_unavailable",
        "Recovery state is temporarily unavailable.",
    )
}

fn recovery_io_error(code: &'static str) -> String {
    safe_error(code, "Recovery data could not be updated safely.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn request(bindings: Map<String, Value>) -> StageRecoveryDraftRequest {
        StageRecoveryDraftRequest {
            session_generation: 1,
            request_generation: 1,
            draft_generation: 1,
            display_name: None,
            source_configuration_handle: None,
            device_plan: "plan.one".to_string(),
            selected_recipes: vec!["recipe.one".to_string()],
            bindings,
        }
    }

    fn store() -> (tempfile::TempDir, RecoveryStore) {
        let temp = tempdir().unwrap();
        let mut store = RecoveryStore::load(
            temp.path().join("recovery.json"),
            temp.path().join("active"),
        );
        store.begin_session().unwrap();
        (temp, store)
    }

    #[test]
    fn schema_metadata_alone_controls_binding_persistence() {
        let (_temp, mut store) = store();
        store.record_schema(&json!({ "inputs": [
            { "key": "recipe.one/password", "sensitive": false },
            { "key": "recipe.one/neutral", "sensitive": true }
        ] }));
        let mut bindings = Map::new();
        bindings.insert("recipe.one/password".to_string(), json!("public-value"));
        bindings.insert("recipe.one/neutral".to_string(), json!("secret-value"));
        store.stage(request(bindings), None).unwrap();
        let record = store.record.as_ref().unwrap();
        assert_eq!(record.bindings["recipe.one/password"], "public-value");
        assert!(!record.bindings.contains_key("recipe.one/neutral"));
        assert_eq!(record.omitted_bindings, ["recipe.one/neutral"]);
        assert!(!fs::read_to_string(&store.path)
            .unwrap()
            .contains("secret-value"));
    }

    #[test]
    fn unknown_schema_is_omitted_conservatively_without_name_heuristics() {
        let (_temp, mut store) = store();
        let mut bindings = Map::new();
        bindings.insert("recipe.one/apiKey".to_string(), json!("value"));
        store.stage(request(bindings), None).unwrap();
        assert!(store.record.as_ref().unwrap().bindings.is_empty());
        assert_eq!(
            store.record.as_ref().unwrap().omitted_bindings,
            ["recipe.one/apiKey"]
        );
    }

    #[test]
    fn restored_omissions_require_reentry_until_a_current_value_is_supplied() {
        let (_temp, mut store) = store();
        store.record_schema(&json!({ "inputs": [
            { "key": "recipe.one/neutral", "sensitive": true }
        ] }));
        let mut bindings = Map::new();
        bindings.insert("recipe.one/neutral".to_string(), json!("secret-value"));
        store.stage(request(bindings), None).unwrap();
        let generation = store.record.as_ref().unwrap().generation;
        store
            .prepare_restore(RecoveryRecordRequest {
                session_generation: 1,
                record_generation: generation,
            })
            .unwrap();
        assert_eq!(store.required_reentry(), ["recipe.one/neutral"]);

        let mut supplied = Map::new();
        supplied.insert("recipe.one/neutral".to_string(), json!("new-value"));
        store.note_current_binding_keys(&supplied.keys().cloned().collect());
        assert!(store.required_reentry().is_empty());
    }

    #[test]
    fn deferred_draft_survives_clean_finish_and_next_launch() {
        let (temp, mut store) = store();
        store.record_schema(&json!({ "inputs": [] }));
        store.stage(request(Map::new()), None).unwrap();
        let generation = store.record.as_ref().unwrap().generation;
        store
            .defer(RecoveryRecordRequest {
                session_generation: 1,
                record_generation: generation,
            })
            .unwrap();
        store
            .finish(FinishAppSessionRequest {
                session_generation: 1,
                current_session_dirty: false,
            })
            .unwrap();
        let mut reloaded = RecoveryStore::load(
            temp.path().join("recovery.json"),
            temp.path().join("active"),
        );
        let status = reloaded.begin_session().unwrap();
        assert_eq!(status["recovery"]["state"], "available");
    }

    #[test]
    fn dirty_draft_and_interruption_marker_survive_process_loss() {
        let (temp, mut store) = store();
        store.stage(request(Map::new()), None).unwrap();
        drop(store);

        let mut reloaded = RecoveryStore::load(
            temp.path().join("recovery.json"),
            temp.path().join("active"),
        );
        let status = reloaded.begin_session().unwrap();
        assert_eq!(status["interruptedSession"], true);
        assert_eq!(status["recovery"]["state"], "available");
    }

    #[test]
    fn newer_dirty_intent_supersedes_deferred_and_stale_generation_cannot_resurrect_it() {
        let (_temp, mut store) = store();
        store.stage(request(Map::new()), None).unwrap();
        let generation = store.record.as_ref().unwrap().generation;
        store
            .defer(RecoveryRecordRequest {
                session_generation: 1,
                record_generation: generation,
            })
            .unwrap();
        let mut newer = request(Map::new());
        newer.request_generation = 2;
        newer.draft_generation = 2;
        newer.device_plan = "plan.two".to_string();
        store.stage(newer, None).unwrap();
        assert_eq!(store.record.as_ref().unwrap().device_plan, "plan.two");
        assert_eq!(store.disposition, Some(DraftDisposition::CurrentSession));
        assert!(store.stage(request(Map::new()), None).is_err());
    }

    #[test]
    fn discard_removes_deferred_record() {
        let (_temp, mut store) = store();
        store.stage(request(Map::new()), None).unwrap();
        let generation = store.record.as_ref().unwrap().generation;
        store
            .defer(RecoveryRecordRequest {
                session_generation: 1,
                record_generation: generation,
            })
            .unwrap();
        store
            .clear(RecoveryRecordRequest {
                session_generation: 1,
                record_generation: generation,
            })
            .unwrap();
        assert!(store.record.is_none());
        assert!(!store.path.exists());

        let mut replacement = request(Map::new());
        replacement.request_generation = 2;
        replacement.draft_generation = 2;
        replacement.device_plan = "plan.two".to_string();
        store.stage(replacement, None).unwrap();
        assert_eq!(store.record.as_ref().unwrap().generation, generation + 1);
        assert_eq!(store.record.as_ref().unwrap().device_plan, "plan.two");
        assert!(store.stage(request(Map::new()), None).is_err());
    }

    #[test]
    fn successful_save_clears_a_restored_generation() {
        let (_temp, mut store) = store();
        store.stage(request(Map::new()), None).unwrap();
        let generation = store.record.as_ref().unwrap().generation;
        store
            .prepare_restore(RecoveryRecordRequest {
                session_generation: 1,
                record_generation: generation,
            })
            .unwrap();
        store.clear_after_save().unwrap();
        assert!(store.record.is_none());
        assert!(!store.path.exists());
    }

    #[test]
    fn corrupt_oversized_and_unsupported_records_fail_closed() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("recovery.json");
        fs::write(&path, b"not json").unwrap();
        assert_eq!(
            RecoveryStore::load(path.clone(), temp.path().join("m")).load_notice,
            Some("invalid")
        );
        fs::write(&path, vec![b'x'; MAX_RECOVERY_BYTES as usize + 1]).unwrap();
        assert_eq!(
            RecoveryStore::load(path.clone(), temp.path().join("m")).load_notice,
            Some("oversized")
        );
        fs::write(&path, br#"{"schemaVersion":99}"#).unwrap();
        let unsupported = RecoveryStore::load(path, temp.path().join("m"));
        assert!(unsupported.record.is_none());
        assert_eq!(unsupported.load_notice, Some("unsupported_version"));
    }

    #[cfg(unix)]
    #[test]
    fn persisted_recovery_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (_temp, mut store) = store();
        store.stage(request(Map::new()), None).unwrap();
        assert_eq!(
            fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn strict_frontend_dto_rejects_authority_fields() {
        assert!(serde_json::from_value::<StageRecoveryDraftRequest>(json!({
            "sessionGeneration": 1,
            "requestGeneration": 1,
            "draftGeneration": 1,
            "displayName": null,
            "sourceConfigurationHandle": null,
            "devicePlan": "plan.one",
            "selectedRecipes": [],
            "bindings": {},
            "executionHandle": "forbidden"
        }))
        .is_err());
    }
}
