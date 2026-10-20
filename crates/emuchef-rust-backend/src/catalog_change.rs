//! Shared publication coordination for authored catalog files.
//!
//! Every backend writer that changes authored App or Recipe files uses this
//! module to serialize publication for one authored root. The lock is held
//! across the final validation and write, while durable operation records live
//! in a private, application-owned directory so recovery can find them after a
//! full Config Editor restart. This is a catalog-specific publication boundary,
//! not a general transaction abstraction: individual file replacements remain
//! atomic renames and a multi-file operation may require recovery after a crash.
//! Replacements are durably staged in the same private application-owned tree
//! as the journal, and publication rejects staging locations on a different
//! filesystem from the authored destination.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TERMINAL_RECORDS: usize = 64;
static NEXT_WRITER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Coordinates authored-file publication within a canonical catalog or standalone directory.
#[derive(Clone, Debug)]
pub struct CatalogChangeCoordinator {
    authored_root: PathBuf,
    authored_catalog_root: Option<PathBuf>,
    storage_directory: PathBuf,
    staging_directory: PathBuf,
    lock_path: PathBuf,
}

/// A held same-root publication lock. Pass this guard to internal publication
/// routines to make lock ownership explicit and avoid recursively locking.
#[derive(Debug)]
pub struct PublicationGuard {
    authored_root: PathBuf,
    lock_file: File,
}

impl Drop for PublicationGuard {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
    }
}

/// Durable lifecycle state for an operation that may publish catalog files.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPhase {
    Prepared,
    Staged,
    Publishing,
    Published,
    RecoveryRequired,
    Completed,
    Rejected,
}

impl OperationPhase {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Rejected)
    }
}

/// Restart-safe information needed to identify, replay, or recover a change.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationRecord {
    pub operation_id: String,
    pub operation_kind: String,
    pub request_digest: String,
    pub snapshot_digest: Option<String>,
    pub phase: OperationPhase,
    pub attempt: u32,
    pub created_unix_ms: u128,
    pub updated_unix_ms: u128,
    /// Absolute paths in application-owned staging storage. Retained file
    /// contents are never serialized into the journal.
    pub staged_paths: Vec<PathBuf>,
    /// Durable single-file publications owned by this operation.
    #[serde(default)]
    pub file_writes: Vec<DurableFileWrite>,
    #[serde(default)]
    pub child_operation_ids: Vec<String>,
    /// Parent refactor that owns a staged child write, when applicable.
    #[serde(default)]
    pub parent_operation_id: Option<String>,
    #[serde(default)]
    pub changed_files: Vec<PathBuf>,
    #[serde(default)]
    pub delete_source: Option<DurableSourceDeletion>,
    /// Refactor-specific dependency closure needed to revalidate retries.
    #[serde(default)]
    pub catalog_refactor_recovery: Option<CatalogRefactorRecovery>,
}

/// Ownership evidence for one staged authored-file replacement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DurableFileWrite {
    /// Authored-root-relative destination restored by publication.
    pub destination: PathBuf,
    /// Absolute application-owned staged replacement consumed by rename.
    pub staged_path: PathBuf,
    /// Absolute application-owned rollback bytes for a prior destination.
    #[serde(default)]
    pub rollback_path: Option<PathBuf>,
    pub expected_destination_digest: Option<String>,
    /// Filesystem identity captured from the preview or ordinary save snapshot.
    #[serde(default)]
    pub expected_destination_identity: Option<DurableFileIdentity>,
    pub staged_digest: String,
    /// Identity of the staged inode, which remains stable across its rename.
    #[serde(default)]
    pub staged_identity: Option<DurableFileIdentity>,
    #[cfg(unix)]
    pub permission_mode: u32,
}

/// Ownership evidence for a source removed only after grouped publication.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DurableSourceDeletion {
    pub source: PathBuf,
    pub expected_digest: String,
    #[serde(default)]
    pub expected_identity: Option<DurableFileIdentity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRefactorRecovery {
    pub intent: CatalogRefactorRecoveryIntent,
    /// Authored-root-relative files that contained the old identity at preview.
    #[serde(default)]
    pub dependent_files: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CatalogRefactorRecoveryIntent {
    RenameApp {
        app_id: String,
        new_id: String,
    },
    RenameArtifact {
        app_id: String,
        artifact_id: String,
        new_id: String,
    },
    RenameTarget {
        app_id: String,
        target_id: String,
        new_id: String,
    },
    DeleteApp {
        app_id: String,
    },
}

/// Stable filesystem evidence used with file contents to detect replacement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableFileIdentity {
    pub length: u64,
    pub modified_unix_seconds: Option<i64>,
    pub modified_nanoseconds: Option<u32>,
    #[cfg(unix)]
    pub device: u64,
    #[cfg(unix)]
    pub inode: u64,
    #[cfg(unix)]
    pub permission_mode: u32,
}

impl DurableFileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                length: metadata.len(),
                modified_unix_seconds: modified.map(|time| time.as_secs() as i64),
                modified_nanoseconds: modified.map(|time| time.subsec_nanos()),
                device: metadata.dev(),
                inode: metadata.ino(),
                permission_mode: metadata.mode() & 0o7777,
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                length: metadata.len(),
                modified_unix_seconds: modified.map(|time| time.as_secs() as i64),
                modified_nanoseconds: modified.map(|time| time.subsec_nanos()),
            }
        }
    }
}

/// One safely opened authored file and the identity observed while reading it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSnapshot {
    pub bytes: Vec<u8>,
    pub identity: DurableFileIdentity,
}

/// Read one existing regular file without following a final-component symlink.
pub(crate) fn snapshot_existing_file(path: &Path) -> Result<FileSnapshot, CatalogChangeError> {
    read_regular_file_snapshot(path)
}

impl OperationRecord {
    /// Start a durable operation record with the caller's idempotency digest.
    pub fn new(operation_id: &str, operation_kind: &str, request_digest: &str) -> Self {
        let now = now_unix_ms();
        Self {
            operation_id: operation_id.to_owned(),
            operation_kind: operation_kind.to_owned(),
            request_digest: request_digest.to_owned(),
            snapshot_digest: None,
            phase: OperationPhase::Prepared,
            attempt: 0,
            created_unix_ms: now,
            updated_unix_ms: now,
            staged_paths: Vec::new(),
            file_writes: Vec::new(),
            child_operation_ids: Vec::new(),
            parent_operation_id: None,
            changed_files: Vec::new(),
            delete_source: None,
            catalog_refactor_recovery: None,
        }
    }
}

/// Errors from catalog publication coordination or its durable journal.
#[derive(Debug)]
pub enum CatalogChangeError {
    Io(io::Error),
    InvalidRoot(String),
    InvalidOperation(String),
    OperationConflict(String),
    RecoveryRequired {
        operation_ids: Vec<String>,
    },
    InvalidTransition {
        from: OperationPhase,
        to: OperationPhase,
    },
    GuardMismatch,
}

impl fmt::Display for CatalogChangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "catalog publication I/O failed: {error}"),
            Self::InvalidRoot(message) => write!(formatter, "invalid authored root: {message}"),
            Self::InvalidOperation(message) => {
                write!(formatter, "invalid catalog operation: {message}")
            }
            Self::OperationConflict(id) => write!(
                formatter,
                "operation id {id:?} has different durable content"
            ),
            Self::RecoveryRequired { operation_ids } => write!(
                formatter,
                "catalog recovery is required for: {}",
                operation_ids.join(", ")
            ),
            Self::InvalidTransition { from, to } => write!(
                formatter,
                "invalid catalog operation transition {from:?} -> {to:?}"
            ),
            Self::GuardMismatch => {
                formatter.write_str("publication guard belongs to a different authored root")
            }
        }
    }
}

impl std::error::Error for CatalogChangeError {}

