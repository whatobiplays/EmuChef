//! Reviewed copy operations for a single selected directory boundary.
//!
//! A commit request binds one host source tree to one selected destination
//! identity. Adapter implementations validate the captured manifests before
//! mutation and report the resulting tree and explicit deletions.

use std::collections::BTreeMap;
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// One copy policy with semantics defined for a selected directory tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyCommitOperation {
    AddUpdate,
    Replace,
    Mirror,
}

impl CopyCommitOperation {
    /// Maps the legacy recipe values only where their directory semantics are
    /// equivalent to the reviewed operation.
    pub fn from_legacy_directory_policy(policy: &str) -> Result<Self, CopyCommitError> {
        match policy {
            "merge" => Ok(Self::AddUpdate),
            "replace" => Ok(Self::Replace),
            "sync" => Ok(Self::Mirror),
            other => Err(CopyCommitError::Rejected(format!(
                "unsupported directory copy policy {other:?}"
            ))),
        }
    }
}

/// An entry in a deterministic source or destination tree manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyManifestEntry {
    pub relative_path: String,
    pub kind: CopyEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Stable filesystem identity used to recheck each destination entry at mutation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_identity: Option<String>,
}

/// Supported filesystem object kinds for a selected copy tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyEntryKind {
    Directory,
    File,
}

/// Identifies whether a reviewed destination is app-private or shared device storage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyCommitDestinationKind {
    #[default]
    AppTarget,
    SharedDevicePath,
}

/// Immutable intent captured before a selected-tree mutation begins.
#[derive(Clone, Debug)]
pub struct CopyCommitRequest {
    pub operation: CopyCommitOperation,
    pub destination_kind: CopyCommitDestinationKind,
    pub source_root: PathBuf,
    pub destination_identity: String,
    pub fake_device_root: PathBuf,
    pub selected_root: PathBuf,
    pub source_manifest: Vec<CopyManifestEntry>,
    pub destination_manifest: Vec<CopyManifestEntry>,
    pub reviewed_deletion_set: Vec<String>,
    /// Device-side canonical path and parent/selected-directory identity
    /// captured before staging. Fake requests use the host filesystem binding.
    pub destination_binding: Option<String>,
}

impl CopyCommitRequest {
    /// Captures the source and destination identities and the exact deletion
    /// set disclosed for this operation before the adapter mutates anything.
    pub fn reviewed_directory(
        source_root: &Path,
        fake_device_root: &Path,
        destination_identity: &str,
        operation: CopyCommitOperation,
    ) -> Result<Self, CopyCommitError> {
        let source_metadata = fs::symlink_metadata(source_root)
            .map_err(|error| CopyCommitError::Rejected(format!("inspect source: {error}")))?;
        if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
            return Err(CopyCommitError::Rejected(
                "copy source must be a non-symlink directory".to_string(),
            ));
        }
        let source_root = fs::canonicalize(source_root)
            .map_err(|error| CopyCommitError::Rejected(format!("resolve source: {error}")))?;
        let fake_root_metadata = fs::symlink_metadata(fake_device_root)
            .map_err(|error| CopyCommitError::Rejected(format!("inspect fake root: {error}")))?;
        if fake_root_metadata.file_type().is_symlink() || !fake_root_metadata.is_dir() {
            return Err(CopyCommitError::Rejected(
                "fake device root must be a non-symlink directory".to_string(),
            ));
        }
        let fake_device_root = fs::canonicalize(fake_device_root)
            .map_err(|error| CopyCommitError::Rejected(format!("resolve fake root: {error}")))?;
        let selected_root = selected_path(&fake_device_root, destination_identity)?;
        validate_path_binding(&fake_device_root, &selected_root)?;

        let source_manifest = scan_tree(&source_root)?;
        let destination_manifest = scan_tree(&selected_root)?;
        let reviewed_deletion_set =
            reviewed_deletions(operation, &source_manifest, &destination_manifest)?;

        Ok(Self {
            operation,
            destination_kind: CopyCommitDestinationKind::AppTarget,
            source_root,
            destination_identity: destination_identity.to_string(),
            fake_device_root,
            selected_root,
            source_manifest,
            destination_manifest,
            reviewed_deletion_set,
            destination_binding: None,
        })
    }

    /// Captures a shared-device destination without changing its reviewed spelling.
    pub fn reviewed_shared_directory(
        source_root: &Path,
        fake_device_root: &Path,
        destination_identity: &str,
        operation: CopyCommitOperation,
    ) -> Result<Self, CopyCommitError> {
        let mut request = Self::reviewed_directory(
            source_root,
            fake_device_root,
            destination_identity,
            operation,
        )?;
        request.destination_kind = CopyCommitDestinationKind::SharedDevicePath;
        Ok(request)
    }

    /// Binds a real-device request to the destination manifest and path
    /// identity observed by the adapter before payload staging begins.
    pub fn reviewed_android_directory(
        source_root: &Path,
        fake_device_root: &Path,
        destination_identity: &str,
        operation: CopyCommitOperation,
        destination_manifest: Vec<CopyManifestEntry>,
        destination_binding: String,
    ) -> Result<Self, CopyCommitError> {
        let mut request = Self::reviewed_directory(
            source_root,
            fake_device_root,
            destination_identity,
            operation,
        )?;
        validate_manifest(&destination_manifest)?;
        request.destination_manifest = destination_manifest;
        request
            .destination_manifest
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        request.reviewed_deletion_set = reviewed_deletions(
            request.operation,
            &request.source_manifest,
            &request.destination_manifest,
        )?;
        request.destination_binding = Some(destination_binding);
        Ok(request)
    }

    /// Captures a shared Android destination while retaining the authored path identity.
    pub fn reviewed_android_shared_directory(
        source_root: &Path,
        fake_device_root: &Path,
        destination_identity: &str,
        operation: CopyCommitOperation,
        destination_manifest: Vec<CopyManifestEntry>,
        destination_binding: String,
    ) -> Result<Self, CopyCommitError> {
        let mut request = Self::reviewed_android_directory(
            source_root,
            fake_device_root,
            destination_identity,
            operation,
            destination_manifest,
            destination_binding,
        )?;
        request.destination_kind = CopyCommitDestinationKind::SharedDevicePath;
        Ok(request)
    }
}

/// Copy state captured before review and reused unchanged during execution.
///
/// The snapshot binds the source tree and selected destination to one copy
/// operation. Its manifests, digests, destination binding, and deletion list
/// are part of the serialized execution plan so the reviewed intent can be
/// compared with the state observed immediately before mutation.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCopyCommitSnapshot {
    /// Execution step whose App Target copy is covered by this snapshot.
    pub(crate) step_id: String,
    /// Add/update, replace, or mirror semantics selected for the copy.
    pub(crate) operation: CopyCommitOperation,
    /// Destination class whose adapter authority was reviewed.
    #[serde(default)]
    pub(crate) destination_kind: CopyCommitDestinationKind,
    /// The source path supplied by the planned host input.
    pub(crate) source_path: PathBuf,
    /// The canonical source root scanned while preparing the review.
    pub(crate) source_root: PathBuf,
    /// Deterministic entries and file digests observed in the source tree.
    pub(crate) source_manifest: Vec<CopyManifestEntry>,
    /// SHA-256 digest of the serialized source manifest.
    pub(crate) source_digest_sha256: String,
    /// Resolved device path of the selected App Target.
    pub(crate) destination_identity: String,
    /// Canonical local device root used to bind the selected tree path.
    pub(crate) fake_device_root: PathBuf,
    /// Selected tree path under the local device root.
    pub(crate) selected_root: PathBuf,
    /// Android path and parent identity captured by the device adapter.
    pub(crate) destination_binding: Option<String>,
    /// Deterministic entries and file digests observed before the copy.
    pub(crate) destination_before_manifest: Vec<CopyManifestEntry>,
    /// SHA-256 digest of the serialized destination-before manifest.
    pub(crate) destination_before_digest_sha256: String,
    /// Exact relative paths that the selected operation is allowed to remove.
    pub(crate) reviewed_deletion_set: Vec<String>,
}

impl ExecutionCopyCommitSnapshot {
    #[allow(dead_code)]
    pub(crate) fn from_request(
        step_id: &str,
        source_path: &Path,
        request: &CopyCommitRequest,
    ) -> Self {
        Self {
            step_id: step_id.to_string(),
            operation: request.operation,
            destination_kind: request.destination_kind,
            source_path: source_path.to_path_buf(),
            source_root: request.source_root.clone(),
            source_manifest: request.source_manifest.clone(),
            source_digest_sha256: manifest_digest(&request.source_manifest),
            destination_identity: request.destination_identity.clone(),
            fake_device_root: request.fake_device_root.clone(),
            selected_root: request.selected_root.clone(),
            destination_binding: request.destination_binding.clone(),
            destination_before_manifest: request.destination_manifest.clone(),
            destination_before_digest_sha256: manifest_digest(&request.destination_manifest),
            reviewed_deletion_set: request.reviewed_deletion_set.clone(),
        }
    }

    /// Rebuild the adapter request from the reviewed bytes, rejecting any
    /// internally inconsistent snapshot before the adapter can mutate a tree.
    pub(crate) fn request_for_execution(
        &self,
        step_id: &str,
        source_path: &Path,
        fake_device_root: &Path,
        destination_identity: &str,
        destination_kind: CopyCommitDestinationKind,
        operation: CopyCommitOperation,
    ) -> Result<CopyCommitRequest, CopyCommitError> {
        if self.step_id != step_id
            || self.source_path != source_path
            || self.destination_identity != destination_identity
            || self.destination_kind != destination_kind
            || self.operation != operation
        {
            return Err(CopyCommitError::Stale(
                "copy step no longer matches its reviewed snapshot".to_string(),
            ));
        }
        let current_source = fs::canonicalize(source_path)
            .map_err(|error| CopyCommitError::Rejected(format!("resolve source: {error}")))?;
        if current_source != self.source_root {
            return Err(CopyCommitError::Stale(
                "source path no longer identifies the reviewed source tree".to_string(),
            ));
        }
        let current_fake_root = fs::canonicalize(fake_device_root)
            .map_err(|error| CopyCommitError::Rejected(format!("resolve fake root: {error}")))?;
        let reviewed_selected_relative = self
            .selected_root
            .strip_prefix(&self.fake_device_root)
            .map_err(|_| {
                CopyCommitError::Rejected(
                    "reviewed selected destination is outside its fake device root".to_string(),
                )
            })?;
        let current_selected_root = selected_path(&current_fake_root, destination_identity)?;
        let current_selected_relative = current_selected_root
            .strip_prefix(&current_fake_root)
            .map_err(|_| {
                CopyCommitError::Rejected(
                    "selected destination is outside its fake device root".to_string(),
                )
            })?;
        if current_selected_relative != reviewed_selected_relative {
            return Err(CopyCommitError::Stale(
                "selected destination no longer identifies the reviewed tree".to_string(),
            ));
        }
        if manifest_digest(&self.source_manifest) != self.source_digest_sha256
            || manifest_digest(&self.destination_before_manifest)
                != self.destination_before_digest_sha256
        {
            return Err(CopyCommitError::Rejected(
                "reviewed copy manifest digest does not match its snapshot".to_string(),
            ));
        }
        let expected_deletions = reviewed_deletions(
            self.operation,
            &self.source_manifest,
            &self.destination_before_manifest,
        )?;
        if expected_deletions != self.reviewed_deletion_set {
            return Err(CopyCommitError::Rejected(
                "reviewed deletion set does not match the selected operation".to_string(),
            ));
        }
        Ok(CopyCommitRequest {
            operation: self.operation,
            destination_kind: self.destination_kind,
            source_root: self.source_root.clone(),
            destination_identity: self.destination_identity.clone(),
            fake_device_root: current_fake_root,
            selected_root: current_selected_root,
            source_manifest: self.source_manifest.clone(),
            destination_manifest: self.destination_before_manifest.clone(),
            reviewed_deletion_set: self.reviewed_deletion_set.clone(),
            destination_binding: self.destination_binding.clone(),
        })
    }

    /// Builds an isolated simulation request from a real reviewed destination snapshot.
    ///
    /// The simulation tree contains empty files with the reviewed paths and kinds. Its
    /// deletion set must still match the real review, so the fake adapter never treats
    /// an unrelated empty destination as the reviewed baseline.
    pub(crate) fn simulation_request_for_execution(
        &self,
        step_id: &str,
        source_path: &Path,
        fake_device_root: &Path,
        destination_identity: &str,
        destination_kind: CopyCommitDestinationKind,
        operation: CopyCommitOperation,
    ) -> Result<CopyCommitRequest, CopyCommitError> {
        let reviewed_request = self.request_for_execution(
            step_id,
            source_path,
            fake_device_root,
            destination_identity,
            destination_kind,
            operation,
        )?;
        materialize_simulation_destination(&reviewed_request)?;

        let simulation_request = match destination_kind {
            CopyCommitDestinationKind::AppTarget => CopyCommitRequest::reviewed_directory(
                source_path,
                fake_device_root,
                destination_identity,
                operation,
            )?,
            CopyCommitDestinationKind::SharedDevicePath => {
                CopyCommitRequest::reviewed_shared_directory(
                    source_path,
                    fake_device_root,
                    destination_identity,
                    operation,
                )?
            }
        };
        if manifest_digest(&simulation_request.source_manifest) != self.source_digest_sha256 {
            return Err(CopyCommitError::Stale(
                "source tree changed after its reviewed manifest was captured".to_string(),
            ));
        }
        if simulation_request.reviewed_deletion_set != self.reviewed_deletion_set {
            return Err(CopyCommitError::Rejected(
                "simulation baseline does not preserve the reviewed deletion set".to_string(),
            ));
        }
        Ok(simulation_request)
    }
}

/// The observable result of a selected-tree copy commit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyCommitReport {
    pub operation: CopyCommitOperation,
    pub destination_identity: String,
    pub succeeded: bool,
    pub outcome: CopyCommitOutcome,
    pub before_paths: Vec<String>,
    pub before_manifest: Vec<CopyManifestEntry>,
    pub after_paths: Vec<String>,
    /// Destination contents observed after mutation, when inspection succeeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_manifest: Option<Vec<CopyManifestEntry>>,
    pub reviewed_deletion_set: Vec<String>,
    pub deleted_paths: Vec<String>,
    pub copied_paths: Vec<String>,
    pub staged_paths: Vec<String>,
    pub staging_verified: bool,
    pub destination_binding_verified: bool,
    pub source_digest_sha256: String,
    /// A pending cleanup message includes the exact stage path and failure reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staging_cleanup_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Detailed failure context retained internally and omitted from reports.
    #[serde(skip)]
    pub(crate) internal_error: Option<String>,
    /// Typed device failure retained for executor fail-stop classification.
    #[serde(skip)]
    pub(crate) internal_device_error: Option<super::DeviceOperationError>,
}

/// Describes the observable destination result after a selected-tree attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CopyCommitOutcome {
    Succeeded,
    Partial,
    Uncertain,
}

/// Errors that occur before a commit can safely begin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CopyCommitError {
    Rejected(String),
    Stale(String),
    Io(String),
    DeviceOperation(super::DeviceOperationError),
}

impl fmt::Display for CopyCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (class, message) = match self {
            Self::Rejected(message) => ("rejected", message),
            Self::Stale(message) => ("stale", message),
            Self::Io(message) => ("I/O failure", message),
            Self::DeviceOperation(error) => ("device operation failed", &error.message),
        };
        write!(formatter, "copy commit {class}: {message}")
    }
}

impl From<super::DeviceOperationError> for CopyCommitError {
    fn from(error: super::DeviceOperationError) -> Self {
        Self::DeviceOperation(error)
    }
}

impl std::error::Error for CopyCommitError {}

/// Narrow adapter contract for committing an already reviewed tree request.
pub trait CopyCommitAdapter {
    fn commit(&mut self, request: &CopyCommitRequest) -> Result<CopyCommitReport, CopyCommitError>;
}

