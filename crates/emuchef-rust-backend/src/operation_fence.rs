use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const OPERATION_FENCE_VERSION: u8 = 1;
const MAX_OPERATION_FENCE_BYTES: usize = 16 * 1024;

/// Durable identity and phase for one device execution attempt.
///
/// The plan digest binds the record to the frozen plan. The attempt fields distinguish
/// retries that use the same plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationFenceRecord {
    pub version: u8,
    pub operation_id: String,
    pub runtime_generation: u64,
    pub attempt_sequence: u64,
    pub device_handle: String,
    pub plan_digest: String,
    pub phase: OperationFencePhase,
    #[serde(default)]
    pub outcome: Option<OperationFenceOutcome>,
    /// True after host-side helper and staging cleanup completes.
    #[serde(default)]
    pub host_cleanup_complete: bool,
    #[serde(default)]
    pub target: Option<OperationFenceTarget>,
    #[serde(default)]
    pub backup: Option<OperationFenceBackup>,
}

/// Reviewed RetroArch file identity retained for post-restart inspection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationFenceTarget {
    pub operation: String,
    pub app_id: String,
    pub target_id: String,
    pub package_id: String,
    pub android_user_id: u32,
    pub target_path: String,
    pub source_sha256: String,
    pub original_file_precondition: serde_json::Value,
    pub backup_mode: String,
}

/// Verified private-backup details retained with an operation for recovery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationFenceBackup {
    pub path: String,
    pub metadata_path: String,
    pub operation: String,
    pub device_handle: String,
    pub package_id: String,
    pub target_path: String,
    pub captured_at_unix_ms: u64,
    pub sha256: String,
    pub length: u64,
}

/// Phase written to the fence while a real device operation is in progress.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationFencePhase {
    Dispatching,
    CommitPending,
    RemoteQuiescent,
}

/// Terminal result recorded only after the remote helper is quiescent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationFenceOutcome {
    NotDispatched,
    VerifiedWrite,
    AbortedBeforeWrite,
    PartialWriteTerminal,
}

/// File location and identity sent from Tauri to the backend for one attempt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationFenceBinding {
    pub path: PathBuf,
    pub record: OperationFenceRecord,
}

impl OperationFenceRecord {
    pub fn validate(&self) -> io::Result<()> {
        let valid_operation_id = !self.operation_id.is_empty()
            && self.operation_id.len() <= 128
            && self
                .operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        let valid_digest = self.plan_digest.len() == 64
            && self
                .plan_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit());
        if self.version != OPERATION_FENCE_VERSION
            || !valid_operation_id
            || !valid_digest
            || self.runtime_generation == 0
            || self.attempt_sequence == 0
            || self.device_handle.is_empty()
            || (self.phase != OperationFencePhase::RemoteQuiescent && self.outcome.is_some())
            || (self.host_cleanup_complete
                && (self.phase != OperationFencePhase::RemoteQuiescent || self.outcome.is_none()))
            || self
                .backup
                .as_ref()
                .is_some_and(|backup| !valid_backup_identity(self, backup))
            || self
                .target
                .as_ref()
                .is_some_and(|target| !valid_target_identity(target))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "operation fence has an unsupported version or incomplete identity",
            ));
        }
        Ok(())
    }

    pub fn has_same_identity(&self, other: &Self) -> bool {
        self.version == other.version
            && self.operation_id == other.operation_id
            && self.runtime_generation == other.runtime_generation
            && self.attempt_sequence == other.attempt_sequence
            && self.device_handle == other.device_handle
            && self.plan_digest == other.plan_digest
    }

    /// Whether durable evidence supports operator-acknowledged clearing.
    pub fn can_acknowledge_quiescence(&self) -> bool {
        self.phase == OperationFencePhase::RemoteQuiescent
            && self.outcome.is_some()
            && self.host_cleanup_complete
    }
}

/// Creates the initial private fence record and syncs it before execution dispatch.
pub fn write_initial(path: &Path, record: &OperationFenceRecord) -> io::Result<()> {
    record.validate()?;
    if record.phase != OperationFencePhase::Dispatching {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "initial operation fence must be in the dispatching phase",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "operation fence path has no parent directory",
        )
    })?;
    let mut file = private_create_new(path)?;
    write_record(&mut file, record)?;
    file.sync_all()?;
    sync_directory(parent)
}