impl From<io::Error> for CatalogChangeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl CatalogChangeCoordinator {
    /// Bind a writer to its declared root, or infer the catalog containing a
    /// destination. Standalone documents coordinate within their own directory.
    pub(crate) fn for_destination_with_storage(
        root: Option<&Path>,
        destination: &Path,
        storage_base: Option<&Path>,
    ) -> Result<Self, CatalogChangeError> {
        let parent = destination
            .parent()
            .ok_or_else(|| CatalogChangeError::InvalidRoot("destination has no parent".into()))?;
        let canonical_parent = fs::canonicalize(parent)?;
        let requested_root = match root {
            Some(root) => {
                let metadata = fs::symlink_metadata(root)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(CatalogChangeError::InvalidRoot(
                        "the requested root must be a real directory, not a symlink".into(),
                    ));
                }
                let canonical_root = fs::canonicalize(root)?;
                canonical_parent
                    .starts_with(&canonical_root)
                    .then_some(root)
            }
            None => None,
        };
        let lexical_candidate = parent
            .ancestors()
            .find(|candidate| parent.starts_with(candidate.join("recipes")));
        let inferred_root = match lexical_candidate {
            Some(candidate) => {
                let recipes = candidate.join("recipes");
                reject_symlink_if_present(candidate)?;
                reject_symlink_if_present(&recipes)?;
                let canonical_root = fs::canonicalize(candidate)?;
                let canonical_recipes = fs::canonicalize(&recipes)?;
                if !canonical_parent.starts_with(&canonical_root)
                    || !canonical_parent.starts_with(&canonical_recipes)
                {
                    return Err(CatalogChangeError::InvalidRoot(
                        "destination resolves outside the inferred authored catalog".into(),
                    ));
                }
                Some(candidate)
            }
            None => None,
        };
        if root.is_some() && requested_root.is_none() && inferred_root.is_some() {
            return Err(CatalogChangeError::InvalidRoot(
                "the destination belongs to a different authored catalog root".into(),
            ));
        }
        let catalog_root = requested_root.or(inferred_root);
        let coordinator_root = catalog_root.unwrap_or(parent);
        let mut coordinator = match storage_base {
            Some(storage_base) => Self::open_with_storage(coordinator_root, storage_base),
            None => Self::open(coordinator_root),
        }?;
        coordinator.authored_catalog_root = catalog_root.map(|_| coordinator.authored_root.clone());
        coordinator.validate_destination(destination)?;
        Ok(coordinator)
    }

    /// Return the authored catalog root when this coordinator represents one.
    pub(crate) fn authored_catalog_root(&self) -> Option<&Path> {
        self.authored_catalog_root.as_deref()
    }

    /// Reject links and destinations outside this lock's canonical root.
    pub(crate) fn validate_destination(
        &self,
        destination: &Path,
    ) -> Result<PathBuf, CatalogChangeError> {
        if destination
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(CatalogChangeError::InvalidRoot(
                "parent traversal is not an authored destination".into(),
            ));
        }
        reject_symlink_if_present(destination)?;
        let parent = destination
            .parent()
            .ok_or_else(|| CatalogChangeError::InvalidRoot("destination has no parent".into()))?;
        let canonical_parent = fs::canonicalize(parent)?;
        if !canonical_parent.starts_with(&self.authored_root) {
            return Err(CatalogChangeError::InvalidRoot(
                "destination lies outside the coordinated authored root".into(),
            ));
        }
        for ancestor in parent.ancestors() {
            if fs::canonicalize(ancestor)? == self.authored_root {
                break;
            }
            reject_symlink_if_present(ancestor)?;
        }
        let name = destination
            .file_name()
            .ok_or_else(|| CatalogChangeError::InvalidRoot("destination has no filename".into()))?;
        Ok(canonical_parent.join(name))
    }

    fn validate_staging_artifact(&self, path: &Path) -> Result<PathBuf, CatalogChangeError> {
        if path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(CatalogChangeError::InvalidRoot(
                "parent traversal in staging artifact is not allowed".into(),
            ));
        }
        reject_symlink_if_present(path)?;
        let parent = path.parent().ok_or_else(|| {
            CatalogChangeError::InvalidRoot("staging artifact has no parent".into())
        })?;
        let canonical_parent = fs::canonicalize(parent)?;
        if !canonical_parent.starts_with(&self.staging_directory) {
            return Err(CatalogChangeError::InvalidRoot(
                "staging artifact lies outside application-owned staging".into(),
            ));
        }
        let name = path.file_name().ok_or_else(|| {
            CatalogChangeError::InvalidRoot("staging artifact has no filename".into())
        })?;
        Ok(canonical_parent.join(name))
    }

    fn snapshot_staging_file(
        &self,
        path: &Path,
    ) -> Result<Option<FileSnapshot>, CatalogChangeError> {
        let path = self.validate_staging_artifact(path)?;
        match read_regular_file_snapshot(&path) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(CatalogChangeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Capture exact destination bytes and filesystem identity without following links.
    pub fn snapshot_file(
        &self,
        destination: &Path,
    ) -> Result<Option<FileSnapshot>, CatalogChangeError> {
        let destination = self.validate_destination(destination)?;
        match read_regular_file_snapshot(&destination) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(CatalogChangeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Open coordination for an authored root using persistent app-owned state.
    pub fn open(authored_root: impl AsRef<Path>) -> Result<Self, CatalogChangeError> {
        let base = stable_storage_directory()?;
        Self::open_with_storage(authored_root, base)
    }

    /// Open coordination with an explicit storage base, primarily for isolated
    /// application profiles and deterministic filesystem tests.
    pub fn open_with_storage(
        authored_root: impl AsRef<Path>,
        storage_base: impl AsRef<Path>,
    ) -> Result<Self, CatalogChangeError> {
        let supplied_root = authored_root.as_ref();
        let root_metadata = fs::symlink_metadata(supplied_root).map_err(CatalogChangeError::Io)?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(CatalogChangeError::InvalidRoot(
                "the root must be a real directory, not a symlink".into(),
            ));
        }
        let authored_root = fs::canonicalize(supplied_root)?;
        let base = storage_base.as_ref();
        create_private_directory(base)?;
        let mut root_hasher = Sha256::new();
        root_hasher.update(authored_root.as_os_str().to_string_lossy().as_bytes());
        let root_key = encode_hex(&root_hasher.finalize());
        let requested_storage = base.join(root_key);
        create_private_directory(&requested_storage)?;
        let requested_storage = fs::canonicalize(&requested_storage)?;
        let staging = requested_storage.join("staging");
        create_private_directory(&staging)?;
        // Runtime processes use the same stable application-owned base, so a
        // root's journal and OS-backed lock are found without trusting a path
        // stored inside the authored catalog.
        let storage_directory = requested_storage;
        let lock_path = storage_directory.join("publication.lock");
        reject_symlink_if_present(&lock_path)?;
        Ok(Self {
            authored_catalog_root: Some(authored_root.clone()),
            authored_root,
            storage_directory,
            staging_directory: staging,
            lock_path,
        })
    }

    /// Return the private journal directory used for this authored root.
    pub fn storage_directory(&self) -> &Path {
        &self.storage_directory
    }

    /// Acquire the root publication lock, then reject ordinary writes if any
    /// durable operation still requires recovery.
    pub fn acquire_publication(&self) -> Result<PublicationGuard, CatalogChangeError> {
        let file = self.open_lock_file()?;
        file.lock()?;
        let guard = PublicationGuard {
            authored_root: self.authored_root.clone(),
            lock_file: file,
        };
        self.cleanup_terminal_artifacts()?;
        self.cleanup_orphan_staged_files()?;
        self.ensure_no_recovery(&guard)?;
        Ok(guard)
    }

    /// Try to acquire the same-root lock without waiting. `None` means another
    /// Config Editor instance currently owns publication.
    pub fn try_acquire_publication(&self) -> Result<Option<PublicationGuard>, CatalogChangeError> {
        let file = self.open_lock_file()?;
        match file.try_lock() {
            Ok(()) => {
                let guard = PublicationGuard {
                    authored_root: self.authored_root.clone(),
                    lock_file: file,
                };
                self.cleanup_terminal_artifacts()?;
                self.cleanup_orphan_staged_files()?;
                self.ensure_no_recovery(&guard)?;
                Ok(Some(guard))
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// Execute ordinary authored-file publication under the shared lock.
    pub fn with_publication<T>(
        &self,
        publish: impl FnOnce(&PublicationGuard) -> Result<T, CatalogChangeError>,
    ) -> Result<T, CatalogChangeError> {
        let guard = self.acquire_publication()?;
        self.publish_with_guard(&guard, publish)
    }

    /// Acquire two root locks in canonical path order for a write that moves
    /// authored content between catalogs. Matching roots share one lock.
    pub(crate) fn with_two_publications<T>(
        first: &Self,
        second: &Self,
        publish: impl FnOnce(&PublicationGuard, &PublicationGuard) -> Result<T, CatalogChangeError>,
    ) -> Result<T, CatalogChangeError> {
        Self::with_publications(&[first, second], |guards| publish(guards[0], guards[1]))
    }

    /// Acquire any number of distinct authored-root locks in canonical path
    /// order, then expose guards in the same order as the input coordinators.
    /// Duplicate roots share one OS lock, preventing self-deadlock.
    pub(crate) fn with_publications<T>(
        coordinators: &[&Self],
        publish: impl FnOnce(&[&PublicationGuard]) -> Result<T, CatalogChangeError>,
    ) -> Result<T, CatalogChangeError> {
        let mut unique: Vec<&Self> = Vec::with_capacity(coordinators.len());
        for coordinator in coordinators {
            if let Some(existing) = unique
                .iter()
                .copied()
                .find(|candidate| candidate.authored_root == coordinator.authored_root)
            {
                if existing.lock_path != coordinator.lock_path {
                    return Err(CatalogChangeError::InvalidRoot(
                        "the same authored root was opened with different publication storage"
                            .into(),
                    ));
                }
            } else {
                unique.push(*coordinator);
            }
        }
        unique.sort_by(|left, right| left.authored_root.cmp(&right.authored_root));

        let mut guards = Vec::with_capacity(unique.len());
        for coordinator in &unique {
            guards.push(coordinator.acquire_publication()?);
        }
        let ordered_guards = coordinators
            .iter()
            .map(|coordinator| {
                let index = unique
                    .iter()
                    .position(|candidate| candidate.authored_root == coordinator.authored_root)
                    .expect("every requested coordinator has a unique root");
                &guards[index]
            })
            .collect::<Vec<_>>();
        publish(&ordered_guards)
    }

    /// Run an internal publication boundary with an already-held root guard.
    /// This is the supported composition point for multi-writer operations.
    pub fn publish_with_guard<T>(
        &self,
        guard: &PublicationGuard,
        publish: impl FnOnce(&PublicationGuard) -> Result<T, CatalogChangeError>,
    ) -> Result<T, CatalogChangeError> {
        self.validate_guard(guard)?;
        publish(guard)
    }

    /// Persist a new operation, or return its existing matching record for
    /// idempotent replay. Reusing an id with a different request is rejected.
    pub fn begin_operation(
        &self,
        guard: &PublicationGuard,
        record: OperationRecord,
    ) -> Result<OperationRecord, CatalogChangeError> {
        self.validate_guard(guard)?;
        validate_operation_id(&record.operation_id)?;
        if record.request_digest.is_empty() || record.operation_kind.is_empty() {
            return Err(CatalogChangeError::InvalidOperation(
                "operation kind and request digest must be non-empty".into(),
            ));
        }
        if let Some(existing) = self.read_record(&record.operation_id)? {
            if existing.request_digest != record.request_digest
                || existing.operation_kind != record.operation_kind
            {
                return Err(CatalogChangeError::OperationConflict(record.operation_id));
            }
            return Ok(existing);
        }
        self.write_record(&record)?;
        Ok(record)
    }

    /// Durably advance an operation lifecycle state. Repeating the current
    /// phase is safe and does not create a duplicate publication.
    pub fn transition(
        &self,
        guard: &PublicationGuard,
        operation_id: &str,
        phase: OperationPhase,
    ) -> Result<OperationRecord, CatalogChangeError> {
        self.validate_guard(guard)?;
        let mut record = self.read_record(operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!("unknown operation {operation_id:?}"))
        })?;
        if record.phase != phase && !valid_transition(record.phase, phase) {
            return Err(CatalogChangeError::InvalidTransition {
                from: record.phase,
                to: phase,
            });
        }
        if record.phase != phase {
            record.phase = phase;
            record.updated_unix_ms = now_unix_ms();
            if phase == OperationPhase::RecoveryRequired {
                record.attempt = record.attempt.saturating_add(1);
            }
            self.write_record(&record)?;
        }
        Ok(record)
    }

    /// Replace durable recovery metadata without changing lifecycle phase.
    pub fn update_operation(
        &self,
        guard: &PublicationGuard,
        mut record: OperationRecord,
    ) -> Result<OperationRecord, CatalogChangeError> {
        self.validate_guard(guard)?;
        let current = self.read_record(&record.operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!(
                "unknown operation {:?}",
                record.operation_id
            ))
        })?;
        if current.operation_kind != record.operation_kind
            || current.request_digest != record.request_digest
            || current.phase != record.phase
        {
            return Err(CatalogChangeError::OperationConflict(record.operation_id));
        }
        record.updated_unix_ms = now_unix_ms();
        self.write_record(&record)?;
        Ok(record)
    }

    /// Read any durable operation state for status replay.
    pub fn operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<OperationRecord>, CatalogChangeError> {
        self.read_record(operation_id)
    }

    /// Find an operation by its backend-generated identifier across all
    /// authored roots in the stable application journal. This is used by
    /// workflows whose status request does not retain an authored-root handle
    /// after a sidecar restart.
    pub fn find_operation(
        operation_id: &str,
    ) -> Result<Option<OperationRecord>, CatalogChangeError> {
        Self::find_operation_at(&stable_storage_directory()?, operation_id)
    }

    /// Read an operation from an application journal base without reopening
    /// its authored root. The root-specific directory name is a hash, and the
    /// record itself must still contain the requested operation identifier.
    pub fn find_operation_at(
        storage_base: impl AsRef<Path>,
        operation_id: &str,
    ) -> Result<Option<OperationRecord>, CatalogChangeError> {
        validate_operation_id(operation_id)?;
        let storage_base = storage_base.as_ref();
        let base_metadata = match fs::symlink_metadata(storage_base) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if base_metadata.file_type().is_symlink() || !base_metadata.is_dir() {
            return Err(CatalogChangeError::InvalidRoot(
                "application journal base must be a real directory".into(),
            ));
        }

        let key = record_file_key(operation_id);
        let mut found: Option<OperationRecord> = None;
        for entry in fs::read_dir(storage_base)? {
            let directory = entry?.path();
            let metadata = fs::symlink_metadata(&directory)?;
            if metadata.file_type().is_symlink() {
                return Err(CatalogChangeError::InvalidRoot(
                    "application journal root must not be a symlink".into(),
                ));
            }
            if !metadata.is_dir() {
                continue;
            }

            let record_path = directory.join(format!("{key}.json"));
            let receipt_path = directory.join(format!("{key}.receipt"));
            let path = match read_regular_file(&record_path) {
                Ok(_) => record_path,
                Err(CatalogChangeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    receipt_path
                }
                Err(error) => return Err(error),
            };
            let bytes = match read_regular_file(&path) {
                Ok(bytes) => bytes,
                Err(CatalogChangeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    continue
                }
                Err(error) => return Err(error),
            };
            let record: OperationRecord = serde_json::from_slice(&bytes)
                .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
            if record.operation_id != operation_id {
                return Err(CatalogChangeError::InvalidOperation(
                    "journal operation identifier does not match its indexed filename".into(),
                ));
            }
            if found.replace(record).is_some() {
                return Err(CatalogChangeError::InvalidOperation(
                    "operation identifier appears in more than one authored-root journal".into(),
                ));
            }
        }
        Ok(found)
    }

    /// Read durable operation state without blocking a concurrent publisher.
    /// The returned boolean is true when another process currently owns the
    /// publication lock, so nonterminal records are still in progress.
    pub(crate) fn operation_for_status(
        &self,
        operation_id: &str,
    ) -> Result<(Option<OperationRecord>, bool), CatalogChangeError> {
        let file = self.open_lock_file()?;
        match file.try_lock() {
            Ok(()) => {
                let _guard = PublicationGuard {
                    authored_root: self.authored_root.clone(),
                    lock_file: file,
                };
                Ok((self.read_record(operation_id)?, false))
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok((self.read_record(operation_id)?, true)),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// Read a durable terminal record for lost-response replay.
    pub fn completed_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<OperationRecord>, CatalogChangeError> {
        Ok(self
            .read_record(operation_id)?
            .filter(|record| record.phase == OperationPhase::Completed))
    }

    /// List all nonterminal operations in deterministic operation-id order.
    pub fn list_recoveries(&self) -> Result<Vec<OperationRecord>, CatalogChangeError> {
        let guard = self.acquire_publication_for_recovery()?;
        self.cleanup_terminal_artifacts()?;
        self.cleanup_orphan_staged_files()?;
        self.list_recoveries_with_guard(&guard)
    }

    /// List recovery records while the caller holds this authored root's lock.
    pub(crate) fn list_recoveries_with_guard(
        &self,
        guard: &PublicationGuard,
    ) -> Result<Vec<OperationRecord>, CatalogChangeError> {
        self.validate_guard(guard)?;
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.storage_directory)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                reject_symlink_if_present(&entry.path())?;
                let record: OperationRecord =
                    serde_json::from_slice(&read_regular_file(&entry.path())?)
                        .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
                if !record.phase.is_terminal() {
                    records.push(record);
                }
            }
        }
        records.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
        Ok(records)
    }

    /// Return all durable children associated with one grouped refactor,
    /// including terminal children that still have replay receipts.
    pub(crate) fn group_child_operations(
        &self,
        guard: &PublicationGuard,
        parent_operation_id: &str,
    ) -> Result<Vec<OperationRecord>, CatalogChangeError> {
        self.validate_guard(guard)?;
        let parent = self.read_record(parent_operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!(
                "unknown parent operation {parent_operation_id:?}"
            ))
        })?;
        let mut operation_ids = parent
            .child_operation_ids
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        for entry in fs::read_dir(&self.storage_directory)? {
            let path = entry?.path();
            if !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("json" | "receipt")
            ) {
                continue;
            }
            let candidate: OperationRecord = serde_json::from_slice(&read_regular_file(&path)?)
                .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
            if candidate.parent_operation_id.as_deref() == Some(parent_operation_id) {
                operation_ids.insert(candidate.operation_id);
            }
        }
        let mut children = Vec::new();
        for operation_id in operation_ids {
            if let Some(record) = self.read_record(&operation_id)? {
                children.push(record);
            }
        }
        children.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
        Ok(children)
    }

    /// Retry a previously blocked operation while retaining the same root lock.
    /// The caller owns conflict resolution and must persist the next phase.
    pub fn retry_recovery<T>(
        &self,
        operation_id: &str,
        retry: impl FnOnce(&PublicationGuard, &OperationRecord) -> Result<T, CatalogChangeError>,
    ) -> Result<T, CatalogChangeError> {
        let guard = self.acquire_publication_for_recovery()?;
        let record = self.read_record(operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!("unknown operation {operation_id:?}"))
        })?;
        if record.parent_operation_id.is_some()
            || self
                .list_recoveries_with_guard(&guard)?
                .iter()
                .any(|candidate| {
                    candidate
                        .child_operation_ids
                        .iter()
                        .any(|child| child == operation_id)
                })
        {
            return Err(CatalogChangeError::InvalidOperation(
                "a grouped child write can only be retried through its parent operation".into(),
            ));
        }
        if record.phase.is_terminal() {
            return Err(CatalogChangeError::InvalidOperation(
                "terminal operations cannot be retried".into(),
            ));
        }
        retry(&guard, &record)
    }

    /// Check durable parent ownership without applying the ordinary writer
    /// gate, which is necessarily closed while a recovery is unresolved.
    pub(crate) fn is_group_child_operation(
        &self,
        operation_id: &str,
    ) -> Result<bool, CatalogChangeError> {
        let guard = self.acquire_publication_for_recovery()?;
        let record = self.read_record(operation_id)?;
        Ok(
            record.is_some_and(|record| record.parent_operation_id.is_some())
                || self
                    .list_recoveries_with_guard(&guard)?
                    .iter()
                    .any(|candidate| {
                        candidate
                            .child_operation_ids
                            .iter()
                            .any(|child| child == operation_id)
                    }),
        )
    }

    /// Stage a crash-recoverable replacement beside the authored catalog.
    ///
    /// The returned operation id remains the durable status key after a lost
    /// response. Callers that already hold the publication guard can publish
    /// the staged bytes without reacquiring the same-root lock.
    pub fn stage_file_write(
        &self,
        guard: &PublicationGuard,
        destination: &Path,
        bytes: &[u8],
    ) -> Result<String, CatalogChangeError> {
        let expected = self.snapshot_file(destination)?;
        self.stage_file_write_with_snapshot(
            guard,
            None,
            destination,
            expected.as_ref(),
            bytes,
            None,
        )
    }

    /// Stage a replacement only if the destination still matches the file
    /// snapshot captured when its document was opened.
    pub fn stage_file_write_with_expected_snapshot(
        &self,
        guard: &PublicationGuard,
        destination: &Path,
        expected: &FileSnapshot,
        bytes: &[u8],
    ) -> Result<String, CatalogChangeError> {
        self.stage_file_write_with_snapshot(guard, None, destination, Some(expected), bytes, None)
    }

    /// Stage a new file only while its destination remains absent.
    pub(crate) fn stage_file_write_if_absent(
        &self,
        guard: &PublicationGuard,
        destination: &Path,
        bytes: &[u8],
    ) -> Result<String, CatalogChangeError> {
        self.stage_file_write_with_snapshot(guard, None, destination, None, bytes, None)
    }

    /// Stage a child replacement using the exact snapshot captured for its
    /// parent refactor. The parent link prevents independent child recovery.
    pub(crate) fn stage_file_write_for_parent(
        &self,
        guard: &PublicationGuard,
        parent_operation_id: &str,
        destination: &Path,
        expected: Option<&FileSnapshot>,
        bytes: &[u8],
    ) -> Result<String, CatalogChangeError> {
        self.stage_file_write_with_snapshot(
            guard,
            Some(parent_operation_id),
            destination,
            expected,
            bytes,
            None,
        )
    }

    /// Stage a grouped replacement while preserving permissions from an
    /// existing source file whose destination path is changing.
    pub(crate) fn stage_file_write_for_parent_with_permissions(
        &self,
        guard: &PublicationGuard,
        parent_operation_id: &str,
        destination: &Path,
        expected: Option<&FileSnapshot>,
        bytes: &[u8],
        permission_source: Option<&FileSnapshot>,
    ) -> Result<String, CatalogChangeError> {
        self.stage_file_write_with_snapshot(
            guard,
            Some(parent_operation_id),
            destination,
            expected,
            bytes,
            permission_source,
        )
    }

    /// Derive the stable journal id for one intended child replacement.
    pub(crate) fn child_write_operation_id(
        &self,
        parent_operation_id: &str,
        destination: &Path,
    ) -> Result<String, CatalogChangeError> {
        let destination = self.validate_destination(destination)?;
        let relative = destination.strip_prefix(&self.authored_root).map_err(|_| {
            CatalogChangeError::InvalidRoot("destination is outside the authored root".into())
        })?;
        let identity = format!("{parent_operation_id}\0{}", relative.to_string_lossy());
        Ok(format!("child-{}", record_file_key(&identity)))
    }

    /// Reject a group whose publication has not started, durably closing any
    /// staged children before releasing the recovery gate.
    pub(crate) fn reject_group_before_publication(
        &self,
        guard: &PublicationGuard,
        parent_operation_id: &str,
    ) -> Result<OperationRecord, CatalogChangeError> {
        self.validate_guard(guard)?;
        let parent = self.read_record(parent_operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!(
                "unknown parent operation {parent_operation_id:?}"
            ))
        })?;
        if !matches!(
            parent.phase,
            OperationPhase::Prepared | OperationPhase::Staged
        ) {
            return Err(CatalogChangeError::InvalidOperation(
                "a group cannot be rejected after publication begins".into(),
            ));
        }
        for child in self.group_child_operations(guard, parent_operation_id)? {
            if !child.phase.is_terminal() {
                self.transition(guard, &child.operation_id, OperationPhase::Rejected)?;
            }
        }
        self.transition(guard, parent_operation_id, OperationPhase::Rejected)
    }

    fn stage_file_write_with_snapshot(
        &self,
        guard: &PublicationGuard,
        parent_operation_id: Option<&str>,
        destination: &Path,
        expected: Option<&FileSnapshot>,
        bytes: &[u8],
        permission_source: Option<&FileSnapshot>,
    ) -> Result<String, CatalogChangeError> {
        self.validate_guard(guard)?;
        let destination = self.validate_destination(destination)?;
        let destination_parent = destination
            .parent()
            .ok_or_else(|| CatalogChangeError::InvalidRoot("destination has no parent".into()))?;
        ensure_same_filesystem(&self.staging_directory, destination_parent)?;
        let relative_destination = destination
            .strip_prefix(&self.authored_root)
            .map_err(|_| {
                CatalogChangeError::InvalidRoot("destination is outside the authored root".into())
            })?
            .to_path_buf();
        let staged_digest = digest_bytes(bytes);
        let actual = self.snapshot_file(&destination)?;
        if actual.as_ref() != expected {
            return Err(CatalogChangeError::InvalidOperation(
                "the destination changed after its authoritative snapshot".into(),
            ));
        }
        let expected_destination_digest = expected.map(|snapshot| digest_bytes(&snapshot.bytes));
        let expected_destination_identity = expected.map(|snapshot| snapshot.identity.clone());
        let sequence = NEXT_WRITER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let operation_id = if let Some(parent) = parent_operation_id {
            self.child_write_operation_id(parent, &destination)?
        } else {
            format!(
                "writer-{}-{}-{sequence:x}",
                &record_file_key(&relative_destination.to_string_lossy())[..16],
                now_unix_ms()
            )
        };
        let staged_name = format!("{}.stage", record_file_key(&operation_id));
        let staged_path = self.staging_directory.join(&staged_name);
        let rollback_name = format!("{}.rollback", record_file_key(&operation_id));
        let rollback_path = self.staging_directory.join(&rollback_name);
        let rollback_relative = if let Some(snapshot) = expected {
            write_private_staged_file(&rollback_path, &snapshot.bytes)?;
            Some(rollback_path.clone())
        } else {
            None
        };
        reject_symlink_if_present(&staged_path)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut record = OperationRecord::new(&operation_id, "ordinary-file-write", &staged_digest);
        record.parent_operation_id = parent_operation_id.map(str::to_owned);
        record.staged_paths.push(staged_path.clone());
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            let mut file = options.open(&staged_path)?;
            file.write_all(bytes)?;
            let permission_mode = expected_destination_identity
                .as_ref()
                .map(|identity| identity.permission_mode)
                .or_else(|| permission_source.map(|snapshot| snapshot.identity.permission_mode))
                .unwrap_or(0o644);
            file.set_permissions(fs::Permissions::from_mode(permission_mode))?;
            file.sync_all()?;
            sync_directory(&self.staging_directory)?;
            let staged_identity = DurableFileIdentity::from_metadata(&file.metadata()?);
            record.file_writes.push(DurableFileWrite {
                destination: relative_destination,
                staged_path,
                rollback_path: rollback_relative,
                expected_destination_digest,
                expected_destination_identity,
                staged_digest,
                staged_identity: Some(staged_identity),
                permission_mode,
            });
            self.begin_operation(guard, record)?;
            self.transition(guard, &operation_id, OperationPhase::Staged)?;
        }
        #[cfg(not(unix))]
        {
            let _ = permission_source;
            let mut file = options.open(&staged_path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            sync_directory(&self.staging_directory)?;
            let staged_identity = DurableFileIdentity::from_metadata(&file.metadata()?);
            record.file_writes.push(DurableFileWrite {
                destination: relative_destination,
                staged_path,
                rollback_path: rollback_relative,
                expected_destination_digest,
                expected_destination_identity,
                staged_digest,
                staged_identity: Some(staged_identity),
            });
            self.begin_operation(guard, record)?;
            self.transition(guard, &operation_id, OperationPhase::Staged)?;
        }
        Ok(operation_id)
    }

    /// Return the staged bytes and private path for post-stage validation.
    pub(crate) fn staged_file_snapshot(
        &self,
        guard: &PublicationGuard,
        operation_id: &str,
    ) -> Result<(PathBuf, FileSnapshot), CatalogChangeError> {
        self.validate_guard(guard)?;
        let record = self.read_record(operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!("unknown operation {operation_id:?}"))
        })?;
        let write = record.file_writes.first().ok_or_else(|| {
            CatalogChangeError::InvalidOperation("operation has no durable file write".into())
        })?;
        let path = write.staged_path.clone();
        self.validate_staging_artifact(&path)?;
        let snapshot = self.snapshot_staging_file(&path)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation("staged bytes are missing".into())
        })?;
        if digest_bytes(&snapshot.bytes) != write.staged_digest
            || write
                .staged_identity
                .as_ref()
                .is_some_and(|expected| expected != &snapshot.identity)
        {
            return Err(CatalogChangeError::InvalidOperation(
                "staged bytes do not match their durable journal record".into(),
            ));
        }
        Ok((path, snapshot))
    }

    /// Publish one previously staged file while retaining the caller's lock.
    pub fn publish_staged_file_write(
        &self,
        guard: &PublicationGuard,
        operation_id: &str,
    ) -> Result<(), CatalogChangeError> {
        self.publish_staged_file_write_with_completion(guard, operation_id, true)
    }

    /// Publish a recovered file but leave its journal open until the caller
    /// confirms that active document sessions have been reconciled.
    pub(crate) fn publish_staged_file_write_until_reconciled(
        &self,
        guard: &PublicationGuard,
        operation_id: &str,
    ) -> Result<(), CatalogChangeError> {
        self.publish_staged_file_write_with_completion(guard, operation_id, false)
    }

    fn publish_staged_file_write_with_completion(
        &self,
        guard: &PublicationGuard,
        operation_id: &str,
        complete_after_publish: bool,
    ) -> Result<(), CatalogChangeError> {
        self.validate_guard(guard)?;
        let record = self.read_record(operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!("unknown operation {operation_id:?}"))
        })?;
        let write = record.file_writes.first().ok_or_else(|| {
            CatalogChangeError::InvalidOperation("operation has no durable file write".into())
        })?;
        let destination = self.authored_root.join(&write.destination);
        let staged = write.staged_path.clone();
        ensure_same_filesystem(
            staged.parent().ok_or_else(|| {
                CatalogChangeError::InvalidRoot("staged file has no parent".into())
            })?,
            destination.parent().ok_or_else(|| {
                CatalogChangeError::InvalidRoot("destination has no parent".into())
            })?,
        )?;
        let current = self.snapshot_file(&destination)?;
        if record.phase == OperationPhase::Completed {
            if matches_staged_destination(&current, write) {
                sync_directory(destination.parent().ok_or_else(|| {
                    CatalogChangeError::InvalidRoot("destination has no parent".into())
                })?)?;
                return Ok(());
            }
            return Err(CatalogChangeError::RecoveryRequired {
                operation_ids: vec![operation_id.to_owned()],
            });
        }
        if matches_staged_destination(&current, write) {
            sync_directory(destination.parent().ok_or_else(|| {
                CatalogChangeError::InvalidRoot("destination has no parent".into())
            })?)?;
            self.finish_published_record(
                guard,
                operation_id,
                record.phase,
                complete_after_publish,
            )?;
            return Ok(());
        }
        if !matches_expected_destination(&current, write) {
            if record.phase != OperationPhase::RecoveryRequired {
                self.transition(guard, operation_id, OperationPhase::RecoveryRequired)?;
            }
            return Err(CatalogChangeError::RecoveryRequired {
                operation_ids: vec![operation_id.to_owned()],
            });
        }
        let staged_snapshot = self.snapshot_staging_file(&staged)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation("staged bytes are missing".into())
        })?;
        if digest_bytes(&staged_snapshot.bytes) != write.staged_digest
            || write
                .staged_identity
                .as_ref()
                .is_some_and(|identity| identity != &staged_snapshot.identity)
        {
            self.transition(guard, operation_id, OperationPhase::RecoveryRequired)?;
            return Err(CatalogChangeError::InvalidOperation(
                "staged bytes no longer match their durable digest".into(),
            ));
        }
        match record.phase {
            OperationPhase::Prepared => {
                self.transition(guard, operation_id, OperationPhase::Staged)?;
                self.transition(guard, operation_id, OperationPhase::Publishing)?;
            }
            OperationPhase::Staged | OperationPhase::RecoveryRequired => {
                self.transition(guard, operation_id, OperationPhase::Publishing)?;
            }
            OperationPhase::Publishing => {}
            OperationPhase::Published => {
                self.transition(guard, operation_id, OperationPhase::RecoveryRequired)?;
                self.transition(guard, operation_id, OperationPhase::Publishing)?;
            }
            OperationPhase::Completed => return Ok(()),
            OperationPhase::Rejected => {
                return Err(CatalogChangeError::InvalidOperation(
                    "rejected operation cannot publish".into(),
                ))
            }
        }
        fs::rename(&staged, &destination)?;
        sync_directory(
            destination.parent().ok_or_else(|| {
                CatalogChangeError::InvalidRoot("destination has no parent".into())
            })?,
        )?;
        self.transition(guard, operation_id, OperationPhase::Published)?;
        if complete_after_publish {
            self.transition(guard, operation_id, OperationPhase::Completed)?;
        }
        Ok(())
    }

    /// Mark a grouped write complete only after its parent operation has
    /// published and reconciled every affected document session.
    pub(crate) fn complete_staged_file_write_after_reconciliation(
        &self,
        guard: &PublicationGuard,
        operation_id: &str,
    ) -> Result<(), CatalogChangeError> {
        self.validate_guard(guard)?;
        let record = self.read_record(operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!("unknown operation {operation_id:?}"))
        })?;
        let write = record.file_writes.first().ok_or_else(|| {
            CatalogChangeError::InvalidOperation("operation has no durable file write".into())
        })?;
        let destination = self.authored_root.join(&write.destination);
        let current = self.snapshot_file(&destination)?;
        if !matches_staged_destination(&current, write) {
            return Err(CatalogChangeError::RecoveryRequired {
                operation_ids: vec![operation_id.to_owned()],
            });
        }
        sync_directory(
            destination.parent().ok_or_else(|| {
                CatalogChangeError::InvalidRoot("destination has no parent".into())
            })?,
        )?;
        match record.phase {
            OperationPhase::Completed => Ok(()),
            OperationPhase::Published => {
                self.transition(guard, operation_id, OperationPhase::Completed)?;
                Ok(())
            }
            phase => Err(CatalogChangeError::InvalidTransition {
                from: phase,
                to: OperationPhase::Completed,
            }),
        }
    }

    /// Recover an interrupted durable file replacement after relaunch.
    pub fn recover_file_write(&self, operation_id: &str) -> Result<(), CatalogChangeError> {
        let guard = self.acquire_publication_for_recovery()?;
        let record = self.read_record(operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!("unknown operation {operation_id:?}"))
        })?;
        if record.parent_operation_id.is_some()
            || self
                .list_recoveries_with_guard(&guard)?
                .iter()
                .any(|candidate| {
                    candidate
                        .child_operation_ids
                        .iter()
                        .any(|child| child == operation_id)
                })
        {
            return Err(CatalogChangeError::InvalidOperation(
                "a grouped child write can only be retried through its parent operation".into(),
            ));
        }
        self.publish_staged_file_write(&guard, operation_id)
    }

    /// Restore a published file only while its current digest proves this
    /// operation still owns the destination. External edits are never
    /// overwritten during rollback.
    pub fn rollback_staged_file_write(
        &self,
        guard: &PublicationGuard,
        operation_id: &str,
    ) -> Result<(), CatalogChangeError> {
        self.validate_guard(guard)?;
        let mut record = self.read_record(operation_id)?.ok_or_else(|| {
            CatalogChangeError::InvalidOperation(format!("unknown operation {operation_id:?}"))
        })?;
        let write = record.file_writes.first().cloned().ok_or_else(|| {
            CatalogChangeError::InvalidOperation("operation has no durable file write".into())
        })?;
        let destination = self.authored_root.join(&write.destination);
        let current = self.snapshot_file(&destination)?;
        if matches_expected_destination(&current, &write) {
            return Ok(());
        }
        if !matches_staged_destination(&current, &write) {
            return Err(CatalogChangeError::RecoveryRequired {
                operation_ids: vec![operation_id.to_owned()],
            });
        }
        let staged = write.staged_path.clone();
        match self.snapshot_staging_file(&staged)? {
            Some(snapshot) if digest_bytes(&snapshot.bytes) == write.staged_digest => {}
            Some(_) => {
                return Err(CatalogChangeError::RecoveryRequired {
                    operation_ids: vec![operation_id.to_owned()],
                });
            }
            None => {
                // Keep the published bytes available for a later roll-forward
                // retry before restoring the previous destination.
                fs::hard_link(&destination, &staged)?;
                sync_directory(staged.parent().ok_or_else(|| {
                    CatalogChangeError::InvalidRoot("staged file has no parent".into())
                })?)?;
            }
        }
        if let Some(rollback_path) = &write.rollback_path {
            let rollback = rollback_path.clone();
            self.validate_staging_artifact(&rollback)?;
            let rollback_snapshot = self.snapshot_staging_file(&rollback)?.ok_or_else(|| {
                CatalogChangeError::InvalidOperation("rollback bytes are missing".into())
            })?;
            if Some(digest_bytes(&rollback_snapshot.bytes)) != write.expected_destination_digest {
                return Err(CatalogChangeError::InvalidOperation(
                    "rollback bytes no longer match their durable digest".into(),
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&rollback, fs::Permissions::from_mode(write.permission_mode))?;
            }
            let restored_snapshot = self.snapshot_staging_file(&rollback)?.ok_or_else(|| {
                CatalogChangeError::InvalidOperation("rollback bytes are missing".into())
            })?;
            record.file_writes[0].expected_destination_identity = Some(restored_snapshot.identity);
            self.update_operation(guard, record)?;
            fs::rename(&rollback, &destination)?;
        } else {
            fs::remove_file(&destination)?;
        }
        sync_directory(
            destination.parent().ok_or_else(|| {
                CatalogChangeError::InvalidRoot("destination has no parent".into())
            })?,
        )?;
        if let Some(staged_parent) = staged.parent() {
            sync_directory(staged_parent)?;
        }
        Ok(())
    }

    fn finish_published_record(
        &self,
        guard: &PublicationGuard,
        operation_id: &str,
        phase: OperationPhase,
        complete_after_publish: bool,
    ) -> Result<(), CatalogChangeError> {
        match phase {
            OperationPhase::Prepared => {
                self.transition(guard, operation_id, OperationPhase::Staged)?;
                self.transition(guard, operation_id, OperationPhase::Publishing)?;
                self.transition(guard, operation_id, OperationPhase::Published)?;
            }
            OperationPhase::Staged => {
                self.transition(guard, operation_id, OperationPhase::Publishing)?;
                self.transition(guard, operation_id, OperationPhase::Published)?;
            }
            OperationPhase::Publishing | OperationPhase::RecoveryRequired => {
                self.transition(guard, operation_id, OperationPhase::Published)?;
            }
            OperationPhase::Published => {}
            OperationPhase::Completed => return Ok(()),
            OperationPhase::Rejected => {
                return Err(CatalogChangeError::InvalidOperation(
                    "rejected operation cannot complete".into(),
                ))
            }
        }
        if complete_after_publish {
            self.transition(guard, operation_id, OperationPhase::Completed)?;
        }
        Ok(())
    }

    fn acquire_publication_for_recovery(&self) -> Result<PublicationGuard, CatalogChangeError> {
        let file = self.open_lock_file()?;
        file.lock()?;
        Ok(PublicationGuard {
            authored_root: self.authored_root.clone(),
            lock_file: file,
        })
    }

    fn open_lock_file(&self) -> Result<File, CatalogChangeError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&self.lock_path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(CatalogChangeError::InvalidRoot(
                "publication lock is not a regular file".into(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        Ok(file)
    }

    fn cleanup_terminal_artifacts(&self) -> Result<(), CatalogChangeError> {
        let mut terminal_records = Vec::new();
        for entry in fs::read_dir(&self.storage_directory)? {
            let path = entry?.path();
            if !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("json" | "receipt")
            ) {
                continue;
            }
            let record: OperationRecord = serde_json::from_slice(&read_regular_file(&path)?)
                .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
            if record.phase.is_terminal() {
                terminal_records.push(record);
            }
        }
        for record in &terminal_records {
            self.cleanup_operation_artifacts(record)?;
        }
        Ok(())
    }

    fn cleanup_orphan_staged_files(&self) -> Result<(), CatalogChangeError> {
        let mut referenced = std::collections::HashSet::new();
        for entry in fs::read_dir(&self.storage_directory)? {
            let path = entry?.path();
            if !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("json" | "receipt")
            ) {
                continue;
            }
            let record: OperationRecord = serde_json::from_slice(&read_regular_file(&path)?)
                .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
            referenced.extend(record.staged_paths);
            for write in record.file_writes {
                referenced.insert(write.staged_path);
                if let Some(rollback) = write.rollback_path {
                    referenced.insert(rollback);
                }
            }
        }
        let mut removed = false;
        for entry in fs::read_dir(&self.staging_directory)? {
            let entry = entry?;
            let path = entry.path();
            if !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("stage" | "rollback")
            ) {
                continue;
            }
            if referenced.contains(&path) {
                continue;
            }
            reject_symlink_if_present(&path)?;
            if !entry.file_type()?.is_file() {
                return Err(CatalogChangeError::InvalidOperation(
                    "orphan staged artifact is not a regular file".into(),
                ));
            }
            fs::remove_file(path)?;
            removed = true;
        }
        if removed {
            sync_directory(&self.staging_directory)?;
        }
        Ok(())
    }

    fn ensure_no_recovery(&self, guard: &PublicationGuard) -> Result<(), CatalogChangeError> {
        self.validate_guard(guard)?;
        let pending = self.list_recoveries_with_guard(guard)?;
        if pending.is_empty() {
            Ok(())
        } else {
            Err(CatalogChangeError::RecoveryRequired {
                operation_ids: pending
                    .into_iter()
                    .map(|record| record.operation_id)
                    .collect(),
            })
        }
    }

    fn validate_guard(&self, guard: &PublicationGuard) -> Result<(), CatalogChangeError> {
        if guard.authored_root == self.authored_root {
            Ok(())
        } else {
            Err(CatalogChangeError::GuardMismatch)
        }
    }

    fn read_record(
        &self,
        operation_id: &str,
    ) -> Result<Option<OperationRecord>, CatalogChangeError> {
        validate_operation_id(operation_id)?;
        let path = self.record_path(operation_id);
        let bytes = match read_regular_file(&path) {
            Ok(bytes) => bytes,
            Err(CatalogChangeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                let archived = self
                    .storage_directory
                    .join(format!("{}.receipt", record_file_key(operation_id)));
                match read_regular_file(&archived) {
                    Ok(bytes) => bytes,
                    Err(CatalogChangeError::Io(error))
                        if error.kind() == io::ErrorKind::NotFound =>
                    {
                        return Ok(None)
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        let record = serde_json::from_slice(&bytes)
            .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
        Ok(Some(record))
    }

    fn write_record(&self, record: &OperationRecord) -> Result<(), CatalogChangeError> {
        let path = self.record_path(&record.operation_id);
        let temp_path = self.storage_directory.join(format!(
            ".{}.{}.tmp",
            record_file_key(&record.operation_id),
            now_unix_ms()
        ));
        let bytes = serde_json::to_vec(record)
            .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&temp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp_path, &path)?;
        sync_directory(&self.storage_directory)?;
        self.cleanup_operation_artifacts(record)?;
        self.cleanup_terminal_records()?;
        Ok(())
    }

    fn cleanup_operation_artifacts(
        &self,
        record: &OperationRecord,
    ) -> Result<(), CatalogChangeError> {
        if !record.phase.is_terminal() {
            return Ok(());
        }
        if record.operation_kind == "catalog-refactor" {
            let mut child_ids = record
                .child_operation_ids
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            for entry in fs::read_dir(&self.storage_directory)? {
                let path = entry?.path();
                if !matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("json" | "receipt")
                ) {
                    continue;
                }
                let child: OperationRecord = serde_json::from_slice(&read_regular_file(&path)?)
                    .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
                if child.parent_operation_id.as_deref() == Some(record.operation_id.as_str()) {
                    child_ids.insert(child.operation_id);
                }
            }
            for child in child_ids {
                if let Some(child_record) = self.read_record(&child)? {
                    self.remove_file_write_artifacts(&child_record)?;
                }
            }
            return Ok(());
        }
        if record.operation_kind != "ordinary-file-write" {
            return Ok(());
        }
        let linked_to_active_group = fs::read_dir(&self.storage_directory)?.try_fold(
            false,
            |linked, entry| -> Result<bool, CatalogChangeError> {
                if linked {
                    return Ok(true);
                }
                let path = entry?.path();
                if path.extension().is_none_or(|extension| extension != "json") {
                    return Ok(false);
                }
                let candidate: OperationRecord = serde_json::from_slice(&read_regular_file(&path)?)
                    .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
                Ok(candidate.operation_kind == "catalog-refactor"
                    && !candidate.phase.is_terminal()
                    && (candidate.child_operation_ids.contains(&record.operation_id)
                        || record.parent_operation_id.as_deref()
                            == Some(candidate.operation_id.as_str())))
            },
        )?;
        if !linked_to_active_group {
            self.remove_file_write_artifacts(record)?;
        }
        Ok(())
    }

    fn remove_file_write_artifacts(
        &self,
        record: &OperationRecord,
    ) -> Result<(), CatalogChangeError> {
        for write in &record.file_writes {
            for path in std::iter::once(Some(&write.staged_path))
                .chain(std::iter::once(write.rollback_path.as_ref()))
                .flatten()
            {
                self.validate_staging_artifact(path)?;
                reject_symlink_if_present(path)?;
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        sync_directory(&self.staging_directory)
    }

    fn record_path(&self, operation_id: &str) -> PathBuf {
        self.storage_directory
            .join(format!("{}.json", record_file_key(operation_id)))
    }

    fn cleanup_terminal_records(&self) -> Result<(), CatalogChangeError> {
        let mut terminal: Vec<(SystemTime, PathBuf)> = Vec::new();
        for entry in fs::read_dir(&self.storage_directory)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                reject_symlink_if_present(&entry.path())?;
                let record: OperationRecord =
                    serde_json::from_slice(&read_regular_file(&entry.path())?)
                        .map_err(|error| CatalogChangeError::InvalidOperation(error.to_string()))?;
                if record.phase.is_terminal() {
                    terminal.push((
                        entry.metadata()?.modified().unwrap_or(UNIX_EPOCH),
                        entry.path(),
                    ));
                }
            }
        }
        terminal.sort_by_key(|(modified, _)| *modified);
        let remove_count = terminal.len().saturating_sub(MAX_TERMINAL_RECORDS);
        for (_, path) in terminal.into_iter().take(remove_count) {
            // Keep a durable terminal receipt after the active-record bound.
            // Pruning receipts would lose status replay and allow an old
            // operation identifier to be mistaken for a new publication.
            fs::rename(&path, path.with_extension("receipt"))?;
        }
        if remove_count > 0 {
            sync_directory(&self.storage_directory)?;
        }
        Ok(())
    }
}

fn stable_storage_directory_from(base: Option<PathBuf>) -> Result<PathBuf, CatalogChangeError> {
    let base = base.ok_or_else(|| {
        CatalogChangeError::InvalidRoot(
            "no stable per-user application data directory is configured".into(),
        )
    })?;
    Ok(base.join("EmuChef").join("catalog-changes"))
}

/// Return the stable per-user application data location shared across launches.
pub fn stable_storage_directory() -> Result<PathBuf, CatalogChangeError> {
    #[cfg(not(test))]
    let base = {
        #[cfg(target_os = "macos")]
        let base = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support"));
        #[cfg(target_os = "windows")]
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            });
        #[cfg(not(any(unix, target_os = "windows")))]
        let base: Option<PathBuf> = None;
        base
    };
    #[cfg(test)]
    let base = std::env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("catalog-change-test-data"))
    });
    stable_storage_directory_from(base)
}