/// Operations required to stage and publish one reviewed tree on Android.
/// Implementations expose typed filesystem actions rather than shell strings.
pub(crate) trait AndroidCopyCommitDevice {
    fn inspect_tree(
        &mut self,
        root: &str,
    ) -> Result<Vec<CopyManifestEntry>, super::DeviceOperationError>;
    fn path_binding(&mut self, path: &str) -> Result<String, super::DeviceOperationError>;
    fn create_directory(&mut self, path: &str) -> Result<(), super::DeviceOperationError>;
    fn mkdir_p(&mut self, path: &str) -> Result<(), super::DeviceOperationError>;
    fn push_file(
        &mut self,
        source: &Path,
        destination: &str,
    ) -> Result<(), super::DeviceOperationError>;
    fn copy_file(
        &mut self,
        source: &str,
        destination: &str,
    ) -> Result<(), super::DeviceOperationError>;
    fn remove_file(&mut self, path: &str) -> Result<(), super::DeviceOperationError>;
    fn remove_empty_directory(&mut self, path: &str) -> Result<(), super::DeviceOperationError>;

    /// Installs the one-shot helper used for selected-root-relative mutations.
    /// In-memory and contract-test devices may use the default path operations.
    fn prepare_mutation_helper(
        &mut self,
        _stage_root: &str,
    ) -> Result<String, super::DeviceOperationError> {
        Ok(String::new())
    }

    /// Applies exactly one operation derived from the reviewed copy manifest.
    fn apply_selected_root_mutation(
        &mut self,
        selected_root: &str,
        destination_binding: &str,
        stage_root: &str,
        helper_path: &str,
        expected_entry: &str,
        mutation: &CopyCommitMutation,
    ) -> Result<(), super::DeviceOperationError> {
        let _ = destination_binding;
        let _ = expected_entry;
        match mutation {
            CopyCommitMutation::EnsureDirectory { relative_path } => {
                let destination = checked_child(Path::new(selected_root), relative_path)
                    .map_err(|error| super::DeviceOperationError::other(error.to_string()))?;
                self.mkdir_p(&destination.to_string_lossy())
            }
            CopyCommitMutation::RemoveFile { relative_path } => {
                let destination = checked_child(Path::new(selected_root), relative_path)
                    .map_err(|error| super::DeviceOperationError::other(error.to_string()))?;
                self.remove_file(&destination.to_string_lossy())
            }
            CopyCommitMutation::RemoveDirectory { relative_path } => {
                let destination = checked_child(Path::new(selected_root), relative_path)
                    .map_err(|error| super::DeviceOperationError::other(error.to_string()))?;
                self.remove_empty_directory(&destination.to_string_lossy())
            }
            CopyCommitMutation::PublishFile { relative_path, .. } => {
                let source = checked_child(Path::new(stage_root), relative_path)
                    .map_err(|error| super::DeviceOperationError::other(error.to_string()))?;
                let destination = checked_child(Path::new(selected_root), relative_path)
                    .map_err(|error| super::DeviceOperationError::other(error.to_string()))?;
                let _ = helper_path;
                self.copy_file(&source.to_string_lossy(), &destination.to_string_lossy())
            }
        }
    }

    /// Removes the helper artifact after selected-root mutations complete.
    fn cleanup_mutation_helper(
        &mut self,
        _helper_path: &str,
    ) -> Result<(), super::DeviceOperationError> {
        Ok(())
    }
}

/// One bounded mutation derived from the reviewed source or deletion manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CopyCommitMutation {
    EnsureDirectory {
        relative_path: String,
    },
    RemoveFile {
        relative_path: String,
    },
    RemoveDirectory {
        relative_path: String,
    },
    PublishFile {
        relative_path: String,
        expected_source_sha256: String,
    },
}

impl<D: super::ExecutorDevice> AndroidCopyCommitDevice for D {
    fn inspect_tree(
        &mut self,
        root: &str,
    ) -> Result<Vec<CopyManifestEntry>, super::DeviceOperationError> {
        self.copy_commit_tree_manifest(root)
    }

    fn path_binding(&mut self, path: &str) -> Result<String, super::DeviceOperationError> {
        self.copy_commit_path_binding(path)
    }

    fn create_directory(&mut self, path: &str) -> Result<(), super::DeviceOperationError> {
        self.copy_commit_create_directory(path)
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), super::DeviceOperationError> {
        super::ExecutorDevice::mkdir_p(self, path)
    }

    fn push_file(
        &mut self,
        source: &Path,
        destination: &str,
    ) -> Result<(), super::DeviceOperationError> {
        super::ExecutorDevice::push(self, source, destination, false)
    }

    fn copy_file(
        &mut self,
        source: &str,
        destination: &str,
    ) -> Result<(), super::DeviceOperationError> {
        super::ExecutorDevice::copy_on_device(self, source, destination, false, true)
    }

    fn remove_file(&mut self, path: &str) -> Result<(), super::DeviceOperationError> {
        super::ExecutorDevice::remove_file(self, path)
    }

    fn remove_empty_directory(&mut self, path: &str) -> Result<(), super::DeviceOperationError> {
        self.copy_commit_remove_empty_directory(path)
    }

    fn prepare_mutation_helper(
        &mut self,
        stage_root: &str,
    ) -> Result<String, super::DeviceOperationError> {
        let helper_path = format!("{stage_root}-helper");
        self.copy_commit_prepare_mutation_helper(stage_root, &helper_path)?;
        Ok(helper_path)
    }

    fn apply_selected_root_mutation(
        &mut self,
        selected_root: &str,
        destination_binding: &str,
        stage_root: &str,
        helper_path: &str,
        expected_entry: &str,
        mutation: &CopyCommitMutation,
    ) -> Result<(), super::DeviceOperationError> {
        self.copy_commit_apply_selected_root_mutation(
            selected_root,
            destination_binding,
            stage_root,
            helper_path,
            expected_entry,
            mutation,
        )
    }

    fn cleanup_mutation_helper(
        &mut self,
        helper_path: &str,
    ) -> Result<(), super::DeviceOperationError> {
        self.copy_commit_cleanup_mutation_helper(helper_path)
    }
}

/// Android adapter for a reviewed selected-tree operation. It stages the full
/// manifest, checks the staged tree, revalidates source and destination state,
/// applies only explicit file and empty-directory operations, and verifies the
/// observable result. It does not claim multi-file atomicity or rollback.
struct CopyCommitAttemptFailure {
    message: String,
    device_error: Option<super::DeviceOperationError>,
}

impl CopyCommitAttemptFailure {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            device_error: None,
        }
    }

    fn device(message: impl Into<String>, error: super::DeviceOperationError) -> Self {
        Self {
            message: message.into(),
            device_error: Some(error),
        }
    }
}

pub(crate) struct AndroidCopyCommitAdapter<'a, D: AndroidCopyCommitDevice> {
    device: &'a mut D,
}

impl<'a, D: AndroidCopyCommitDevice> AndroidCopyCommitAdapter<'a, D> {
    pub(crate) fn new(device: &'a mut D) -> Self {
        Self { device }
    }

    fn observe_after(
        &mut self,
        request: &CopyCommitRequest,
        stage_root: &str,
        copied_paths: &[String],
        deleted_paths: &[String],
        staging_verified: bool,
        failure: CopyCommitAttemptFailure,
    ) -> CopyCommitReport {
        let CopyCommitAttemptFailure {
            message: error,
            device_error,
        } = failure;
        let (binding_verified, binding_error) = match request.destination_binding.as_ref() {
            Some(expected) => match self.device.path_binding(&request.destination_identity) {
                Ok(actual) => (actual == *expected, None),
                Err(error) => (false, Some(error)),
            },
            None => (false, None),
        };
        let mut internal_device_error = device_error.or(binding_error);
        let (after_manifest, outcome, report_error, internal_error) =
            match self.device.inspect_tree(&request.destination_identity) {
                Ok(manifest) => {
                    let outcome = if binding_verified {
                        CopyCommitOutcome::Partial
                    } else {
                        CopyCommitOutcome::Uncertain
                    };
                    (Some(manifest), outcome, error, None)
                }
                Err(inspection_error) => {
                    if internal_device_error.is_none() {
                        internal_device_error = Some(inspection_error.clone());
                    }
                    (
                        None,
                        CopyCommitOutcome::Uncertain,
                        "copy result could not be verified after mutation".to_string(),
                        Some(format!(
                            "{error}; post-mutation destination inspection failed: {}",
                            inspection_error.message
                        )),
                    )
                }
            };
        let after_paths = after_manifest
            .as_ref()
            .map(|manifest| manifest_paths(manifest))
            .unwrap_or_default();
        let staged_paths = if staging_verified {
            manifest_paths(&request.source_manifest)
        } else {
            Vec::new()
        };
        let _ = stage_root;
        CopyCommitReport {
            operation: request.operation,
            destination_identity: request.destination_identity.clone(),
            succeeded: false,
            outcome,
            before_paths: manifest_paths(&request.destination_manifest),
            before_manifest: request.destination_manifest.clone(),
            after_paths,
            after_manifest,
            reviewed_deletion_set: request.reviewed_deletion_set.clone(),
            deleted_paths: deleted_paths.to_vec(),
            copied_paths: copied_paths.to_vec(),
            staged_paths,
            staging_verified,
            destination_binding_verified: binding_verified,
            source_digest_sha256: manifest_digest(&request.source_manifest),
            staging_cleanup_error: None,
            error: Some(report_error),
            internal_error,
            internal_device_error,
        }
    }