/// Restores a removed record after a directory-sync failure during fence clearing.
pub fn restore_removed(path: &Path, record: &OperationFenceRecord) -> io::Result<()> {
    record.validate()?;
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "operation fence path has no parent directory",
        )
    })?;
    let mut file = private_create_new(path)?;
    write_record(&mut file, record)?;
    file.sync_all()?;
    sync_directory(parent)
}

/// Reads and validates a versioned fence. Unknown versions and symlinks are rejected.
pub fn read(path: &Path) -> io::Result<OperationFenceRecord> {
    reject_symlink(path)?;
    let file = private_open_existing(path)?;
    let mut bytes = Vec::new();
    file.take((MAX_OPERATION_FENCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_OPERATION_FENCE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "operation fence exceeds the size limit",
        ));
    }
    let record: OperationFenceRecord = serde_json::from_slice(&bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("operation fence is malformed: {error}"),
        )
    })?;
    record.validate()?;
    Ok(record)
}

/// Advances one matching fence after the expected phase has been reached.
pub fn advance(
    path: &Path,
    expected_identity: &OperationFenceRecord,
    expected_phase: OperationFencePhase,
    next_phase: OperationFencePhase,
) -> io::Result<OperationFenceRecord> {
    expected_identity.validate()?;
    if next_phase == OperationFencePhase::RemoteQuiescent {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "remote quiescence requires a terminal operation outcome",
        ));
    }
    let mut current = read(path)?;
    if !current.has_same_identity(expected_identity) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "operation fence identity does not match the active attempt",
        ));
    }
    if current.phase == next_phase {
        return Ok(current);
    }
    if current.phase != expected_phase || !valid_transition(expected_phase, next_phase) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "operation fence phase transition is invalid",
        ));
    }
    current.phase = next_phase;
    replace_record(path, &current)?;
    Ok(current)
}

/// Records remote quiescence together with the outcome operators must review.
pub fn advance_with_outcome(
    path: &Path,
    expected_identity: &OperationFenceRecord,
    expected_phase: OperationFencePhase,
    outcome: OperationFenceOutcome,
) -> io::Result<OperationFenceRecord> {
    expected_identity.validate()?;
    let mut current = read(path)?;
    if !current.has_same_identity(expected_identity) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "operation fence identity does not match active attempt",
        ));
    }
    if current.phase == OperationFencePhase::RemoteQuiescent {
        if current.outcome == Some(outcome) {
            return Ok(current);
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "operation fence already records a different terminal outcome",
        ));
    }
    if current.phase != expected_phase
        || !valid_transition(expected_phase, OperationFencePhase::RemoteQuiescent)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "operation fence phase transition invalid",
        ));
    }
    current.phase = OperationFencePhase::RemoteQuiescent;
    current.outcome = Some(outcome);
    current.host_cleanup_complete = false;
    replace_record(path, &current)?;
    Ok(current)
}

/// Records successful host-side helper and staging cleanup for a terminal operation.
pub fn mark_host_cleanup_complete(
    path: &Path,
    expected: &OperationFenceRecord,
) -> io::Result<bool> {
    let mut current = read(path)?;
    if !current.has_same_identity(expected) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "host cleanup does not match the active operation fence",
        ));
    }
    if current.phase != OperationFencePhase::RemoteQuiescent || current.outcome.is_none() {
        return Ok(false);
    }
    current.host_cleanup_complete = true;
    current.validate()?;
    replace_record(path, &current)?;
    Ok(true)
}

/// Persist verified backup identity before an operation can reach dispatch.
pub fn record_verified_backup(
    path: &Path,
    expected: &OperationFenceRecord,
    backup: OperationFenceBackup,
) -> io::Result<OperationFenceRecord> {
    let current = read(path)?;
    if current != *expected
        || current.phase != OperationFencePhase::Dispatching
        || current.outcome.is_some()
        || current.backup.is_some()
        || !valid_backup_identity(&current, &backup)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "verified backup does not match the active operation fence",
        ));
    }
    let mut updated = current;
    updated.backup = Some(backup);
    replace_record(path, &updated)?;
    Ok(updated)
}