fn create_private_directory(path: &Path) -> Result<(), CatalogChangeError> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CatalogChangeError::InvalidRoot(format!(
            "{} must be a real directory",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    sync_directory(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

/// Read through a no-follow handle and verify the named regular file's exact
/// identity before and after reading its contents.
pub(crate) fn read_regular_file_snapshot(path: &Path) -> Result<FileSnapshot, CatalogChangeError> {
    reject_symlink_if_present(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(CatalogChangeError::InvalidOperation(
            "expected a regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let after = fs::symlink_metadata(path)?;
    if !after.is_file() || before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(CatalogChangeError::InvalidOperation(
            "file changed while reading".into(),
        ));
    }
    let identity = DurableFileIdentity::from_metadata(&before);
    if identity != DurableFileIdentity::from_metadata(&after) {
        return Err(CatalogChangeError::InvalidOperation(
            "file identity changed while reading".into(),
        ));
    }
    Ok(FileSnapshot { bytes, identity })
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>, CatalogChangeError> {
    Ok(read_regular_file_snapshot(path)?.bytes)
}

fn matches_expected_destination(current: &Option<FileSnapshot>, write: &DurableFileWrite) -> bool {
    match current {
        None => {
            write.expected_destination_digest.is_none()
                && write.expected_destination_identity.is_none()
        }
        Some(snapshot) => {
            write
                .expected_destination_digest
                .as_deref()
                .is_some_and(|expected| expected == digest_bytes(&snapshot.bytes))
                && write
                    .expected_destination_identity
                    .as_ref()
                    .is_none_or(|expected| expected == &snapshot.identity)
        }
    }
}

fn matches_staged_destination(current: &Option<FileSnapshot>, write: &DurableFileWrite) -> bool {
    current.as_ref().is_some_and(|snapshot| {
        digest_bytes(&snapshot.bytes) == write.staged_digest
            && write
                .staged_identity
                .as_ref()
                .is_none_or(|staged| staged == &snapshot.identity)
    })
}

fn reject_symlink_if_present(path: &Path) -> Result<(), CatalogChangeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CatalogChangeError::InvalidRoot(
            format!("{} must not be a symlink", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn sync_directory(path: &Path) -> Result<(), CatalogChangeError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn ensure_same_filesystem(
    staging_directory: &Path,
    destination_parent: &Path,
) -> Result<(), CatalogChangeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let staging_device = fs::metadata(staging_directory)?.dev();
        let destination_device = fs::metadata(destination_parent)?.dev();
        if staging_device != destination_device {
            return Err(CatalogChangeError::InvalidRoot(
                "private staging and destination must be on the same filesystem".into(),
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = (staging_directory, destination_parent);

    Ok(())
}

fn write_private_staged_file(path: &Path, bytes: &[u8]) -> Result<(), CatalogChangeError> {
    reject_symlink_if_present(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    let parent = path
        .parent()
        .ok_or_else(|| CatalogChangeError::InvalidRoot("staged file has no parent".into()))?;
    sync_directory(parent)
}

fn record_file_key(operation_id: &str) -> String {
    encode_hex(&Sha256::digest(operation_id.as_bytes()))
}

fn digest_bytes(bytes: &[u8]) -> String {
    encode_hex(&Sha256::digest(bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn validate_operation_id(operation_id: &str) -> Result<(), CatalogChangeError> {
    if operation_id.is_empty()
        || operation_id.len() > 160
        || operation_id.contains('/')
        || operation_id.contains('\\')
    {
        return Err(CatalogChangeError::InvalidOperation(
            "operation id must be 1-160 safe characters".into(),
        ));
    }
    Ok(())
}

fn valid_transition(from: OperationPhase, to: OperationPhase) -> bool {
    matches!(
        (from, to),
        (
            OperationPhase::Prepared,
            OperationPhase::Staged | OperationPhase::Rejected | OperationPhase::RecoveryRequired
        ) | (
            OperationPhase::Staged,
            OperationPhase::Publishing
                | OperationPhase::RecoveryRequired
                | OperationPhase::Rejected
        ) | (
            OperationPhase::Publishing,
            OperationPhase::Published | OperationPhase::RecoveryRequired
        ) | (
            OperationPhase::Published,
            OperationPhase::Completed | OperationPhase::RecoveryRequired
        ) | (
            OperationPhase::RecoveryRequired,
            OperationPhase::Staged
                | OperationPhase::Publishing
                | OperationPhase::Published
                | OperationPhase::Completed
        )
    )
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    #[test]
    fn coordinators_for_same_root_share_lock_and_recovery_gate() {
        let root = TempDir::new().unwrap();
        let storage = root.path().join("application-state");
        let first = CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let second = CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        assert_eq!(first.storage_directory(), second.storage_directory());
        let guard = first.acquire_publication().unwrap();
        assert!(second.try_acquire_publication().unwrap().is_none());
        first
            .begin_operation(&guard, OperationRecord::new("pending", "test", "digest"))
            .unwrap();
        drop(guard);
        assert!(second.acquire_publication().is_err());
    }

    #[test]
    fn journal_location_file_in_authored_root_cannot_redirect_recovery_storage() {
        let temporary = TempDir::new().unwrap();
        let authored = temporary.path().join("authored");
        fs::create_dir(&authored).unwrap();
        let staging = authored.join(".emuchef-catalog-staging");
        fs::create_dir(&staging).unwrap();
        let redirected = temporary.path().join("redirected-journal");
        fs::create_dir(&redirected).unwrap();
        fs::write(
            staging.join("journal-location"),
            serde_json::to_vec(&redirected).unwrap(),
        )
        .unwrap();
        let storage = temporary.path().join("application-state");

        let coordinator = CatalogChangeCoordinator::open_with_storage(&authored, &storage).unwrap();

        assert!(coordinator
            .storage_directory()
            .starts_with(fs::canonicalize(&storage).unwrap()));
        assert!(!redirected.join("publication.lock").exists());
    }

    #[test]
    fn operation_lookup_finds_a_terminal_receipt_without_the_authored_root() {
        let temporary = TempDir::new().unwrap();
        let authored_root = temporary.path().join("authored");
        fs::create_dir(&authored_root).unwrap();
        let storage_base = temporary.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(&authored_root, &storage_base).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        coordinator
            .begin_operation(
                &guard,
                OperationRecord::new("app-generator-replay", "app-generator-pair", "digest"),
            )
            .unwrap();
        for phase in [
            OperationPhase::Staged,
            OperationPhase::Publishing,
            OperationPhase::Published,
            OperationPhase::Completed,
        ] {
            coordinator
                .transition(&guard, "app-generator-replay", phase)
                .unwrap();
        }
        drop(guard);

        fs::rename(
            coordinator.record_path("app-generator-replay"),
            coordinator
                .record_path("app-generator-replay")
                .with_extension("receipt"),
        )
        .unwrap();

        let replay =
            CatalogChangeCoordinator::find_operation_at(&storage_base, "app-generator-replay")
                .unwrap()
                .expect("terminal status should be discoverable by operation id");
        assert_eq!(replay.operation_kind, "app-generator-pair");
        assert_eq!(replay.phase, OperationPhase::Completed);
    }

    #[test]
    fn terminal_replay_survives_more_than_the_cleanup_bound() {
        let root = TempDir::new().unwrap();
        let coordinator = make_coordinator(&root);
        let guard = coordinator.acquire_publication().unwrap();
        for index in 0..70 {
            let id = format!("operation-{index}");
            coordinator
                .begin_operation(&guard, OperationRecord::new(&id, "test", "digest"))
                .unwrap();
            coordinator
                .transition(&guard, &id, OperationPhase::Rejected)
                .unwrap();
        }
        drop(guard);
        let reopened = make_coordinator(&root);
        let guard = reopened.acquire_publication().unwrap();
        let replay = reopened
            .begin_operation(
                &guard,
                OperationRecord::new("operation-0", "test", "digest"),
            )
            .unwrap();
        assert_eq!(replay.phase, OperationPhase::Rejected);
    }

    #[test]
    fn durable_file_publication_recovers_staged_bytes_after_reopen() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("recipe.yaml");
        fs::write(&destination, b"old").unwrap();
        let coordinator = make_coordinator(&root);
        let guard = coordinator.acquire_publication().unwrap();
        let operation_id = coordinator
            .stage_file_write(&guard, &destination, b"new")
            .expect("stage durable write");
        drop(guard);

        let relaunched = make_coordinator(&root);
        relaunched
            .recover_file_write(&operation_id)
            .expect("recover staged write");
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert_eq!(
            relaunched
                .completed_operation(&operation_id)
                .unwrap()
                .expect("terminal replay")
                .phase,
            OperationPhase::Completed
        );
    }

    #[test]
    fn recovery_preserves_external_edits_instead_of_rolling_them_back() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("recipe.yaml");
        fs::write(&destination, b"old").unwrap();
        let coordinator = make_coordinator(&root);
        let guard = coordinator.acquire_publication().unwrap();
        let operation_id = coordinator
            .stage_file_write(&guard, &destination, b"new")
            .expect("stage durable write");
        drop(guard);
        fs::write(&destination, b"external").unwrap();

        let relaunched = make_coordinator(&root);
        assert!(relaunched.recover_file_write(&operation_id).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"external");
        assert!(matches!(
            relaunched.list_recoveries().unwrap()[0].phase,
            OperationPhase::RecoveryRequired
        ));
    }

    #[cfg(unix)]
    #[test]
    fn recovery_rejects_same_bytes_replaced_by_an_external_file() {
        use std::os::unix::fs::MetadataExt;

        let root = TempDir::new().unwrap();
        let destination = root.path().join("recipe.yaml");
        fs::write(&destination, b"old").unwrap();
        let coordinator = make_coordinator(&root);
        let guard = coordinator.acquire_publication().unwrap();
        let operation_id = coordinator
            .stage_file_write(&guard, &destination, b"new")
            .expect("stage durable write");
        drop(guard);

        let external = root.path().join("external-replacement.yaml");
        fs::write(&external, b"old").unwrap();
        fs::rename(&external, &destination).unwrap();
        let external_inode = fs::metadata(&destination).unwrap().ino();

        let relaunched = make_coordinator(&root);
        assert!(relaunched.recover_file_write(&operation_id).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert_eq!(fs::metadata(&destination).unwrap().ino(), external_inode);
        assert_eq!(
            relaunched.list_recoveries().unwrap()[0].phase,
            OperationPhase::RecoveryRequired
        );
    }

    #[cfg(unix)]
    #[test]
    fn staging_rejects_same_bytes_replaced_after_the_preview_snapshot() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("recipe.yaml");
        fs::write(&destination, b"original").unwrap();
        let coordinator = make_coordinator(&root);
        let expected = coordinator.snapshot_file(&destination).unwrap();
        let guard = coordinator.acquire_publication().unwrap();

        let replacement = root.path().join("external.yaml");
        fs::write(&replacement, b"original").unwrap();
        fs::rename(&replacement, &destination).unwrap();

        assert!(coordinator
            .stage_file_write_for_parent(
                &guard,
                "preview-bound-parent",
                &destination,
                expected.as_ref(),
                b"candidate",
            )
            .is_err());
        assert_eq!(fs::read(destination).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_record_symlink_cannot_hide_an_operation() {
        let root = TempDir::new().unwrap();
        let coordinator = make_coordinator(&root);
        std::os::unix::fs::symlink(
            root.path().join("missing"),
            coordinator.record_path("hidden"),
        )
        .unwrap();
        assert!(coordinator.completed_operation("hidden").is_err());
    }

    fn make_coordinator(temp: &TempDir) -> CatalogChangeCoordinator {
        CatalogChangeCoordinator::open_with_storage(
            temp.path(),
            temp.path().join("application-state"),
        )
        .expect("open coordinator")
    }

    #[test]
    fn distinct_instances_cannot_publish_to_the_same_authored_root_concurrently() {
        let temp = TempDir::new().expect("temporary root");
        let first = make_coordinator(&temp);
        let second = make_coordinator(&temp);
        let held = first.try_acquire_publication().expect("first lock attempt");
        assert!(
            held.is_some(),
            "first instance should acquire the root lock"
        );
        assert!(second
            .try_acquire_publication()
            .expect("second lock attempt")
            .is_none());
        drop(held);
        assert!(second
            .try_acquire_publication()
            .expect("lock after release")
            .is_some());
    }

    #[test]
    fn unresolved_durable_operation_blocks_ordinary_publication_and_is_listed() {
        let temp = TempDir::new().expect("temporary root");
        let coordinator = make_coordinator(&temp);
        let guard = coordinator.acquire_publication().expect("publication lock");
        let record = OperationRecord::new("op-1", "app-identity-rename", "request-digest");
        coordinator
            .begin_operation(&guard, record)
            .expect("begin operation");
        coordinator
            .transition(&guard, "op-1", OperationPhase::Staged)
            .expect("persist staged phase");
        coordinator
            .transition(&guard, "op-1", OperationPhase::Publishing)
            .expect("persist publishing phase");
        drop(guard);

        let restarted = make_coordinator(&temp);
        let error = restarted
            .acquire_publication()
            .expect_err("unresolved recovery must gate writers");
        assert!(matches!(error, CatalogChangeError::RecoveryRequired { .. }));
        let pending = restarted
            .list_recoveries()
            .expect("list durable recoveries");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation_id, "op-1");
        assert_eq!(pending[0].phase, OperationPhase::Publishing);
    }

    #[test]
    fn durable_terminal_record_can_be_replayed_after_a_complete_reopen() {
        let temp = TempDir::new().expect("temporary root");
        let coordinator = make_coordinator(&temp);
        let guard = coordinator.acquire_publication().expect("publication lock");
        coordinator
            .begin_operation(
                &guard,
                OperationRecord::new("op-2", "app-identity-rename", "same-request"),
            )
            .expect("begin operation");
        coordinator
            .transition(&guard, "op-2", OperationPhase::Staged)
            .expect("staged");
        coordinator
            .transition(&guard, "op-2", OperationPhase::Publishing)
            .expect("publishing");
        coordinator
            .transition(&guard, "op-2", OperationPhase::Published)
            .expect("published");
        coordinator
            .transition(&guard, "op-2", OperationPhase::Completed)
            .expect("completed");
        drop(guard);

        let restarted = make_coordinator(&temp);
        let replay = restarted
            .completed_operation("op-2")
            .expect("replay lookup");
        assert_eq!(
            replay.expect("completed record").request_digest,
            "same-request"
        );
        assert!(restarted.list_recoveries().expect("recoveries").is_empty());
    }

    #[test]
    fn stable_storage_configuration_is_independent_of_current_directory() {
        let temporary = TempDir::new().expect("temporary application data");
        let base = temporary.path().join("application-data");
        let first = stable_storage_directory_from(Some(base.clone())).expect("stable storage path");
        let second =
            stable_storage_directory_from(Some(base)).expect("stable storage path after relaunch");
        assert_eq!(first, second);
        assert!(first.is_absolute());
        assert_eq!(
            first.file_name().and_then(|name| name.to_str()),
            Some("catalog-changes")
        );
    }

    #[cfg(unix)]
    #[test]
    fn coordinator_rejects_symlinked_authored_roots_and_uses_private_storage() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = TempDir::new().expect("temporary root");
        let authored = temp.path().join("authored");
        fs::create_dir(&authored).expect("authored directory");
        let link = temp.path().join("linked-authored");
        symlink(&authored, &link).expect("symlink");
        assert!(CatalogChangeCoordinator::open(&link).is_err());

        let storage = temp.path().join("private-state");
        let coordinator = CatalogChangeCoordinator::open_with_storage(&authored, &storage)
            .expect("private storage");
        let mode = fs::metadata(coordinator.storage_directory())
            .expect("storage metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn state_directory_is_private_and_staging_is_in_application_storage() {
        let root = TempDir::new().expect("temporary root");
        let storage = root.path().join("application-state");
        let coordinator = CatalogChangeCoordinator::open_with_storage(root.path(), &storage)
            .expect("coordinator");
        let staging = coordinator.storage_directory().join("staging");
        assert!(
            staging.is_dir(),
            "durable staging must live in application-owned storage"
        );
        assert!(!root.path().join(".emuchef-catalog-staging").exists());
        let reopened = CatalogChangeCoordinator::open_with_storage(root.path(), &storage)
            .expect("reopened coordinator");
        assert_eq!(
            coordinator.storage_directory(),
            reopened.storage_directory()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&storage).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(coordinator.storage_directory())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&staging).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn durable_journal_is_discovered_after_reopening_and_each_crash_phase_is_unresolved() {
        let root = TempDir::new().expect("temporary root");
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        for (id, phases) in [
            ("crash-prepared", vec![]),
            ("crash-staged", vec![OperationPhase::Staged]),
            (
                "crash-publishing",
                vec![OperationPhase::Staged, OperationPhase::Publishing],
            ),
            (
                "crash-published",
                vec![
                    OperationPhase::Staged,
                    OperationPhase::Publishing,
                    OperationPhase::Published,
                ],
            ),
            (
                "crash-recovery-required",
                vec![OperationPhase::RecoveryRequired],
            ),
        ] {
            coordinator
                .begin_operation(&guard, OperationRecord::new(id, "test", id))
                .unwrap();
            for phase in phases {
                coordinator.transition(&guard, id, phase).unwrap();
            }
        }
        drop(guard);

        let relaunched = CatalogChangeCoordinator::open_with_storage(root.path(), &storage)
            .expect("full relaunch coordinator");
        let recovered = relaunched.list_recoveries().unwrap();
        assert_eq!(recovered.len(), 5);
        assert_eq!(
            recovered
                .iter()
                .map(|record| record.operation_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "crash-prepared",
                "crash-published",
                "crash-publishing",
                "crash-recovery-required",
                "crash-staged"
            ]
        );
        assert!(matches!(
            relaunched.acquire_publication(),
            Err(CatalogChangeError::RecoveryRequired { .. })
        ));
    }

    #[test]
    fn journal_and_lock_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().expect("temporary root");
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let lock_path = coordinator.lock_path.clone();
        symlink(root.path().join("outside"), &lock_path).unwrap();
        assert!(coordinator.acquire_publication().is_err());

        fs::remove_file(&lock_path).unwrap();
        drop(coordinator.acquire_publication().unwrap());
        let journal_path = coordinator.record_path("linked-record");
        symlink(root.path().join("outside-record"), &journal_path).unwrap();
        assert!(coordinator.list_recoveries().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn journal_records_and_lock_file_have_restrictive_permissions_and_survive_reopen() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempDir::new().expect("temporary root");
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let record = coordinator
            .begin_operation(
                &guard,
                OperationRecord::new("secure-record", "test", "request"),
            )
            .unwrap();
        assert_eq!(
            fs::metadata(&coordinator.lock_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(coordinator.record_path(&record.operation_id))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        drop(guard);
        let relaunched =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        assert_eq!(relaunched.list_recoveries().unwrap().len(), 1);
    }

    #[test]
    fn separate_processes_contend_for_the_same_root_lock() {
        let root = TempDir::new().expect("temporary root");
        let storage = root.path().join("application-state");
        let marker = root.path().join("child-held");
        let release = root.path().join("release-child");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("catalog_change::tests::lock_child_process_helper")
            .arg("--nocapture")
            .env("EMUCHEF_LOCK_CHILD_ROOT", root.path())
            .env("EMUCHEF_LOCK_CHILD_STORAGE", &storage)
            .env("EMUCHEF_LOCK_CHILD_MARKER", &marker)
            .env("EMUCHEF_LOCK_CHILD_RELEASE", &release)
            .spawn()
            .expect("start child test process");
        wait_for_file(&marker, &mut child);
        let parent = CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        assert!(parent.try_acquire_publication().unwrap().is_none());
        fs::write(release, b"release").unwrap();
        assert!(child.wait().unwrap().success());
        assert!(parent.try_acquire_publication().unwrap().is_some());
    }

    #[test]
    fn recovery_discovery_survives_a_complete_process_relaunch() {
        let root = TempDir::new().expect("temporary root");
        let storage = root.path().join("application-state");
        let marker = root.path().join("recovery-discovered");
        let coordinator = CatalogChangeCoordinator::open_with_storage(root.path(), &storage)
            .expect("open journal");
        let guard = coordinator
            .acquire_publication()
            .expect("hold publication lock");
        coordinator
            .begin_operation(
                &guard,
                OperationRecord::new("relaunch-recovery", "catalog-refactor", "snapshot"),
            )
            .unwrap();
        coordinator
            .transition(&guard, "relaunch-recovery", OperationPhase::Staged)
            .unwrap();
        coordinator
            .transition(&guard, "relaunch-recovery", OperationPhase::Publishing)
            .unwrap();
        drop(guard);

        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("catalog_change::tests::recovery_discovery_child_process_helper")
            .arg("--nocapture")
            .env("EMUCHEF_RECOVERY_CHILD_ROOT", root.path())
            .env("EMUCHEF_RECOVERY_CHILD_STORAGE", &storage)
            .env("EMUCHEF_RECOVERY_CHILD_MARKER", &marker)
            .spawn()
            .expect("start fresh backend process");
        wait_for_file(&marker, &mut child);
        assert!(child.wait().unwrap().success());
        assert_eq!(
            fs::read_to_string(marker).unwrap(),
            "relaunch-recovery:RecoveryRequired"
        );
    }

    #[test]
    fn recovery_discovery_child_process_helper() {
        let Some(root) = std::env::var_os("EMUCHEF_RECOVERY_CHILD_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let storage = PathBuf::from(std::env::var_os("EMUCHEF_RECOVERY_CHILD_STORAGE").unwrap());
        let marker = PathBuf::from(std::env::var_os("EMUCHEF_RECOVERY_CHILD_MARKER").unwrap());
        let session = crate::catalog_refactor::CatalogRefactorSession::with_storage(storage);
        let recoveries = session.list_recoveries(&root).unwrap();
        assert_eq!(recoveries.len(), 1);
        fs::write(
            marker,
            format!("{}:{:?}", recoveries[0].operation_id, recoveries[0].state),
        )
        .unwrap();
    }

    #[test]
    fn rollback_restores_only_a_destination_still_owned_by_the_operation() {
        let temp = TempDir::new().expect("temporary root");
        let apps = temp.path().join("apps");
        fs::create_dir(&apps).unwrap();
        let destination = apps.join("demo.yaml");
        fs::write(&destination, b"original").unwrap();
        let coordinator = make_coordinator(&temp);
        let guard = coordinator.acquire_publication().unwrap();
        let operation = coordinator
            .stage_file_write(&guard, &destination, b"published")
            .unwrap();
        let mut parent = OperationRecord::new("rollback-parent", "catalog-refactor", "request");
        parent.child_operation_ids.push(operation.clone());
        coordinator.begin_operation(&guard, parent).unwrap();
        coordinator
            .publish_staged_file_write(&guard, &operation)
            .unwrap();
        coordinator
            .rollback_staged_file_write(&guard, &operation)
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"original");

        let second = coordinator
            .stage_file_write(&guard, &destination, b"published-again")
            .unwrap();
        let mut parent = OperationRecord::new("rollback-parent-2", "catalog-refactor", "request");
        parent.child_operation_ids.push(second.clone());
        coordinator.begin_operation(&guard, parent).unwrap();
        coordinator
            .publish_staged_file_write(&guard, &second)
            .unwrap();
        fs::write(&destination, b"external-edit").unwrap();
        assert!(matches!(
            coordinator.rollback_staged_file_write(&guard, &second),
            Err(CatalogChangeError::RecoveryRequired { .. })
        ));
        assert_eq!(fs::read(&destination).unwrap(), b"external-edit");
    }

    #[test]
    fn rollback_removes_a_new_destination_still_owned_by_the_operation() {
        let temp = TempDir::new().expect("temporary root");
        let apps = temp.path().join("apps");
        fs::create_dir(&apps).unwrap();
        let destination = apps.join("new.yaml");
        let coordinator = make_coordinator(&temp);
        let guard = coordinator.acquire_publication().unwrap();
        let operation = coordinator
            .stage_file_write(&guard, &destination, b"published")
            .unwrap();
        let mut parent = OperationRecord::new("rollback-new-parent", "catalog-refactor", "request");
        parent.child_operation_ids.push(operation.clone());
        coordinator.begin_operation(&guard, parent).unwrap();
        coordinator
            .publish_staged_file_write(&guard, &operation)
            .unwrap();
        coordinator
            .rollback_staged_file_write(&guard, &operation)
            .unwrap();
        assert!(!destination.exists());
    }

    #[test]
    fn lock_child_process_helper() {
        let Some(root) = std::env::var_os("EMUCHEF_LOCK_CHILD_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let storage = PathBuf::from(std::env::var_os("EMUCHEF_LOCK_CHILD_STORAGE").unwrap());
        let marker = PathBuf::from(std::env::var_os("EMUCHEF_LOCK_CHILD_MARKER").unwrap());
        let release = PathBuf::from(std::env::var_os("EMUCHEF_LOCK_CHILD_RELEASE").unwrap());
        let coordinator = CatalogChangeCoordinator::open_with_storage(root, storage).unwrap();
        let _guard = coordinator.acquire_publication().unwrap();
        fs::write(marker, b"held").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !release.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(release.exists(), "parent releases the child lock");
    }

    fn wait_for_file(path: &Path, child: &mut Child) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !path.exists() && Instant::now() < deadline {
            if let Some(status) = child.try_wait().expect("child status") {
                panic!("child exited before acquiring lock: {status}");
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(path.exists(), "child process must signal acquired lock");
    }
}