    /// Cleans the created stage after every later result and preserves its failure.
    fn finish_staged_result(
        &mut self,
        request: &CopyCommitRequest,
        stage_root: &str,
        helper_path: Option<&str>,
        result: Result<CopyCommitReport, CopyCommitError>,
    ) -> Result<CopyCommitReport, CopyCommitError> {
        let mut cleanup_errors = Vec::new();
        if let Some(helper_path) = helper_path.filter(|path| !path.is_empty()) {
            if let Err(error) = self.device.cleanup_mutation_helper(helper_path) {
                cleanup_errors.push((
                    format!("helper {helper_path}: {}", error.message),
                    Some(error),
                ));
            }
        }
        if let Err(error) = cleanup_staged_tree(self.device, stage_root, &request.source_manifest) {
            let device_error = match &error {
                CopyCommitError::DeviceOperation(error) => Some(error.clone()),
                _ => None,
            };
            cleanup_errors.push((error.to_string(), device_error));
        }

        if cleanup_errors.is_empty() {
            return result;
        }

        let cleanup_message = format!(
            "cleanup pending for staging path {stage_root}: {}",
            cleanup_errors
                .iter()
                .map(|(message, _)| message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        );
        let cleanup_device_error = cleanup_errors
            .into_iter()
            .find_map(|(_, device_error)| device_error);

        match result {
            Ok(mut report) if report.succeeded => {
                // Destination verification completed, so cleanup remains pending work only.
                report.staging_cleanup_error = Some(cleanup_message);
                Ok(report)
            }
            Ok(mut report) => {
                report.succeeded = false;
                report.outcome = CopyCommitOutcome::Uncertain;
                report.staging_cleanup_error = Some(cleanup_message.clone());
                if report.error.is_none() {
                    report.error = Some(cleanup_message);
                }
                if report.internal_device_error.is_none() {
                    report.internal_device_error = cleanup_device_error;
                }
                Ok(report)
            }
            Err(original_error) => {
                let original_message = original_error.to_string();
                let device_error = match original_error {
                    CopyCommitError::DeviceOperation(error) => Some(error),
                    _ => cleanup_device_error,
                };
                let mut report = self.observe_after(
                    request,
                    stage_root,
                    &[],
                    &[],
                    false,
                    CopyCommitAttemptFailure {
                        message: original_message.clone(),
                        device_error,
                    },
                );
                report.outcome = CopyCommitOutcome::Uncertain;
                report.staging_cleanup_error = Some(cleanup_message);
                report.error = Some(original_message);
                Ok(report)
            }
        }
    }
}

impl<D: AndroidCopyCommitDevice> CopyCommitAdapter for AndroidCopyCommitAdapter<'_, D> {
    fn commit(&mut self, request: &CopyCommitRequest) -> Result<CopyCommitReport, CopyCommitError> {
        validate_android_destination(&request.destination_identity)?;
        validate_manifest(&request.source_manifest)?;
        validate_manifest(&request.destination_manifest)?;
        let reviewed_binding = request.destination_binding.as_ref().ok_or_else(|| {
            CopyCommitError::Rejected(
                "Android copy commit requires a reviewed destination binding".to_string(),
            )
        })?;
        let expected_deletions = reviewed_deletions(
            request.operation,
            &request.source_manifest,
            &request.destination_manifest,
        )?;
        if expected_deletions != request.reviewed_deletion_set {
            return Err(CopyCommitError::Rejected(
                "reviewed deletion set does not match the selected operation".to_string(),
            ));
        }

        let initial_binding = self
            .device
            .path_binding(&request.destination_identity)
            .map_err(CopyCommitError::from)?;
        if initial_binding != *reviewed_binding {
            return Err(CopyCommitError::Stale(
                "selected destination identity changed after review".to_string(),
            ));
        }
        let initial_manifest = self
            .device
            .inspect_tree(&request.destination_identity)
            .map_err(CopyCommitError::from)?;
        if initial_manifest != request.destination_manifest {
            return Err(CopyCommitError::Stale(
                "selected destination changed after review".to_string(),
            ));
        }
        let source_tree = SourceTreeReader::open(&request.source_root)?;
        if scan_tree(&request.source_root)? != request.source_manifest {
            return Err(CopyCommitError::Stale(
                "source tree changed after review".to_string(),
            ));
        }

        let stage_root = unique_stage_root();
        let stage_parent = Path::new(&stage_root).parent().ok_or_else(|| {
            CopyCommitError::Rejected("Android staging root has no parent".to_string())
        })?;
        self.device
            .mkdir_p(&stage_parent.to_string_lossy())
            .map_err(CopyCommitError::from)?;
        let actions = android_stage_actions(request, &stage_root)?;
        if !matches!(
            actions.first(),
            Some(AndroidStageAction::CreateDirectory(path)) if path == &stage_root
        ) {
            return Err(CopyCommitError::Rejected(
                "Android staging plan did not begin by creating its unique root".to_string(),
            ));
        }
        self.device
            .create_directory(&stage_root)
            .map_err(CopyCommitError::from)?;
        let mut helper_path_for_cleanup = None;
        let staged_result: Result<CopyCommitReport, CopyCommitError> = (|| {
            for action in actions.iter().skip(1) {
                match action {
                    AndroidStageAction::CreateDirectory(path) => {
                        self.device.mkdir_p(path).map_err(CopyCommitError::from)?
                    }
                    AndroidStageAction::PushFile {
                        relative_path,
                        expected_sha256,
                        destination,
                    } => {
                        let staged_source = stage_reviewed_source_file(
                            &source_tree,
                            relative_path,
                            expected_sha256,
                        )?;
                        self.device
                            .push_file(staged_source.path(), destination)
                            .map_err(CopyCommitError::from)?;
                    }
                }
            }
            let staged_manifest = self
                .device
                .inspect_tree(&stage_root)
                .map_err(CopyCommitError::from)?;
            if !manifest_content_matches(&staged_manifest, &request.source_manifest) {
                return Err(CopyCommitError::Stale(
                    "staged Android tree does not match the reviewed source manifest".to_string(),
                ));
            }

            // Recheck immediately before the first selected-destination mutation.
            if scan_tree(&request.source_root)? != request.source_manifest
                || self
                    .device
                    .path_binding(&request.destination_identity)
                    .map_err(CopyCommitError::from)?
                    != *reviewed_binding
                || self
                    .device
                    .inspect_tree(&request.destination_identity)
                    .map_err(CopyCommitError::from)?
                    != request.destination_manifest
            {
                return Err(CopyCommitError::Stale(
                "source, selected destination identity, or destination tree changed before mutation".to_string(),
            ));
            }

            let expected_helper_path = format!("{stage_root}-helper");
            helper_path_for_cleanup = Some(expected_helper_path.clone());
            let prepared_helper_path = self
                .device
                .prepare_mutation_helper(&stage_root)
                .map_err(CopyCommitError::from)?;
            let helper_path = if prepared_helper_path.is_empty() {
                expected_helper_path
            } else {
                prepared_helper_path
            };
            helper_path_for_cleanup = Some(helper_path.clone());

            let expected_after = expected_manifest(request)?;
            let mut deleted_paths = Vec::new();
            let mut copied_paths = Vec::new();
            let mut current_manifest = request.destination_manifest.clone();
            let mut deletion_order = request.reviewed_deletion_set.clone();
            deletion_order
                .sort_by_key(|path| (std::cmp::Reverse(path.matches('/').count()), path.clone()));
            for relative_path in &deletion_order {
                let kind = request
                    .destination_manifest
                    .iter()
                    .find(|entry| entry.relative_path == *relative_path)
                    .map(|entry| entry.kind)
                    .ok_or_else(|| {
                        CopyCommitError::Rejected(format!(
                            "reviewed deletion is absent from its manifest: {relative_path}"
                        ))
                    })?;
                let mutation = match kind {
                    CopyEntryKind::File => CopyCommitMutation::RemoveFile {
                        relative_path: relative_path.clone(),
                    },
                    CopyEntryKind::Directory => CopyCommitMutation::RemoveDirectory {
                        relative_path: relative_path.clone(),
                    },
                };
                let expected_entry = mutation_entry_precondition(&current_manifest, relative_path)?;
                if let Err(error) = self.device.apply_selected_root_mutation(
                    &request.destination_identity,
                    reviewed_binding,
                    &stage_root,
                    &helper_path,
                    &expected_entry,
                    &mutation,
                ) {
                    return Ok(self.observe_after(
                        request,
                        &stage_root,
                        &copied_paths,
                        &deleted_paths,
                        true,
                        CopyCommitAttemptFailure::device(
                            format!(
                                "reviewed deletion {relative_path} failed: {}",
                                error.message
                            ),
                            error,
                        ),
                    ));
                }
                apply_manifest_mutation(
                    &mut current_manifest,
                    &request.source_manifest,
                    &mutation,
                )?;
                deleted_paths.push(relative_path.clone());
            }
            deleted_paths.sort();

            for entry in &request.source_manifest {
                if entry.kind == CopyEntryKind::Directory {
                    let mutation = CopyCommitMutation::EnsureDirectory {
                        relative_path: entry.relative_path.clone(),
                    };
                    let expected_entry =
                        mutation_entry_precondition(&current_manifest, &entry.relative_path)?;
                    if let Err(error) = self.device.apply_selected_root_mutation(
                        &request.destination_identity,
                        reviewed_binding,
                        &stage_root,
                        &helper_path,
                        &expected_entry,
                        &mutation,
                    ) {
                        return Ok(self.observe_after(
                            request,
                            &stage_root,
                            &copied_paths,
                            &deleted_paths,
                            true,
                            CopyCommitAttemptFailure::device(
                                format!(
                                    "create reviewed directory {} failed: {}",
                                    entry.relative_path, error.message
                                ),
                                error,
                            ),
                        ));
                    }
                    apply_manifest_mutation(
                        &mut current_manifest,
                        &request.source_manifest,
                        &mutation,
                    )?;
                }
            }
            for entry in request
                .source_manifest
                .iter()
                .filter(|entry| entry.kind == CopyEntryKind::File)
            {
                let mutation = CopyCommitMutation::PublishFile {
                    relative_path: entry.relative_path.clone(),
                    expected_source_sha256: entry.sha256.clone().ok_or_else(|| {
                        CopyCommitError::Rejected(format!(
                            "reviewed file lacks a digest: {}",
                            entry.relative_path
                        ))
                    })?,
                };
                let expected_entry =
                    mutation_entry_precondition(&current_manifest, &entry.relative_path)?;
                if let Err(error) = self.device.apply_selected_root_mutation(
                    &request.destination_identity,
                    reviewed_binding,
                    &stage_root,
                    &helper_path,
                    &expected_entry,
                    &mutation,
                ) {
                    return Ok(self.observe_after(
                        request,
                        &stage_root,
                        &copied_paths,
                        &deleted_paths,
                        true,
                        CopyCommitAttemptFailure::device(
                            format!(
                                "publish reviewed file {} failed: {}",
                                entry.relative_path, error.message
                            ),
                            error,
                        ),
                    ));
                }
                apply_manifest_mutation(
                    &mut current_manifest,
                    &request.source_manifest,
                    &mutation,
                )?;
                copied_paths.push(entry.relative_path.clone());
            }
            copied_paths.sort();

            let final_binding = match self.device.path_binding(&request.destination_identity) {
                Ok(binding) => binding,
                Err(error) => {
                    return Ok(self.observe_after(
                        request,
                        &stage_root,
                        &copied_paths,
                        &deleted_paths,
                        true,
                        CopyCommitAttemptFailure::device(
                            "selected destination binding could not be verified after mutation",
                            error,
                        ),
                    ));
                }
            };
            let after_manifest = match self.device.inspect_tree(&request.destination_identity) {
                Ok(manifest) => manifest,
                Err(error) => {
                    return Ok(self.observe_after(
                        request,
                        &stage_root,
                        &copied_paths,
                        &deleted_paths,
                        true,
                        CopyCommitAttemptFailure::device(
                            "selected destination could not be inspected after mutation",
                            error,
                        ),
                    ));
                }
            };
            if final_binding != *reviewed_binding {
                return Ok(self.observe_after(
                request,
                &stage_root,
                &copied_paths,
                &deleted_paths,
                true,
                    CopyCommitAttemptFailure::message(
                        "selected destination path no longer names the reviewed directory after mutation",
                    ),
            ));
            }
            if !manifest_content_matches(&after_manifest, &expected_after) {
                return Ok(self.observe_after(
                    request,
                    &stage_root,
                    &copied_paths,
                    &deleted_paths,
                    true,
                    CopyCommitAttemptFailure::message(
                        "selected destination manifest differs from the reviewed expected tree",
                    ),
                ));
            }

            Ok(CopyCommitReport {
                operation: request.operation,
                destination_identity: request.destination_identity.clone(),
                succeeded: true,
                outcome: CopyCommitOutcome::Succeeded,
                before_paths: manifest_paths(&request.destination_manifest),
                before_manifest: request.destination_manifest.clone(),
                after_paths: manifest_paths(&after_manifest),
                after_manifest: Some(after_manifest),
                reviewed_deletion_set: request.reviewed_deletion_set.clone(),
                deleted_paths,
                copied_paths,
                staged_paths: manifest_paths(&staged_manifest),
                staging_verified: true,
                destination_binding_verified: true,
                source_digest_sha256: manifest_digest(&request.source_manifest),
                staging_cleanup_error: None,
                error: None,
                internal_error: None,
                internal_device_error: None,
            })
        })();
        self.finish_staged_result(
            request,
            &stage_root,
            helper_path_for_cleanup.as_deref(),
            staged_result,
        )
    }
}

fn unique_stage_root() -> String {
    static NEXT_STAGE_ID: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_STAGE_ID.fetch_add(1, Ordering::Relaxed);
    format!(
        "/data/local/tmp/emuchef/t03-copy-{}-{sequence}",
        std::process::id()
    )
}

fn validate_android_destination(path: &str) -> Result<(), CopyCommitError> {
    let path = Path::new(path);
    if !path.is_absolute()
        || path == Path::new("/")
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir | Component::ParentDir | Component::Prefix(_)
            )
        })
    {
        return Err(CopyCommitError::Rejected(
            "Android destination must be a non-root absolute path without traversal".to_string(),
        ));
    }
    Ok(())
}

fn cleanup_stage<D: AndroidCopyCommitDevice>(
    device: &mut D,
    stage_root: &str,
    manifest: &[CopyManifestEntry],
) -> Result<(), CopyCommitError> {
    if device.inspect_tree(stage_root)? != manifest {
        return Err(CopyCommitError::Stale(
            "staging tree changed; explicit cleanup was skipped".to_string(),
        ));
    }
    let mut entries = manifest.to_vec();
    entries.sort_by_key(|entry| {
        (
            std::cmp::Reverse(entry.relative_path.matches('/').count()),
            entry.relative_path.clone(),
        )
    });
    for entry in entries
        .iter()
        .filter(|entry| entry.kind == CopyEntryKind::File)
    {
        let path = checked_child(Path::new(stage_root), &entry.relative_path)?
            .to_string_lossy()
            .into_owned();
        device.remove_file(&path).map_err(CopyCommitError::from)?;
    }
    for entry in entries
        .iter()
        .filter(|entry| entry.kind == CopyEntryKind::Directory)
    {
        let path = checked_child(Path::new(stage_root), &entry.relative_path)?
            .to_string_lossy()
            .into_owned();
        device
            .remove_empty_directory(&path)
            .map_err(CopyCommitError::from)?;
    }
    device
        .remove_empty_directory(stage_root)
        .map_err(CopyCommitError::from)
}

/// Removes only validated entries from the reviewed tree beneath the exact stage root.
fn cleanup_staged_tree<D: AndroidCopyCommitDevice>(
    device: &mut D,
    stage_root: &str,
    reviewed_manifest: &[CopyManifestEntry],
) -> Result<(), CopyCommitError> {
    let manifest = device.inspect_tree(stage_root)?;
    validate_manifest(&manifest).map_err(|error| {
        CopyCommitError::Rejected(format!("staging tree is not safe to clean up: {error}"))
    })?;
    let reviewed_entries = reviewed_manifest
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry.kind))
        .collect::<BTreeMap<_, _>>();
    if manifest
        .iter()
        .any(|entry| reviewed_entries.get(entry.relative_path.as_str()) != Some(&entry.kind))
    {
        return Err(CopyCommitError::Rejected(
            "staging tree contains entries outside the reviewed source manifest; cleanup was skipped"
                .to_string(),
        ));
    }
    cleanup_stage(device, stage_root, &manifest)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AndroidStageAction {
    CreateDirectory(String),
    PushFile {
        relative_path: String,
        expected_sha256: String,
        destination: String,
    },
}

fn android_stage_actions(
    request: &CopyCommitRequest,
    stage_root: &str,
) -> Result<Vec<AndroidStageAction>, CopyCommitError> {
    let stage_path = Path::new(stage_root);
    if !stage_path.is_absolute()
        || stage_path.components().any(|component| {
            matches!(
                component,
                Component::CurDir | Component::ParentDir | Component::Prefix(_)
            )
        })
    {
        return Err(CopyCommitError::Rejected(
            "Android staging root must be an absolute path without traversal".to_string(),
        ));
    }

    let mut actions = vec![AndroidStageAction::CreateDirectory(
        stage_path.to_string_lossy().into_owned(),
    )];
    let mut directories = request
        .source_manifest
        .iter()
        .filter(|entry| entry.kind == CopyEntryKind::Directory)
        .collect::<Vec<_>>();
    directories.sort_by_key(|entry| {
        (
            entry.relative_path.matches('/').count(),
            entry.relative_path.clone(),
        )
    });
    for entry in directories {
        let destination = checked_child(stage_path, &entry.relative_path)?;
        actions.push(AndroidStageAction::CreateDirectory(
            destination.to_string_lossy().into_owned(),
        ));
    }

    let mut files = request
        .source_manifest
        .iter()
        .filter(|entry| entry.kind == CopyEntryKind::File)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    for entry in files {
        let destination = checked_child(stage_path, &entry.relative_path)?;
        actions.push(AndroidStageAction::PushFile {
            relative_path: entry.relative_path.clone(),
            expected_sha256: entry.sha256.clone().ok_or_else(|| {
                CopyCommitError::Rejected(format!(
                    "reviewed file lacks a digest: {}",
                    entry.relative_path
                ))
            })?,
            destination: destination.to_string_lossy().into_owned(),
        });
    }
    Ok(actions)
}

fn stage_reviewed_source_file(
    source_tree: &SourceTreeReader,
    relative_path: &str,
    expected_sha256: &str,
) -> Result<tempfile::NamedTempFile, CopyCommitError> {
    let mut source = source_tree.open_file(relative_path)?;
    let permissions = source
        .metadata()
        .map_err(|error| CopyCommitError::Io(format!("inspect source entry: {error}")))?
        .permissions();
    let mut staged = tempfile::NamedTempFile::new()
        .map_err(|error| CopyCommitError::Io(format!("create local staging file: {error}")))?;
    copy_reviewed_file(
        &mut source,
        staged.as_file_mut(),
        expected_sha256,
        relative_path,
    )?;
    staged
        .as_file()
        .set_permissions(permissions)
        .map_err(|error| CopyCommitError::Io(format!("set local staging permissions: {error}")))?;
    staged
        .as_file()
        .sync_all()
        .map_err(|error| CopyCommitError::Io(format!("flush local staging file: {error}")))?;
    Ok(staged)
}

/// Filesystem-backed fake adapter used by trusted executor tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct FakeFilesystemCopyCommitAdapter;

#[cfg(unix)]
fn fake_post_mutation_report(
    request: &CopyCommitRequest,
    selected: &SelectedRootMutator,
    mut deleted_paths: Vec<String>,
    mut copied_paths: Vec<String>,
    error: CopyCommitError,
) -> CopyCommitReport {
    deleted_paths.sort();
    copied_paths.sort();
    let binding_error = validate_path_binding(&request.fake_device_root, &request.selected_root)
        .err()
        .map(|error| error.to_string());
    let after_result = selected.scan_tree();
    let internal_error = match (&binding_error, &after_result) {
        (Some(binding), Err(inspection)) => Some(format!(
            "destination binding check failed: {binding}; post-mutation inspection failed: {inspection}"
        )),
        (Some(binding), Ok(_)) => Some(format!("destination binding check failed: {binding}")),
        (None, Err(inspection)) => {
            Some(format!("post-mutation destination inspection failed: {inspection}"))
        }
        (None, Ok(_)) => None,
    };
    let after_manifest = after_result.ok();
    let binding_verified = binding_error.is_none();
    let outcome = if binding_verified && after_manifest.is_some() {
        CopyCommitOutcome::Partial
    } else {
        CopyCommitOutcome::Uncertain
    };
    let after_paths = after_manifest
        .as_deref()
        .map(manifest_paths)
        .unwrap_or_default();

    CopyCommitReport {
        operation: request.operation,
        destination_identity: request.destination_identity.clone(),
        succeeded: false,
        outcome,
        before_paths: manifest_paths(&request.destination_manifest),
        before_manifest: request.destination_manifest.clone(),
        after_paths,
        after_manifest,
        reviewed_deletion_set: request.reviewed_deletion_set.clone(),
        deleted_paths,
        copied_paths,
        staged_paths: Vec::new(),
        staging_verified: false,
        destination_binding_verified: binding_verified,
        source_digest_sha256: manifest_digest(&request.source_manifest),
        staging_cleanup_error: None,
        error: Some(error.to_string()),
        internal_error,
        internal_device_error: None,
    }
}