/// Persist a reviewed RetroArch target before any helper can be dispatched.
pub fn record_reviewed_target(
    path: &Path,
    expected: &OperationFenceRecord,
    target: OperationFenceTarget,
) -> io::Result<OperationFenceRecord> {
    let current = read(path)?;
    if current != *expected
        || current.phase != OperationFencePhase::Dispatching
        || current.outcome.is_some()
        || current.target.is_some()
        || !valid_target_identity(&target)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "reviewed target does not match the active operation fence",
        ));
    }
    let mut updated = current;
    updated.target = Some(target);
    replace_record(path, &updated)?;
    Ok(updated)
}

fn valid_backup_identity(record: &OperationFenceRecord, backup: &OperationFenceBackup) -> bool {
    let Some(target) = record.target.as_ref() else {
        return false;
    };
    backup.device_handle == record.device_handle
        && backup.operation == target.operation
        && backup.package_id == target.package_id
        && backup.target_path == target.target_path
        && target.operation == "replace"
        && backup.path.starts_with('/')
        && backup.metadata_path.starts_with('/')
        && backup.captured_at_unix_ms > 0
        && backup.sha256.len() == 64
        && backup.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_target_identity(target: &OperationFenceTarget) -> bool {
    let precondition_status = match target.operation.as_str() {
        "create" => "confirmed_absent",
        "replace" => "confirmed_existing",
        _ => return false,
    };
    target.app_id == "retroarch"
        && target.target_id == "config"
        && target.android_user_id == 0
        && !target.package_id.is_empty()
        && !target.package_id.contains('/')
        && target.target_path.starts_with('/')
        && target
            .target_path
            .ends_with(&format!("/{}/files/retroarch.cfg", target.package_id))
        && target.source_sha256.len() == 64
        && target
            .source_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && target.original_file_precondition["status"].as_str() == Some(precondition_status)
        && target.original_file_precondition["targetPath"].as_str()
            == Some(target.target_path.as_str())
}

fn valid_transition(from: OperationFencePhase, to: OperationFencePhase) -> bool {
    matches!(
        (from, to),
        (
            OperationFencePhase::Dispatching,
            OperationFencePhase::CommitPending
        ) | (
            OperationFencePhase::Dispatching,
            OperationFencePhase::RemoteQuiescent
        ) | (
            OperationFencePhase::CommitPending,
            OperationFencePhase::RemoteQuiescent
        )
    )
}

fn replace_record(path: &Path, record: &OperationFenceRecord) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "operation fence path has no parent directory",
        )
    })?;
    reject_symlink(path)?;
    let temporary = temporary_path(path);
    let result = (|| {
        let mut file = private_create_new(&temporary)?;
        write_record(&mut file, record)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_record(file: &mut File, record: &OperationFenceRecord) -> io::Result<()> {
    serde_json::to_writer(&mut *file, record)
        .map_err(|error| io::Error::other(format!("serialize operation fence: {error}")))?;
    file.write_all(b"\n")
}

fn temporary_path(path: &Path) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("operation-fence");
    path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), nonce))
}