impl CopyCommitAdapter for FakeFilesystemCopyCommitAdapter {
    fn commit(&mut self, request: &CopyCommitRequest) -> Result<CopyCommitReport, CopyCommitError> {
        if selected_path(&request.fake_device_root, &request.destination_identity)?
            != request.selected_root
        {
            return Err(CopyCommitError::Rejected(
                "selected path no longer matches its reviewed destination identity".to_string(),
            ));
        }
        validate_path_binding(&request.fake_device_root, &request.selected_root)?;
        let source_tree = SourceTreeReader::open(&request.source_root)?;
        let current_source = scan_tree(&request.source_root)?;
        let current_destination = scan_tree(&request.selected_root)?;
        if current_source != request.source_manifest {
            return Err(CopyCommitError::Stale(
                "source tree changed after its reviewed manifest was captured".to_string(),
            ));
        }
        if current_destination != request.destination_manifest {
            return Err(CopyCommitError::Stale(
                "selected destination changed after its reviewed manifest was captured".to_string(),
            ));
        }

        let expected_deletions = reviewed_deletions(
            request.operation,
            &request.source_manifest,
            &request.destination_manifest,
        )?;
        if expected_deletions != request.reviewed_deletion_set {
            return Err(CopyCommitError::Rejected(
                "reviewed deletion set does not match the selected operation".to_string(),
            ));
        }
        let expected_after = expected_manifest(request)?;
        let before_paths = manifest_paths(&request.destination_manifest);
        let source_digest_sha256 = manifest_digest(&request.source_manifest);
        let mut deleted_paths = Vec::new();
        let mut copied_paths = Vec::new();

        let selected =
            SelectedRootMutator::open_or_create(&request.fake_device_root, &request.selected_root)?;
        validate_path_binding(&request.fake_device_root, &request.selected_root)?;
        if scan_tree(&request.source_root)? != request.source_manifest
            || selected.scan_tree()? != request.destination_manifest
        {
            return Err(CopyCommitError::Stale(
                "source or selected destination changed before mutation".to_string(),
            ));
        }
        let mutation_result = (|| {
            let mut deletion_order = request.reviewed_deletion_set.clone();
            deletion_order
                .sort_by_key(|path| (std::cmp::Reverse(path.matches('/').count()), path.clone()));
            let mut current_manifest = request.destination_manifest.clone();
            for relative_path in &deletion_order {
                let kind = request
                    .destination_manifest
                    .iter()
                    .find(|entry| entry.relative_path == *relative_path)
                    .map(|entry| entry.kind)
                    .ok_or_else(|| {
                        CopyCommitError::Rejected(format!(
                            "reviewed deletion is absent from its manifest: {relative_path}"
                        ))
                    })?;
                validate_path_binding(&request.fake_device_root, &request.selected_root)?;
                verify_selected_entry(&selected, &current_manifest, relative_path)?;
                match kind {
                    CopyEntryKind::File => selected.remove_file(relative_path)?,
                    CopyEntryKind::Directory => selected.remove_directory(relative_path)?,
                }
                apply_manifest_mutation(
                    &mut current_manifest,
                    &request.source_manifest,
                    &match kind {
                        CopyEntryKind::File => CopyCommitMutation::RemoveFile {
                            relative_path: relative_path.clone(),
                        },
                        CopyEntryKind::Directory => CopyCommitMutation::RemoveDirectory {
                            relative_path: relative_path.clone(),
                        },
                    },
                )?;
                deleted_paths.push(relative_path.clone());
            }

            for entry in &request.source_manifest {
                validate_path_binding(&request.fake_device_root, &request.selected_root)?;
                verify_selected_entry(&selected, &current_manifest, &entry.relative_path)?;
                let mutation = match entry.kind {
                    CopyEntryKind::Directory => CopyCommitMutation::EnsureDirectory {
                        relative_path: entry.relative_path.clone(),
                    },
                    CopyEntryKind::File => CopyCommitMutation::PublishFile {
                        relative_path: entry.relative_path.clone(),
                        expected_source_sha256: entry.sha256.clone().ok_or_else(|| {
                            CopyCommitError::Rejected(format!(
                                "reviewed file lacks a digest: {}",
                                entry.relative_path
                            ))
                        })?,
                    },
                };
                match entry.kind {
                    CopyEntryKind::Directory => {
                        selected.ensure_directory(&entry.relative_path)?;
                    }
                    CopyEntryKind::File => {
                        let expected_sha256 = entry.sha256.as_deref().ok_or_else(|| {
                            CopyCommitError::Rejected(format!(
                                "reviewed file lacks a digest: {}",
                                entry.relative_path
                            ))
                        })?;
                        selected
                            .publish_file(&source_tree, &entry.relative_path, expected_sha256)
                            .map_err(|error| {
                                CopyCommitError::Io(format!(
                                    "publish selected file {}: {error}",
                                    entry.relative_path
                                ))
                            })?;
                        copied_paths.push(entry.relative_path.clone());
                    }
                }
                apply_manifest_mutation(
                    &mut current_manifest,
                    &request.source_manifest,
                    &mutation,
                )?;
            }

            validate_path_binding(&request.fake_device_root, &request.selected_root)?;
            let after_manifest = selected.scan_tree()?;
            if !manifest_content_matches(&after_manifest, &expected_after) {
                return Err(CopyCommitError::Stale(
                    "selected destination did not match the expected post-commit manifest"
                        .to_string(),
                ));
            }
            Ok(after_manifest)
        })();
        let after_manifest = match mutation_result {
            Ok(manifest) => manifest,
            Err(error) => {
                return Ok(fake_post_mutation_report(
                    request,
                    &selected,
                    deleted_paths,
                    copied_paths,
                    error,
                ));
            }
        };
        deleted_paths.sort();
        copied_paths.sort();

        Ok(CopyCommitReport {
            operation: request.operation,
            destination_identity: request.destination_identity.clone(),
            succeeded: true,
            outcome: CopyCommitOutcome::Succeeded,
            before_paths,
            before_manifest: request.destination_manifest.clone(),
            after_paths: manifest_paths(&after_manifest),
            after_manifest: Some(after_manifest),
            reviewed_deletion_set: request.reviewed_deletion_set.clone(),
            deleted_paths,
            copied_paths,
            staged_paths: Vec::new(),
            staging_verified: false,
            destination_binding_verified: true,
            source_digest_sha256,
            staging_cleanup_error: None,
            error: None,
            internal_error: None,
            internal_device_error: None,
        })
    }
}

/// Holds the reviewed source root open and resolves files beneath its
/// descriptor without following symbolic links.
struct SourceTreeReader {
    #[cfg(unix)]
    root: File,
}

impl SourceTreeReader {
    fn open(source_root: &Path) -> Result<Self, CopyCommitError> {
        #[cfg(unix)]
        {
            let metadata = fs::symlink_metadata(source_root).map_err(|error| {
                CopyCommitError::Io(format!("inspect reviewed source root: {error}"))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(CopyCommitError::Stale(
                    "reviewed source root is no longer a regular directory".to_string(),
                ));
            }
            let canonical_root = fs::canonicalize(source_root).map_err(|error| {
                CopyCommitError::Io(format!("resolve reviewed source root: {error}"))
            })?;
            Ok(Self {
                root: open_absolute_directory_nofollow(&canonical_root)?,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = source_root;
            Err(no_follow_unavailable())
        }
    }

    fn open_file(&self, relative_path: &str) -> Result<File, CopyCommitError> {
        #[cfg(unix)]
        {
            let components = relative_components(relative_path)?;
            let (leaf, parents) = components.split_last().ok_or_else(|| {
                CopyCommitError::Rejected(format!(
                    "unsafe reviewed source relative path {relative_path:?}"
                ))
            })?;
            let mut parent = self
                .root
                .try_clone()
                .map_err(|error| CopyCommitError::Io(format!("duplicate source root: {error}")))?;
            for component in parents {
                let name = component_cstring(component)?;
                parent = open_child_directory(parent.as_raw_fd(), &name).map_err(|error| {
                    CopyCommitError::Stale(format!(
                        "reviewed source parent changed for {relative_path}: {error}"
                    ))
                })?;
            }
            let name = component_cstring(leaf)?;
            let file = open_child_file(parent.as_raw_fd(), &name).map_err(|error| {
                CopyCommitError::Stale(format!(
                    "reviewed source entry changed for {relative_path}: {error}"
                ))
            })?;
            if !file
                .metadata()
                .map_err(|error| CopyCommitError::Io(format!("inspect source entry: {error}")))?
                .is_file()
            {
                return Err(CopyCommitError::Stale(format!(
                    "reviewed source entry is no longer a regular file: {relative_path}"
                )));
            }
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            let _ = (self, relative_path);
            Err(no_follow_unavailable())
        }
    }
}

/// Holds the selected directory open and applies child operations through its
/// descriptor. Each child component is opened without following symbolic links.
#[cfg(unix)]
struct SelectedRootMutator {
    root: std::fs::File,
}

#[cfg(unix)]
impl SelectedRootMutator {
    #[cfg(test)]
    fn open(selected_root: &Path) -> Result<Self, CopyCommitError> {
        let canonical = fs::canonicalize(selected_root)
            .map_err(|error| CopyCommitError::Io(format!("resolve selected root: {error}")))?;
        Ok(Self {
            root: open_absolute_directory_nofollow(&canonical)?,
        })
    }

    fn open_or_create(fake_root: &Path, selected_root: &Path) -> Result<Self, CopyCommitError> {
        let relative = selected_root.strip_prefix(fake_root).map_err(|_| {
            CopyCommitError::Rejected("selected root escaped the fake device root".to_string())
        })?;
        let components = normal_components(relative)?;
        if components.is_empty() {
            return Err(CopyCommitError::Rejected(
                "selected root cannot be the fake device root".to_string(),
            ));
        }

        let canonical_fake_root = fs::canonicalize(fake_root)
            .map_err(|error| CopyCommitError::Io(format!("resolve fake device root: {error}")))?;
        let mut current = open_absolute_directory_nofollow(&canonical_fake_root)?;
        for component in components {
            let name = component_cstring(component)?;
            let mkdir_result = unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o755) };
            if mkdir_result != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(CopyCommitError::Io(format!(
                        "create selected-root component: {error}"
                    )));
                }
            }
            let next = open_child_directory(current.as_raw_fd(), &name).map_err(|error| {
                CopyCommitError::Rejected(format!(
                    "selected-root component is not a no-follow directory: {error}"
                ))
            })?;
            current = next;
        }
        Ok(Self { root: current })
    }

    fn scan_tree(&self) -> Result<Vec<CopyManifestEntry>, CopyCommitError> {
        fn visit(
            directory_fd: std::os::fd::RawFd,
            prefix: &str,
            entries: &mut Vec<CopyManifestEntry>,
        ) -> Result<(), CopyCommitError> {
            // openat(".") creates an independent directory stream offset.
            let duplicate = unsafe {
                libc::openat(
                    directory_fd,
                    c".".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if duplicate < 0 {
                return Err(CopyCommitError::Io(format!(
                    "duplicate selected-root directory: {}",
                    std::io::Error::last_os_error()
                )));
            }
            let stream = unsafe { libc::fdopendir(duplicate) };
            if stream.is_null() {
                unsafe { libc::close(duplicate) };
                return Err(CopyCommitError::Io(format!(
                    "read selected-root directory: {}",
                    std::io::Error::last_os_error()
                )));
            }

            let mut children = Vec::new();
            loop {
                unsafe { *errno_location() = 0 };
                let entry = unsafe { libc::readdir(stream) };
                if entry.is_null() {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error().is_some_and(|code| code != 0) {
                        unsafe { libc::closedir(stream) };
                        return Err(CopyCommitError::Io(format!(
                            "read selected-root directory entry: {error}"
                        )));
                    }
                    break;
                }
                let name_bytes =
                    unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()).to_bytes() };
                if name_bytes == b"." || name_bytes == b".." {
                    continue;
                }
                let name = match std::str::from_utf8(name_bytes) {
                    Ok(name) => name,
                    Err(_) => {
                        unsafe { libc::closedir(stream) };
                        return Err(CopyCommitError::Rejected(
                            "selected tree contains a non-UTF-8 path component".to_string(),
                        ));
                    }
                };
                children.push(name.to_string());
            }
            unsafe { libc::closedir(stream) };
            children.sort();

            for name in children {
                let component = component_cstring(std::ffi::OsStr::new(&name))?;
                let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
                if unsafe {
                    libc::fstatat(
                        directory_fd,
                        component.as_ptr(),
                        &mut metadata,
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                } != 0
                {
                    return Err(CopyCommitError::Stale(format!(
                        "selected entry changed during inspection: {}",
                        std::io::Error::last_os_error()
                    )));
                }
                let relative = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                if metadata.st_mode & libc::S_IFMT == libc::S_IFLNK {
                    return Err(CopyCommitError::Rejected(format!(
                        "selected tree contains a symbolic link: {relative}"
                    )));
                }
                if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
                    entries.push(CopyManifestEntry {
                        relative_path: relative.clone(),
                        kind: CopyEntryKind::Directory,
                        sha256: None,
                        entry_identity: Some(format!("{}:{}", metadata.st_dev, metadata.st_ino)),
                    });
                    let child =
                        open_child_directory(directory_fd, &component).map_err(|error| {
                            CopyCommitError::Stale(format!(
                                "selected directory changed during inspection: {error}"
                            ))
                        })?;
                    visit(child.as_raw_fd(), &relative, entries)?;
                } else if metadata.st_mode & libc::S_IFMT == libc::S_IFREG {
                    let file = open_child_file(directory_fd, &component).map_err(|error| {
                        CopyCommitError::Stale(format!(
                            "selected file changed during inspection: {error}"
                        ))
                    })?;
                    entries.push(CopyManifestEntry {
                        relative_path: relative,
                        kind: CopyEntryKind::File,
                        sha256: Some(hash_open_file(file)?),
                        entry_identity: Some(format!("{}:{}", metadata.st_dev, metadata.st_ino)),
                    });
                } else {
                    return Err(CopyCommitError::Rejected(format!(
                        "selected tree contains an unsupported object: {relative}"
                    )));
                }
            }
            Ok(())
        }

        let mut entries = Vec::new();
        visit(self.root.as_raw_fd(), "", &mut entries)?;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(entries)
    }

    fn ensure_directory(&self, relative_path: &str) -> Result<(), CopyCommitError> {
        let parts = relative_components(relative_path)?;
        let mut current = self
            .root
            .try_clone()
            .map_err(|error| CopyCommitError::Io(format!("duplicate selected root: {error}")))?;
        for component in parts {
            let name = component_cstring(component)?;
            let result = unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o755) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(CopyCommitError::Io(format!(
                        "create selected directory {relative_path}: {error}"
                    )));
                }
            }
            current = open_child_directory(current.as_raw_fd(), &name).map_err(|error| {
                CopyCommitError::Stale(format!(
                    "selected directory {relative_path} is not a no-follow directory: {error}"
                ))
            })?;
        }
        Ok(())
    }

    fn remove_file(&self, relative_path: &str) -> Result<(), CopyCommitError> {
        self.remove_entry(relative_path, false)
    }

    fn remove_directory(&self, relative_path: &str) -> Result<(), CopyCommitError> {
        self.remove_entry(relative_path, true)
    }

    fn remove_entry(&self, relative_path: &str, directory: bool) -> Result<(), CopyCommitError> {
        let (parent, name) = self.open_parent(relative_path)?;
        let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                &mut metadata,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(CopyCommitError::Stale(format!(
                "reviewed deletion {relative_path}: {}",
                std::io::Error::last_os_error()
            )));
        }
        let expected_type = if directory {
            libc::S_IFDIR
        } else {
            libc::S_IFREG
        };
        if metadata.st_mode & libc::S_IFMT != expected_type {
            return Err(CopyCommitError::Stale(format!(
                "reviewed deletion changed type: {relative_path}"
            )));
        }
        let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } != 0 {
            return Err(CopyCommitError::Io(format!(
                "remove reviewed entry {relative_path}: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }

    fn publish_file(
        &self,
        source_tree: &SourceTreeReader,
        relative_path: &str,
        expected_sha256: &str,
    ) -> Result<(), std::io::Error> {
        let (parent, name) = self
            .open_parent(relative_path)
            .map_err(copy_commit_error_to_io)?;
        let mut source_file = source_tree
            .open_file(relative_path)
            .map_err(copy_commit_error_to_io)?;
        let mut target = None;
        let mut temporary_name = None;
        for attempt in 0..32u32 {
            let candidate = CString::new(format!(".emuchef-copy-{}-{attempt}", std::process::id()))
                .map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid temp name")
                })?;
            let descriptor = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    candidate.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if descriptor >= 0 {
                target = Some(unsafe { std::fs::File::from_raw_fd(descriptor) });
                temporary_name = Some(candidate);
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        let mut target = target.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "temporary name exhausted",
            )
        })?;
        let temporary_name = temporary_name.expect("temporary name accompanies open file");
        let copy_result = copy_reviewed_file(
            &mut source_file,
            &mut target,
            expected_sha256,
            relative_path,
        )
        .map_err(copy_commit_error_to_io)
        .and_then(|_| target.sync_all());
        if let Err(error) = copy_result {
            unsafe { libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0) };
            return Err(error);
        }
        drop(target);
        if unsafe {
            libc::renameat(
                parent.as_raw_fd(),
                temporary_name.as_ptr(),
                parent.as_raw_fd(),
                name.as_ptr(),
            )
        } != 0
        {
            let error = std::io::Error::last_os_error();
            unsafe { libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0) };
            return Err(error);
        }
        if unsafe { libc::fsync(parent.as_raw_fd()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn open_parent(
        &self,
        relative_path: &str,
    ) -> Result<(std::fs::File, CString), CopyCommitError> {
        let components = relative_components(relative_path)?;
        let (leaf, parents) = components.split_last().ok_or_else(|| {
            CopyCommitError::Rejected(format!("unsafe selected relative path {relative_path:?}"))
        })?;
        let mut current = self
            .root
            .try_clone()
            .map_err(|error| CopyCommitError::Io(format!("duplicate selected root: {error}")))?;
        for component in parents {
            let name = component_cstring(component)?;
            current = open_child_directory(current.as_raw_fd(), &name).map_err(|error| {
                CopyCommitError::Stale(format!(
                    "selected path parent changed for {relative_path}: {error}"
                ))
            })?;
        }
        Ok((current, component_cstring(leaf)?))
    }
}

#[cfg(unix)]
fn open_absolute_directory_nofollow(path: &Path) -> Result<std::fs::File, CopyCommitError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    if !path.is_absolute() {
        return Err(CopyCommitError::Rejected(
            "selected root must be absolute".to_string(),
        ));
    }
    let root = CString::new("/").expect("root path has no NUL");
    let root_descriptor = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if root_descriptor < 0 {
        return Err(CopyCommitError::Io(format!(
            "open filesystem root: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut current = unsafe { std::fs::File::from_raw_fd(root_descriptor) };
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let name = component_cstring(name)?;
                current = open_child_directory(current.as_raw_fd(), &name).map_err(|error| {
                    CopyCommitError::Rejected(format!(
                        "selected root crosses a non-directory or symbolic link: {error}"
                    ))
                })?;
            }
            _ => {
                return Err(CopyCommitError::Rejected(
                    "selected root contains a non-normal component".to_string(),
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(unix)]
fn open_child_directory(
    parent: std::os::fd::RawFd,
    name: &CString,
) -> std::io::Result<std::fs::File> {
    use std::os::fd::FromRawFd;

    let descriptor = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { std::fs::File::from_raw_fd(descriptor) })
    }
}

#[cfg(unix)]
fn open_child_file(parent: std::os::fd::RawFd, name: &CString) -> std::io::Result<std::fs::File> {
    use std::os::fd::FromRawFd;

    let descriptor = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { std::fs::File::from_raw_fd(descriptor) })
    }
}

#[cfg(unix)]
fn relative_components(path: &str) -> Result<Vec<&std::ffi::OsStr>, CopyCommitError> {
    let path = Path::new(path);
    normal_components(path)
}

#[cfg(unix)]
fn normal_components(path: &Path) -> Result<Vec<&std::ffi::OsStr>, CopyCommitError> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => components.push(name),
            _ => {
                return Err(CopyCommitError::Rejected(
                    "selected-root-relative path contains a non-normal component".to_string(),
                ));
            }
        }
    }
    Ok(components)
}

#[cfg(unix)]
fn component_cstring(component: &std::ffi::OsStr) -> Result<CString, CopyCommitError> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(component.as_bytes())
        .map_err(|_| CopyCommitError::Rejected("selected path contains a NUL byte".to_string()))
}

#[cfg(unix)]
fn hash_open_file(mut file: std::fs::File) -> Result<String, CopyCommitError> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| CopyCommitError::Io(format!("read selected file: {error}")))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex_digest(&hasher.finalize()))
}

#[cfg(unix)]
fn copy_commit_error_to_io(error: CopyCommitError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
unsafe fn errno_location() -> *mut libc::c_int {
    libc::__errno_location()
}

#[cfg(target_os = "macos")]
unsafe fn errno_location() -> *mut libc::c_int {
    libc::__error()
}

#[cfg(not(unix))]
struct SelectedRootMutator;

#[cfg(not(unix))]
impl SelectedRootMutator {
    fn open(_selected_root: &Path) -> Result<Self, CopyCommitError> {
        Err(no_follow_unavailable())
    }

    fn open_or_create(_fake_root: &Path, _selected_root: &Path) -> Result<Self, CopyCommitError> {
        Err(no_follow_unavailable())
    }

    fn scan_tree(&self) -> Result<Vec<CopyManifestEntry>, CopyCommitError> {
        Err(no_follow_unavailable())
    }

    fn ensure_directory(&self, _relative_path: &str) -> Result<(), CopyCommitError> {
        Err(no_follow_unavailable())
    }

    fn remove_file(&self, _relative_path: &str) -> Result<(), CopyCommitError> {
        Err(no_follow_unavailable())
    }

    fn remove_directory(&self, _relative_path: &str) -> Result<(), CopyCommitError> {
        Err(no_follow_unavailable())
    }

    fn publish_file(&self, _source: &Path, _relative_path: &str) -> Result<(), std::io::Error> {
        Err(std::io::Error::other(no_follow_unavailable().to_string()))
    }
}

#[cfg(not(unix))]
fn no_follow_unavailable() -> CopyCommitError {
    CopyCommitError::Rejected(
        "selected-root mutation requires an openat no-follow filesystem primitive".to_string(),
    )
}

fn selected_path(root: &Path, device_path: &str) -> Result<PathBuf, CopyCommitError> {
    let path = Path::new(device_path);
    if !path.is_absolute() {
        return Err(CopyCommitError::Rejected(
            "selected destination identity must be absolute".to_string(),
        ));
    }
    let mut selected = root.to_path_buf();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => selected.push(part),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(CopyCommitError::Rejected(
                    "selected destination identity contains traversal components".to_string(),
                ));
            }
        }
    }
    Ok(selected)
}

fn validate_path_binding(root: &Path, selected: &Path) -> Result<(), CopyCommitError> {
    let relative = selected.strip_prefix(root).map_err(|_| {
        CopyCommitError::Rejected("selected destination escaped the fake device root".to_string())
    })?;
    let mut current = root.to_path_buf();
    if fs::symlink_metadata(root)
        .map_err(|error| CopyCommitError::Io(format!("inspect fake root: {error}")))?
        .file_type()
        .is_symlink()
    {
        return Err(CopyCommitError::Rejected(
            "fake device root is a symbolic link".to_string(),
        ));
    }
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(CopyCommitError::Rejected(
                "selected destination contains a non-normal path component".to_string(),
            ));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CopyCommitError::Rejected(format!(
                    "selected destination crosses a symbolic link: {}",
                    current.display()
                )));
            }
            Ok(metadata) if current != selected && !metadata.is_dir() => {
                return Err(CopyCommitError::Rejected(format!(
                    "selected destination parent is not a directory: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(CopyCommitError::Io(format!(
                    "inspect selected path: {error}"
                )))
            }
        }
    }
    Ok(())
}

fn scan_tree(root: &Path) -> Result<Vec<CopyManifestEntry>, CopyCommitError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(CopyCommitError::Rejected(format!(
                "tree root is a symbolic link: {}",
                root.display()
            )));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(CopyCommitError::Rejected(format!(
                "tree root is not a directory: {}",
                root.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(CopyCommitError::Io(format!("inspect tree root: {error}"))),
    }

    fn visit(
        root: &Path,
        current: &Path,
        entries: &mut Vec<CopyManifestEntry>,
    ) -> Result<(), CopyCommitError> {
        let mut children = fs::read_dir(current)
            .map_err(|error| {
                CopyCommitError::Io(format!("read tree {}: {error}", current.display()))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| CopyCommitError::Io(format!("read tree entry: {error}")))?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            let relative = checked_relative(path.strip_prefix(root).map_err(|_| {
                CopyCommitError::Rejected("tree entry escaped its selected root".to_string())
            })?)?;
            let file_type = child
                .file_type()
                .map_err(|error| CopyCommitError::Io(format!("inspect {relative}: {error}")))?;
            if file_type.is_symlink() {
                return Err(CopyCommitError::Rejected(format!(
                    "tree contains a symbolic link: {relative}"
                )));
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| CopyCommitError::Io(format!("inspect {relative}: {error}")))?;
            let entry_identity = filesystem_identity(&metadata);
            if file_type.is_dir() {
                entries.push(CopyManifestEntry {
                    relative_path: relative,
                    kind: CopyEntryKind::Directory,
                    sha256: None,
                    entry_identity,
                });
                visit(root, &path, entries)?;
            } else if file_type.is_file() {
                entries.push(CopyManifestEntry {
                    relative_path: relative,
                    kind: CopyEntryKind::File,
                    sha256: Some(hash_file(&path)?),
                    entry_identity,
                });
            } else {
                return Err(CopyCommitError::Rejected(format!(
                    "tree contains an unsupported object: {relative}"
                )));
            }
        }
        Ok(())
    }

    let mut entries = Vec::new();
    visit(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(entries)
}

fn checked_relative(path: &Path) -> Result<String, CopyCommitError> {
    let mut components = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(CopyCommitError::Rejected(
                "manifest path contains traversal or root components".to_string(),
            ));
        };
        let name = name.to_str().ok_or_else(|| {
            CopyCommitError::Rejected("manifest path is not valid UTF-8".to_string())
        })?;
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            return Err(CopyCommitError::Rejected(format!(
                "manifest path contains an unsafe component {name:?}"
            )));
        }
        components.push(name);
    }
    if components.is_empty() {
        return Err(CopyCommitError::Rejected(
            "manifest path is empty".to_string(),
        ));
    }
    Ok(components.join("/"))
}

fn checked_child(root: &Path, relative_path: &str) -> Result<PathBuf, CopyCommitError> {
    let mut child = root.to_path_buf();
    for component in Path::new(relative_path).components() {
        let Component::Normal(name) = component else {
            return Err(CopyCommitError::Rejected(format!(
                "unsafe reviewed relative path {relative_path:?}"
            )));
        };
        child.push(name);
    }
    if child.strip_prefix(root).is_err() || child == root {
        return Err(CopyCommitError::Rejected(format!(
            "reviewed path escaped its selected tree: {relative_path:?}"
        )));
    }
    Ok(child)
}

fn materialize_simulation_destination(request: &CopyCommitRequest) -> Result<(), CopyCommitError> {
    validate_manifest(&request.destination_manifest)?;
    validate_path_binding(&request.fake_device_root, &request.selected_root)?;

    if request.selected_root.exists() {
        let existing = scan_tree(&request.selected_root)?;
        if !existing.is_empty() {
            return Err(CopyCommitError::Stale(
                "simulation destination was not empty before baseline seeding".to_string(),
            ));
        }
    } else {
        fs::create_dir_all(&request.selected_root).map_err(|error| {
            CopyCommitError::Io(format!(
                "create simulation destination {}: {error}",
                request.selected_root.display()
            ))
        })?;
    }
    validate_path_binding(&request.fake_device_root, &request.selected_root)?;

    let mut entries = request.destination_manifest.iter().collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        (
            entry.relative_path.matches('/').count(),
            entry.relative_path.as_str(),
        )
    });
    for entry in entries {
        let path = checked_child(&request.selected_root, &entry.relative_path)?;
        match entry.kind {
            CopyEntryKind::Directory => fs::create_dir_all(&path).map_err(|error| {
                CopyCommitError::Io(format!(
                    "create simulation directory {}: {error}",
                    path.display()
                ))
            })?,
            CopyEntryKind::File => {
                let parent = path.parent().ok_or_else(|| {
                    CopyCommitError::Rejected(format!(
                        "simulation file has no parent: {}",
                        path.display()
                    ))
                })?;
                fs::create_dir_all(parent).map_err(|error| {
                    CopyCommitError::Io(format!(
                        "create simulation parent {}: {error}",
                        parent.display()
                    ))
                })?;
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|error| {
                        CopyCommitError::Io(format!(
                            "create simulation file {}: {error}",
                            path.display()
                        ))
                    })?;
            }
        }
    }

    validate_path_binding(&request.fake_device_root, &request.selected_root)?;
    let simulation_manifest = scan_tree(&request.selected_root)?;
    let mut reviewed_shape = request
        .destination_manifest
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry.kind))
        .collect::<Vec<_>>();
    let mut simulation_shape = simulation_manifest
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry.kind))
        .collect::<Vec<_>>();
    reviewed_shape.sort_by_key(|(path, _)| *path);
    simulation_shape.sort_by_key(|(path, _)| *path);
    if simulation_shape != reviewed_shape {
        return Err(CopyCommitError::Rejected(
            "simulation baseline could not reproduce reviewed destination paths and kinds"
                .to_string(),
        ));
    }
    Ok(())
}

fn reviewed_deletions(
    operation: CopyCommitOperation,
    source: &[CopyManifestEntry],
    destination: &[CopyManifestEntry],
) -> Result<Vec<String>, CopyCommitError> {
    let source_by_path: BTreeMap<_, _> = source
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry.kind))
        .collect();
    let mut deletions = Vec::new();
    for entry in destination {
        let has_source_file_ancestor = entry
            .relative_path
            .split('/')
            .scan(String::new(), |prefix, component| {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(component);
                Some(prefix.clone())
            })
            .take_while(|prefix| prefix != &entry.relative_path)
            .any(|prefix| source_by_path.get(prefix.as_str()) == Some(&CopyEntryKind::File));
        let compatible = source_by_path.get(entry.relative_path.as_str()) == Some(&entry.kind);
        let should_delete = match operation {
            CopyCommitOperation::AddUpdate => false,
            CopyCommitOperation::Replace => true,
            CopyCommitOperation::Mirror => !compatible || has_source_file_ancestor,
        };
        if should_delete {
            deletions.push(entry.relative_path.clone());
        }
    }
    if operation == CopyCommitOperation::AddUpdate {
        validate_add_update_types(source, destination)?;
    }
    deletions.sort();
    Ok(deletions)
}

fn validate_add_update_types(
    source: &[CopyManifestEntry],
    destination: &[CopyManifestEntry],
) -> Result<(), CopyCommitError> {
    let destination_by_path: BTreeMap<_, _> = destination
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry.kind))
        .collect();
    for entry in source {
        if destination_by_path
            .get(entry.relative_path.as_str())
            .is_some_and(|existing| existing != &entry.kind)
        {
            return Err(CopyCommitError::Rejected(format!(
                "Add/Update cannot change the object kind at {:?}",
                entry.relative_path
            )));
        }
        let mut parent = Path::new(&entry.relative_path).parent();
        while let Some(parent_path) = parent {
            if parent_path.as_os_str().is_empty() {
                break;
            }
            let parent_relative = checked_relative(parent_path)?;
            if destination_by_path.get(parent_relative.as_str()) == Some(&CopyEntryKind::File) {
                return Err(CopyCommitError::Rejected(format!(
                    "Add/Update destination parent is a file: {parent_relative:?}"
                )));
            }
            parent = parent_path.parent();
        }
    }
    Ok(())
}

fn expected_manifest(
    request: &CopyCommitRequest,
) -> Result<Vec<CopyManifestEntry>, CopyCommitError> {
    match request.operation {
        CopyCommitOperation::Replace | CopyCommitOperation::Mirror => {
            Ok(request.source_manifest.clone())
        }
        CopyCommitOperation::AddUpdate => {
            let mut merged: BTreeMap<String, CopyManifestEntry> = request
                .destination_manifest
                .iter()
                .cloned()
                .map(|entry| (entry.relative_path.clone(), entry))
                .collect();
            for entry in &request.source_manifest {
                merged.insert(entry.relative_path.clone(), entry.clone());
            }
            Ok(merged.into_values().collect())
        }
    }
}