fn reject_symlink(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "operation fence is not a regular file",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn private_create_new(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(not(unix))]
fn private_create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(unix)]
fn private_open_existing(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(not(unix))]
fn private_open_existing(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn sample_record() -> OperationFenceRecord {
        OperationFenceRecord {
            version: OPERATION_FENCE_VERSION,
            operation_id: "g7-a13-aaaaaaaaaaaa".to_string(),
            runtime_generation: 7,
            attempt_sequence: 13,
            device_handle: "device-handle".to_string(),
            plan_digest: "a".repeat(64),
            phase: OperationFencePhase::Dispatching,
            outcome: None,
            host_cleanup_complete: false,
            target: None,
            backup: None,
        }
    }

    #[test]
    fn advances_only_the_matching_fence_identity_and_phase() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("active-device-operation.marker");
        let initial = sample_record();
        write_initial(&path, &initial).unwrap();

        let updated = advance(
            &path,
            &initial,
            OperationFencePhase::Dispatching,
            OperationFencePhase::CommitPending,
        )
        .unwrap();

        assert_eq!(updated.phase, OperationFencePhase::CommitPending);
        assert_eq!(read(&path).unwrap(), updated);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn rejects_a_stale_identity_and_unknown_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("active-device-operation.marker");
        let initial = sample_record();
        write_initial(&path, &initial).unwrap();

        let mut stale = initial.clone();
        stale.attempt_sequence += 1;
        assert_eq!(
            advance(
                &path,
                &stale,
                OperationFencePhase::Dispatching,
                OperationFencePhase::CommitPending,
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );

        let mut unknown = initial;
        unknown.version += 1;
        fs::write(&path, serde_json::to_vec(&unknown).unwrap()).unwrap();
        assert_eq!(read(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn terminal_partial_write_is_persisted_as_quiescent_recovery_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("active-device-operation.marker");
        let initial = sample_record();
        write_initial(&path, &initial).unwrap();
        advance(
            &path,
            &initial,
            OperationFencePhase::Dispatching,
            OperationFencePhase::CommitPending,
        )
        .unwrap();

        let quiescent = advance_with_outcome(
            &path,
            &initial,
            OperationFencePhase::CommitPending,
            OperationFenceOutcome::PartialWriteTerminal,
        )
        .unwrap();

        assert_eq!(quiescent.phase, OperationFencePhase::RemoteQuiescent);
        assert_eq!(
            quiescent.outcome,
            Some(OperationFenceOutcome::PartialWriteTerminal)
        );
        assert_eq!(read(&path).unwrap(), quiescent);
        assert!(!quiescent.can_acknowledge_quiescence());
        assert!(mark_host_cleanup_complete(&path, &initial).unwrap());
        let cleaned = read(&path).unwrap();
        assert!(cleaned.host_cleanup_complete);
        assert!(cleaned.can_acknowledge_quiescence());
    }

    #[test]
    fn remote_quiescence_is_not_acknowledgeable_before_host_cleanup_completes() {
        let mut record = sample_record();
        record.phase = OperationFencePhase::RemoteQuiescent;
        record.outcome = Some(OperationFenceOutcome::PartialWriteTerminal);

        assert!(!record.can_acknowledge_quiescence());
    }

    #[test]
    fn remote_quiescent_record_without_an_outcome_is_invalid() {
        let mut record = sample_record();
        record.phase = OperationFencePhase::RemoteQuiescent;

        assert!(record.validate().is_ok());
        assert!(!record.can_acknowledge_quiescence());
    }

    #[test]
    fn older_remote_quiescent_records_default_host_cleanup_to_unverified() {
        let mut value = serde_json::to_value(sample_record()).unwrap();
        value["phase"] = serde_json::json!("remote_quiescent");
        value["outcome"] = serde_json::json!("partial_write_terminal");
        value.as_object_mut().unwrap().remove("hostCleanupComplete");

        let record: OperationFenceRecord = serde_json::from_value(value).unwrap();

        assert!(!record.host_cleanup_complete);
        assert!(!record.can_acknowledge_quiescence());
    }

    #[test]
    fn verified_backup_identity_is_durable_before_dispatch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("active-device-operation.marker");
        let initial = sample_record();
        write_initial(&path, &initial).unwrap();
        let target_path =
            "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg";
        let record = record_reviewed_target(
            &path,
            &initial,
            OperationFenceTarget {
                operation: "replace".to_string(),
                app_id: "retroarch".to_string(),
                target_id: "config".to_string(),
                package_id: "com.retroarch.aarch64".to_string(),
                android_user_id: 0,
                target_path: target_path.to_string(),
                source_sha256: "c".repeat(64),
                original_file_precondition: serde_json::json!({
                    "status": "confirmed_existing",
                    "targetPath": target_path,
                }),
                backup_mode: "verified".to_string(),
            },
        )
        .unwrap();
        let backup = OperationFenceBackup {
            path: "/private/configuration-backups/op-1/original.cfg".to_string(),
            metadata_path: "/private/configuration-backups/op-1/metadata.v2.json".to_string(),
            operation: "replace".to_string(),
            device_handle: initial.device_handle.clone(),
            package_id: "com.retroarch.aarch64".to_string(),
            target_path: target_path.to_string(),
            captured_at_unix_ms: 1_800_000_000_000,
            sha256: "b".repeat(64),
            length: 42,
        };

        let updated = record_verified_backup(&path, &record, backup.clone()).unwrap();
        assert_eq!(updated.phase, OperationFencePhase::Dispatching);
        assert_eq!(updated.backup, Some(backup.clone()));
        assert_eq!(read(&path).unwrap().backup, Some(backup));
        assert!(record_verified_backup(&path, &initial, updated.backup.clone().unwrap()).is_err());
    }
}