fn mutation_entry_precondition(
    manifest: &[CopyManifestEntry],
    relative_path: &str,
) -> Result<String, CopyCommitError> {
    let Some(entry) = manifest
        .iter()
        .find(|entry| entry.relative_path == relative_path)
    else {
        return Ok("absent".to_string());
    };
    let identity = entry.entry_identity.as_deref().ok_or_else(|| {
        CopyCommitError::Rejected(format!(
            "reviewed destination entry lacks a mutation identity: {relative_path}"
        ))
    })?;
    match entry.kind {
        CopyEntryKind::Directory => Ok(format!("directory|{identity}")),
        CopyEntryKind::File => {
            let digest = entry.sha256.as_deref().ok_or_else(|| {
                CopyCommitError::Rejected(format!(
                    "reviewed destination file lacks a digest: {relative_path}"
                ))
            })?;
            Ok(format!("file|{identity}|{digest}"))
        }
    }
}

fn apply_manifest_mutation(
    manifest: &mut Vec<CopyManifestEntry>,
    source_manifest: &[CopyManifestEntry],
    mutation: &CopyCommitMutation,
) -> Result<(), CopyCommitError> {
    let (relative_path, replacement) = match mutation {
        CopyCommitMutation::RemoveFile { relative_path }
        | CopyCommitMutation::RemoveDirectory { relative_path } => (relative_path, None),
        CopyCommitMutation::EnsureDirectory { relative_path } => {
            let already_present = manifest
                .iter()
                .any(|entry| entry.relative_path == *relative_path);
            if already_present {
                return Ok(());
            }
            (
                relative_path,
                Some(CopyManifestEntry {
                    relative_path: relative_path.clone(),
                    kind: CopyEntryKind::Directory,
                    sha256: None,
                    entry_identity: None,
                }),
            )
        }
        CopyCommitMutation::PublishFile { relative_path, .. } => {
            let replacement = source_manifest
                .iter()
                .find(|entry| entry.relative_path == *relative_path)
                .cloned()
                .ok_or_else(|| {
                    CopyCommitError::Rejected(format!(
                        "published path is absent from the reviewed source manifest: {relative_path}"
                    ))
                })?;
            (relative_path, Some(replacement))
        }
    };
    manifest.retain(|entry| entry.relative_path != *relative_path);
    if let Some(replacement) = replacement {
        manifest.push(replacement);
    }
    manifest.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(())
}

fn manifest_paths(manifest: &[CopyManifestEntry]) -> Vec<String> {
    manifest
        .iter()
        .map(|entry| entry.relative_path.clone())
        .collect()
}

fn validate_manifest(manifest: &[CopyManifestEntry]) -> Result<(), CopyCommitError> {
    let mut kinds = BTreeMap::new();
    for entry in manifest {
        if checked_relative(Path::new(&entry.relative_path))? != entry.relative_path {
            return Err(CopyCommitError::Rejected(format!(
                "manifest path is not canonical: {:?}",
                entry.relative_path
            )));
        }
        if kinds
            .insert(entry.relative_path.clone(), entry.kind)
            .is_some()
        {
            return Err(CopyCommitError::Rejected(format!(
                "manifest contains duplicate path {:?}",
                entry.relative_path
            )));
        }
        match entry.kind {
            CopyEntryKind::Directory if entry.sha256.is_some() => {
                return Err(CopyCommitError::Rejected(format!(
                    "directory manifest entry has a file digest: {:?}",
                    entry.relative_path
                )));
            }
            CopyEntryKind::File => {
                let digest = entry.sha256.as_deref().ok_or_else(|| {
                    CopyCommitError::Rejected(format!(
                        "file manifest entry lacks a digest: {:?}",
                        entry.relative_path
                    ))
                })?;
                if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(CopyCommitError::Rejected(format!(
                        "file manifest entry has an invalid digest: {:?}",
                        entry.relative_path
                    )));
                }
            }
            CopyEntryKind::Directory => {}
        }
    }
    for entry in manifest {
        let components = entry.relative_path.split('/').collect::<Vec<_>>();
        for length in 1..components.len() {
            let parent = components[..length].join("/");
            if kinds.get(&parent) == Some(&CopyEntryKind::File) {
                return Err(CopyCommitError::Rejected(format!(
                    "manifest path {:?} is beneath a file entry",
                    entry.relative_path
                )));
            }
        }
    }
    Ok(())
}

fn manifest_digest(manifest: &[CopyManifestEntry]) -> String {
    let mut digest = Sha256::new();
    for entry in manifest {
        digest.update(entry.relative_path.as_bytes());
        digest.update([0]);
        digest.update(format!("{:?}", entry.kind).as_bytes());
        digest.update([0]);
        if let Some(file_digest) = &entry.sha256 {
            digest.update(file_digest.as_bytes());
        }
        digest.update([0]);
        if let Some(identity) = &entry.entry_identity {
            digest.update(identity.as_bytes());
        }
        digest.update(b"\n");
    }
    hex_digest(&digest.finalize())
}

fn manifest_content_matches(
    observed: &[CopyManifestEntry],
    expected: &[CopyManifestEntry],
) -> bool {
    observed.len() == expected.len()
        && observed.iter().zip(expected).all(|(observed, expected)| {
            observed.relative_path == expected.relative_path
                && observed.kind == expected.kind
                && observed.sha256 == expected.sha256
        })
}

fn filesystem_identity(metadata: &fs::Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn verify_selected_entry(
    selected: &SelectedRootMutator,
    expected_manifest: &[CopyManifestEntry],
    relative_path: &str,
) -> Result<(), CopyCommitError> {
    let expected = expected_manifest
        .iter()
        .find(|entry| entry.relative_path == relative_path);
    let observed_manifest = selected.scan_tree()?;
    let observed = observed_manifest
        .iter()
        .find(|entry| entry.relative_path == relative_path);
    if observed != expected {
        return Err(CopyCommitError::Stale(format!(
            "reviewed destination entry changed after review: {relative_path}"
        )));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, CopyCommitError> {
    let mut file = File::open(path)
        .map_err(|error| CopyCommitError::Io(format!("open {}: {error}", path.display())))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| CopyCommitError::Io(format!("read {}: {error}", path.display())))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex_digest(&digest.finalize()))
}

/// Copies one opened source file and accepts the bytes only when their digest
/// still matches the reviewed manifest entry.
fn copy_reviewed_file(
    source: &mut File,
    destination: &mut File,
    expected_sha256: &str,
    relative_path: &str,
) -> Result<(), CopyCommitError> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer).map_err(|error| {
            CopyCommitError::Io(format!(
                "read reviewed source entry {relative_path}: {error}"
            ))
        })?;
        if read == 0 {
            break;
        }
        destination.write_all(&buffer[..read]).map_err(|error| {
            CopyCommitError::Io(format!(
                "write staged source entry {relative_path}: {error}"
            ))
        })?;
        digest.update(&buffer[..read]);
    }
    if hex_digest(&digest.finalize()) != expected_sha256 {
        return Err(CopyCommitError::Stale(format!(
            "reviewed source bytes changed for {relative_path}"
        )));
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_destination_validation_accepts_generic_absolute_shared_paths() {
        for path in [
            "/sdcard/ROMs",
            "/storage/emulated/0/ROMs",
            "/mnt/media/ROMs",
        ] {
            validate_android_destination(path)
                .unwrap_or_else(|error| panic!("{path:?} should be accepted: {error}"));
        }
    }

    #[test]
    fn fake_adapter_does_not_claim_staging_it_does_not_perform() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let fake_root = temp.path().join("device");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(source.join("payload.bin"), b"payload").unwrap();
        let destination_identity = "/data/user/0/com.emuchef.fixture/files/assets";
        let selected_root = selected_path(&fake_root, destination_identity).unwrap();
        fs::create_dir_all(&selected_root).unwrap();
        let request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            destination_identity,
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();

        let report = FakeFilesystemCopyCommitAdapter
            .commit(&request)
            .expect("fake copy should succeed");

        assert!(!report.staging_verified);
        assert!(report.staged_paths.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn fake_adapter_reports_partial_tree_after_a_post_mutation_failure() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let fake_root = temp.path().join("device");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(source.join("first.txt"), b"first payload").unwrap();
        fs::write(source.join("nested/second.txt"), b"second payload").unwrap();
        let destination_identity = "/data/user/0/com.emuchef.fixture/files/assets";
        let selected_root = selected_path(&fake_root, destination_identity).unwrap();
        fs::create_dir_all(selected_root.join("nested")).unwrap();
        let request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            destination_identity,
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();
        let nested = selected_root.join("nested");
        let mut permissions = fs::metadata(&nested).unwrap().permissions();
        permissions.set_mode(0o555);
        fs::set_permissions(&nested, permissions).unwrap();

        let result = FakeFilesystemCopyCommitAdapter.commit(&request);

        let mut permissions = fs::metadata(&nested).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&nested, permissions).unwrap();
        let report = result.expect("post-mutation failure should return its observed report");
        assert_eq!(report.outcome, CopyCommitOutcome::Partial);
        assert!(!report.succeeded);
        assert_eq!(report.copied_paths, ["first.txt"]);
        assert!(report.deleted_paths.is_empty());
        assert_eq!(report.after_paths, ["first.txt", "nested"]);
        assert!(report.after_manifest.is_some());
        assert!(!report.staging_verified);
        assert!(report.staged_paths.is_empty());
        assert!(report
            .error
            .as_deref()
            .is_some_and(|error| error.contains("nested/second.txt")));
    }

    struct FailAfterDestinationInspectionDevice {
        destination_identity: String,
        destination_manifest: Vec<CopyManifestEntry>,
        staged_manifest: Vec<CopyManifestEntry>,
        destination_inspections: usize,
        fail_destination_inspection_at: Option<usize>,
    }

    impl AndroidCopyCommitDevice for FailAfterDestinationInspectionDevice {
        fn inspect_tree(
            &mut self,
            root: &str,
        ) -> Result<Vec<CopyManifestEntry>, super::super::DeviceOperationError> {
            if root != self.destination_identity {
                return Ok(self.staged_manifest.clone());
            }
            self.destination_inspections += 1;
            if self.fail_destination_inspection_at == Some(self.destination_inspections) {
                return Err(super::super::DeviceOperationError::other(
                    "sensitive after-inspection diagnostic",
                ));
            }
            Ok(self.destination_manifest.clone())
        }

        fn path_binding(
            &mut self,
            _path: &str,
        ) -> Result<String, super::super::DeviceOperationError> {
            Ok("reviewed-binding".to_string())
        }

        fn create_directory(
            &mut self,
            _path: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }

        fn mkdir_p(&mut self, _path: &str) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }

        fn push_file(
            &mut self,
            _source: &Path,
            _destination: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }

        fn copy_file(
            &mut self,
            _source: &str,
            _destination: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            Err(super::super::DeviceOperationError::transport(
                super::super::DeviceOperationKind::DeviceOffline,
            ))
        }

        fn remove_file(&mut self, _path: &str) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }

        fn remove_empty_directory(
            &mut self,
            _path: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }
    }

    #[test]
    fn android_observation_failure_reports_uncertain_without_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let fake_root = temp.path().join("device");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(source.join("payload.bin"), b"payload").unwrap();

        let mut request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            "/data/user/0/com.emuchef.fixture/files/assets",
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();
        request.destination_binding = Some("reviewed-binding".to_string());
        let mut device = FailAfterDestinationInspectionDevice {
            destination_identity: request.destination_identity.clone(),
            destination_manifest: request.destination_manifest.clone(),
            staged_manifest: request.source_manifest.clone(),
            destination_inspections: 0,
            fail_destination_inspection_at: Some(3),
        };
        let mut adapter = AndroidCopyCommitAdapter::new(&mut device);

        let report = adapter.commit(&request).unwrap();
        let external = serde_json::to_value(&report).unwrap();

        assert_eq!(report.outcome, CopyCommitOutcome::Uncertain);
        assert!(report.after_manifest.is_none());
        assert!(
            !external.as_object().unwrap().contains_key("afterManifest"),
            "an unavailable after-manifest must be omitted from the external report"
        );
        assert_eq!(
            external["error"],
            "copy result could not be verified after mutation"
        );
        assert!(
            report
                .internal_error
                .as_deref()
                .unwrap()
                .contains("sensitive after-inspection diagnostic"),
            "the inspection diagnostic must remain available internally"
        );
        assert!(
            !external
                .to_string()
                .contains("sensitive after-inspection diagnostic"),
            "internal inspection diagnostics must not be exposed in reports"
        );
        assert_eq!(
            report
                .internal_device_error
                .as_ref()
                .map(|error| error.kind),
            Some(super::super::DeviceOperationKind::DeviceOffline)
        );
        let failure = super::super::copy_commit_report_failure(report);
        assert_eq!(
            failure.failure_kind,
            Some(super::super::StepFailureKind::DeviceOffline)
        );
        assert_eq!(
            failure.outputs["copy_commit_report"].value["outcome"],
            serde_json::to_value(CopyCommitOutcome::Uncertain).unwrap()
        );
    }

    #[test]
    fn android_observed_destination_failure_reports_partial_with_fatal_kind() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let fake_root = temp.path().join("device");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(source.join("payload.bin"), b"payload").unwrap();

        let mut request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            "/data/user/0/com.emuchef.fixture/files/assets",
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();
        request.destination_binding = Some("reviewed-binding".to_string());
        let mut device = FailAfterDestinationInspectionDevice {
            destination_identity: request.destination_identity.clone(),
            destination_manifest: request.destination_manifest.clone(),
            staged_manifest: request.source_manifest.clone(),
            destination_inspections: 0,
            fail_destination_inspection_at: None,
        };

        let report = AndroidCopyCommitAdapter::new(&mut device)
            .commit(&request)
            .unwrap();
        assert_eq!(report.outcome, CopyCommitOutcome::Partial);
        assert!(report.destination_binding_verified);
        assert_eq!(
            report.after_manifest,
            Some(request.destination_manifest.clone())
        );
        assert_eq!(
            report
                .internal_device_error
                .as_ref()
                .map(|error| error.kind),
            Some(super::super::DeviceOperationKind::DeviceOffline)
        );

        let failure = super::super::copy_commit_report_failure(report);
        assert_eq!(
            failure.failure_kind,
            Some(super::super::StepFailureKind::DeviceOffline)
        );
        assert_eq!(
            failure.outputs["copy_commit_report"].value["outcome"],
            serde_json::to_value(CopyCommitOutcome::Partial).unwrap()
        );
        assert!(failure.outputs["copy_commit_report"].value["afterManifest"].is_array());
    }

    #[test]
    fn reviewed_mirror_deletes_only_destination_entries_outside_the_source_tree() {
        let temp = tempfile::tempdir().unwrap();
        let fake_root = temp.path().join("device");
        let source = temp.path().join("source");
        let destination = fake_root.join("data/user/0/fixture/files/assets");
        fs::create_dir_all(source.join("kept")).unwrap();
        fs::create_dir_all(source.join("games")).unwrap();
        fs::create_dir_all(destination.join("stale")).unwrap();
        fs::create_dir_all(destination.join("kept")).unwrap();
        fs::write(source.join("kept/file.bin"), b"source").unwrap();
        fs::write(source.join("games/A.rom"), b"new game").unwrap();
        fs::write(destination.join("kept/file.bin"), b"old").unwrap();
        fs::write(destination.join("stale/file.bin"), b"stale").unwrap();
        fs::write(destination.join("games"), b"old file at directory path").unwrap();

        let request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            "/data/user/0/fixture/files/assets",
            CopyCommitOperation::Mirror,
        )
        .unwrap();

        assert_eq!(
            request.reviewed_deletion_set,
            ["games", "stale", "stale/file.bin"]
        );

        let report = FakeFilesystemCopyCommitAdapter
            .commit(&request)
            .expect("Mirror should replace the file/directory conflict");
        assert!(report.succeeded, "Mirror should complete: {report:#?}");
        assert_eq!(
            report.after_paths,
            manifest_paths(&scan_tree(&source).unwrap())
        );
        assert!(manifest_content_matches(
            &scan_tree(&destination).unwrap(),
            &scan_tree(&source).unwrap()
        ));
    }

    #[test]
    fn android_staging_plan_creates_every_reviewed_directory_before_pushing_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let fake_root = temp.path().join("device");
        fs::create_dir_all(source.join("empty-source")).unwrap();
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(source.join("A.rom"), b"A").unwrap();
        fs::write(source.join("nested/inside.bin"), b"inside").unwrap();
        let request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            "/data/user/0/com.emuchef.fixture/files/assets",
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();

        let stage_root = "/data/local/tmp/emuchef/t03-copy/source";
        let actions = android_stage_actions(&request, stage_root).unwrap();

        assert_eq!(
            actions,
            vec![
                AndroidStageAction::CreateDirectory(stage_root.to_string()),
                AndroidStageAction::CreateDirectory(format!("{stage_root}/empty-source")),
                AndroidStageAction::CreateDirectory(format!("{stage_root}/nested")),
                AndroidStageAction::PushFile {
                    relative_path: "A.rom".to_string(),
                    expected_sha256: hash_file(&source.join("A.rom")).unwrap(),
                    destination: format!("{stage_root}/A.rom"),
                },
                AndroidStageAction::PushFile {
                    relative_path: "nested/inside.bin".to_string(),
                    expected_sha256: hash_file(&source.join("nested/inside.bin")).unwrap(),
                    destination: format!("{stage_root}/nested/inside.bin"),
                },
            ],
            "empty and nested reviewed directories must be created with typed mkdir operations before file pushes"
        );
    }

    struct StagePushFailureDevice {
        destination_identity: String,
        destination_manifest: Vec<CopyManifestEntry>,
        stage_root: Option<String>,
        cleanup_paths: Vec<String>,
        cleanup_fails: bool,
    }

    impl AndroidCopyCommitDevice for StagePushFailureDevice {
        fn inspect_tree(
            &mut self,
            root: &str,
        ) -> Result<Vec<CopyManifestEntry>, super::super::DeviceOperationError> {
            if root == self.destination_identity {
                return Ok(self.destination_manifest.clone());
            }
            if self.stage_root.as_deref() == Some(root) {
                return Ok(Vec::new());
            }
            Err(super::super::DeviceOperationError::other(format!(
                "unexpected tree inspection: {root}"
            )))
        }

        fn path_binding(
            &mut self,
            _path: &str,
        ) -> Result<String, super::super::DeviceOperationError> {
            Ok("reviewed-binding".to_string())
        }

        fn create_directory(
            &mut self,
            path: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            if self.stage_root.is_none() {
                self.stage_root = Some(path.to_string());
            }
            Ok(())
        }

        fn mkdir_p(&mut self, _path: &str) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }

        fn push_file(
            &mut self,
            _source: &Path,
            _destination: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            Err(super::super::DeviceOperationError::other(
                "injected stage push failure",
            ))
        }

        fn copy_file(
            &mut self,
            _source: &str,
            _destination: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            Err(super::super::DeviceOperationError::other(
                "unexpected destination copy",
            ))
        }

        fn remove_file(&mut self, path: &str) -> Result<(), super::super::DeviceOperationError> {
            self.cleanup_paths.push(path.to_string());
            Ok(())
        }

        fn remove_empty_directory(
            &mut self,
            path: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            self.cleanup_paths.push(path.to_string());
            if self.cleanup_fails {
                Err(super::super::DeviceOperationError::other(
                    "injected cleanup failure",
                ))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn android_stage_push_failure_cleans_exact_stage_and_preserves_original_error() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let fake_root = temp.path().join("device");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(source.join("payload.bin"), b"payload").unwrap();
        let mut request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            "/data/user/0/com.emuchef.fixture/files/assets",
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();
        request.destination_binding = Some("reviewed-binding".to_string());
        let mut device = StagePushFailureDevice {
            destination_identity: request.destination_identity.clone(),
            destination_manifest: request.destination_manifest.clone(),
            stage_root: None,
            cleanup_paths: Vec::new(),
            cleanup_fails: false,
        };

        let result = {
            let mut adapter = AndroidCopyCommitAdapter::new(&mut device);
            adapter.commit(&request)
        };

        let stage_root = device.stage_root.as_deref().unwrap();
        assert!(matches!(
            result,
            Err(CopyCommitError::DeviceOperation(error))
                if error.message.contains("injected stage push failure")
        ));
        assert_eq!(device.cleanup_paths, [stage_root]);
    }

    #[test]
    fn android_stage_push_failure_with_cleanup_failure_reports_uncertain_stage_identity() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let fake_root = temp.path().join("device");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(source.join("payload.bin"), b"payload").unwrap();
        let mut request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            "/data/user/0/com.emuchef.fixture/files/assets",
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();
        request.destination_binding = Some("reviewed-binding".to_string());
        let mut device = StagePushFailureDevice {
            destination_identity: request.destination_identity.clone(),
            destination_manifest: request.destination_manifest.clone(),
            stage_root: None,
            cleanup_paths: Vec::new(),
            cleanup_fails: true,
        };

        let report = {
            let mut adapter = AndroidCopyCommitAdapter::new(&mut device);
            adapter
                .commit(&request)
                .expect("failed cleanup must return an uncertain report")
        };

        let stage_root = device.stage_root.as_deref().unwrap();
        assert_eq!(report.outcome, CopyCommitOutcome::Uncertain);
        assert!(report
            .error
            .as_deref()
            .unwrap()
            .contains("injected stage push failure"));
        let cleanup_error = report.staging_cleanup_error.as_deref().unwrap();
        assert!(cleanup_error.contains("cleanup pending"));
        assert!(cleanup_error.contains(stage_root));
        assert!(cleanup_error.contains("injected cleanup failure"));
        assert_eq!(device.cleanup_paths, [stage_root]);
    }

    #[test]
    fn verified_copy_success_remains_succeeded_when_stage_cleanup_is_pending() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let fake_root = temp.path().join("device");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(source.join("payload.bin"), b"payload").unwrap();
        let mut request = CopyCommitRequest::reviewed_directory(
            &source,
            &fake_root,
            "/data/user/0/com.emuchef.fixture/files/assets",
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();
        request.destination_binding = Some("reviewed-binding".to_string());
        let mut successful_report = FakeFilesystemCopyCommitAdapter.commit(&request).unwrap();
        successful_report.staging_verified = true;
        successful_report.destination_binding_verified = true;

        let stage_root = "/data/user/0/com.emuchef.fixture/files/.assets-stage".to_string();
        let mut device = StagePushFailureDevice {
            destination_identity: request.destination_identity.clone(),
            destination_manifest: request.destination_manifest.clone(),
            stage_root: Some(stage_root.clone()),
            cleanup_paths: Vec::new(),
            cleanup_fails: true,
        };

        let report = {
            let mut adapter = AndroidCopyCommitAdapter::new(&mut device);
            adapter
                .finish_staged_result(&request, &stage_root, None, Ok(successful_report))
                .expect("cleanup-only failure should keep the verified report")
        };

        assert!(report.succeeded);
        assert_eq!(report.outcome, CopyCommitOutcome::Succeeded);
        assert!(report.error.is_none());
        let cleanup_error = report.staging_cleanup_error.as_deref().unwrap();
        assert!(cleanup_error.contains("cleanup pending"));
        assert!(cleanup_error.contains(&stage_root));
        assert!(cleanup_error.contains("injected cleanup failure"));
        assert_eq!(device.cleanup_paths, [stage_root]);
    }

    #[test]
    fn copy_commit_device_errors_preserve_fatal_failure_kind() {
        let error =
            CopyCommitError::DeviceOperation(super::super::DeviceOperationError::transport(
                super::super::DeviceOperationKind::DeviceOffline,
            ));

        let failure: super::super::StepFailure = error.into();

        assert_eq!(
            failure.failure_kind,
            Some(super::super::StepFailureKind::DeviceOffline)
        );
        assert!(failure
            .failure_kind
            .is_some_and(super::super::StepFailureKind::requires_device_fail_stop));
    }

    #[test]
    fn failed_copy_commit_reports_remain_structured_and_keep_fatal_kind() {
        for outcome in [CopyCommitOutcome::Partial, CopyCommitOutcome::Uncertain] {
            let report = CopyCommitReport {
                operation: CopyCommitOperation::AddUpdate,
                destination_identity: "/data/user/0/fixture/files/assets".to_string(),
                succeeded: false,
                outcome,
                before_paths: vec!["before.txt".to_string()],
                before_manifest: Vec::new(),
                after_paths: Vec::new(),
                after_manifest: None,
                reviewed_deletion_set: Vec::new(),
                deleted_paths: Vec::new(),
                copied_paths: vec!["new.txt".to_string()],
                staged_paths: vec!["new.txt".to_string()],
                staging_verified: true,
                destination_binding_verified: outcome == CopyCommitOutcome::Partial,
                source_digest_sha256: "source-digest".to_string(),
                staging_cleanup_error: None,
                error: Some("copy result could not be verified after mutation".to_string()),
                internal_error: Some("transport failed after one file was published".to_string()),
                internal_device_error: Some(super::super::DeviceOperationError::transport(
                    super::super::DeviceOperationKind::TransportFailure,
                )),
            };

            let failure = super::super::copy_commit_report_failure(report);

            assert_eq!(
                failure.failure_kind,
                Some(super::super::StepFailureKind::TransportFailure)
            );
            assert!(failure
                .failure_kind
                .is_some_and(super::super::StepFailureKind::requires_device_fail_stop));
            assert_eq!(
                failure.outputs["copy_commit_report"].value["outcome"],
                serde_json::to_value(outcome).unwrap()
            );
            assert_eq!(
                failure.outputs["copy_commit_report"].value["copiedPaths"],
                serde_json::json!(["new.txt"])
            );
        }
    }

    #[cfg(unix)]
    #[derive(Clone, Copy)]
    enum LaterSourceMutation {
        ReplaceBytes,
        ReplaceWithSymlink,
    }

    #[cfg(unix)]
    struct SourceMutationAfterFirstPushDevice {
        destination_identity: String,
        destination_manifest: Vec<CopyManifestEntry>,
        later_source: PathBuf,
        outside_source: PathBuf,
        mutation: LaterSourceMutation,
        stage_root: Option<String>,
        staged_manifest: Vec<CopyManifestEntry>,
        push_count: usize,
        destination_mutation_count: usize,
    }

    #[cfg(unix)]
    impl AndroidCopyCommitDevice for SourceMutationAfterFirstPushDevice {
        fn inspect_tree(
            &mut self,
            root: &str,
        ) -> Result<Vec<CopyManifestEntry>, super::super::DeviceOperationError> {
            if root == self.destination_identity {
                return Ok(self.destination_manifest.clone());
            }
            if self.stage_root.as_deref() == Some(root) {
                return Ok(self.staged_manifest.clone());
            }
            Err(super::super::DeviceOperationError::other(
                "unexpected source-mutation test tree inspection",
            ))
        }

        fn path_binding(
            &mut self,
            _path: &str,
        ) -> Result<String, super::super::DeviceOperationError> {
            Ok("reviewed-binding".to_string())
        }

        fn create_directory(
            &mut self,
            path: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            if self.stage_root.is_none() {
                self.stage_root = Some(path.to_string());
            }
            Ok(())
        }

        fn mkdir_p(&mut self, _path: &str) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }

        fn push_file(
            &mut self,
            source: &Path,
            destination: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            let bytes = fs::read(source)
                .map_err(|error| super::super::DeviceOperationError::other(error.to_string()))?;
            let stage_root = self.stage_root.as_deref().ok_or_else(|| {
                super::super::DeviceOperationError::other("stage root was not created")
            })?;
            let relative_path = Path::new(destination)
                .strip_prefix(stage_root)
                .map_err(|error| super::super::DeviceOperationError::other(error.to_string()))?
                .to_string_lossy()
                .into_owned();
            self.staged_manifest.push(CopyManifestEntry {
                relative_path,
                kind: CopyEntryKind::File,
                sha256: Some(hex_digest(&Sha256::digest(&bytes))),
                entry_identity: None,
            });
            self.push_count += 1;
            if self.push_count == 1 {
                match self.mutation {
                    LaterSourceMutation::ReplaceBytes => {
                        let replacement = self.later_source.with_extension("replacement");
                        fs::write(&replacement, b"unreviewed later source bytes").map_err(
                            |error| super::super::DeviceOperationError::other(error.to_string()),
                        )?;
                        fs::rename(&replacement, &self.later_source).map_err(|error| {
                            super::super::DeviceOperationError::other(error.to_string())
                        })?;
                    }
                    LaterSourceMutation::ReplaceWithSymlink => {
                        use std::os::unix::fs::symlink;

                        fs::remove_file(&self.later_source).map_err(|error| {
                            super::super::DeviceOperationError::other(error.to_string())
                        })?;
                        symlink(&self.outside_source, &self.later_source).map_err(|error| {
                            super::super::DeviceOperationError::other(error.to_string())
                        })?;
                    }
                }
            }
            Ok(())
        }

        fn copy_file(
            &mut self,
            _source: &str,
            _destination: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            self.destination_mutation_count += 1;
            Ok(())
        }

        fn remove_file(&mut self, _path: &str) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }

        fn remove_empty_directory(
            &mut self,
            _path: &str,
        ) -> Result<(), super::super::DeviceOperationError> {
            Ok(())
        }
    }

    #[cfg(unix)]
    fn assert_android_adapter_rejects_later_source_mutation(mutation: LaterSourceMutation) {
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        let fake_root = temp.path().join("device");
        let outside_source = temp.path().join("outside.bin");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&fake_root).unwrap();
        fs::write(
            source_root.join("a-first.bin"),
            b"reviewed first source bytes",
        )
        .unwrap();
        let later_source = source_root.join("z-later.bin");
        fs::write(&later_source, b"reviewed later source bytes").unwrap();
        fs::write(&outside_source, b"unreviewed outside source bytes").unwrap();
        let destination_identity = "/storage/emulated/0/ROMs";
        let selected = selected_path(&fake_root, destination_identity).unwrap();
        fs::create_dir_all(&selected).unwrap();
        let mut request = CopyCommitRequest::reviewed_directory(
            &source_root,
            &fake_root,
            destination_identity,
            CopyCommitOperation::AddUpdate,
        )
        .unwrap();
        request.destination_binding = Some("reviewed-binding".to_string());
        let mut device = SourceMutationAfterFirstPushDevice {
            destination_identity: request.destination_identity.clone(),
            destination_manifest: request.destination_manifest.clone(),
            later_source,
            outside_source: outside_source.clone(),
            mutation,
            stage_root: None,
            staged_manifest: Vec::new(),
            push_count: 0,
            destination_mutation_count: 0,
        };

        let result = AndroidCopyCommitAdapter::new(&mut device).commit(&request);

        assert!(result.is_err(), "source drift must stop the copy commit");
        assert_eq!(
            device.push_count, 1,
            "the changed later source must be rejected before staging"
        );
        assert_eq!(device.destination_mutation_count, 0);
        assert_eq!(scan_tree(&selected).unwrap(), request.destination_manifest);
        if matches!(mutation, LaterSourceMutation::ReplaceWithSymlink) {
            assert_eq!(
                fs::read(outside_source).unwrap(),
                b"unreviewed outside source bytes"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn android_adapter_revalidates_later_source_bytes_before_staging() {
        assert_android_adapter_rejects_later_source_mutation(LaterSourceMutation::ReplaceBytes);
    }

    #[cfg(unix)]
    #[test]
    fn android_adapter_rejects_later_source_symlink_before_staging() {
        assert_android_adapter_rejects_later_source_mutation(
            LaterSourceMutation::ReplaceWithSymlink,
        );
    }

    #[cfg(unix)]
    #[test]
    fn selected_root_mutations_fail_closed_after_nested_symlink_substitution() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let selected_path = temp.path().join("selected");
        let source_root = temp.path().join("source");
        let nested = selected_path.join("nested");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(source_root.join("nested")).unwrap();
        fs::write(nested.join("payload.bin"), b"reviewed destination").unwrap();
        fs::write(outside.join("payload.bin"), b"outside sentinel").unwrap();
        fs::write(source_root.join("nested/payload.bin"), b"replacement").unwrap();
        let source_tree = SourceTreeReader::open(&source_root).unwrap();
        let expected_sha256 = hash_file(&source_root.join("nested/payload.bin")).unwrap();
        let selected = SelectedRootMutator::open(&selected_path).unwrap();

        fs::rename(&nested, selected_path.join("detached")).unwrap();
        symlink(&outside, &nested).unwrap();

        assert!(selected.remove_file("nested/payload.bin").is_err());
        assert!(selected
            .publish_file(&source_tree, "nested/payload.bin", &expected_sha256)
            .is_err());
        assert_eq!(
            fs::read(outside.join("payload.bin")).unwrap(),
            b"outside sentinel"
        );
    }

    #[cfg(unix)]
    #[test]
    fn selected_root_publish_replaces_leaf_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let selected_path = temp.path().join("selected");
        let outside = temp.path().join("outside.bin");
        let source_root = temp.path().join("source");
        fs::create_dir_all(&selected_path).unwrap();
        fs::create_dir_all(&source_root).unwrap();
        fs::write(&outside, b"outside sentinel").unwrap();
        fs::write(source_root.join("payload.bin"), b"replacement").unwrap();
        let source_tree = SourceTreeReader::open(&source_root).unwrap();
        let expected_sha256 = hash_file(&source_root.join("payload.bin")).unwrap();
        let selected = SelectedRootMutator::open(&selected_path).unwrap();
        symlink(&outside, selected_path.join("payload.bin")).unwrap();

        selected
            .publish_file(&source_tree, "payload.bin", &expected_sha256)
            .unwrap();

        assert_eq!(fs::read(&outside).unwrap(), b"outside sentinel");
        assert_eq!(
            fs::read(selected_path.join("payload.bin")).unwrap(),
            b"replacement"
        );
    }

    #[cfg(unix)]
    #[test]
    fn selected_root_publish_rejects_later_source_replaced_after_review() {
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        let selected_path = temp.path().join("selected");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&selected_path).unwrap();
        let first_source = source_root.join("first.bin");
        let later_source = source_root.join("later.bin");
        fs::write(&first_source, b"reviewed first bytes").unwrap();
        fs::write(&later_source, b"reviewed later bytes").unwrap();
        let reviewed_manifest = scan_tree(&source_root).unwrap();
        let reviewed_later_digest = reviewed_manifest
            .iter()
            .find(|entry| entry.relative_path == "later.bin")
            .and_then(|entry| entry.sha256.as_deref())
            .unwrap()
            .to_string();
        let reviewed_first_digest = reviewed_manifest
            .iter()
            .find(|entry| entry.relative_path == "first.bin")
            .and_then(|entry| entry.sha256.as_deref())
            .unwrap()
            .to_string();
        let source_tree = SourceTreeReader::open(&source_root).unwrap();

        let selected = SelectedRootMutator::open(&selected_path).unwrap();
        selected
            .publish_file(&source_tree, "first.bin", &reviewed_first_digest)
            .unwrap();
        fs::write(
            selected_path.join("later.bin"),
            b"existing destination bytes",
        )
        .unwrap();
        let replacement = source_root.join("later.replacement");
        fs::write(&replacement, b"unreviewed later bytes").unwrap();
        fs::rename(&replacement, &later_source).unwrap();

        let result = selected.publish_file(&source_tree, "later.bin", &reviewed_later_digest);

        assert!(
            result.is_err(),
            "a later file whose bytes changed after review must not be published"
        );
        assert_eq!(
            fs::read(selected_path.join("first.bin")).unwrap(),
            b"reviewed first bytes",
            "earlier successful publication remains truthful"
        );
        assert_eq!(
            fs::read(selected_path.join("later.bin")).unwrap(),
            b"existing destination bytes",
            "the changed later file must leave its destination untouched"
        );
        assert_ne!(hash_file(&later_source).unwrap(), reviewed_later_digest);
    }

    #[cfg(unix)]
    #[test]
    fn selected_root_publish_rejects_later_source_symlink_after_review() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        let selected_path = temp.path().join("selected");
        let outside = temp.path().join("outside.bin");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&selected_path).unwrap();
        let first_source = source_root.join("first.bin");
        let later_source = source_root.join("later.bin");
        fs::write(&first_source, b"reviewed first bytes").unwrap();
        fs::write(&later_source, b"reviewed later bytes").unwrap();
        fs::write(&outside, b"unreviewed outside bytes").unwrap();
        let reviewed_manifest = scan_tree(&source_root).unwrap();
        let reviewed_later_digest = reviewed_manifest
            .iter()
            .find(|entry| entry.relative_path == "later.bin")
            .and_then(|entry| entry.sha256.as_deref())
            .unwrap()
            .to_string();
        let reviewed_first_digest = reviewed_manifest
            .iter()
            .find(|entry| entry.relative_path == "first.bin")
            .and_then(|entry| entry.sha256.as_deref())
            .unwrap()
            .to_string();
        let source_tree = SourceTreeReader::open(&source_root).unwrap();

        let selected = SelectedRootMutator::open(&selected_path).unwrap();
        selected
            .publish_file(&source_tree, "first.bin", &reviewed_first_digest)
            .unwrap();
        fs::write(
            selected_path.join("later.bin"),
            b"existing destination bytes",
        )
        .unwrap();
        fs::remove_file(&later_source).unwrap();
        symlink(&outside, &later_source).unwrap();

        let result = selected.publish_file(&source_tree, "later.bin", &reviewed_later_digest);

        assert!(
            result.is_err(),
            "a source leaf replaced by a symlink after review must not be followed"
        );
        assert_eq!(
            fs::read(selected_path.join("later.bin")).unwrap(),
            b"existing destination bytes"
        );
        assert_eq!(fs::read(&outside).unwrap(), b"unreviewed outside bytes");
        assert_ne!(hash_file(&outside).unwrap(), reviewed_later_digest);
    }

    #[cfg(all(unix, feature = "android-copy-commit-helper"))]
    #[test]
    fn compiled_android_helper_rejects_staged_bytes_changed_after_review() {
        use std::os::unix::fs::MetadataExt;
        use std::process::Command;

        let temp = tempfile::tempdir().unwrap();
        let selected_path = temp.path().join("selected");
        let stage_root = temp.path().join("stage");
        fs::create_dir_all(&selected_path).unwrap();
        fs::create_dir_all(&stage_root).unwrap();
        let staged_file = stage_root.join("later.bin");
        fs::write(&staged_file, b"reviewed staged bytes").unwrap();
        let reviewed_digest = hash_file(&staged_file).unwrap();

        let selected_root = fs::canonicalize(&selected_path).unwrap();
        let stage_root = fs::canonicalize(&stage_root).unwrap();
        let parent = fs::metadata(selected_root.parent().unwrap()).unwrap();
        let selected = fs::metadata(&selected_root).unwrap();
        let binding = format!(
            "{}|parent:{}:{}|selected:{}:{}",
            selected_root.display(),
            parent.dev(),
            parent.ino(),
            selected.dev(),
            selected.ino()
        );
        let replacement = stage_root.join("later.replacement");
        fs::write(&replacement, b"unreviewed staged bytes").unwrap();
        fs::rename(&replacement, &staged_file).unwrap();
        assert_ne!(reviewed_digest, hash_file(&staged_file).unwrap());

        let result = Command::new(env!("EMU_CHEF_COPY_COMMIT_HELPER_HOST"))
            .args([
                selected_root.to_str().unwrap(),
                &binding,
                stage_root.to_str().unwrap(),
                "publish",
                "later.bin",
                "absent",
                &reviewed_digest,
            ])
            .output()
            .unwrap();

        assert!(
            !result.status.success(),
            "helper must reject staged bytes that differ from the reviewed digest"
        );
        assert!(!selected_root.join("later.bin").exists());
    }

    #[cfg(all(unix, feature = "android-copy-commit-helper"))]
    #[test]
    fn compiled_android_helper_rejects_staged_source_symlink_after_review() {
        use std::os::unix::fs::{symlink, MetadataExt};
        use std::process::Command;

        let temp = tempfile::tempdir().unwrap();
        let selected_path = temp.path().join("selected");
        let stage_root = temp.path().join("stage");
        let outside = temp.path().join("outside.bin");
        fs::create_dir_all(&selected_path).unwrap();
        fs::create_dir_all(&stage_root).unwrap();
        fs::write(&outside, b"unreviewed outside bytes").unwrap();
        let staged_file = stage_root.join("later.bin");
        fs::write(&staged_file, b"reviewed staged bytes").unwrap();
        let reviewed_digest = hash_file(&staged_file).unwrap();

        let selected_root = fs::canonicalize(&selected_path).unwrap();
        let stage_root = fs::canonicalize(&stage_root).unwrap();
        let parent = fs::metadata(selected_root.parent().unwrap()).unwrap();
        let selected = fs::metadata(&selected_root).unwrap();
        let binding = format!(
            "{}|parent:{}:{}|selected:{}:{}",
            selected_root.display(),
            parent.dev(),
            parent.ino(),
            selected.dev(),
            selected.ino()
        );
        fs::remove_file(&staged_file).unwrap();
        symlink(&outside, &staged_file).unwrap();
        assert_ne!(reviewed_digest, hash_file(&outside).unwrap());

        let result = Command::new(env!("EMU_CHEF_COPY_COMMIT_HELPER_HOST"))
            .args([
                selected_root.to_str().unwrap(),
                &binding,
                stage_root.to_str().unwrap(),
                "publish",
                "later.bin",
                "absent",
                &reviewed_digest,
            ])
            .output()
            .unwrap();

        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("open staged file without following links"),
            "helper must reject a substituted staged symlink; stderr={}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!selected_root.join("later.bin").exists());
        assert_eq!(fs::read(&outside).unwrap(), b"unreviewed outside bytes");
    }

    #[cfg(all(unix, feature = "android-copy-commit-helper"))]
    #[test]
    fn compiled_android_helper_rejects_nested_symlink_substitution() {
        use std::os::unix::fs::{symlink, MetadataExt};
        use std::process::Command;

        let temp = tempfile::tempdir().unwrap();
        let selected_path = temp.path().join("selected");
        let nested = selected_path.join("nested");
        let outside = temp.path().join("outside");
        let stage_root = temp.path().join("stage");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&stage_root).unwrap();
        fs::write(outside.join("payload.bin"), b"outside sentinel").unwrap();

        let selected_root = fs::canonicalize(&selected_path).unwrap();
        let parent = fs::metadata(selected_root.parent().unwrap()).unwrap();
        let selected = fs::metadata(&selected_root).unwrap();
        let binding = format!(
            "{}|parent:{}:{}|selected:{}:{}",
            selected_root.display(),
            parent.dev(),
            parent.ino(),
            selected.dev(),
            selected.ino()
        );

        fs::rename(&nested, selected_root.join("detached")).unwrap();
        symlink(&outside, selected_root.join("nested")).unwrap();
        let result = Command::new(env!("EMU_CHEF_COPY_COMMIT_HELPER_HOST"))
            .args([
                selected_root.to_str().unwrap(),
                &binding,
                stage_root.to_str().unwrap(),
                "remove-file",
                "nested/payload.bin",
                "absent",
            ])
            .output()
            .unwrap();

        assert!(!result.status.success());
        assert_eq!(
            fs::read(outside.join("payload.bin")).unwrap(),
            b"outside sentinel"
        );
    }

    #[cfg(all(unix, feature = "android-copy-commit-helper"))]
    #[test]
    fn compiled_android_helper_rejects_same_path_bytes_changed_after_review() {
        use std::os::unix::fs::MetadataExt;
        use std::process::Command;

        let temp = tempfile::tempdir().unwrap();
        let selected_path = temp.path().join("selected");
        let stage_root = temp.path().join("stage");
        fs::create_dir_all(&selected_path).unwrap();
        fs::create_dir_all(&stage_root).unwrap();
        let destination_file = selected_path.join("B.rom");
        fs::write(&destination_file, b"reviewed old B bytes\n").unwrap();
        fs::write(stage_root.join("B.rom"), b"updated B bytes\n").unwrap();
        let staged_digest = hash_file(&stage_root.join("B.rom")).unwrap();

        let selected_root = fs::canonicalize(&selected_path).unwrap();
        let stage_root = fs::canonicalize(&stage_root).unwrap();
        let parent = fs::metadata(selected_root.parent().unwrap()).unwrap();
        let selected = fs::metadata(&selected_root).unwrap();
        let binding = format!(
            "{}|parent:{}:{}|selected:{}:{}",
            selected_root.display(),
            parent.dev(),
            parent.ino(),
            selected.dev(),
            selected.ino()
        );
        let reviewed_digest = hash_file(&destination_file).unwrap();
        let reviewed_metadata = fs::metadata(&destination_file).unwrap();
        let reviewed_entry = format!(
            "file|{}:{}|{}",
            reviewed_metadata.dev(),
            reviewed_metadata.ino(),
            reviewed_digest
        );
        fs::write(&destination_file, b"interference changed B bytes\n").unwrap();

        let result = Command::new(env!("EMU_CHEF_COPY_COMMIT_HELPER_HOST"))
            .args([
                selected_root.to_str().unwrap(),
                &binding,
                stage_root.to_str().unwrap(),
                "publish",
                "B.rom",
                &reviewed_entry,
                &staged_digest,
            ])
            .output()
            .unwrap();

        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("reviewed destination entry changed after review"),
            "helper must reject entry drift at the mutation boundary; stderr={}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            fs::read(destination_file).unwrap(),
            b"interference changed B bytes\n",
            "the changed reviewed bytes must not be overwritten"
        );
    }

    #[cfg(all(unix, feature = "android-copy-commit-helper"))]
    #[test]
    fn compiled_android_helper_rejects_leaf_symlink_added_after_review() {
        use std::os::unix::fs::{symlink, MetadataExt};
        use std::process::Command;

        let temp = tempfile::tempdir().unwrap();
        let selected_path = temp.path().join("selected");
        let outside = temp.path().join("outside.bin");
        let stage_root = temp.path().join("stage");
        fs::create_dir_all(&selected_path).unwrap();
        fs::create_dir_all(&stage_root).unwrap();
        fs::write(&outside, b"outside sentinel").unwrap();
        fs::write(stage_root.join("payload.bin"), b"reviewed replacement").unwrap();
        let staged_digest = hash_file(&stage_root.join("payload.bin")).unwrap();

        let selected_root = fs::canonicalize(&selected_path).unwrap();
        let stage_root = fs::canonicalize(&stage_root).unwrap();
        let parent = fs::metadata(selected_root.parent().unwrap()).unwrap();
        let selected = fs::metadata(&selected_root).unwrap();
        let binding = format!(
            "{}|parent:{}:{}|selected:{}:{}",
            selected_root.display(),
            parent.dev(),
            parent.ino(),
            selected.dev(),
            selected.ino()
        );
        symlink(&outside, selected_root.join("payload.bin")).unwrap();

        let result = Command::new(env!("EMU_CHEF_COPY_COMMIT_HELPER_HOST"))
            .args([
                selected_root.to_str().unwrap(),
                &binding,
                stage_root.to_str().unwrap(),
                "publish",
                "payload.bin",
                "absent",
                &staged_digest,
            ])
            .output()
            .unwrap();

        assert!(!result.status.success());
        assert_eq!(fs::read(&outside).unwrap(), b"outside sentinel");
        assert!(fs::symlink_metadata(selected_root.join("payload.bin"))
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
