//! Safe executor implementation for dry-run, sandboxed filesystem operations,
//! and real-ADB execution.
//!
//! The real-ADB adapter remains crate-private. Protocol and CLI layers call the
//! executor through typed Rust interfaces, and filesystem effects remain
//! constrained to explicit sandbox roots.

pub mod adb;
pub mod copy_commit;
pub(crate) mod identity;
pub(crate) mod root_authority;
pub(crate) mod root_requirements;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use tempfile::TempPath;

use crate::apk_manifest::inspect_apk_manifest;
use crate::artifact_resolver::{ArtifactResolveRequest, ArtifactResolver};
use crate::executor::copy_commit::{
    AndroidCopyCommitAdapter, CopyCommitAdapter, CopyCommitDestinationKind, CopyCommitError,
    CopyCommitMutation, CopyCommitOperation, CopyCommitRequest, CopyEntryKind, CopyManifestEntry,
    ExecutionCopyCommitSnapshot, FakeFilesystemCopyCommitAdapter,
};
use crate::model::OrderedMap;
use crate::owned_process::ProcessCleanup;
use crate::planner::{
    ExecutionArtifactAppProvenance, ExecutionArtifactSource, ExecutionParamValue, ExecutionPlan,
    ExecutionStep, ExecutionStepCondition, PermissionActionApplicability, PermissionFailurePolicy,
    PermissionPlan, PermissionPlanAction, PermissionSetKind, RuntimeValue, TargetDeviceBinding,
};
use crate::remote_release_resolver::{
    resolve_remote_latest, RemoteReleaseOptions, ResolvedRemoteRelease,
};
use crate::storage_targets::{
    writer_effects_conflict, SetupDestinationPrecondition, SetupLocationMutationLease,
    StorageTargetBinding, StorageTargetVolumeKind, WriterEffect,
};

/// Materialization details resolved from one planned artifact source.
enum MaterializedArtifactSource {
    RemoteFile {
        url: String,
    },
    DirectUrl {
        url: String,
        sha256: Option<String>,
    },
    LocalInput {
        path: PathBuf,
    },
    RemoteRelease {
        url: String,
        provider: String,
        release: ResolvedRemoteRelease,
    },
}

impl MaterializedArtifactSource {
    fn url(&self) -> &str {
        match self {
            Self::RemoteFile { url }
            | Self::DirectUrl { url, .. }
            | Self::RemoteRelease { url, .. } => url,
            Self::LocalInput { .. } => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionRunResult {
    pub success: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub cancelled: bool,
    pub total_steps: usize,
    pub steps: Vec<StepRunRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resolved_releases: Vec<ExecutionResolvedRelease>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resolved_artifacts: Vec<ExecutionResolvedArtifact>,
}

/// Actual release identity selected while materializing an App-owned APK.
/// The download URL is deliberately excluded from the execution result.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResolvedRelease {
    pub recipe_artifact_id: String,
    pub provider: String,
    pub app_id: String,
    pub artifact_id: String,
    pub kind: String,
    pub release_tag: String,
    pub asset_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub timestamp_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub calculated_sha256: String,
    pub cache_hit: bool,
    /// The observed response URL with credentials and query data removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_final_url: Option<String>,
}

/// Exact App Definition direct-URL artifact bytes retained for one execution.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResolvedArtifact {
    /// Fully qualified Recipe artifact identity from the reviewed plan.
    pub recipe_artifact_id: String,
    pub app_id: String,
    pub app_artifact_id: String,
    pub filename: String,
    pub calculated_sha256: String,
    pub cache_hit: bool,
    /// The HTTP response URL with credentials and query/fragment data removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_final_url: Option<String>,
}

fn app_release_details(
    provenance: Option<&ExecutionArtifactAppProvenance>,
    recipe_artifact_id: &str,
    provider: &str,
    release: &ResolvedRemoteRelease,
    resolved: &crate::artifact_resolver::ResolvedArtifact,
) -> Option<ExecutionResolvedRelease> {
    let provenance = provenance?;
    let calculated_sha256 = resolved.calculated_sha256.clone()?;
    Some(ExecutionResolvedRelease {
        recipe_artifact_id: recipe_artifact_id.to_string(),
        provider: provider.to_string(),
        app_id: provenance.app_id.clone(),
        artifact_id: provenance.artifact_id.clone(),
        kind: provenance.kind.clone(),
        release_tag: release.release_tag.clone(),
        asset_name: release.asset_name.clone(),
        published_at: release.published_at.clone(),
        created_at: release.created_at.clone(),
        timestamp_source: release.timestamp_source.to_string(),
        size: release.size,
        calculated_sha256,
        cache_hit: resolved.cache_hit,
        redacted_final_url: resolved.redacted_final_url.clone(),
    })
}

#[cfg(test)]
mod resolved_release_result_tests {
    use super::*;

    #[test]
    fn execution_result_reports_selected_app_release_without_download_url() {
        let provenance = ExecutionArtifactAppProvenance {
            app_id: "armsx1".to_string(),
            artifact_id: "apk".to_string(),
            kind: "apk".to_string(),
        };
        let release = ResolvedRemoteRelease {
            download_url: "https://example.invalid/private-download.apk".to_string(),
            asset_name: "ARMSX1-release-20261008-arm64-v8a.apk".to_string(),
            release_tag: "v20261008".to_string(),
            published_at: Some("2026-10-08T10:00:00Z".to_string()),
            created_at: None,
            timestamp_source: "published_at",
            size: Some(42),
        };
        let resolved = crate::artifact_resolver::ResolvedArtifact {
            local_path: std::path::PathBuf::new(),
            filename: release.asset_name.clone(),
            cache_hit: false,
            calculated_sha256: Some("cd".repeat(32)),
            redacted_final_url: Some("https://example.invalid/download.apk".to_string()),
            verified_path_guard: None,
        };

        let details = app_release_details(
            Some(&provenance),
            "app.armsx1.install/installer",
            "github",
            &release,
            &resolved,
        )
        .unwrap();
        let result = ExecutionRunResult {
            success: true,
            cancelled: false,
            total_steps: 0,
            steps: Vec::new(),
            resolved_releases: vec![details],
            resolved_artifacts: Vec::new(),
        };

        let value = serde_json::to_value(result).unwrap();
        let selected = &value["resolved_releases"][0];
        assert_eq!(selected["appId"], "armsx1");
        assert_eq!(selected["artifactId"], "apk");
        assert_eq!(selected["kind"], "apk");
        assert_eq!(selected["provider"], "github");
        assert_eq!(selected["releaseTag"], "v20261008");
        assert_eq!(
            selected["assetName"],
            "ARMSX1-release-20261008-arm64-v8a.apk"
        );
        assert_eq!(selected["publishedAt"], "2026-10-08T10:00:00Z");
        assert_eq!(selected["size"], 42);
        assert_eq!(selected["calculatedSha256"], "cd".repeat(32));
        assert_eq!(selected["cacheHit"], false);
        assert_eq!(
            selected["redactedFinalUrl"],
            "https://example.invalid/download.apk"
        );
        assert!(selected.get("downloadUrl").is_none());
    }

    #[test]
    fn execution_result_reports_generic_file_release_provenance_without_inventing_published_at() {
        let provenance = ExecutionArtifactAppProvenance {
            app_id: "example-app".to_string(),
            artifact_id: "data".to_string(),
            kind: "file".to_string(),
        };
        let release_metadata = serde_json::json!([{
            "draft": false,
            "prerelease": false,
            "tag_name": "v1",
            "created_at": "2026-01-01T00:00:00Z",
            "assets": [{
                "name": "data.zip",
                "browser_download_url": "https://forge.example.net/data.zip"
            }]
        }]);
        let release = crate::remote_release_resolver::resolve_remote_latest_from_fixture(
            "forgejo",
            "https://forge.example.net",
            "owner/repo",
            "file",
            crate::remote_release_resolver::RemoteReleaseOptions {
                include_prereleases: false,
                asset_pattern: None,
                invert_asset_pattern: false,
            },
            &release_metadata,
        )
        .expect("the valid file asset should be selected for the terminal report");
        let resolved = crate::artifact_resolver::ResolvedArtifact {
            local_path: std::path::PathBuf::new(),
            filename: release.asset_name.clone(),
            cache_hit: true,
            calculated_sha256: Some("ef".repeat(32)),
            redacted_final_url: None,
            verified_path_guard: None,
        };
        let details = app_release_details(
            Some(&provenance),
            "recipe.example/data",
            "forgejo",
            &release,
            &resolved,
        )
        .unwrap();
        let result = ExecutionRunResult {
            success: true,
            cancelled: false,
            total_steps: 0,
            steps: Vec::new(),
            resolved_releases: vec![details],
            resolved_artifacts: Vec::new(),
        };
        let value = serde_json::to_value(result).unwrap();
        let selected = &value["resolved_releases"][0];
        assert_eq!(selected["kind"], "file");
        assert_eq!(selected["assetName"], "data.zip");
        assert_eq!(selected["createdAt"], "2026-01-01T00:00:00Z");
        assert_eq!(selected["timestampSource"], "created_at");
        assert_eq!(selected["cacheHit"], true);
        assert!(selected.get("publishedAt").is_none());
        assert!(selected.get("redactedFinalUrl").is_none());

        let empty_result = ExecutionRunResult {
            success: true,
            cancelled: false,
            total_steps: 0,
            steps: Vec::new(),
            resolved_releases: Vec::new(),
            resolved_artifacts: Vec::new(),
        };
        let value = serde_json::to_value(empty_result).unwrap();
        assert!(value.get("resolved_releases").is_none());
    }

    #[test]
    fn execution_result_serializes_direct_url_materialization_provenance() {
        let result = ExecutionRunResult {
            success: true,
            cancelled: false,
            total_steps: 0,
            steps: Vec::new(),
            resolved_releases: Vec::new(),
            resolved_artifacts: vec![ExecutionResolvedArtifact {
                recipe_artifact_id: "app.obtainium.install/obtainium_apk".to_string(),
                app_id: "obtainium".to_string(),
                app_artifact_id: "apk".to_string(),
                filename: "app-release.apk".to_string(),
                calculated_sha256: "cd".repeat(32),
                cache_hit: true,
                redacted_final_url: None,
            }],
        };

        let value = serde_json::to_value(result).unwrap();
        let artifact = &value["resolved_artifacts"][0];
        assert_eq!(
            artifact["recipeArtifactId"],
            "app.obtainium.install/obtainium_apk"
        );
        assert_eq!(artifact["appId"], "obtainium");
        assert_eq!(artifact["appArtifactId"], "apk");
        assert_eq!(artifact["filename"], "app-release.apk");
        assert_eq!(artifact["calculatedSha256"], "cd".repeat(32));
        assert_eq!(artifact["cacheHit"], true);
        assert!(artifact.get("redactedFinalUrl").is_none());
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StepRunRecord {
    pub step_id: String,
    pub status: StepRunStatus,
    pub message: Option<String>,
    pub outputs: OrderedMap<RuntimeValue>,
    #[serde(skip)]
    pub(crate) failure_kind: Option<StepFailureKind>,
    #[serde(skip)]
    pub(crate) cleanup: Option<ProcessCleanup>,
}

/// Typed private failure kinds used to preserve safety decisions without
/// parsing presentation strings or changing the serialized execution schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepFailureKind {
    OperationTimedOut,
    OperationCommandFailed,
    OperationProcessFailed,
    DeviceOffline,
    DeviceUnauthorized,
    DeviceDisconnected,
    DeviceStorageExhausted,
    AdbServerUnavailable,
    TransportReset,
    TransportFailure,
    DeviceIdentityChanged(identity::IdentityCheckPhase),
    DeviceIdentityUnverified(identity::IdentityCheckPhase),
    RootAuthorityRevoked,
    RootAuthorityUnverified,
    RootDenied,
    RootUnavailable,
    InstallSigningRejected,
    InstallVersionRejected,
    InstallRejected,
    OperationFailed,
}

impl StepFailureKind {
    /// Device safety failures terminate a real run at the current atomic step.
    pub(crate) const fn requires_device_fail_stop(self) -> bool {
        matches!(
            self,
            Self::OperationTimedOut
                | Self::DeviceOffline
                | Self::DeviceUnauthorized
                | Self::DeviceDisconnected
                | Self::DeviceStorageExhausted
                | Self::AdbServerUnavailable
                | Self::TransportReset
                | Self::TransportFailure
                | Self::DeviceIdentityChanged(_)
                | Self::DeviceIdentityUnverified(_)
                | Self::RootAuthorityRevoked
                | Self::RootAuthorityUnverified
        )
    }
}

/// Internal device-operation error carrying stable classification and cleanup
/// evidence across the executor boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceOperationKind {
    TimedOut,
    CommandFailed,
    ProcessFailed,
    DeviceOffline,
    DeviceUnauthorized,
    DeviceDisconnected,
    DeviceStorageExhausted,
    AdbServerUnavailable,
    TransportReset,
    TransportFailure,
    DeviceIdentityChanged(identity::IdentityCheckPhase),
    DeviceIdentityUnverified(identity::IdentityCheckPhase),
    RootAuthorityRevoked,
    RootAuthorityUnverified,
    RootDenied,
    RootUnavailable,
    InstallSigningRejected,
    InstallVersionRejected,
    InstallRejected,
    Other,
}

impl DeviceOperationKind {
    pub(crate) const fn requires_device_fail_stop(self) -> bool {
        matches!(
            self,
            Self::TimedOut
                | Self::DeviceOffline
                | Self::DeviceUnauthorized
                | Self::DeviceDisconnected
                | Self::DeviceStorageExhausted
                | Self::AdbServerUnavailable
                | Self::TransportReset
                | Self::TransportFailure
                | Self::DeviceIdentityChanged(_)
                | Self::DeviceIdentityUnverified(_)
                | Self::RootAuthorityRevoked
                | Self::RootAuthorityUnverified
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceOperationError {
    pub(crate) kind: DeviceOperationKind,
    pub(crate) message: String,
    pub(crate) cleanup: Option<ProcessCleanup>,
}

impl DeviceOperationError {
    pub(crate) fn other(message: impl Into<String>) -> Self {
        Self {
            kind: DeviceOperationKind::Other,
            message: message.into(),
            cleanup: None,
        }
    }

    pub(crate) fn timed_out(cleanup: ProcessCleanup) -> Self {
        Self {
            kind: DeviceOperationKind::TimedOut,
            message: "The device operation timed out.".to_string(),
            cleanup: Some(cleanup),
        }
    }

    pub(crate) fn transport(kind: DeviceOperationKind) -> Self {
        let message = match kind {
            DeviceOperationKind::DeviceOffline => "The reviewed device is offline.",
            DeviceOperationKind::DeviceUnauthorized => "The reviewed device is unauthorized.",
            DeviceOperationKind::DeviceDisconnected => {
                "The reviewed device is disconnected or missing."
            }
            DeviceOperationKind::AdbServerUnavailable => "The local ADB service is unavailable.",
            DeviceOperationKind::TransportReset => adb::DEVICE_TRANSPORT_INTERRUPTED_MESSAGE,
            DeviceOperationKind::TransportFailure => {
                "The device connection was lost during execution."
            }
            _ => "The device connection was lost during execution.",
        };
        Self {
            kind,
            message: message.to_string(),
            cleanup: None,
        }
    }

    pub(crate) fn storage_exhausted() -> Self {
        Self {
            kind: DeviceOperationKind::DeviceStorageExhausted,
            message: "The device ran out of storage during execution.".to_string(),
            cleanup: None,
        }
    }

    pub(crate) fn identity(kind: DeviceOperationKind, phase: identity::IdentityCheckPhase) -> Self {
        let message = if phase == identity::IdentityCheckPhase::PostOperation {
            identity::POST_OPERATION_IDENTITY_FAILURE_MARKER
        } else {
            "The reviewed device identity could not be confirmed."
        };
        Self {
            kind,
            message: message.to_string(),
            cleanup: None,
        }
    }
}

impl From<String> for DeviceOperationError {
    fn from(message: String) -> Self {
        Self::other(message)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepRunStatus {
    Executed,
    Skipped,
    Blocked,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    CheckingSkipConditions,
    Executing,
    Verifying,
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStatus {
    Skipped,
    Blocked,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionProgressEvent {
    pub step_index: usize,
    pub total_steps: usize,
    pub step_id: String,
    pub recipe_ref: String,
    pub step_name: String,
    pub note: String,
    pub phase: ProgressPhase,
    pub status: Option<ProgressStatus>,
    pub message: Option<String>,
}

/// Device boundary used by the executor. Its fourteen typed operations cover
/// bounded install, file, package, permission, and app lifecycle work; identity
/// setup and fake-filesystem selection are lifecycle/configuration hooks.
pub trait ExecutorDevice: std::fmt::Debug {
    /// Configure private real-device identity evidence from the reviewed plan.
    /// Dry-run devices intentionally keep the default no-op implementation.
    fn configure_identity_guard(&mut self, _target: Option<&TargetDeviceBinding>) {}

    /// Configure private execution-time root authority from the reviewed plan.
    /// This lifecycle hook is not part of the executor-visible operation surface.
    fn configure_root_authority(&mut self, _reviewed_root_authorized: bool) {}
    /// Configure the invocation form captured by root preflight and review.
    fn configure_root_shell_route(&mut self, _route: Option<crate::planner::RootShellRoute>) {}

    /// Identify adapters that revalidate root at each root-wrapped command.
    /// This lifecycle hook prevents duplicate runner-level preflight probes.
    fn owns_per_command_root_authority(&self) -> bool {
        false
    }

    fn revalidate_root(&mut self) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn uses_fake_device_filesystem(&self) -> bool {
        false
    }
    /// Selects the sandbox-backed adapter for reviewed selected-tree copies.
    ///
    /// This capability is independent from ordinary fake-device filesystem
    /// behavior. Composite test devices can delegate normal device operations
    /// while preparing and committing reviewed trees in the sandbox. Devices
    /// default to the same adapter choice as their ordinary filesystem mode.
    fn uses_fake_copy_commit_adapter(&self) -> bool {
        self.uses_fake_device_filesystem()
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError>;

    /// Record synthetic package state for deterministic dry-run predicates.
    /// Real devices intentionally retain the default no-op implementation and
    /// query their actual package manager state instead.
    fn record_installed_package(&mut self, _package_name: &str) {}

    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError>;
    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError>;
    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError>;
    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError>;
    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError>;
    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError>;
    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError>;
    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError>;
    /// Apply one planned permission action through the adapter's fixed command
    /// mapping. The operation contains no caller-supplied shell command.
    /// Baseline actions use ordinary ADB shell execution. Elevated actions use
    /// reviewed root authorization and the adapter's guarded root path; an
    /// adapter unable to perform an elevated action must return a fail-stop
    /// root-authority error.
    fn apply_permission_operation(
        &mut self,
        operation: PermissionOperation,
    ) -> Result<(), DeviceOperationError>;
    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError>;
    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError>;

    /// Inspect one selected Android directory without following symbolic links.
    /// Adapters that cannot provide a deterministic manifest reject this
    /// capability so reviewed copy operations fail closed.
    fn copy_commit_tree_manifest(
        &mut self,
        _root: &str,
    ) -> Result<Vec<CopyManifestEntry>, DeviceOperationError> {
        Err(DeviceOperationError::other(
            "adapter does not support selected-tree manifest inspection",
        ))
    }

    /// Return a canonical pathname plus device/inode identities for the
    /// selected directory and its parent.
    fn copy_commit_path_binding(&mut self, _path: &str) -> Result<String, DeviceOperationError> {
        Err(DeviceOperationError::other(
            "adapter does not support selected-path identity binding",
        ))
    }

    /// Check a path for a symbolic link without following it.
    fn copy_commit_path_is_symlink(&mut self, _path: &str) -> Result<bool, DeviceOperationError> {
        Err(DeviceOperationError::other(
            "adapter cannot verify selected-path symbolic-link state",
        ))
    }

    /// Re-observe the T04 volume and verify the Setup Location still has
    /// matching identity, continuity, visible root, mount state, and write access.
    fn copy_commit_verify_setup_location(
        &mut self,
        _binding: &StorageTargetBinding,
    ) -> Result<(), DeviceOperationError> {
        Err(DeviceOperationError::other(
            "adapter cannot revalidate Setup Location volume continuity and access",
        ))
    }

    /// Create exactly one new directory, failing if the path already exists.
    fn copy_commit_create_directory(&mut self, _path: &str) -> Result<(), DeviceOperationError> {
        Err(DeviceOperationError::other(
            "adapter does not support exclusive directory creation",
        ))
    }

    /// Remove exactly one empty directory. Recursive deletion is not part of
    /// the selected-tree copy contract.
    fn copy_commit_remove_empty_directory(
        &mut self,
        _path: &str,
    ) -> Result<(), DeviceOperationError> {
        Err(DeviceOperationError::other(
            "adapter does not support exact empty-directory removal",
        ))
    }

    /// Prepares the optional Android helper used for selected-root-relative
    /// mutations. Adapters without this capability reject that operation.
    fn copy_commit_prepare_mutation_helper(
        &mut self,
        _stage_root: &str,
        _helper_path: &str,
    ) -> Result<(), DeviceOperationError> {
        Err(DeviceOperationError::other(
            "selected App Target mutation requires the optional Android no-follow helper",
        ))
    }

    /// Applies one reviewed mutation relative to the selected directory.
    fn copy_commit_apply_selected_root_mutation(
        &mut self,
        _selected_root: &str,
        _destination_binding: &str,
        _stage_root: &str,
        _helper_path: &str,
        _expected_entry: &str,
        _mutation: &CopyCommitMutation,
    ) -> Result<(), DeviceOperationError> {
        Err(DeviceOperationError::other(
            "device adapter does not expose selected-root-relative no-follow mutation",
        ))
    }

    /// Removes the temporary one-shot helper from its staging directory.
    fn copy_commit_cleanup_mutation_helper(
        &mut self,
        _helper_path: &str,
    ) -> Result<(), DeviceOperationError> {
        Ok(())
    }
}

/// Closed ADB permission action accepted by the executor device boundary.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PermissionOperation {
    GrantRuntimePermission {
        package_name: String,
        permission: String,
        permission_set: PermissionSetKind,
    },
    AllowAppOp {
        package_name: String,
        operation: String,
        permission_set: PermissionSetKind,
    },
}

impl PermissionOperation {
    fn permission_set(&self) -> PermissionSetKind {
        match self {
            Self::GrantRuntimePermission { permission_set, .. }
            | Self::AllowAppOp { permission_set, .. } => *permission_set,
        }
    }

    fn is_elevated(&self) -> bool {
        self.permission_set() == PermissionSetKind::Elevated
    }

    fn adb_shell_arguments(&self) -> Vec<String> {
        match self {
            Self::GrantRuntimePermission {
                package_name,
                permission,
                ..
            } => vec![
                "pm".to_string(),
                "grant".to_string(),
                package_name.clone(),
                permission.clone(),
            ],
            Self::AllowAppOp {
                package_name,
                operation,
                ..
            } => vec![
                "appops".to_string(),
                "set".to_string(),
                package_name.clone(),
                operation.clone(),
                "allow".to_string(),
            ],
        }
    }
}

#[derive(Debug)]
pub struct ExecutorRunner<D: ExecutorDevice = FakeDryRunDevice> {
    adapters: ExecutorAdapters<D>,
    setup_location_lease: Option<Arc<SetupLocationMutationLease>>,
    root_preflight: RootPreflightState,
    root_preflight_failure: Option<String>,
    #[cfg(test)]
    github_release_metadata_fixtures: Option<std::collections::VecDeque<Value>>,
}

/// Internal lifecycle markers used only by qualification seams.  They are
/// emitted at the production runner boundary immediately around the selected
/// atomic operation; they are not serialized execution state or a public DTO.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OperationLifecycle {
    Started { step_id: String },
    Finished { step_id: String },
}

impl Default for ExecutorRunner<FakeDryRunDevice> {
    fn default() -> Self {
        Self {
            adapters: ExecutorAdapters::default(),
            setup_location_lease: None,
            root_preflight: RootPreflightState::NotRun,
            root_preflight_failure: None,
            #[cfg(test)]
            github_release_metadata_fixtures: None,
        }
    }
}

/// Admit every user-provided local-input artifact source in a plan without
/// mutating the device or materializing artifacts.
///
/// Structural binding validation runs first so the same-Recipe binding guard
/// keeps reporting `app_artifact_input_binding_invalid`. Each bound source is
/// then admitted through the authoritative local-input resolver, which checks
/// existence, file kind, readability, and sandbox containment. The first
/// failure is returned as a stable artifact error code so callers can fail
/// closed before any device-mutating step or artifact materialization.
pub(crate) fn admit_plan_local_inputs(
    plan: &ExecutionPlan,
    sandbox: &SandboxRoots,
) -> Result<(), &'static str> {
    crate::planner::validate_execution_plan_local_input_bindings(plan)?;
    let resolver = ArtifactResolver::new(sandbox);
    for artifact in &plan.artifacts {
        let ExecutionArtifactSource::LocalInput { input_id } = &artifact.source else {
            continue;
        };
        let path = crate::planner::validated_local_input_path(plan, input_id)
            .map_err(|_| "app_artifact_input_binding_invalid")?;
        resolver
            .admit_local_input(&artifact.type_name, &artifact.cache, Path::new(path))
            .map_err(|error| error.code())?;
    }
    Ok(())
}

/// Admit local-input artifacts when the executor has no sandbox roots.
///
/// Local-input artifacts are only admitted through the sandbox-backed
/// resolver, so without sandbox roots their existence, file kind, readability,
/// and sandbox containment cannot be checked. This fails closed with the
/// stable sandbox-rejection code so a plan that declares any local-input
/// artifact is rejected before a device-mutating step or artifact
/// materialization. Plans that declare no local-input artifacts are admitted
/// unchanged.
fn admit_plan_local_inputs_without_sandbox_roots(plan: &ExecutionPlan) -> Result<(), &'static str> {
    crate::planner::validate_execution_plan_local_input_bindings(plan)?;
    let has_local_input = plan
        .artifacts
        .iter()
        .any(|artifact| matches!(&artifact.source, ExecutionArtifactSource::LocalInput { .. }));
    if has_local_input {
        return Err("artifact_sandbox_rejected");
    }
    Ok(())
}

impl<D: ExecutorDevice> ExecutorRunner<D> {
    pub fn new(adapters: ExecutorAdapters<D>) -> Self {
        Self::new_with_setup_location_lease(adapters, None)
    }

    pub(crate) fn new_with_setup_location_lease(
        adapters: ExecutorAdapters<D>,
        setup_location_lease: Option<Arc<SetupLocationMutationLease>>,
    ) -> Self {
        Self {
            adapters,
            setup_location_lease,
            root_preflight: RootPreflightState::NotRun,
            root_preflight_failure: None,
            #[cfg(test)]
            github_release_metadata_fixtures: None,
        }
    }

    /// Capture the source and selected destination state for each App Target
    /// directory copy before review. Execution reuses these snapshots and the
    /// copy adapter rejects source or destination drift before mutation.
    /// Configuration-only planning has no device adapter, so it does not
    /// prepare these snapshots itself.
    #[allow(dead_code)]
    pub fn prepare_copy_commit_snapshots(
        &mut self,
        plan: &mut ExecutionPlan,
    ) -> Result<(), String> {
        self.prepare_copy_commit_snapshots_with_storage_targets(plan, &[])
    }

    /// Prepare copy snapshots with trusted bindings for active Setup Location
    /// destination references.
    pub fn prepare_copy_commit_snapshots_with_storage_targets(
        &mut self,
        plan: &mut ExecutionPlan,
        storage_targets: &[StorageTargetBinding],
    ) -> Result<(), String> {
        plan.copy_commit_snapshots.clear();
        validate_supported_copy_plan(plan)?;
        self.adapters
            .device
            .configure_identity_guard(plan.target_device.as_ref());
        self.adapters
            .device
            .configure_root_authority(reviewed_root_authorized(plan));
        self.adapters
            .device
            .configure_root_shell_route(plan.device_context.root_shell_mode);

        let sandbox = self
            .adapters
            .sandbox()
            .map_err(|error| error.message)?
            .clone();
        let state = ExecutionState::from_plan(plan);
        let mut snapshots = Vec::new();

        for step in plan
            .steps
            .iter()
            .filter(|step| step.type_name == "copy_files")
        {
            if has_runtime_output_reference(step) {
                continue;
            }
            let resolved_params =
                resolve_step_params(&state, &step.params).map_err(|error| error.message)?;
            let Some(destination_kind) = copy_commit_destination_kind(step, &resolved_params)?
            else {
                continue;
            };
            let destination_value = resolved_params.get("dest");
            let setup_destination_value = if destination_kind
                == CopyCommitDestinationKind::SetupLocation
            {
                Some(runtime_value_param(&resolved_params, "dest").map_err(|error| error.message)?)
            } else {
                None
            };
            let destination = if let Some(runtime) = &setup_destination_value {
                runtime
                    .value
                    .as_str()
                    .ok_or_else(|| "Setup Location reference must resolve to an ID".to_string())?
            } else {
                destination_value.and_then(Value::as_str).ok_or_else(|| {
                    "selected directory validation requires a destination".to_string()
                })?
            };
            let source =
                runtime_value_param(&resolved_params, "source").map_err(|error| error.message)?;
            let source_path = source_path_from_value(&source).map_err(|error| error.message)?;
            sandbox
                .ensure_read_allowed(&source_path)
                .map_err(|error| error.message)?;
            let setup_binding = if destination_kind == CopyCommitDestinationKind::SetupLocation {
                Some(active_setup_location_binding(
                    storage_targets,
                    destination,
                    plan,
                )?)
            } else {
                None
            };
            match destination_kind {
                CopyCommitDestinationKind::AppTarget => self
                    .ensure_root_preflight()
                    .map_err(|error| error.message)?,
                CopyCommitDestinationKind::SharedDevicePath
                    if !self.adapters.device.uses_fake_copy_commit_adapter() =>
                {
                    self.adapters
                        .device
                        .copy_commit_path_binding(destination)
                        .map_err(|error| error.message)?;
                }
                CopyCommitDestinationKind::SharedDevicePath => {}
                CopyCommitDestinationKind::SetupLocation => {}
            }

            let copy_policy = resolved_params
                .get("copy_policy")
                .and_then(Value::as_str)
                .unwrap_or("merge");
            let operation = CopyCommitOperation::from_legacy_directory_policy(copy_policy)
                .map_err(|error| error.to_string())?;
            let request = if self.adapters.device.uses_fake_copy_commit_adapter() {
                match destination_kind {
                    CopyCommitDestinationKind::AppTarget => CopyCommitRequest::reviewed_directory(
                        &source_path,
                        &sandbox.fake_device_root,
                        destination,
                        operation,
                    ),
                    CopyCommitDestinationKind::SharedDevicePath => {
                        CopyCommitRequest::reviewed_shared_directory(
                            &source_path,
                            &sandbox.fake_device_root,
                            destination,
                            operation,
                        )
                    }
                    CopyCommitDestinationKind::SetupLocation => {
                        CopyCommitRequest::reviewed_fake_setup_directory(
                            &source_path,
                            &sandbox.fake_device_root,
                            setup_binding.clone().expect(
                                "Setup Location binding is resolved before request creation",
                            ),
                            operation,
                        )
                    }
                }
            } else {
                match destination_kind {
                    CopyCommitDestinationKind::SetupLocation => {
                        let binding = setup_binding
                            .clone()
                            .expect("Setup Location binding is resolved before request creation");
                        let (precondition, destination_manifest) =
                            inspect_android_setup_destination(&mut self.adapters.device, &binding)?;
                        CopyCommitRequest::reviewed_setup_directory(
                            &source_path,
                            &sandbox.fake_device_root,
                            binding,
                            precondition,
                            operation,
                            destination_manifest,
                        )
                    }
                    CopyCommitDestinationKind::AppTarget
                    | CopyCommitDestinationKind::SharedDevicePath => {
                        let destination_manifest = self
                            .adapters
                            .device
                            .copy_commit_tree_manifest(destination)
                            .map_err(|error| error.message)?;
                        let destination_binding = self
                            .adapters
                            .device
                            .copy_commit_path_binding(destination)
                            .map_err(|error| error.message)?;
                        if destination_kind == CopyCommitDestinationKind::AppTarget {
                            CopyCommitRequest::reviewed_android_directory(
                                &source_path,
                                &sandbox.fake_device_root,
                                destination,
                                operation,
                                destination_manifest,
                                destination_binding,
                            )
                        } else {
                            CopyCommitRequest::reviewed_android_shared_directory(
                                &source_path,
                                &sandbox.fake_device_root,
                                destination,
                                operation,
                                destination_manifest,
                                destination_binding,
                            )
                        }
                    }
                }
            }
            .map_err(|error| error.to_string())?;
            snapshots.push(ExecutionCopyCommitSnapshot::from_request(
                &step.id,
                &source_path,
                &request,
            ));
        }

        validate_active_copy_writer_collisions(&snapshots)?;
        validate_unreviewed_legacy_shared_copy_writers(
            &mut self.adapters.device,
            &sandbox.fake_device_root,
            plan,
            &snapshots,
        )?;
        plan.copy_commit_snapshots = snapshots;
        Ok(())
    }

    /// Supply release metadata fixtures to exercise the production selection path offline.
    #[cfg(test)]
    pub(crate) fn set_github_release_metadata_fixtures(&mut self, fixtures: Vec<Value>) {
        self.github_release_metadata_fixtures = Some(std::collections::VecDeque::from(fixtures));
    }

    #[cfg(test)]
    pub fn adapters(&self) -> &ExecutorAdapters<D> {
        &self.adapters
    }

    #[cfg(test)]
    pub fn run(&mut self, plan: &ExecutionPlan) -> ExecutionRunResult {
        self.run_with_progress(plan, |_| {})
    }

    pub fn run_with_progress(
        &mut self,
        plan: &ExecutionPlan,
        progress_callback: impl FnMut(ExecutionProgressEvent),
    ) -> ExecutionRunResult {
        self.run_with_progress_and_cancel(plan, progress_callback, || false)
    }

    /// Execute in plan order and observe cancellation only between atomic steps.
    ///
    /// A cancellation request never interrupts or rolls back the current device
    /// operation. Once observed, no later step resolves parameters, executes,
    /// or verifies. Completed progress remains intact; later runner records are
    /// internally cancelled while the session report leaves those never-started
    /// steps pending so a terminal projection can label them "Not attempted"
    /// without adding a serialized status.
    pub fn run_with_progress_and_cancel(
        &mut self,
        plan: &ExecutionPlan,
        mut progress_callback: impl FnMut(ExecutionProgressEvent),
        should_cancel: impl Fn() -> bool,
    ) -> ExecutionRunResult {
        self.run_with_progress_and_cancel_inner(plan, &mut progress_callback, should_cancel, |_| {})
    }

    /// Execute one qualification run while observing the production runner
    /// lifecycle around each atomic operation. This is private test
    /// infrastructure; production callers keep the unchanged public runner
    /// method above. Execution-session slot ownership is observed separately
    /// at the manager boundary rather than inferred from this callback.
    #[cfg(test)]
    pub(crate) fn run_with_progress_and_cancel_observed(
        &mut self,
        plan: &ExecutionPlan,
        mut progress_callback: impl FnMut(ExecutionProgressEvent),
        should_cancel: impl Fn() -> bool,
        operation_observer: impl FnMut(OperationLifecycle),
    ) -> ExecutionRunResult {
        self.run_with_progress_and_cancel_inner(
            plan,
            &mut progress_callback,
            should_cancel,
            operation_observer,
        )
    }

    fn run_with_progress_and_cancel_inner(
        &mut self,
        plan: &ExecutionPlan,
        progress_callback: &mut impl FnMut(ExecutionProgressEvent),
        should_cancel: impl Fn() -> bool,
        mut operation_observer: impl FnMut(OperationLifecycle),
    ) -> ExecutionRunResult {
        let total_steps = plan.steps.len();
        self.root_preflight = RootPreflightState::NotRun;
        self.root_preflight_failure = None;
        let local_input_admission_code = match self.adapters.sandbox() {
            Ok(sandbox) => admit_plan_local_inputs(plan, sandbox).err(),
            Err(_) => admit_plan_local_inputs_without_sandbox_roots(plan).err(),
        };
        if let Some(artifact_code) = local_input_admission_code {
            let steps = plan
                .steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let (status, message) = if index == 0 {
                        (
                            StepRunStatus::Failed,
                            format!("{artifact_code}: Local input admission failed."),
                        )
                    } else {
                        (
                            StepRunStatus::Blocked,
                            "execution blocked after plan admission failed".to_string(),
                        )
                    };
                    record(step, status, Some(message), OrderedMap::new())
                })
                .collect();
            return ExecutionRunResult {
                success: false,
                cancelled: false,
                total_steps,
                steps,
                resolved_releases: Vec::new(),
                resolved_artifacts: Vec::new(),
            };
        }
        let copy_plan_admission = validate_supported_copy_plan(plan)
            .and_then(|()| validate_copy_commit_snapshot_coverage(plan));
        if let Err(message) = copy_plan_admission {
            let steps = plan
                .steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    let (status, detail) = if index == 0 {
                        (
                            StepRunStatus::Failed,
                            format!("invalid_execution_plan: {message}"),
                        )
                    } else {
                        (
                            StepRunStatus::Blocked,
                            "execution blocked after copy review validation failed".to_string(),
                        )
                    };
                    record(step, status, Some(detail), OrderedMap::new())
                })
                .collect();
            return ExecutionRunResult {
                success: false,
                cancelled: false,
                total_steps,
                steps,
                resolved_releases: Vec::new(),
                resolved_artifacts: Vec::new(),
            };
        }
        let step_ids_in_plan = plan
            .steps
            .iter()
            .map(|step| step.id.clone())
            .collect::<HashSet<_>>();
        let mut state = ExecutionState::from_plan(plan);
        let mut records = Vec::new();
        let mut cancelled = false;
        let mut aborted = false;
        self.adapters
            .device
            .configure_identity_guard(plan.target_device.as_ref());
        self.adapters
            .device
            .configure_root_authority(reviewed_root_authorized(plan));
        self.adapters
            .device
            .configure_root_shell_route(plan.device_context.root_shell_mode);

        for step in &plan.steps {
            if aborted {
                let message = "execution aborted after root preflight failure".to_string();
                records.push(record(
                    step,
                    StepRunStatus::Blocked,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }
            if cancelled || should_cancel() {
                cancelled = true;
                let message = "execution cancelled before step scheduling".to_string();
                records.push(record(
                    step,
                    StepRunStatus::Cancelled,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }
            progress_callback(progress_event(
                step,
                total_steps,
                plan,
                ProgressPhase::CheckingSkipConditions,
                None,
                None,
            ));

            let blocking_dependencies = blocking_dependencies(&state, &step.dependencies);
            if !blocking_dependencies.is_empty() {
                let message = format!("dependency blocked: {}", blocking_dependencies.join(", "));
                state.steps.insert(
                    step.id.clone(),
                    StepRuntimeState {
                        status: StepRuntimeStatus::Blocked,
                        outputs: OrderedMap::new(),
                    },
                );
                progress_callback(progress_event(
                    step,
                    total_steps,
                    plan,
                    ProgressPhase::Finished,
                    Some(ProgressStatus::Blocked),
                    Some(message.clone()),
                ));
                records.push(record(
                    step,
                    StepRunStatus::Blocked,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }

            if let Some(message) = missing_capabilities_message(plan, step) {
                state.steps.insert(
                    step.id.clone(),
                    StepRuntimeState {
                        status: StepRuntimeStatus::Failed,
                        outputs: OrderedMap::new(),
                    },
                );
                progress_callback(progress_event(
                    step,
                    total_steps,
                    plan,
                    ProgressPhase::Finished,
                    Some(ProgressStatus::Failed),
                    Some(message.clone()),
                ));
                records.push(record(
                    step,
                    StepRunStatus::Failed,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }

            if step.type_name == "grant_permissions"
                && (step.permission_plan.is_some() || step.app_id.is_some())
            {
                match crate::planner::validated_plan_permission_plan(plan, step) {
                    Err(reason) => {
                        let message = format!(
                            "The reviewed permission plan is invalid: {reason}. Prepare a fresh review before execution."
                        );
                        state.steps.insert(
                            step.id.clone(),
                            StepRuntimeState {
                                status: StepRuntimeStatus::Failed,
                                outputs: OrderedMap::new(),
                            },
                        );
                        progress_callback(progress_event(
                            step,
                            total_steps,
                            plan,
                            ProgressPhase::Finished,
                            Some(ProgressStatus::Failed),
                            Some(message.clone()),
                        ));
                        records.push(record_with_failure(
                            step,
                            StepRunStatus::Failed,
                            Some(message),
                            OrderedMap::new(),
                            None,
                            None,
                        ));
                        break;
                    }
                    Ok((_, permission_plan)) if !permission_plan.has_applicable_actions() => {
                        let outputs = permission_not_applicable_outputs(step, permission_plan);
                        let message =
                            "No actions in the selected permission set apply to this Android API level."
                                .to_string();
                        state.steps.insert(
                            step.id.clone(),
                            StepRuntimeState {
                                status: StepRuntimeStatus::Skipped,
                                outputs: outputs.clone(),
                            },
                        );
                        progress_callback(progress_event(
                            step,
                            total_steps,
                            plan,
                            ProgressPhase::Finished,
                            Some(ProgressStatus::Skipped),
                            Some(message.clone()),
                        ));
                        records.push(record(step, StepRunStatus::Skipped, Some(message), outputs));
                        continue;
                    }
                    Ok((_, _)) => {}
                }
            }

            if plan_step_root_requirements(plan, step).any() {
                if let Err(failure) = self.ensure_root_preflight() {
                    let device_fail_stop = failure
                        .failure_kind
                        .is_some_and(StepFailureKind::requires_device_fail_stop);
                    let message = failure.message.clone();
                    self.root_preflight_failure = Some(message.clone());
                    state.steps.insert(
                        step.id.clone(),
                        StepRuntimeState {
                            status: StepRuntimeStatus::Failed,
                            outputs: OrderedMap::new(),
                        },
                    );
                    progress_callback(progress_event(
                        step,
                        total_steps,
                        plan,
                        ProgressPhase::Finished,
                        Some(ProgressStatus::Failed),
                        Some(message.clone()),
                    ));
                    records.push(record_with_failure(
                        step,
                        StepRunStatus::Failed,
                        Some(message),
                        OrderedMap::new(),
                        failure.failure_kind,
                        failure.cleanup,
                    ));
                    if device_fail_stop {
                        break;
                    }
                    aborted = true;
                    continue;
                }
            }

            let active_conflicts = step
                .constraints
                .conflicts_with
                .iter()
                .filter(|conflict| step_ids_in_plan.contains(*conflict))
                .cloned()
                .collect::<Vec<_>>();
            if !active_conflicts.is_empty() {
                let message = format!("conflicting steps present: {}", active_conflicts.join(", "));
                state.steps.insert(
                    step.id.clone(),
                    StepRuntimeState {
                        status: StepRuntimeStatus::Failed,
                        outputs: OrderedMap::new(),
                    },
                );
                progress_callback(progress_event(
                    step,
                    total_steps,
                    plan,
                    ProgressPhase::Finished,
                    Some(ProgressStatus::Failed),
                    Some(message.clone()),
                ));
                records.push(record(
                    step,
                    StepRunStatus::Failed,
                    Some(message),
                    OrderedMap::new(),
                ));
                continue;
            }

            match self.skip_if_matched(plan, step) {
                Ok(true) => {
                    state.steps.insert(
                        step.id.clone(),
                        StepRuntimeState {
                            status: StepRuntimeStatus::Skipped,
                            outputs: OrderedMap::new(),
                        },
                    );
                    progress_callback(progress_event(
                        step,
                        total_steps,
                        plan,
                        ProgressPhase::Finished,
                        Some(ProgressStatus::Skipped),
                        Some("skip_if matched".to_string()),
                    ));
                    records.push(record(
                        step,
                        StepRunStatus::Skipped,
                        Some("skip_if matched".to_string()),
                        OrderedMap::new(),
                    ));
                    continue;
                }
                Ok(false) => {}
                Err(failure) => {
                    let device_fail_stop = failure
                        .failure_kind
                        .is_some_and(StepFailureKind::requires_device_fail_stop);
                    let message = failure.message.clone();
                    state.steps.insert(
                        step.id.clone(),
                        StepRuntimeState {
                            status: StepRuntimeStatus::Failed,
                            outputs: OrderedMap::new(),
                        },
                    );
                    progress_callback(progress_event(
                        step,
                        total_steps,
                        plan,
                        ProgressPhase::Finished,
                        Some(ProgressStatus::Failed),
                        Some(message.clone()),
                    ));
                    records.push(record_with_failure(
                        step,
                        StepRunStatus::Failed,
                        Some(message),
                        OrderedMap::new(),
                        failure.failure_kind,
                        failure.cleanup,
                    ));
                    if device_fail_stop {
                        break;
                    }
                    if self.root_preflight_failure.is_some() {
                        aborted = true;
                    }
                    continue;
                }
            }

            progress_callback(progress_event(
                step,
                total_steps,
                plan,
                ProgressPhase::Executing,
                None,
                None,
            ));
            operation_observer(OperationLifecycle::Started {
                step_id: step.id.clone(),
            });
            let run_result = self.run_step(plan, &mut state, step);
            operation_observer(OperationLifecycle::Finished {
                step_id: step.id.clone(),
            });
            let (status, message, outputs, runtime_status, failure_kind, cleanup, device_fail_stop) =
                match run_result {
                    Ok(outputs) => {
                        progress_callback(progress_event(
                            step,
                            total_steps,
                            plan,
                            ProgressPhase::Verifying,
                            None,
                            None,
                        ));
                        let failed_verify = step
                            .verify
                            .iter()
                            .filter_map(|condition| {
                                match self.evaluate_condition(plan, condition) {
                                    Ok(true) => None,
                                    Ok(false) => Some(Ok(condition.type_name.clone())),
                                    Err(error) => Some(Err(error)),
                                }
                            })
                            .collect::<Result<Vec<_>, _>>();
                        match failed_verify {
                            Ok(failed_verify) if failed_verify.is_empty() => (
                                StepRunStatus::Executed,
                                None,
                                outputs.clone(),
                                StepRuntimeStatus::Succeeded,
                                None,
                                None,
                                false,
                            ),
                            Ok(failed_verify) => (
                                StepRunStatus::Failed,
                                Some(format!("verify failed: {}", failed_verify.join(", "))),
                                OrderedMap::new(),
                                StepRuntimeStatus::Failed,
                                None,
                                None,
                                false,
                            ),
                            Err(failure) => (
                                StepRunStatus::Failed,
                                Some(failure.message),
                                if failure
                                    .failure_kind
                                    .is_some_and(StepFailureKind::requires_device_fail_stop)
                                {
                                    outputs.clone()
                                } else {
                                    OrderedMap::new()
                                },
                                StepRuntimeStatus::Failed,
                                failure.failure_kind,
                                failure.cleanup,
                                failure
                                    .failure_kind
                                    .is_some_and(StepFailureKind::requires_device_fail_stop),
                            ),
                        }
                    }
                    Err(failure) => {
                        let device_fail_stop = failure
                            .failure_kind
                            .is_some_and(StepFailureKind::requires_device_fail_stop);
                        (
                            StepRunStatus::Failed,
                            Some(failure.message),
                            failure.outputs,
                            StepRuntimeStatus::Failed,
                            failure.failure_kind,
                            failure.cleanup,
                            device_fail_stop,
                        )
                    }
                };

            state.steps.insert(
                step.id.clone(),
                StepRuntimeState {
                    status: runtime_status,
                    outputs: outputs.clone(),
                },
            );
            let progress_status = match status {
                StepRunStatus::Executed => ProgressStatus::Succeeded,
                StepRunStatus::Skipped => ProgressStatus::Skipped,
                StepRunStatus::Blocked => ProgressStatus::Blocked,
                StepRunStatus::Failed => ProgressStatus::Failed,
                StepRunStatus::Cancelled => ProgressStatus::Cancelled,
            };
            progress_callback(progress_event(
                step,
                total_steps,
                plan,
                ProgressPhase::Finished,
                Some(progress_status),
                message.clone(),
            ));
            records.push(record_with_failure(
                step,
                status,
                message,
                outputs,
                failure_kind,
                cleanup,
            ));
            if device_fail_stop {
                break;
            }
            if self.root_preflight_failure.is_some() {
                aborted = true;
            }
        }

        let success = !records.iter().any(|record| {
            matches!(
                record.status,
                StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
            )
        });
        ExecutionRunResult {
            success,
            cancelled,
            total_steps,
            steps: records,
            resolved_releases: plan
                .artifacts
                .iter()
                .filter_map(|artifact| {
                    state
                        .artifacts
                        .get(&artifact.id)
                        .and_then(|artifact| artifact.resolved_release.clone())
                })
                .collect(),
            resolved_artifacts: plan
                .artifacts
                .iter()
                .filter_map(|artifact| {
                    state
                        .artifacts
                        .get(&artifact.id)
                        .and_then(|runtime| runtime.resolved_artifact.clone())
                })
                .collect(),
        }
    }

    fn skip_if_matched(
        &mut self,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
    ) -> Result<bool, StepFailure> {
        step.skip_if.iter().try_fold(false, |matched, condition| {
            if matched {
                Ok(true)
            } else {
                self.evaluate_condition(plan, condition)
            }
        })
    }

    fn ensure_root_preflight(&mut self) -> Result<(), StepFailure> {
        if self.adapters.device.owns_per_command_root_authority() {
            return Ok(());
        }
        match &self.root_preflight {
            RootPreflightState::Granted => return Ok(()),
            RootPreflightState::Failed(message) => {
                return Err(StepFailure::new(message.clone()));
            }
            RootPreflightState::NotRun => {}
        }
        match self.adapters.device.revalidate_root() {
            Ok(()) => {
                self.root_preflight = RootPreflightState::Granted;
                Ok(())
            }
            Err(error) => {
                let failure: StepFailure = error.into();
                self.root_preflight = RootPreflightState::Failed(failure.message.clone());
                self.root_preflight_failure = Some(failure.message.clone());
                Err(failure)
            }
        }
    }

    fn run_step(
        &mut self,
        plan: &ExecutionPlan,
        state: &mut ExecutionState,
        step: &ExecutionStep,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let resolved_params = resolve_step_params(state, &step.params)?;
        match step.type_name.as_str() {
            "wait" => self.execute_wait(&resolved_params),
            "grant_permissions" => self.execute_grant_permissions(plan, step),
            "download_remote_file" => self.execute_download_remote_file(step, &resolved_params),
            "resolve_artifacts" => self.execute_resolve_artifacts(plan, state, &resolved_params),
            "extract_artifacts" => self.execute_extract_artifacts(state, step, &resolved_params),
            "extract_archive" => self.execute_extract_archive(step, &resolved_params),
            "copy_files" => self.execute_copy_files(plan, step, &resolved_params),
            "install_apk" => self.execute_install_apk(plan, state, step, &resolved_params),
            "launch_app" => self.execute_launch_app(plan, step),
            "force_stop_app" => self.execute_force_stop_app(plan, step),
            other => Err(StepFailure::new(format!(
                "Unsupported executor step type: {other}"
            ))),
        }
    }

    fn execute_wait(
        &mut self,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let raw_duration = resolved_params.get("duration_ms").unwrap_or(&Value::Null);
        let duration = raw_duration.as_i64().filter(|duration| *duration > 0);
        if raw_duration.is_boolean() || duration.is_none() {
            return Err(StepFailure::new(format!(
                "wait step requires a positive integer duration_ms: {}",
                stable_repr(raw_duration)
            )));
        }
        self.adapters.sleep(duration.unwrap() as f64 / 1000.0);
        Ok(OrderedMap::new())
    }

    fn execute_grant_permissions(
        &mut self,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let (_, permission_plan) = crate::planner::validated_plan_permission_plan(plan, step)
            .map_err(|reason| {
                StepFailure::new(format!(
                    "The reviewed permission plan is invalid: {reason}."
                ))
            })?;
        self.execute_planned_permission_actions(step, permission_plan)
    }

    fn execute_planned_permission_actions(
        &mut self,
        step: &ExecutionStep,
        permission_plan: &PermissionPlan,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let mut action_results = Vec::with_capacity(permission_plan.actions.len());
        let mut ordinary_failure = None;
        let mut safety_failure = None;

        for action in &permission_plan.actions {
            if !matches!(
                action.applicability(),
                PermissionActionApplicability::Applicable
            ) {
                action_results.push(permission_plan_action_result(
                    step,
                    permission_plan,
                    action,
                    "not_applicable",
                    None,
                ));
                continue;
            }

            let operation = permission_operation(permission_plan.permission_set, action);
            match self.adapters.device.apply_permission_operation(operation) {
                Ok(()) => action_results.push(permission_plan_action_result(
                    step,
                    permission_plan,
                    action,
                    "executed",
                    None,
                )),
                Err(error) => {
                    action_results.push(permission_plan_action_result(
                        step,
                        permission_plan,
                        action,
                        "failed",
                        Some(error.message.clone()),
                    ));
                    if error.kind.requires_device_fail_stop() {
                        safety_failure = Some(error);
                        break;
                    }
                    if ordinary_failure.is_none() {
                        ordinary_failure = Some(error);
                    }
                }
            }
        }

        let outputs = permission_action_outputs(action_results);
        if let Some(error) = safety_failure {
            return Err(step_failure_with_outputs(error, outputs));
        }
        if permission_plan.on_failure == PermissionFailurePolicy::Fail {
            if let Some(error) = ordinary_failure {
                return Err(step_failure_with_outputs(error, outputs));
            }
        }
        Ok(outputs)
    }

    fn execute_download_remote_file(
        &mut self,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let url = resolved_params
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| StepFailure::new("download_remote_file requires url".to_string()))?;
        let cache = resolved_params
            .get("cache")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let artifact_id = format!("dynamic.{}", step.id);
        let mut resolver = ArtifactResolver::new(self.adapters.sandbox()?);
        let request = ArtifactResolveRequest {
            artifact_id: &artifact_id,
            type_name: "remote_file",
            url,
            cache_mode: cache,
        };
        let resolved = resolver
            .resolve(request)
            .map_err(|error| StepFailure::new(error.executor_message(request)))?;
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "local_path".to_string(),
            RuntimeValue {
                type_name: "file_path".to_string(),
                value: Value::String(resolved.local_path.to_string_lossy().into_owned()),
                location: Some("host".to_string()),
            },
        );
        outputs.insert(
            "filename".to_string(),
            RuntimeValue {
                type_name: "string".to_string(),
                value: Value::String(resolved.filename),
                location: None,
            },
        );
        Ok(outputs)
    }

    fn execute_resolve_artifacts(
        &mut self,
        plan: &ExecutionPlan,
        state: &mut ExecutionState,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let artifacts_by_id = plan
            .artifacts
            .iter()
            .map(|artifact| (artifact.id.as_str(), artifact))
            .collect::<HashMap<_, _>>();
        let mut resolver = ArtifactResolver::new(self.adapters.sandbox()?);
        for artifact_id in string_list_param(resolved_params.get("artifacts")) {
            let Some(artifact) = artifacts_by_id.get(artifact_id.as_str()) else {
                return Err(StepFailure::new(format!(
                    "unknown_artifact_ref: Unknown artifact ref: 'artifacts.{artifact_id}'."
                )));
            };
            #[cfg(test)]
            let release_metadata_fixture = if matches!(
                &artifact.source,
                ExecutionArtifactSource::RemoteRelease { .. }
            ) {
                self.github_release_metadata_fixtures
                    .as_mut()
                    .map(|fixtures| fixtures.pop_front())
            } else {
                None
            };
            let result: Result<_, StepFailure> = (|| {
                let materialized_source = match &artifact.source {
                    ExecutionArtifactSource::RemoteFile { url } => {
                        MaterializedArtifactSource::RemoteFile { url: url.clone() }
                    }
                    ExecutionArtifactSource::DirectUrl { url, sha256 } => {
                        MaterializedArtifactSource::DirectUrl {
                            url: url.clone(),
                            sha256: sha256.clone(),
                        }
                    }
                    ExecutionArtifactSource::LocalInput { input_id } => {
                        let path = crate::planner::validated_local_input_path(plan, input_id)
                            .map_err(|message| StepFailure::new(message.to_string()))?;
                        MaterializedArtifactSource::LocalInput {
                            path: PathBuf::from(path),
                        }
                    }
                    ExecutionArtifactSource::RemoteRelease {
                        provider,
                        service_origin,
                        repository,
                        include_prereleases,
                        asset_pattern,
                        invert_asset_pattern,
                    } => {
                        let provenance = artifact
                            .app_provenance
                            .as_ref()
                            .filter(|provenance| {
                                crate::authored_models::is_valid_identifier(&provenance.app_id)
                                    && crate::authored_models::is_valid_identifier(
                                        &provenance.artifact_id,
                                    )
                                    && matches!(provenance.kind.as_str(), "apk" | "file")
                                    && plan.apps.iter().filter(|app| app.id == provenance.app_id).count()
                                        == 1
                            })
                            .ok_or_else(|| {
                                StepFailure::new(
                                    "app_artifact_provenance_invalid: Release artifact provenance is missing or inconsistent.".to_string(),
                                )
                            })?;
                        crate::remote_release_resolver::validate_remote_release_policy(
                            provider,
                            service_origin,
                            repository,
                            &provenance.kind,
                            asset_pattern.as_deref(),
                            *invert_asset_pattern,
                        )
                        .map_err(StepFailure::new)?;
                        #[cfg(test)]
                        let release = match release_metadata_fixture.as_ref() {
                            Some(Some(fixture)) => {
                                crate::remote_release_resolver::resolve_remote_latest_from_fixture(
                                    provider,
                                    service_origin,
                                    repository,
                                    &provenance.kind,
                                    crate::remote_release_resolver::RemoteReleaseOptions {
                                        include_prereleases: *include_prereleases,
                                        asset_pattern: asset_pattern.as_deref(),
                                        invert_asset_pattern: *invert_asset_pattern,
                                    },
                                    fixture,
                                )
                            }
                            .map_err(StepFailure::new)?,
                            Some(None) => {
                                return Err(StepFailure::new(
                                    "remote_release_fixture_missing: No release metadata fixture remains for this run".to_string(),
                                ));
                            }
                            None => {
                                let transport = resolver
                                    .release_metadata_transport()
                                    .map_err(|_| StepFailure::new(
                                        "remote_release_client_failed: Network access could not be initialized".to_string(),
                                    ))?;
                                resolve_remote_latest(
                                    transport,
                                    provider,
                                    service_origin,
                                    repository,
                                    &provenance.kind,
                                    RemoteReleaseOptions {
                                        include_prereleases: *include_prereleases,
                                        asset_pattern: asset_pattern.as_deref(),
                                        invert_asset_pattern: *invert_asset_pattern,
                                    },
                                )
                                .map_err(StepFailure::new)?
                            }
                        };
                        #[cfg(not(test))]
                        let release = {
                            let transport = resolver
                                .release_metadata_transport()
                                .map_err(|_| StepFailure::new(
                                    "remote_release_client_failed: Network access could not be initialized".to_string(),
                                ))?;
                            resolve_remote_latest(
                                transport,
                                provider,
                                service_origin,
                                repository,
                                &provenance.kind,
                                RemoteReleaseOptions {
                                    include_prereleases: *include_prereleases,
                                    asset_pattern: asset_pattern.as_deref(),
                                    invert_asset_pattern: *invert_asset_pattern,
                                },
                            )
                            .map_err(StepFailure::new)?
                        };
                        MaterializedArtifactSource::RemoteRelease {
                            url: release.download_url.clone(),
                            provider: provider.clone(),
                            release,
                        }
                    }
                };
                let reported_provenance = if matches!(
                    &artifact.source,
                    ExecutionArtifactSource::DirectUrl { .. }
                        | ExecutionArtifactSource::LocalInput { .. }
                ) {
                    Some(
                        artifact
                            .app_provenance
                            .as_ref()
                            .filter(|provenance| {
                                matches!(provenance.kind.as_str(), "apk" | "file")
                                    && crate::authored_models::is_valid_identifier(
                                        &provenance.app_id,
                                    )
                                    && crate::authored_models::is_valid_identifier(
                                        &provenance.artifact_id,
                                    )
                                    && plan.apps.iter().filter(|app| app.id == provenance.app_id).count()
                                        == 1
                            })
                            .ok_or_else(|| {
                                StepFailure::new(
                                    "app_artifact_provenance_invalid: App Artifact source is missing consistent App Definition provenance.".to_string(),
                                )
                            })?,
                    )
                } else {
                    None
                };
                let request = ArtifactResolveRequest {
                    artifact_id: &artifact.id,
                    type_name: &artifact.type_name,
                    url: materialized_source.url(),
                    cache_mode: &artifact.cache,
                };
                let resolved = match &materialized_source {
                    MaterializedArtifactSource::DirectUrl { sha256, .. } => {
                        resolver.resolve_direct_url(request, sha256.as_deref())
                    }
                    MaterializedArtifactSource::LocalInput { path } => {
                        resolver.resolve_local_input(request, path)
                    }
                    MaterializedArtifactSource::RemoteRelease { .. } => {
                        resolver.resolve_app_release(request)
                    }
                    MaterializedArtifactSource::RemoteFile { .. } => resolver.resolve(request),
                }
                .map_err(|error| StepFailure::new(error.executor_message(request)))?;
                let resolved_artifact = if let Some(provenance) = reported_provenance {
                    let calculated_sha256 = resolved
                        .calculated_sha256
                        .clone()
                        .ok_or_else(|| {
                            StepFailure::new(
                                "artifact_sha256_unavailable: App Artifact materialization did not produce a calculated SHA-256.".to_string(),
                            )
                        })?;
                    Some(ExecutionResolvedArtifact {
                        recipe_artifact_id: artifact.id.clone(),
                        app_id: provenance.app_id.clone(),
                        app_artifact_id: provenance.artifact_id.clone(),
                        filename: resolved.filename.clone(),
                        calculated_sha256,
                        cache_hit: resolved.cache_hit,
                        redacted_final_url: resolved.redacted_final_url.clone(),
                    })
                } else {
                    None
                };
                let resolved_release = match &materialized_source {
                    MaterializedArtifactSource::RemoteRelease {
                        provider, release, ..
                    } => Some(
                        app_release_details(
                            artifact.app_provenance.as_ref(),
                            &artifact.id,
                            provider,
                            release,
                            &resolved,
                        )
                        .ok_or_else(|| {
                            StepFailure::new(
                                "artifact_sha256_unavailable: Release materialization did not produce verifiable bytes."
                                    .to_string(),
                            )
                        })?,
                    ),
                    _ => None,
                };
                Ok((
                    materialized_source.url().to_string(),
                    resolved,
                    resolved_artifact,
                    resolved_release,
                ))
            })();
            let artifact_state = state
                .artifacts
                .get_mut(&artifact.id)
                .expect("artifact runtime state should be initialized from plan");
            artifact_state.resolved_release = None;
            match result {
                Ok((resolved_url, resolved, resolved_artifact, resolved_release)) => {
                    artifact_state.status = ArtifactRuntimeStatus::Resolved;
                    artifact_state.local_path =
                        Some(resolved.local_path.to_string_lossy().into_owned());
                    artifact_state.resolved_url = if matches!(
                        &artifact.source,
                        ExecutionArtifactSource::DirectUrl { .. }
                            | ExecutionArtifactSource::LocalInput { .. }
                            | ExecutionArtifactSource::RemoteRelease { .. }
                    ) {
                        None
                    } else {
                        Some(resolved_url)
                    };
                    artifact_state.resolved_release = resolved_release;
                    artifact_state.filename = Some(resolved.filename);
                    artifact_state.cache_hit = resolved.cache_hit;
                    artifact_state.resolved_artifact = resolved_artifact;
                    artifact_state.verified_path_guard = resolved.verified_path_guard;
                    artifact_state.error = None;
                }
                Err(failure) => {
                    let message = failure.message.clone();
                    artifact_state.status = ArtifactRuntimeStatus::Failed;
                    artifact_state.error = Some(message);
                    return Err(failure);
                }
            }
        }
        Ok(OrderedMap::new())
    }

    fn execute_extract_artifacts(
        &mut self,
        state: &ExecutionState,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let extract_on = resolved_params
            .get("extract_on")
            .and_then(Value::as_str)
            .unwrap_or("host");
        if !matches!(extract_on, "host" | "device") {
            return Err(StepFailure::new(format!(
                "extract_artifacts extract_on must be 'host' or 'device', got {extract_on:?}"
            )));
        }
        let sandbox = self.adapters.sandbox()?.clone();
        let extract_root = sandbox
            .runtime_root
            .join("extract")
            .join(sanitize_step_id(&step.id));
        let mut output_paths = Vec::new();
        let device_base = format!("/data/local/tmp/emuchef/{}", sanitize_step_id(&step.id));
        for artifact_id in string_list_param(resolved_params.get("artifacts")) {
            let Some(artifact_state) = state.artifacts.get(&artifact_id) else {
                return Err(StepFailure::new(format!(
                    "unknown_artifact_ref: Unknown artifact ref: 'artifacts.{artifact_id}'."
                )));
            };
            if artifact_state.status != ArtifactRuntimeStatus::Resolved {
                return Err(StepFailure::new(format!(
                    "artifact_not_resolved: Artifact is not resolved: 'artifacts.{artifact_id}.local_path'."
                )));
            }
            let Some(local_path) = &artifact_state.local_path else {
                return Err(StepFailure::new(format!(
                    "artifact_not_resolved: Artifact is not resolved: 'artifacts.{artifact_id}.local_path'."
                )));
            };
            let archive_path = PathBuf::from(local_path);
            sandbox.ensure_read_allowed(&archive_path)?;
            let artifact_dir =
                extract_root.join(artifact_id.rsplit('/').next().unwrap_or(&artifact_id));
            let members = sandbox.extract_zip_to_directory(&archive_path, &artifact_dir)?;
            if extract_on == "host" {
                output_paths.extend(
                    members
                        .into_iter()
                        .map(|member| json!(member.to_string_lossy().to_string())),
                );
            } else {
                let device_dest = join_device_path(
                    &device_base,
                    artifact_id.rsplit('/').next().unwrap_or(&artifact_id),
                );
                self.adapters.device.mkdir_p(&device_dest)?;
                for member in members {
                    let name = member.file_name().ok_or_else(|| {
                        StepFailure::new(format!(
                            "extracted path has no basename: {}",
                            member.display()
                        ))
                    })?;
                    let target = join_device_path(&device_dest, &name.to_string_lossy());
                    self.adapters.device.push(&member, &target, false)?;
                }
                output_paths.push(json!(device_dest));
            }
        }
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "extracted_paths".to_string(),
            RuntimeValue {
                type_name: "path_list".to_string(),
                value: Value::Array(output_paths),
                location: Some(extract_on.to_string()),
            },
        );
        Ok(outputs)
    }

    fn execute_extract_archive(
        &mut self,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let extract_on = resolved_params
            .get("extract_on")
            .and_then(Value::as_str)
            .unwrap_or("host");
        if !matches!(extract_on, "host" | "device") {
            return Err(StepFailure::new(format!(
                "extract_archive extract_on must be 'host' or 'device', got {extract_on:?}"
            )));
        }
        let archive = runtime_value_param(resolved_params, "archive")?;
        if archive.location.as_deref() != Some("host") {
            return Err(StepFailure::new(
                "Rust host-side extraction requires a host-side archive path.".to_string(),
            ));
        }
        let Some(archive_path) = archive.value.as_str() else {
            return Err(StepFailure::new(
                "extract_archive archive runtime value must contain a string path".to_string(),
            ));
        };
        let archive_path = PathBuf::from(archive_path);
        let sandbox = self.adapters.sandbox()?.clone();
        sandbox.ensure_read_allowed(&archive_path)?;
        let extract_root = sandbox
            .runtime_root
            .join("extract")
            .join(sanitize_step_id(&step.id));
        let members = sandbox.extract_zip_to_directory(&archive_path, &extract_root)?;
        if extract_on == "device" {
            let operation = (|| {
                let dest = resolved_params
                    .get("dest")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StepFailure::new(
                            "extract_archive targeting a device requires a string dest".to_string(),
                        )
                    })?;
                self.adapters.device.mkdir_p(dest)?;
                for member in &members {
                    let name = member.file_name().ok_or_else(|| {
                        StepFailure::new(format!(
                            "extracted path has no basename: {}",
                            member.display()
                        ))
                    })?;
                    let target = join_device_path(dest, &name.to_string_lossy());
                    self.adapters.device.push(member, &target, false)?;
                }
                let mut outputs = OrderedMap::new();
                outputs.insert(
                    "extracted_path".to_string(),
                    RuntimeValue {
                        type_name: "directory_path".to_string(),
                        value: json!(dest),
                        location: Some("device".to_string()),
                    },
                );
                Ok(outputs)
            })();
            // Device-targeted extraction never exposes host staging as an output,
            // so callers cannot opt out of removing it.
            let cleanup = remove_host_staging(&extract_root);
            return combine_operation_and_cleanup(operation, cleanup);
        }
        let (type_name, value) = if members.len() == 1 {
            let member = &members[0];
            let type_name = if member.is_dir() {
                "directory_path"
            } else {
                "file_path"
            };
            (type_name, member.to_string_lossy().to_string())
        } else {
            ("directory_path", extract_root.to_string_lossy().to_string())
        };
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "extracted_path".to_string(),
            RuntimeValue {
                type_name: type_name.to_string(),
                value: json!(value),
                location: Some("host".to_string()),
            },
        );
        Ok(outputs)
    }

    fn reviewed_copy_commit_request(
        &self,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
        source_path: &Path,
        sandbox: &SandboxRoots,
        destination: (&str, CopyCommitDestinationKind),
        operation: CopyCommitOperation,
    ) -> Result<CopyCommitRequest, StepFailure> {
        let (destination_identity, destination_kind) = destination;
        let mut snapshots = plan
            .copy_commit_snapshots
            .iter()
            .filter(|snapshot| snapshot.step_id == step.id);
        let Some(snapshot) = snapshots.next() else {
            return Err(StepFailure::new(
                "reviewed copy snapshot is missing; prepare a fresh review before execution"
                    .to_string(),
            ));
        };
        if snapshots.next().is_some() {
            return Err(StepFailure::new(
                "reviewed copy snapshot is duplicated".to_string(),
            ));
        }
        let request = if self.adapters.device.uses_fake_copy_commit_adapter()
            && (snapshot.destination_binding.is_some()
                || destination_kind == CopyCommitDestinationKind::SetupLocation)
        {
            snapshot.simulation_request_for_execution(
                &step.id,
                source_path,
                &sandbox.fake_device_root,
                destination_identity,
                destination_kind,
                operation,
            )
        } else {
            snapshot.request_for_execution(
                &step.id,
                source_path,
                &sandbox.fake_device_root,
                destination_identity,
                destination_kind,
                operation,
            )
        };
        let mut request = request.map_err(|error| StepFailure::new(error.to_string()))?;
        if destination_kind == CopyCommitDestinationKind::SetupLocation {
            request.setup_location_lease = self.setup_location_lease.clone();
        }
        Ok(request)
    }

    fn execute_copy_files(
        &mut self,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let source = runtime_value_param(resolved_params, "source")?;
        let setup_destination = runtime_value_param(resolved_params, "dest")
            .ok()
            .filter(|runtime| runtime.type_name == "setup_location_ref");
        let dest = if let Some(runtime) = &setup_destination {
            runtime.value.as_str().ok_or_else(|| {
                StepFailure::new("copy_files Setup Location reference must be an ID".to_string())
            })?
        } else {
            resolved_params
                .get("dest")
                .and_then(Value::as_str)
                .ok_or_else(|| StepFailure::new("copy_files dest must be a string".to_string()))?
        };
        let copy_policy = resolved_params
            .get("copy_policy")
            .and_then(Value::as_str)
            .unwrap_or("merge");
        let dest_kind = resolved_params.get("dest_kind").and_then(Value::as_str);
        if dest_kind.is_some_and(|kind| !matches!(kind, "file" | "directory")) {
            return Err(StepFailure::new(
                "copy_files dest_kind must be file or directory".to_string(),
            ));
        }
        if !matches!(copy_policy, "merge" | "replace" | "sync") {
            return Err(StepFailure::new(format!(
                "copy_files copy_policy must be merge, replace, or sync, got {copy_policy:?}"
            )));
        }
        if source.type_name == "path_list" && copy_policy == "sync" {
            return Err(StepFailure::new(
                "copy_files unsupported: sync path_list has no single reviewed source boundary"
                    .to_string(),
            ));
        }
        let sandbox = self.adapters.sandbox()?.clone();
        let app_private_dest = root_requirements::is_app_private_path(dest);
        let destination_kind =
            copy_commit_destination_kind(step, resolved_params).map_err(StepFailure::new)?;
        let selected_directory = destination_kind.is_some();
        if source.type_name == "directory_path"
            && matches!(copy_policy, "replace" | "sync")
            && !selected_directory
        {
            return Err(StepFailure::new(
                "copy_files unsupported: destructive directory operations require a reviewed App Target destination"
                    .to_string(),
            ));
        }
        if source.type_name == "path_list" && copy_policy == "replace" {
            return Err(StepFailure::new(
                "copy_files unsupported: replacing a path list would delete the selected destination tree"
                    .to_string(),
            ));
        }
        let app_private_source = source
            .location
            .as_deref()
            .is_some_and(|location| location == "device")
            && root_requirements::is_app_private_path(&value_to_string(&source.value));
        if app_private_dest || app_private_source {
            self.ensure_root_preflight()?;
        }
        if app_private_dest
            && !(plan.runtime_capabilities.app_data_write && plan.runtime_capabilities.root_shell)
        {
            return Err(StepFailure::new(format!(
                "app_data_write_unavailable: Destination {dest:?} requires both app_data_write and root_shell runtime capabilities."
            )));
        }
        if dest_kind == Some("file") {
            if source.type_name != "file_path" || copy_policy == "sync" {
                return Err(StepFailure::new(
                    "copy_files exact file target requires a single file and non-sync policy"
                        .to_string(),
                ));
            }
            if self.adapters.device.path_is_dir(dest)?
                || (self.adapters.device.uses_fake_device_filesystem()
                    && sandbox.fake_device_path(dest)?.is_dir())
            {
                return Err(StepFailure::new(
                    "copy_files exact file target is an existing directory".to_string(),
                ));
            }
        } else if dest_kind == Some("directory") && source.type_name == "file_path" {
            self.adapters.device.mkdir_p(dest)?;
        }
        let (copied_paths, copy_commit_report) = if self
            .adapters
            .device
            .uses_fake_copy_commit_adapter()
            && selected_directory
            && source.location.as_deref() == Some("host")
            && source.type_name == "directory_path"
        {
            let source_path = source_path_from_value(&source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            let operation = CopyCommitOperation::from_legacy_directory_policy(copy_policy)
                .map_err(|error| StepFailure::new(error.to_string()))?;
            let request = self.reviewed_copy_commit_request(
                plan,
                step,
                &source_path,
                &sandbox,
                (
                    dest,
                    destination_kind.expect("selected directory has a destination kind"),
                ),
                operation,
            )?;
            let mut adapter = FakeFilesystemCopyCommitAdapter;
            let report = adapter.commit(&request).map_err(StepFailure::from)?;
            if !report.succeeded {
                return Err(copy_commit_report_failure(report));
            }
            let copied: Vec<String> = request
                .source_manifest
                .iter()
                .filter(|entry| !entry.relative_path.contains('/'))
                .map(|entry| join_device_path(dest, &entry.relative_path))
                .collect();
            (copied, Some(report))
        } else if selected_directory
            && !self.adapters.device.uses_fake_copy_commit_adapter()
            && source.location.as_deref() == Some("host")
            && source.type_name == "directory_path"
        {
            let source_path = source_path_from_value(&source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            let operation = CopyCommitOperation::from_legacy_directory_policy(copy_policy)
                .map_err(|error| StepFailure::new(error.to_string()))?;
            let request = self.reviewed_copy_commit_request(
                plan,
                step,
                &source_path,
                &sandbox,
                (
                    dest,
                    destination_kind.expect("selected directory has a destination kind"),
                ),
                operation,
            )?;
            let mut adapter = AndroidCopyCommitAdapter::new(&mut self.adapters.device);
            let report = adapter.commit(&request).map_err(StepFailure::from)?;
            if !report.succeeded {
                return Err(copy_commit_report_failure(report));
            }
            let copied: Vec<String> = request
                .source_manifest
                .iter()
                .filter(|entry| !entry.relative_path.contains('/'))
                .map(|entry| join_device_path(dest, &entry.relative_path))
                .collect();
            (copied, Some(report))
        } else if self.adapters.device.uses_fake_device_filesystem()
            && source.location.as_deref() == Some("host")
            && !app_private_dest
        {
            (
                sandbox.copy_host_source_to_fake_device(&source, dest, copy_policy, dest_kind)?,
                None,
            )
        } else if source.location.as_deref() == Some("device") {
            (
                copy_device_source(&mut self.adapters.device, &source, dest, copy_policy)?,
                None,
            )
        } else if app_private_dest {
            (
                copy_host_source_to_app_private(
                    &mut self.adapters.device,
                    &sandbox,
                    step,
                    &source,
                    dest,
                    copy_policy,
                )?,
                None,
            )
        } else {
            (
                copy_host_source(
                    &mut self.adapters.device,
                    &sandbox,
                    &source,
                    dest,
                    copy_policy,
                )?,
                None,
            )
        };
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "copied_paths".to_string(),
            RuntimeValue {
                type_name: "path_list".to_string(),
                value: Value::Array(copied_paths.into_iter().map(Value::String).collect()),
                location: Some("device".to_string()),
            },
        );
        if let Some(report) = copy_commit_report {
            let value = serde_json::to_value(report).map_err(|error| {
                StepFailure::new(format!(
                    "copy commit report could not be serialized: {error}"
                ))
            })?;
            outputs.insert(
                "copy_commit_report".to_string(),
                RuntimeValue {
                    type_name: "copy_commit_report".to_string(),
                    value,
                    location: Some("device".to_string()),
                },
            );
        }
        Ok(outputs)
    }

    fn execute_install_apk(
        &mut self,
        plan: &ExecutionPlan,
        state: &ExecutionState,
        step: &ExecutionStep,
        resolved_params: &OrderedMap<Value>,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let app = runtime_value_param(resolved_params, "app")?;
        if app.type_name != "file_path" || app.location.as_deref() != Some("host") {
            return Err(StepFailure::new(
                "install_apk requires a host-side file_path runtime value.".to_string(),
            ));
        }
        let apk_path = PathBuf::from(value_to_string(&app.value));
        if apk_path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| !extension.eq_ignore_ascii_case("apk"))
            .unwrap_or(true)
        {
            return Err(StepFailure::new(format!(
                "install_apk requires an .apk file, got: {}",
                apk_path.display()
            )));
        }
        if !apk_path.exists() {
            return Err(StepFailure::new(format!(
                "APK file not found: {}",
                apk_path.display()
            )));
        }
        let app_id = step.app_id.as_deref().ok_or_else(|| {
            StepFailure::new(
                "app_context_invalid: APK installation requires reviewed App Definition context."
                    .to_string(),
            )
        })?;
        let app = plan
            .apps
            .iter()
            .find(|app| app.id == app_id)
            .ok_or_else(|| {
                StepFailure::new(
                    "app_context_invalid: Reviewed App Definition data is unavailable.".to_string(),
                )
            })?;
        let path = apk_path.to_string_lossy();
        let provenance =
            plan.artifacts.iter().find(|artifact| {
                artifact.app_provenance.as_ref().is_some_and(|provenance| {
                    provenance.app_id == app_id && provenance.kind == "apk"
                }) && artifact.type_name == "remote_file"
                    && state
                        .artifacts
                        .get(&artifact.id)
                        .and_then(|artifact| artifact.local_path.as_deref())
                        == Some(path.as_ref())
            });
        if provenance.is_none() {
            return Err(StepFailure::new(
                "app_install_artifact_provenance_invalid: Reviewed APK artifact provenance is unavailable.".to_string(),
            ));
        }
        let expected_package_name = &app.package_id;
        let manifest = inspect_apk_manifest(&apk_path).map_err(|error| {
            StepFailure::new(format!(
                "apk_package_inspection_failed: manifest inspection failed with reason '{}'.",
                error.code()
            ))
        })?;
        if manifest.package_name != *expected_package_name {
            return Err(StepFailure::new(format!(
                "apk_package_mismatch: expected package '{}', actual package '{}'.",
                expected_package_name, manifest.package_name
            )));
        }
        let replace_existing = resolved_params
            .get("replace_existing")
            .map(value_is_truthy)
            .unwrap_or(false);
        self.adapters
            .device
            .install_apk(&apk_path, replace_existing)?;
        self.adapters
            .device
            .record_installed_package(expected_package_name);
        Ok(OrderedMap::new())
    }

    fn execute_launch_app(
        &mut self,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let (package, activity) = crate::planner::validated_plan_lifecycle_target(plan, step)
            .map_err(|reason| {
                StepFailure::new(format!("App launch identity is invalid: {reason}."))
            })?;
        self.adapters
            .device
            .launch_app(&package, activity.as_deref())?;
        Ok(OrderedMap::new())
    }

    fn execute_force_stop_app(
        &mut self,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
    ) -> Result<OrderedMap<RuntimeValue>, StepFailure> {
        let (package, _) =
            crate::planner::validated_plan_lifecycle_target(plan, step).map_err(|reason| {
                StepFailure::new(format!("App force-stop identity is invalid: {reason}."))
            })?;
        self.adapters.device.force_stop_app(&package)?;
        Ok(OrderedMap::new())
    }

    fn evaluate_condition(
        &mut self,
        plan: &ExecutionPlan,
        condition: &ExecutionStepCondition,
    ) -> Result<bool, StepFailure> {
        match condition.type_name.as_str() {
            "package_installed" => {
                let app_id = condition.app_id.as_deref().ok_or_else(|| {
                    StepFailure::new(
                        "app_condition_context_required: package_installed requires App Definition context.".to_string(),
                    )
                })?;
                if !condition.params.is_empty() {
                    return Err(StepFailure::new(
                        "app_condition_package_parameter_forbidden: App-context package_installed conditions are parameterless.".to_string(),
                    ));
                }
                let package_name = plan
                    .apps
                    .iter()
                    .find(|app| app.id == app_id)
                    .map(|app| app.package_id.clone())
                    .ok_or_else(|| {
                        StepFailure::new(
                            "app_context_invalid: Reviewed App Definition data is unavailable."
                                .to_string(),
                        )
                    })?;
                self.adapters
                    .device
                    .package_installed(&package_name)
                    .map_err(StepFailure::from)
            }
            "path_exists" => {
                let path = required_string_param(condition, "path").map_err(StepFailure::new)?;
                if root_requirements::is_app_private_path(&path) {
                    self.ensure_root_preflight()?;
                }
                let device_exists = self
                    .adapters
                    .device
                    .path_exists(&path)
                    .map_err(StepFailure::from)?;
                if !self.adapters.device.uses_fake_device_filesystem() {
                    return Ok(device_exists);
                }
                let sandbox_exists = self
                    .adapters
                    .sandbox()
                    .and_then(|sandbox| sandbox.fake_device_path(&path))
                    .map(|path| path.exists())
                    .unwrap_or(false);
                Ok(device_exists || sandbox_exists)
            }
            "file_exists" => {
                let path = required_string_param(condition, "path").map_err(StepFailure::new)?;
                if root_requirements::is_app_private_path(&path) {
                    self.ensure_root_preflight()?;
                }
                let device_exists = self
                    .adapters
                    .device
                    .path_exists(&path)
                    .map_err(StepFailure::from)?;
                let device_is_dir = self
                    .adapters
                    .device
                    .path_is_dir(&path)
                    .map_err(StepFailure::from)?;
                if !self.adapters.device.uses_fake_device_filesystem() {
                    return Ok(device_exists && !device_is_dir);
                }
                let sandbox_is_file = self
                    .adapters
                    .sandbox()
                    .and_then(|sandbox| sandbox.fake_device_path(&path))
                    .map(|path| path.is_file())
                    .unwrap_or(false);
                Ok((device_exists && !device_is_dir) || sandbox_is_file)
            }
            other => Err(StepFailure::new(format!(
                "Unsupported condition type: {other}"
            ))),
        }
    }
}

pub(crate) fn plan_requires_copy_commit_preparation(plan: &ExecutionPlan) -> bool {
    if validate_supported_copy_plan(plan).is_err() {
        return true;
    }
    let state = ExecutionState::from_plan(plan);
    plan.steps
        .iter()
        .filter(|step| step.type_name == "copy_files")
        .filter(|step| !has_runtime_output_reference(step))
        .any(|step| {
            resolve_step_params(&state, &step.params)
                .ok()
                .is_some_and(|params| is_selected_directory_step(step, &params).unwrap_or(false))
        })
}

fn has_runtime_output_reference(step: &ExecutionStep) -> bool {
    step.params.values().any(|param| {
        matches!(
            param,
            crate::planner::ExecutionParamValue::Ref { ref_value }
                if ref_value.starts_with("steps.")
        )
    })
}

fn has_runtime_source_output_reference(step: &ExecutionStep) -> bool {
    matches!(
        step.params.get("source"),
        Some(crate::planner::ExecutionParamValue::Ref { ref_value })
            if ref_value.starts_with("steps.")
    )
}

fn has_setup_location_destination_with_runtime_source(
    state: &ExecutionState,
    step: &ExecutionStep,
) -> Result<bool, String> {
    if !has_runtime_source_output_reference(step) {
        return Ok(false);
    }
    let Some(destination) = step.params.get("dest") else {
        return Ok(false);
    };
    if matches!(
        destination,
        crate::planner::ExecutionParamValue::Ref { ref_value }
            if ref_value.starts_with("steps.")
    ) {
        return Ok(false);
    }

    let mut destination_params = OrderedMap::new();
    destination_params.insert("dest".to_string(), destination.clone());
    let resolved = resolve_step_params(state, &destination_params).map_err(|error| {
        format!(
            "copy_files step {} has an invalid destination ({})",
            step.id, error.message
        )
    })?;
    Ok(runtime_value_param(&resolved, "dest")
        .is_ok_and(|value| value.type_name == "setup_location_ref"))
}

fn is_selected_directory_step(
    step: &ExecutionStep,
    params: &OrderedMap<Value>,
) -> Result<bool, String> {
    Ok(copy_commit_destination_kind(step, params)?.is_some())
}

fn copy_commit_destination_kind(
    step: &ExecutionStep,
    params: &OrderedMap<Value>,
) -> Result<Option<CopyCommitDestinationKind>, String> {
    let setup_destination = runtime_value_param(params, "dest")
        .ok()
        .filter(|runtime| runtime.type_name == "setup_location_ref");
    if let Some(setup_destination) = setup_destination {
        if params
            .get("dest_kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "directory")
        {
            return Err(format!(
                "copy_files step {} Setup Location destination must be a directory",
                step.id
            ));
        }
        let source = runtime_value_param(params, "source").map_err(|error| {
            format!(
                "copy_files step {} has an invalid Setup Location source: {}",
                step.id, error.message
            )
        })?;
        if source.type_name != "directory_path" || source.location.as_deref() != Some("host") {
            return Err(format!(
                "copy_files step {} Setup Location requires one host directory source",
                step.id
            ));
        }
        let source_path = source_path_from_value(&source).map_err(|error| error.message)?;
        let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
            format!(
                "copy_files step {} host directory source is unavailable: {error}",
                step.id
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "copy_files step {} Setup Location source must be one non-symlink host directory",
                step.id
            ));
        }
        let copy_policy = params
            .get("copy_policy")
            .and_then(Value::as_str)
            .unwrap_or("merge");
        CopyCommitOperation::from_legacy_directory_policy(copy_policy)
            .map_err(|error| error.to_string())?;
        if setup_destination.value.as_str().is_none_or(str::is_empty) {
            return Err(format!(
                "copy_files step {} Setup Location reference must resolve to a non-empty ID",
                step.id
            ));
        }
        return Ok(Some(CopyCommitDestinationKind::SetupLocation));
    }

    if params
        .get("dest_kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "directory")
    {
        return Ok(None);
    }
    let source = match runtime_value_param(params, "source") {
        Ok(source) => source,
        Err(_) => return Ok(None),
    };
    if source.type_name != "directory_path" || source.location.as_deref() != Some("host") {
        return Ok(None);
    }
    let Some(destination) = params.get("dest").and_then(Value::as_str) else {
        return Ok(None);
    };
    let destination_kind = if root_requirements::is_app_private_path(destination) {
        if step.app_id.is_none() {
            return Err(format!(
                "copy_files step {} app-private destination requires an authored App Target",
                step.id
            ));
        }
        CopyCommitDestinationKind::AppTarget
    } else if is_reviewable_shared_device_path(destination) {
        CopyCommitDestinationKind::SharedDevicePath
    } else {
        return Ok(None);
    };
    let source_path = source_path_from_value(&source).map_err(|error| error.message)?;
    if !source_path.is_dir() {
        return Err(format!(
            "copy_files step {} selected destination source must be one existing host directory",
            step.id
        ));
    }
    Ok(Some(destination_kind))
}

fn active_setup_location_binding(
    storage_targets: &[StorageTargetBinding],
    location_id: &str,
    plan: &ExecutionPlan,
) -> Result<StorageTargetBinding, String> {
    let mut matches = storage_targets
        .iter()
        .filter(|binding| binding.id == location_id);
    let binding = matches
        .next()
        .ok_or_else(|| "active Setup Location binding is missing".to_string())?;
    if matches.next().is_some() {
        return Err("active Setup Location binding is duplicated".to_string());
    }
    binding.validate()?;
    if plan
        .device_plan
        .as_ref()
        .is_none_or(|device_plan| device_plan.id != binding.device_plan)
    {
        return Err("active Setup Location belongs to another Device Plan".to_string());
    }
    Ok(binding.clone())
}

/// Structured internal identity keeps valid path characters separate from device metadata.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CopyCommitPathBinding {
    canonical_path: String,
    parent_identity: String,
    selected_identity: String,
}

pub(crate) fn canonical_path_from_binding(value: &str) -> Result<PathBuf, String> {
    let canonical = match serde_json::from_str::<CopyCommitPathBinding>(value) {
        Ok(binding) => {
            if binding.parent_identity.is_empty() || binding.selected_identity.is_empty() {
                return Err("selected path has incomplete identity".to_string());
            }
            binding.canonical_path
        }
        Err(_) => legacy_canonical_path(value)?,
    };
    canonical_path_from_scope(&canonical)
}

fn legacy_canonical_path(value: &str) -> Result<String, String> {
    let Some((canonical, identity_fields)) = value.split_once("|parent:") else {
        return Ok(value.to_string());
    };
    let Some((parent_identity, selected_identity)) = identity_fields.split_once("|selected:")
    else {
        return Err("selected path has incomplete identity".to_string());
    };
    if canonical.is_empty()
        || parent_identity.is_empty()
        || selected_identity.is_empty()
        || selected_identity.contains('|')
    {
        return Err("selected path has incomplete identity".to_string());
    }
    Ok(canonical.to_string())
}

pub(crate) fn canonical_path_from_scope(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("selected path canonical identity is not absolute".to_string());
    }
    Ok(path)
}

pub(crate) fn path_binding_private_values(value: &str) -> Vec<String> {
    let mut private_values = vec![value.to_string()];
    match serde_json::from_str::<CopyCommitPathBinding>(value) {
        Ok(binding) => private_values.extend([
            binding.canonical_path,
            binding.parent_identity,
            binding.selected_identity,
        ]),
        Err(_) => private_values.extend(value.split('|').map(ToString::to_string)),
    }
    private_values
}

#[cfg(test)]
mod canonical_path_from_binding_tests {
    use super::canonical_path_from_binding;
    use std::path::PathBuf;

    #[test]
    fn canonical_path_binding_preserves_pipe_characters_in_the_path() {
        let canonical_path = "/storage/ABCD-1234/Games|Retro";
        let binding = serde_json::json!({
            "canonicalPath": canonical_path,
            "parentIdentity": "253:102",
            "selectedIdentity": "253:103",
        })
        .to_string();

        assert_eq!(
            canonical_path_from_binding(&binding).unwrap(),
            PathBuf::from(canonical_path),
        );
    }
}

fn verify_setup_destination_precondition_with_device<D: ExecutorDevice>(
    device: &mut D,
    binding: &StorageTargetBinding,
    precondition: &SetupDestinationPrecondition,
) -> Result<(), String> {
    device
        .copy_commit_verify_setup_location(binding)
        .map_err(|error| error.message)?;
    let components = binding.relative_path.split('/').collect::<Vec<_>>();
    if precondition.missing_components.len() > components.len() {
        return Err("Setup Location creation precondition is inconsistent".to_string());
    }
    let existing_count = components.len() - precondition.missing_components.len();
    if components[existing_count..]
        .iter()
        .copied()
        .ne(precondition.missing_components.iter().map(String::as_str))
    {
        return Err(
            "Setup Location missing components do not match the reviewed destination".to_string(),
        );
    }
    let expected_anchor = components[..existing_count].iter().fold(
        PathBuf::from(&binding.mount_root),
        |mut path, component| {
            path.push(component);
            path
        },
    );
    if expected_anchor.to_string_lossy() != precondition.existing_anchor_path
        || !device
            .path_is_dir(&precondition.existing_anchor_path)
            .map_err(|error| error.message)?
    {
        return Err("Setup Location creation anchor changed after review".to_string());
    }
    if precondition.existing_anchor_path != binding.mount_root
        && device
            .copy_commit_path_is_symlink(&precondition.existing_anchor_path)
            .map_err(|error| error.message)?
    {
        return Err("Setup Location creation anchor is a symbolic link".to_string());
    }
    if device
        .copy_commit_path_binding(&precondition.existing_anchor_path)
        .map_err(|error| error.message)?
        != precondition.existing_anchor_binding
    {
        return Err("Setup Location creation anchor identity changed after review".to_string());
    }
    let canonical_root = canonical_path_from_binding(
        &device
            .copy_commit_path_binding(&binding.mount_root)
            .map_err(|error| error.message)?,
    )?;
    let canonical_scope = components
        .iter()
        .fold(canonical_root, |mut path, component| {
            path.push(component);
            path
        });
    if canonical_scope.to_string_lossy() != precondition.canonical_scope {
        return Err("Setup Location canonical destination changed after review".to_string());
    }
    let mut missing_path = PathBuf::from(&precondition.existing_anchor_path);
    for component in &precondition.missing_components {
        missing_path.push(component);
        let path = missing_path.to_string_lossy();
        if device.path_exists(&path).map_err(|error| error.message)?
            || device
                .copy_commit_path_is_symlink(&path)
                .map_err(|error| error.message)?
        {
            return Err("Setup Location creation precondition changed after review".to_string());
        }
    }
    Ok(())
}

fn inspect_android_setup_destination<D: ExecutorDevice>(
    device: &mut D,
    binding: &StorageTargetBinding,
) -> Result<(SetupDestinationPrecondition, Vec<CopyManifestEntry>), String> {
    binding.validate()?;
    device
        .copy_commit_verify_setup_location(binding)
        .map_err(|error| error.message)?;
    let root_binding = device
        .copy_commit_path_binding(&binding.mount_root)
        .map_err(|error| error.message)?;
    let canonical_root = canonical_path_from_binding(&root_binding)?;
    if !device
        .path_is_dir(&binding.mount_root)
        .map_err(|error| error.message)?
    {
        return Err("Setup Location volume root is not an accessible directory".to_string());
    }

    let components = binding.relative_path.split('/').collect::<Vec<_>>();
    let mut lexical_path = PathBuf::from(&binding.mount_root);
    let mut canonical_path = canonical_root;
    let mut existing_anchor_path = binding.mount_root.clone();
    let mut existing_anchor_binding = root_binding;
    let mut missing_components = Vec::new();
    let mut found_missing = false;
    for (index, component) in components.iter().enumerate() {
        lexical_path.push(component);
        canonical_path.push(component);
        let path = lexical_path.to_string_lossy();
        let is_symlink = device
            .copy_commit_path_is_symlink(&path)
            .map_err(|error| error.message)?;
        let exists = device.path_exists(&path).map_err(|error| error.message)?;
        if is_symlink {
            return Err("Setup Location destination contains a symbolic link".to_string());
        }
        if found_missing || !exists {
            if exists {
                return Err("Setup Location destination is discontinuous".to_string());
            }
            found_missing = true;
            missing_components.push((*component).to_string());
            continue;
        }
        if !device.path_is_dir(&path).map_err(|error| error.message)? {
            return Err(
                "Setup Location destination contains a non-directory component".to_string(),
            );
        }
        let path_binding = device
            .copy_commit_path_binding(&path)
            .map_err(|error| error.message)?;
        if canonical_path_from_binding(&path_binding)? != canonical_path {
            return Err("Setup Location destination escaped its reviewed volume".to_string());
        }
        existing_anchor_path = path.into_owned();
        existing_anchor_binding = path_binding;
        if index + 1 == components.len() {
            found_missing = false;
        }
    }
    let precondition = SetupDestinationPrecondition {
        canonical_scope: canonical_path.to_string_lossy().into_owned(),
        existing_anchor_path,
        existing_anchor_binding,
        missing_components,
    };
    verify_setup_destination_precondition_with_device(device, binding, &precondition)?;
    let destination_manifest = if precondition.missing_components.is_empty() {
        let destination = binding
            .destination_path()
            .map_err(|_| "Setup Location destination path is invalid".to_string())?;
        device
            .copy_commit_tree_manifest(&destination.to_string_lossy())
            .map_err(|error| error.message)?
    } else {
        Vec::new()
    };
    Ok((precondition, destination_manifest))
}

fn validate_active_copy_writer_collisions(
    snapshots: &[ExecutionCopyCommitSnapshot],
) -> Result<(), String> {
    if !snapshots
        .iter()
        .any(|snapshot| snapshot.destination_kind == CopyCommitDestinationKind::SetupLocation)
    {
        return Ok(());
    }
    let removable_volume_roots = active_removable_volume_roots(snapshots)?;
    let active = snapshots
        .iter()
        .filter(|snapshot| {
            matches!(
                snapshot.destination_kind,
                CopyCommitDestinationKind::SharedDevicePath
                    | CopyCommitDestinationKind::SetupLocation
            )
        })
        .map(|snapshot| {
            let scope = active_copy_writer_scope(snapshot)?;
            let scope = canonical_active_writer_scope(scope, &removable_volume_roots)?;
            let (effects, creation_paths) = copy_writer_effects(snapshot, &scope)?;
            Ok((snapshot, effects, creation_paths))
        })
        .collect::<Result<Vec<_>, String>>()?;

    for (index, (left, left_effects, left_creation_paths)) in active.iter().enumerate() {
        for (right, right_effects, right_creation_paths) in active.iter().skip(index + 1) {
            let creation_conflict = creation_effects_conflict(
                left_creation_paths,
                right_creation_paths,
                left_effects,
                right_effects,
            );
            if creation_conflict || writer_effects_conflict(left_effects, right_effects) {
                return Err(format!(
                    "active copy steps {} and {} have conflicting destination effects",
                    left.step_id, right.step_id
                ));
            }
        }
    }
    Ok(())
}

fn active_removable_volume_roots(
    snapshots: &[ExecutionCopyCommitSnapshot],
) -> Result<Vec<String>, String> {
    snapshots
        .iter()
        .filter(|snapshot| snapshot.destination_kind == CopyCommitDestinationKind::SetupLocation)
        .filter_map(|snapshot| snapshot.setup_location_binding.as_ref())
        .filter(|binding| binding.volume_kind == StorageTargetVolumeKind::RemovableSd)
        .map(|binding| {
            binding.validate()?;
            let volume_id = binding
                .mount_root
                .strip_prefix("/storage/")
                .ok_or_else(|| "Setup Location removable volume root is invalid".to_string())?;
            Ok(volume_id.to_string())
        })
        .collect()
}

/// Checks active legacy shared-storage copy effects against reviewed writers.
/// Plans without an active Setup Location keep the existing T03 execution path.
fn validate_unreviewed_legacy_shared_copy_writers<D: ExecutorDevice>(
    device: &mut D,
    fake_device_root: &Path,
    plan: &ExecutionPlan,
    snapshots: &[ExecutionCopyCommitSnapshot],
) -> Result<(), String> {
    if !snapshots
        .iter()
        .any(|snapshot| snapshot.destination_kind == CopyCommitDestinationKind::SetupLocation)
    {
        return Ok(());
    }

    let removable_volume_roots = active_removable_volume_roots(snapshots)?;
    let mut active = Vec::new();
    for snapshot in snapshots.iter().filter(|snapshot| {
        matches!(
            snapshot.destination_kind,
            CopyCommitDestinationKind::SharedDevicePath | CopyCommitDestinationKind::SetupLocation
        )
    }) {
        let scope = canonical_active_writer_scope(
            active_copy_writer_scope(snapshot)?,
            &removable_volume_roots,
        )?;
        let (effects, creations) = copy_writer_effects(snapshot, &scope)?;
        active.push((snapshot.step_id.clone(), effects, creations));
    }

    let state = ExecutionState::from_plan(plan);
    let mut active_extract_writers: Vec<(String, PathBuf, Vec<PathBuf>)> = Vec::new();
    for step in plan
        .steps
        .iter()
        .filter(|step| matches!(step.type_name.as_str(), "copy_files" | "extract_archive"))
    {
        if step.type_name == "extract_archive" {
            let mut extraction_mode_params = OrderedMap::new();
            if let Some(extract_on) = step.params.get("extract_on") {
                extraction_mode_params.insert("extract_on".to_string(), extract_on.clone());
            }
            let resolved_mode = resolve_step_params(&state, &extraction_mode_params)
                .map_err(|error| error.message)?;
            if resolved_mode
                .get("extract_on")
                .and_then(Value::as_str)
                .unwrap_or("host")
                != "device"
            {
                continue;
            }
            let mut destination_params = OrderedMap::new();
            if let Some(destination) = step.params.get("dest") {
                destination_params.insert("dest".to_string(), destination.clone());
            }
            let resolved_destination =
                resolve_step_params(&state, &destination_params).map_err(|error| error.message)?;
            let destination = resolved_destination
                .get("dest")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "active Setup Location plan has an unresolved device extract destination in step {}",
                        step.id
                    )
                })?;
            let normalized_destination =
                canonical_active_writer_destination(destination, &removable_volume_roots)?;
            if root_requirements::is_app_private_path(&normalized_destination) {
                continue;
            }
            if !is_reviewable_shared_device_path(&normalized_destination) {
                return Err(format!(
                    "active Setup Location plan has an unreviewed device extract destination in step {}",
                    step.id
                ));
            }
            let (scope, creations) = canonical_device_path_for_collision(
                device,
                fake_device_root,
                &normalized_destination,
            )?;
            let scope = canonical_active_writer_scope(scope, &removable_volume_roots)?;
            let creations = canonical_active_writer_paths(creations, &removable_volume_roots)?;
            check_unreviewed_extract_writer_scope(
                &step.id,
                &scope,
                &creations,
                &active,
                &active_extract_writers,
            )?;
            active_extract_writers.push((step.id.clone(), scope, creations));
            continue;
        }
        if snapshots.iter().any(|snapshot| snapshot.step_id == step.id) {
            continue;
        }
        let Some(destination_parameter) = step.params.get("dest") else {
            continue;
        };
        if matches!(
            destination_parameter,
            crate::planner::ExecutionParamValue::Ref { ref_value }
                if ref_value.starts_with("steps.")
        ) {
            return Err(format!(
                "active Setup Location plan has unresolved copy destination in step {}",
                step.id
            ));
        }

        let mut destination_params = OrderedMap::new();
        destination_params.insert("dest".to_string(), destination_parameter.clone());
        if let Some(destination_kind) = step.params.get("dest_kind") {
            destination_params.insert("dest_kind".to_string(), destination_kind.clone());
        }
        let resolved_destination =
            resolve_step_params(&state, &destination_params).map_err(|error| error.message)?;
        let destination_value = &resolved_destination["dest"];
        if destination_value.get("type").and_then(Value::as_str) == Some("setup_location_ref") {
            return Err(format!(
                "active Setup Location plan has an unreviewed Setup Location writer in step {}",
                step.id
            ));
        }
        let Some(destination) = destination_value.as_str() else {
            continue;
        };
        if !Path::new(destination).is_absolute() {
            continue;
        }
        // Canonicalize before deciding this writer is outside the reviewable
        // shared-path set. Path components collapse repeated and trailing
        // separators, matching the destination scope used by copy_files.
        let normalized_destination =
            canonical_active_writer_destination(destination, &removable_volume_roots)?;
        if root_requirements::is_app_private_path(&normalized_destination)
            || !is_reviewable_shared_device_path(&normalized_destination)
        {
            continue;
        }
        let destination = normalized_destination.as_str();

        let source_is_deferred = has_runtime_source_output_reference(step);
        let resolved_params = if source_is_deferred {
            resolved_destination.clone()
        } else {
            resolve_step_params(&state, &step.params).map_err(|error| error.message)?
        };
        let destination_kind = resolved_params.get("dest_kind").and_then(Value::as_str);
        if source_is_deferred {
            let (scope, _) =
                canonical_device_path_for_collision(device, fake_device_root, destination)?;
            let scope = canonical_active_writer_scope(scope, &removable_volume_roots)?;
            let effects = if destination_kind == Some("file") {
                vec![WriterEffect::WriteFile(scope)]
            } else {
                vec![WriterEffect::DeleteDirectory(scope)]
            };
            check_unreviewed_writer_effects(
                &step.id,
                &effects,
                &[],
                &active,
                &active_extract_writers,
            )?;
            active.push((step.id.clone(), effects, Vec::new()));
            continue;
        }

        let source =
            runtime_value_param(&resolved_params, "source").map_err(|error| error.message)?;
        let is_reviewable_device_source =
            matches!(source.type_name.as_str(), "file_path" | "path_list")
                && source.location.as_deref() == Some("device");
        if source.location.as_deref() != Some("host") && !is_reviewable_device_source {
            continue;
        }
        let copy_policy = resolved_params
            .get("copy_policy")
            .and_then(Value::as_str)
            .unwrap_or("merge");
        let mut effects = Vec::new();
        let mut creations = Vec::new();

        match source.type_name.as_str() {
            "file_path" => {
                let source_path = source_path_from_value(&source).map_err(|error| error.message)?;
                let basename = source_path
                    .file_name()
                    .ok_or_else(|| format!("copy_files step {} has no source filename", step.id))?;
                let destination_is_directory = match destination_kind {
                    Some("file") => false,
                    Some("directory") => true,
                    _ => device
                        .path_is_dir(destination)
                        .map_err(|error| error.message)?,
                };
                if destination_kind == Some("file")
                    && device
                        .path_is_dir(destination)
                        .map_err(|error| error.message)?
                {
                    return Err(format!(
                        "copy_files step {} exact file destination is a directory",
                        step.id
                    ));
                }
                let (destination_path, _) =
                    canonical_device_path_for_collision(device, fake_device_root, destination)?;
                let target = if destination_is_directory {
                    destination_path.join(basename)
                } else {
                    destination_path.clone()
                };
                if copy_policy == "replace" {
                    effects.push(WriterEffect::DeleteFile(target.clone()));
                }
                effects.push(WriterEffect::WriteFile(target));
                if !destination_is_directory {
                    if let Some(parent) = Path::new(destination).parent() {
                        if parent != Path::new("/") {
                            add_legacy_directory_effect(
                                device,
                                fake_device_root,
                                &parent.to_string_lossy(),
                                &mut effects,
                                &mut creations,
                            )?;
                        }
                    }
                }
            }
            "path_list" => {
                if destination_kind == Some("file") || copy_policy == "replace" {
                    continue;
                }
                let paths = source
                    .value
                    .as_array()
                    .and_then(|values| values.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
                    .ok_or_else(|| {
                        format!("copy_files step {} has an invalid path list", step.id)
                    })?;
                let destination_path = add_legacy_directory_effect(
                    device,
                    fake_device_root,
                    destination,
                    &mut effects,
                    &mut creations,
                )?;
                for path in paths {
                    let basename = Path::new(path).file_name().ok_or_else(|| {
                        format!("copy_files step {} has a path without a filename", step.id)
                    })?;
                    effects.push(WriterEffect::WriteFile(destination_path.join(basename)));
                }
            }
            "directory_path" => {
                return Err(format!(
                    "active Setup Location plan has an unreviewed directory writer in step {}",
                    step.id
                ));
            }
            _ => continue,
        }

        let effects = canonical_active_writer_effects(effects, &removable_volume_roots)?;
        let creations = canonical_active_writer_paths(creations, &removable_volume_roots)?;
        if effects.is_empty() {
            continue;
        }
        check_unreviewed_writer_effects(
            &step.id,
            &effects,
            &creations,
            &active,
            &active_extract_writers,
        )?;
        active.push((step.id.clone(), effects, creations));
    }
    Ok(())
}

fn check_unreviewed_writer_effects(
    step_id: &str,
    effects: &[WriterEffect],
    creations: &[PathBuf],
    active: &[(String, Vec<WriterEffect>, Vec<PathBuf>)],
    active_extract_writers: &[(String, PathBuf, Vec<PathBuf>)],
) -> Result<(), String> {
    for (active_id, active_effects, active_creations) in active {
        if creation_effects_conflict(creations, active_creations, effects, active_effects)
            || writer_effects_conflict(effects, active_effects)
        {
            return Err(format!(
                "active copy steps {step_id} and {active_id} have conflicting destination effects"
            ));
        }
    }
    for (extract_id, scope, extract_creations) in active_extract_writers {
        if extract_writer_scope_conflicts(scope, extract_creations, effects, creations) {
            return Err(format!(
                "active Setup Location and device extract writers {step_id} and {extract_id} have conflicting destination effects"
            ));
        }
    }
    Ok(())
}

fn check_unreviewed_extract_writer_scope(
    step_id: &str,
    scope: &Path,
    creations: &[PathBuf],
    active: &[(String, Vec<WriterEffect>, Vec<PathBuf>)],
    active_extract_writers: &[(String, PathBuf, Vec<PathBuf>)],
) -> Result<(), String> {
    for (active_id, effects, active_creations) in active {
        if extract_writer_scope_conflicts(scope, creations, effects, active_creations) {
            return Err(format!(
                "active Setup Location and device extract writers {step_id} and {active_id} have conflicting destination effects"
            ));
        }
    }
    for (active_id, active_scope, active_creations) in active_extract_writers {
        if paths_overlap(scope, active_scope)
            || creation_effects_conflict(creations, active_creations, &[], &[])
        {
            return Err(format!(
                "active device extract writers {step_id} and {active_id} have conflicting destination scopes"
            ));
        }
    }
    Ok(())
}

fn extract_writer_scope_conflicts(
    scope: &Path,
    scope_creations: &[PathBuf],
    effects: &[WriterEffect],
    creations: &[PathBuf],
) -> bool {
    effects.iter().any(|effect| match effect {
        WriterEffect::EnsureDirectory(path) => path != scope && path.starts_with(scope),
        _ => paths_overlap(scope, writer_effect_path(effect)),
    }) || creations
        .iter()
        .any(|created| created != scope && created.starts_with(scope))
        || creation_effects_conflict(scope_creations, creations, &[], effects)
}

fn add_legacy_directory_effect<D: ExecutorDevice>(
    device: &mut D,
    fake_device_root: &Path,
    directory: &str,
    effects: &mut Vec<WriterEffect>,
    creations: &mut Vec<PathBuf>,
) -> Result<PathBuf, String> {
    let (canonical, missing_directories) =
        canonical_device_path_for_collision(device, fake_device_root, directory)?;
    if missing_directories.is_empty() {
        if !device
            .path_is_dir(directory)
            .map_err(|error| error.message)?
        {
            return Err("legacy copy parent is not an accessible directory".to_string());
        }
        effects.push(WriterEffect::EnsureDirectory(canonical.clone()));
    } else {
        for path in missing_directories {
            effects.push(WriterEffect::EnsureDirectory(path.clone()));
            creations.push(path);
        }
    }
    Ok(canonical)
}

/// Resolves a destination from an existing trusted ancestor and records missing descendants.
fn canonical_device_path_for_collision<D: ExecutorDevice>(
    device: &mut D,
    fake_device_root: &Path,
    destination: &str,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let logical_path = canonical_shared_destination(destination)?;
    if device.uses_fake_copy_commit_adapter() {
        return canonical_fake_device_path(fake_device_root, &logical_path);
    }

    let mut probe = logical_path.clone();
    let mut missing_components = Vec::new();
    loop {
        let probe_text = probe.to_string_lossy();
        if device
            .path_exists(&probe_text)
            .map_err(|error| error.message)?
        {
            if !missing_components.is_empty()
                && !device
                    .path_is_dir(&probe_text)
                    .map_err(|error| error.message)?
            {
                return Err("legacy copy path has a non-directory missing parent".to_string());
            }
            let binding = device
                .copy_commit_path_binding(&probe_text)
                .map_err(|error| error.message)?;
            let mut canonical = canonical_path_from_binding(&binding)?;
            let mut missing_paths = Vec::new();
            for component in missing_components.iter().rev() {
                canonical.push(component);
                missing_paths.push(canonical.clone());
            }
            return Ok((canonical, missing_paths));
        }
        let Some(name) = probe.file_name() else {
            return Err("legacy copy destination has no existing trusted ancestor".to_string());
        };
        missing_components.push(name.to_os_string());
        let Some(parent) = probe.parent() else {
            return Err("legacy copy destination has no existing trusted ancestor".to_string());
        };
        probe = parent.to_path_buf();
    }
}

fn canonical_fake_device_path(
    fake_device_root: &Path,
    logical_path: &Path,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let root = fs::canonicalize(fake_device_root)
        .map_err(|_| "fake device root is unavailable".to_string())?;
    let relative = fake_device_relative_components(logical_path).map_err(|error| error.message)?;
    let mut probe = root.clone();
    let mut existing = root.clone();
    let mut missing_paths = Vec::new();
    let mut missing = false;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err("fake shared destination has an invalid component".to_string());
        };
        probe.push(name);
        if missing {
            missing_paths.push(probe.clone());
            continue;
        }
        match fs::symlink_metadata(&probe) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("fake shared destination contains a symbolic link".to_string());
            }
            Ok(_) => existing = probe.clone(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing = true;
                missing_paths.push(probe.clone());
            }
            Err(_) => return Err("fake shared destination could not be inspected".to_string()),
        }
    }
    let mut canonical = fs::canonicalize(&existing)
        .map_err(|_| "fake shared destination ancestor is unavailable".to_string())?;
    if !canonical.starts_with(&root) {
        return Err("fake shared destination escaped its device root".to_string());
    }
    let names = missing_paths
        .iter()
        .map(|path| {
            path.file_name()
                .map(std::ffi::OsStr::to_os_string)
                .ok_or_else(|| {
                    "fake shared destination has an invalid missing component".to_string()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut canonical_missing = Vec::with_capacity(names.len());
    for name in names {
        canonical.push(name);
        canonical_missing.push(canonical.clone());
    }
    Ok((canonical, canonical_missing))
}

fn active_copy_writer_scope(snapshot: &ExecutionCopyCommitSnapshot) -> Result<PathBuf, String> {
    match snapshot.destination_kind {
        CopyCommitDestinationKind::AppTarget => {
            Err("App Target is not a shared-storage writer".to_string())
        }
        CopyCommitDestinationKind::SharedDevicePath => match &snapshot.destination_binding {
            Some(binding) => canonical_path_from_binding(binding),
            None => canonical_shared_destination(&snapshot.destination_identity),
        },
        CopyCommitDestinationKind::SetupLocation => {
            let binding = snapshot.setup_location_binding.as_ref().ok_or_else(|| {
                "Setup Location snapshot is missing its trusted volume binding".to_string()
            })?;
            binding.validate()?;
            let precondition = snapshot
                .setup_destination_precondition
                .as_ref()
                .ok_or_else(|| {
                    "Setup Location snapshot is missing its destination precondition".to_string()
                })?;
            canonical_path_from_scope(&precondition.canonical_scope)
        }
    }
}

/// Map the trusted removable-volume alias to the visible root for collision checks.
///
/// Tauri validates Setup Location roots as `/storage/<fsUuid>`, and the device
/// storage classifier recognizes `/mnt/media_rw/<fsUuid>` as the same public
/// volume. This normalized scope is used only to compare active writer effects;
/// reviewed bindings continue to supply the actual mutation paths.
fn canonical_active_writer_scope(
    scope: PathBuf,
    removable_volume_roots: &[String],
) -> Result<PathBuf, String> {
    let Ok(relative) = scope.strip_prefix("/mnt/media_rw") else {
        return Ok(scope);
    };
    let mut components = relative.components();
    let Some(Component::Normal(volume_id)) = components.next() else {
        return Err("removable-volume writer scope has no volume identity".to_string());
    };
    let volume_id = volume_id.to_string_lossy();
    if !removable_volume_roots
        .iter()
        .any(|root| root == volume_id.as_ref())
    {
        return Ok(scope);
    }

    let mut canonical = PathBuf::from("/storage");
    canonical.push(volume_id.as_ref());
    for component in components {
        let Component::Normal(component) = component else {
            return Err("removable-volume writer scope is not canonical".to_string());
        };
        canonical.push(component);
    }
    Ok(canonical)
}

fn canonical_active_writer_destination(
    destination: &str,
    removable_volume_roots: &[String],
) -> Result<String, String> {
    Ok(canonical_active_writer_scope(
        canonical_shared_destination(destination)?,
        removable_volume_roots,
    )?
    .to_string_lossy()
    .into_owned())
}

fn canonical_active_writer_paths(
    paths: Vec<PathBuf>,
    removable_volume_roots: &[String],
) -> Result<Vec<PathBuf>, String> {
    paths
        .into_iter()
        .map(|path| canonical_active_writer_scope(path, removable_volume_roots))
        .collect()
}

fn canonical_active_writer_effects(
    effects: Vec<WriterEffect>,
    removable_volume_roots: &[String],
) -> Result<Vec<WriterEffect>, String> {
    effects
        .into_iter()
        .map(|effect| {
            Ok(match effect {
                WriterEffect::EnsureDirectory(path) => WriterEffect::EnsureDirectory(
                    canonical_active_writer_scope(path, removable_volume_roots)?,
                ),
                WriterEffect::WriteFile(path) => WriterEffect::WriteFile(
                    canonical_active_writer_scope(path, removable_volume_roots)?,
                ),
                WriterEffect::DeleteFile(path) => WriterEffect::DeleteFile(
                    canonical_active_writer_scope(path, removable_volume_roots)?,
                ),
                WriterEffect::DeleteDirectory(path) => WriterEffect::DeleteDirectory(
                    canonical_active_writer_scope(path, removable_volume_roots)?,
                ),
            })
        })
        .collect()
}

pub(crate) fn canonical_shared_destination(destination: &str) -> Result<PathBuf, String> {
    let mut canonical = destination.to_string();
    for (alias, target) in [
        ("/sdcard", "/storage/emulated/0"),
        ("/storage/self/primary", "/storage/emulated/0"),
    ] {
        if canonical == alias {
            canonical = target.to_string();
            break;
        }
        if let Some(suffix) = canonical
            .strip_prefix(alias)
            .filter(|suffix| suffix.starts_with('/'))
        {
            canonical = format!("{target}{suffix}");
            break;
        }
    }
    let path = PathBuf::from(canonical);
    if !path.is_absolute() {
        return Err("shared destination has no safe canonical scope".to_string());
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err("shared destination has no safe canonical scope".to_string());
            }
        }
    }
    Ok(normalized)
}

fn copy_writer_effects(
    snapshot: &ExecutionCopyCommitSnapshot,
    scope: &Path,
) -> Result<(Vec<WriterEffect>, Vec<PathBuf>), String> {
    let mut effects = Vec::new();
    let mut creation_paths = Vec::new();
    for entry in &snapshot.source_manifest {
        let path = scope.join(&entry.relative_path);
        match entry.kind {
            CopyEntryKind::Directory => {
                effects.push(WriterEffect::EnsureDirectory(path.clone()));
                let already_directory = snapshot.destination_before_manifest.iter().any(|before| {
                    before.relative_path == entry.relative_path
                        && before.kind == CopyEntryKind::Directory
                });
                if !already_directory {
                    creation_paths.push(path);
                }
            }
            CopyEntryKind::File => effects.push(WriterEffect::WriteFile(path)),
        }
    }

    if snapshot.destination_kind == CopyCommitDestinationKind::SetupLocation {
        let precondition = snapshot
            .setup_destination_precondition
            .as_ref()
            .ok_or_else(|| {
                "Setup Location snapshot is missing its destination precondition".to_string()
            })?;
        if !precondition.missing_components.is_empty() {
            let mut path = scope.to_path_buf();
            for _ in 0..precondition.missing_components.len() {
                path = path
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| "Setup Location creation scope has no parent".to_string())?;
            }
            for component in &precondition.missing_components {
                path.push(component);
                effects.push(WriterEffect::EnsureDirectory(path.clone()));
                creation_paths.push(path.clone());
            }
            if path != scope {
                return Err(
                    "Setup Location creation components do not match the canonical scope"
                        .to_string(),
                );
            }
        }
    }

    for relative_path in &snapshot.reviewed_deletion_set {
        let kind = snapshot
            .destination_before_manifest
            .iter()
            .find(|entry| entry.relative_path == *relative_path)
            .map(|entry| entry.kind)
            .ok_or_else(|| "copy destination deletion is missing its reviewed entry".to_string())?;
        let path = scope.join(relative_path);
        effects.push(match kind {
            CopyEntryKind::File => WriterEffect::DeleteFile(path),
            CopyEntryKind::Directory => WriterEffect::DeleteDirectory(path),
        });
    }
    Ok((effects, creation_paths))
}

fn creation_effects_conflict(
    left_creations: &[PathBuf],
    right_creations: &[PathBuf],
    left_effects: &[WriterEffect],
    right_effects: &[WriterEffect],
) -> bool {
    left_creations.iter().any(|created| {
        right_creations
            .iter()
            .any(|other| paths_overlap(created, other))
            || creation_conflicts_with_effects(created, right_effects, right_creations)
    }) || right_creations
        .iter()
        .any(|created| creation_conflicts_with_effects(created, left_effects, left_creations))
}

fn creation_conflicts_with_effects(
    created: &Path,
    effects: &[WriterEffect],
    creation_paths: &[PathBuf],
) -> bool {
    effects.iter().any(|effect| match effect {
        WriterEffect::EnsureDirectory(path) if !creation_paths.contains(path) => {
            // An existing directory is an idempotent review effect. It allows
            // another writer to create a descendant while preserving conflicts
            // for equal paths or attempts to create above that directory.
            paths_overlap(created, path) && (path == created || !created.starts_with(path))
        }
        _ => paths_overlap(created, writer_effect_path(effect)),
    })
}

fn writer_effect_path(effect: &WriterEffect) -> &Path {
    match effect {
        WriterEffect::EnsureDirectory(path)
        | WriterEffect::WriteFile(path)
        | WriterEffect::DeleteFile(path)
        | WriterEffect::DeleteDirectory(path) => path,
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn is_reviewable_shared_device_path(destination: &str) -> bool {
    let path = Path::new(destination);
    path.is_absolute()
        && destination != "/"
        && !destination.contains("//")
        && !destination.ends_with('/')
        && !destination.contains(['|', '\n', '\r'])
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

/// Validates static copy operands while leaving step output references for the
/// executor to resolve in dependency order.
pub(crate) fn validate_supported_copy_plan(plan: &ExecutionPlan) -> Result<(), String> {
    let state = ExecutionState::from_plan(plan);
    for step in plan
        .steps
        .iter()
        .filter(|step| step.type_name == "copy_files")
    {
        if has_runtime_output_reference(step) {
            if has_runtime_source_output_reference(step) && step.app_id.is_some() {
                return Err(format!(
                    "copy_files step {} is unsupported: App Target sources from runtime step outputs cannot be reviewed before execution",
                    step.id
                ));
            }
            if has_setup_location_destination_with_runtime_source(&state, step)? {
                return Err(format!(
                    "copy_files step {} unsupported: Setup Location sources from runtime step outputs cannot be reviewed before execution",
                    step.id
                ));
            }
            continue;
        }
        let params = resolve_step_params(&state, &step.params).map_err(|error| {
            format!(
                "copy_files step {} has invalid static operands ({})",
                step.id, error.message
            )
        })?;
        let source = runtime_value_param(&params, "source").map_err(|error| {
            format!(
                "copy_files step {} has invalid source ({})",
                step.id, error.message
            )
        })?;
        if source.type_name == "path_list"
            && params
                .get("copy_policy")
                .and_then(Value::as_str)
                .unwrap_or("merge")
                == "sync"
        {
            return Err(format!(
                "copy_files step {} is unsupported: sync path_list has no single reviewed source boundary",
                step.id
            ));
        }
        if is_selected_directory_step(step, &params)? {
            let copy_policy = params
                .get("copy_policy")
                .and_then(Value::as_str)
                .unwrap_or("merge");
            CopyCommitOperation::from_legacy_directory_policy(copy_policy)
                .map_err(|error| error.to_string())?;
            continue;
        }
        if source.type_name == "path" {
            return Err(format!(
                "copy_files step {} is unsupported: source type 'path' has no copy policy",
                step.id
            ));
        }
        if source.type_name == "path_list"
            && !source
                .value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        {
            return Err(format!(
                "copy_files step {} is unsupported: path_list source must contain string paths",
                step.id
            ));
        }
        if source.type_name == "directory_path" && source.location.as_deref() != Some("host") {
            return Err(format!(
                "copy_files step {} is unsupported: directory sources must be host paths",
                step.id
            ));
        }
        if source.type_name == "directory_path"
            && params.get("dest_kind").and_then(Value::as_str) == Some("file")
        {
            return Err(format!(
                "copy_files step {} is unsupported: a directory source cannot target one file",
                step.id
            ));
        }
        let copy_policy = params
            .get("copy_policy")
            .and_then(Value::as_str)
            .unwrap_or("merge");
        if source.type_name == "directory_path" && matches!(copy_policy, "replace" | "sync") {
            return Err(format!(
                "copy_files step {} is unsupported: destructive directory policy {copy_policy:?} requires a statically resolved selected App Target directory",
                step.id,
            ));
        }
        if source.type_name == "path_list" && copy_policy == "replace" {
            return Err(format!(
                "copy_files step {} is unsupported: replacing a path list would delete the destination tree",
                step.id
            ));
        }
    }
    Ok(())
}

/// Rejects execution plans that have not bound every copy operation to the
/// source and destination manifests reviewed before execution.
pub(crate) fn validate_copy_commit_snapshot_coverage(plan: &ExecutionPlan) -> Result<(), String> {
    let state = ExecutionState::from_plan(plan);
    for step in plan
        .steps
        .iter()
        .filter(|step| step.type_name == "copy_files")
    {
        if has_runtime_output_reference(step) {
            if has_setup_location_destination_with_runtime_source(&state, step)? {
                return Err(format!(
                    "copy_files step {} unsupported: Setup Location sources from runtime step outputs cannot be reviewed before execution",
                    step.id
                ));
            }
            continue;
        }
        let params = resolve_step_params(&state, &step.params).map_err(|error| error.message)?;
        if !is_selected_directory_step(step, &params)? {
            continue;
        }
        let matches = plan
            .copy_commit_snapshots
            .iter()
            .filter(|snapshot| snapshot.step_id == step.id)
            .count();
        match matches {
            0 => {
                return Err(format!(
                    "copy_files step {} requires a pre-review copy snapshot",
                    step.id
                ));
            }
            1 => {}
            _ => {
                return Err(format!(
                    "copy_files step {} has duplicate pre-review copy snapshots",
                    step.id
                ));
            }
        }
    }
    if plan.copy_commit_snapshots.iter().any(|snapshot| {
        !plan
            .steps
            .iter()
            .any(|step| step.type_name == "copy_files" && step.id == snapshot.step_id)
    }) {
        return Err("execution plan contains an orphaned copy snapshot".to_string());
    }
    Ok(())
}

pub(crate) fn reviewed_plan_requires_root(plan: &ExecutionPlan) -> bool {
    plan.steps
        .iter()
        .any(|step| plan_step_root_requirements(plan, step).any())
}

pub(crate) fn reviewed_root_authorized(plan: &ExecutionPlan) -> bool {
    if !reviewed_plan_requires_root(plan) {
        return false;
    }
    let requirements = plan
        .steps
        .iter()
        .map(|step| plan_step_root_requirements(plan, step))
        .fold(
            root_requirements::RootRequirements::default(),
            |mut all, current| {
                all.merge(current);
                all
            },
        );
    requirements.any()
        && (!requirements.root_shell || plan.runtime_capabilities.root_shell)
        && (!requirements.app_data_write || plan.runtime_capabilities.app_data_write)
}

fn plan_step_root_requirements(
    plan: &ExecutionPlan,
    step: &ExecutionStep,
) -> root_requirements::RootRequirements {
    let operation = root_requirements::root_work_operation(&step.type_name);
    let (mut root_shell_constraint, app_data_write_constraint) =
        root_requirements::root_requirement_constraints(
            step.constraints.capabilities.iter().map(String::as_str),
        );
    if step.type_name == "copy_files" {
        let state = ExecutionState::from_plan(plan);
        let shared_destination = resolve_step_params(&state, &step.params)
            .ok()
            .and_then(|params| copy_commit_destination_kind(step, &params).ok().flatten())
            == Some(CopyCommitDestinationKind::SharedDevicePath);
        root_shell_constraint |= shared_destination;
    }
    let condition_requires_root = step
        .skip_if
        .iter()
        .chain(step.verify.iter())
        .any(condition_requires_root);
    let source_requires_root = step
        .params
        .get("source")
        .is_some_and(|value| execution_param_runtime_value_requires_root(plan, value));
    let destination_requires_root = step
        .params
        .get("dest")
        .is_some_and(|value| execution_param_destination_requires_root(plan, value));
    let extracts_to_device = step
        .params
        .get("extract_on")
        .is_some_and(|value| execution_param_is_string(plan, value, "device"));
    root_requirements::classify_root_requirement_step(root_requirements::RootRequirementStep {
        operation,
        root_shell_constraint,
        app_data_write_constraint,
        permission_plan_requires_root: step.permission_plan.as_ref().is_some_and(
            |permission_plan| {
                permission_plan.permission_set == PermissionSetKind::Elevated
                    && (permission_plan.actions.is_empty()
                        || permission_plan.has_applicable_actions())
            },
        ),
        condition_requires_root,
        source_requires_root,
        destination_requires_root,
        extracts_to_device,
    })
}

fn input_runtime_value<'a>(plan: &'a ExecutionPlan, ref_value: &str) -> Option<&'a RuntimeValue> {
    let input_id = ref_value.strip_prefix("inputs.")?;
    if input_id.is_empty() {
        return None;
    }
    plan.inputs
        .iter()
        .find(|input| input.id == input_id)
        .map(|input| &input.value)
}

fn execution_param_runtime_value_requires_root(
    plan: &ExecutionPlan,
    value: &ExecutionParamValue,
) -> bool {
    match value {
        ExecutionParamValue::Literal { value } => {
            runtime_value_json_is_app_private_device_path(value)
        }
        ExecutionParamValue::Ref { ref_value } => input_runtime_value(plan, ref_value)
            .is_some_and(runtime_value_device_value_requires_root),
    }
}

fn execution_param_destination_requires_root(
    plan: &ExecutionPlan,
    value: &ExecutionParamValue,
) -> bool {
    match value {
        ExecutionParamValue::Literal { value } => value
            .as_str()
            .is_some_and(root_requirements::is_app_private_path),
        ExecutionParamValue::Ref { ref_value } => input_runtime_value(plan, ref_value)
            .is_some_and(runtime_value_device_path_requires_root),
    }
}

fn execution_param_is_string(
    plan: &ExecutionPlan,
    value: &ExecutionParamValue,
    expected: &str,
) -> bool {
    match value {
        ExecutionParamValue::Literal { value } => value.as_str() == Some(expected),
        ExecutionParamValue::Ref { ref_value } => {
            input_runtime_value(plan, ref_value).and_then(|value| value.value.as_str())
                == Some(expected)
        }
    }
}

fn runtime_value_device_value_requires_root(value: &RuntimeValue) -> bool {
    value.location.as_deref() == Some("device")
        && match &value.value {
            Value::String(path) => root_requirements::is_app_private_path(path),
            Value::Array(paths) => paths
                .iter()
                .filter_map(Value::as_str)
                .any(root_requirements::is_app_private_path),
            _ => false,
        }
}

fn runtime_value_device_path_requires_root(value: &RuntimeValue) -> bool {
    value.location.as_deref() == Some("device")
        && value
            .value
            .as_str()
            .is_some_and(root_requirements::is_app_private_path)
}

fn runtime_value_json_is_app_private_device_path(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.get("location").and_then(Value::as_str) != Some("device") {
        return false;
    }
    match object.get("value") {
        Some(Value::String(path)) => root_requirements::is_app_private_path(path),
        Some(Value::Array(paths)) => paths
            .iter()
            .filter_map(Value::as_str)
            .any(root_requirements::is_app_private_path),
        _ => false,
    }
}

fn condition_requires_root(condition: &ExecutionStepCondition) -> bool {
    root_requirements::condition_requires_root(
        &condition.type_name,
        condition.params.get("path").and_then(Value::as_str),
    )
}

fn copy_host_source<D: ExecutorDevice>(
    device: &mut D,
    sandbox: &SandboxRoots,
    source: &RuntimeValue,
    dest: &str,
    copy_policy: &str,
) -> Result<Vec<String>, StepFailure> {
    match source.type_name.as_str() {
        "directory_path" => {
            let source_path = source_path_from_value(source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for child in sorted_children(&source_path)? {
                let child_name = child
                    .file_name()
                    .expect("source child should have a basename")
                    .to_string_lossy();
                let target = join_device_path(dest, &child_name);
                device.push(&child, &target, copy_policy == "sync")?;
                copied.push(target);
            }
            Ok(copied)
        }
        "path_list" => {
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for source_path in source_path_list(source)? {
                sandbox.ensure_read_allowed(&source_path)?;
                let child_name = source_path
                    .file_name()
                    .ok_or_else(|| {
                        StepFailure::new(format!(
                            "copy_files source path has no basename: {}",
                            source_path.display()
                        ))
                    })?
                    .to_string_lossy();
                let target = join_device_path(dest, &child_name);
                device.push(&source_path, &target, copy_policy == "sync")?;
                copied.push(target);
            }
            Ok(copied)
        }
        "file_path" => {
            let source_path = source_path_from_value(source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            let target = if device.path_is_dir(dest)? {
                let name = source_path.file_name().ok_or_else(|| {
                    StepFailure::new(format!(
                        "copy_files source path has no basename: {}",
                        source_path.display()
                    ))
                })?;
                join_device_path(dest, &name.to_string_lossy())
            } else {
                if let Some(parent) = device_parent(dest) {
                    device.mkdir_p(parent)?;
                }
                dest.to_string()
            };
            if copy_policy == "replace" {
                device.remove_file(&target)?;
            }
            device.push(&source_path, &target, copy_policy == "sync")?;
            Ok(vec![target])
        }
        other => Err(StepFailure::new(format!(
            "copy_files does not support source runtime type {other:?}."
        ))),
    }
}

fn copy_device_source<D: ExecutorDevice>(
    device: &mut D,
    source: &RuntimeValue,
    dest: &str,
    copy_policy: &str,
) -> Result<Vec<String>, StepFailure> {
    match source.type_name.as_str() {
        "directory_path" => {
            let source_path = value_to_string(&source.value);
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            device.copy_on_device(
                &format!("{}/.", source_path.trim_end_matches('/')),
                dest,
                true,
                false,
            )?;
            Ok(vec![dest.to_string()])
        }
        "path_list" => {
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for item in source.value.as_array().cloned().unwrap_or_default() {
                let source_path = item.as_str().ok_or_else(|| {
                    StepFailure::new(
                        "copy_files path_list source values must be strings".to_string(),
                    )
                })?;
                device.copy_on_device(source_path, dest, true, false)?;
                copied.push(join_device_path(dest, device_basename(source_path)));
            }
            Ok(copied)
        }
        "file_path" => {
            let source_path = value_to_string(&source.value);
            let target = if device.path_is_dir(dest)? {
                join_device_path(dest, device_basename(&source_path))
            } else {
                if let Some(parent) = device_parent(dest) {
                    device.mkdir_p(parent)?;
                }
                dest.to_string()
            };
            if copy_policy == "replace" {
                device.remove_file(&target)?;
            }
            device.copy_on_device(&source_path, &target, false, false)?;
            Ok(vec![target])
        }
        other => Err(StepFailure::new(format!(
            "copy_files does not support device source runtime type {other:?}."
        ))),
    }
}

fn copy_host_source_to_app_private<D: ExecutorDevice>(
    device: &mut D,
    sandbox: &SandboxRoots,
    step: &ExecutionStep,
    source: &RuntimeValue,
    dest: &str,
    copy_policy: &str,
) -> Result<Vec<String>, StepFailure> {
    let stage_root = format!("/data/local/tmp/emuchef/{}", sanitize_step_id(&step.id));
    device.remove_tree(&stage_root)?;
    device.mkdir_p(&stage_root)?;
    let result = (|| match source.type_name.as_str() {
        "directory_path" => {
            let source_path = source_path_from_value(source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for child in sorted_children(&source_path)? {
                let name = child
                    .file_name()
                    .expect("source child should have a basename")
                    .to_string_lossy();
                let staged = join_device_path(&stage_root, &name);
                device.push(&child, &staged, false)?;
                device.copy_on_device(&staged, dest, child.is_dir(), true)?;
                copied.push(join_device_path(dest, &name));
            }
            Ok(copied)
        }
        "path_list" => {
            if copy_policy == "replace" {
                device.remove_tree(dest)?;
            }
            device.mkdir_p(dest)?;
            let mut copied = Vec::new();
            for source_path in source_path_list(source)? {
                sandbox.ensure_read_allowed(&source_path)?;
                let name = source_path
                    .file_name()
                    .ok_or_else(|| {
                        StepFailure::new(format!(
                            "copy_files source path has no basename: {}",
                            source_path.display()
                        ))
                    })?
                    .to_string_lossy();
                let staged = join_device_path(&stage_root, &name);
                device.push(&source_path, &staged, false)?;
                device.copy_on_device(&staged, dest, source_path.is_dir(), true)?;
                copied.push(join_device_path(dest, &name));
            }
            Ok(copied)
        }
        "file_path" => {
            let source_path = source_path_from_value(source)?;
            sandbox.ensure_read_allowed(&source_path)?;
            let name = source_path
                .file_name()
                .ok_or_else(|| {
                    StepFailure::new(format!(
                        "copy_files source path has no basename: {}",
                        source_path.display()
                    ))
                })?
                .to_string_lossy();
            let staged = join_device_path(&stage_root, &name);
            device.push(&source_path, &staged, false)?;
            let target = if device.path_is_dir(dest)? {
                join_device_path(dest, &name)
            } else {
                if let Some(parent) = device_parent(dest) {
                    device.mkdir_p(parent)?;
                }
                dest.to_string()
            };
            if copy_policy == "replace" {
                device.remove_file(&target)?;
            }
            device.copy_on_device(&staged, &target, false, true)?;
            Ok(vec![target])
        }
        other => Err(StepFailure::new(format!(
            "copy_files does not support source runtime type {other:?}."
        ))),
    })();
    let cleanup = device.remove_tree(&stage_root).map_err(StepFailure::from);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(copied), Ok(())) => Ok(copied),
    }
}

fn source_path_list(source: &RuntimeValue) -> Result<Vec<PathBuf>, StepFailure> {
    source
        .value
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            item.as_str().map(PathBuf::from).ok_or_else(|| {
                StepFailure::new("copy_files path_list source values must be strings".to_string())
            })
        })
        .collect()
}

fn device_parent(path: &str) -> Option<&str> {
    let path = path.trim_end_matches('/');
    let (parent, _) = path.rsplit_once('/')?;
    if parent.is_empty() {
        Some("/")
    } else {
        Some(parent)
    }
}

fn device_basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

#[cfg(test)]
pub type DryRunExecutorAdapters = ExecutorAdapters<FakeDryRunDevice>;

#[derive(Debug)]
pub struct ExecutorAdapters<D: ExecutorDevice = FakeDryRunDevice> {
    device: D,
    sleep_calls: Vec<f64>,
    sandbox: Option<SandboxRoots>,
    perform_sleep: bool,
}

impl Default for ExecutorAdapters<FakeDryRunDevice> {
    fn default() -> Self {
        Self {
            device: FakeDryRunDevice::default(),
            sleep_calls: Vec::new(),
            sandbox: None,
            perform_sleep: false,
        }
    }
}

impl<D: ExecutorDevice> ExecutorAdapters<D> {
    #[cfg(test)]
    pub fn with_device(device: D) -> Self {
        Self {
            device,
            sleep_calls: Vec::new(),
            sandbox: None,
            perform_sleep: false,
        }
    }

    pub fn with_device_and_sandbox_roots(
        device: D,
        runtime_root: PathBuf,
        cache_root: PathBuf,
        fake_device_root: PathBuf,
        read_only_roots: Vec<PathBuf>,
        perform_sleep: bool,
    ) -> Self {
        Self {
            device,
            sleep_calls: Vec::new(),
            sandbox: Some(SandboxRoots {
                runtime_root,
                cache_root,
                fake_device_root,
                read_only_roots,
            }),
            perform_sleep,
        }
    }

    #[cfg(test)]
    pub fn device(&self) -> &D {
        &self.device
    }

    #[cfg(test)]
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    #[cfg(test)]
    pub fn sleep_calls(&self) -> &[f64] {
        &self.sleep_calls
    }

    fn sleep(&mut self, seconds: f64) {
        self.sleep_calls.push(seconds);
        if self.perform_sleep {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
        }
    }

    fn sandbox(&self) -> Result<&SandboxRoots, StepFailure> {
        self.sandbox.as_ref().ok_or_else(|| {
            StepFailure::new(
                "Filesystem and artifact handlers require explicit sandbox roots".to_string(),
            )
        })
    }
}

impl ExecutorAdapters<FakeDryRunDevice> {
    pub fn with_sandbox_roots(
        runtime_root: PathBuf,
        cache_root: PathBuf,
        fake_device_root: PathBuf,
        read_only_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            device: FakeDryRunDevice::default(),
            sleep_calls: Vec::new(),
            sandbox: Some(SandboxRoots {
                runtime_root,
                cache_root,
                fake_device_root,
                read_only_roots,
            }),
            perform_sleep: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct FakeDryRunDevice {
    installed_packages: HashSet<String>,
    remote_paths: HashSet<String>,
    remote_dirs: HashSet<String>,
    commands: Vec<Vec<String>>,
    permission_failures: HashMap<PermissionOperation, String>,
    #[cfg(test)]
    permission_operation_errors: HashMap<PermissionOperation, DeviceOperationError>,
    install_failures: HashMap<(String, bool), String>,
    launch_failures: HashMap<(String, String), String>,
    force_stop_failures: HashMap<String, String>,
    root_preflight_result: Option<Result<(), String>>,
    root_preflight_calls: usize,
}

impl FakeDryRunDevice {
    #[cfg(test)]
    pub fn installed_packages_mut(&mut self) -> &mut HashSet<String> {
        &mut self.installed_packages
    }

    #[cfg(test)]
    pub fn remote_paths_mut(&mut self) -> &mut HashSet<String> {
        &mut self.remote_paths
    }

    #[cfg(test)]
    pub fn remote_dirs_mut(&mut self) -> &mut HashSet<String> {
        &mut self.remote_dirs
    }

    #[cfg(test)]
    pub fn commands(&self) -> &[Vec<String>] {
        &self.commands
    }

    #[cfg(test)]
    pub fn set_root_preflight_result(&mut self, result: Result<(), String>) {
        self.root_preflight_result = Some(result);
    }

    #[cfg(test)]
    pub fn root_preflight_calls(&self) -> usize {
        self.root_preflight_calls
    }

    #[cfg(test)]
    pub fn fail_permission_operation(&mut self, operation: PermissionOperation, message: &str) {
        self.permission_failures
            .insert(operation, message.to_string());
    }

    #[cfg(test)]
    pub fn fail_permission_operation_with_error(
        &mut self,
        operation: PermissionOperation,
        error: DeviceOperationError,
    ) {
        self.permission_operation_errors.insert(operation, error);
    }

    #[cfg(test)]
    pub fn fail_launch_app(&mut self, package_name: &str, activity: Option<&str>, message: &str) {
        self.launch_failures.insert(
            (package_name.to_string(), activity.unwrap_or("").to_string()),
            message.to_string(),
        );
    }

    fn install_apk(&mut self, apk_path: &Path, replace_existing: bool) -> Result<(), String> {
        let apk_path = apk_path.to_string_lossy().to_string();
        self.commands.push(vec![
            "install_apk".to_string(),
            apk_path.clone(),
            py_bool(replace_existing).to_string(),
        ]);
        if let Some(message) = self.install_failures.get(&(apk_path, replace_existing)) {
            return Err(message.clone());
        }
        Ok(())
    }

    fn package_installed(&mut self, package_name: &str) -> bool {
        self.commands.push(vec![
            "package_installed".to_string(),
            package_name.to_string(),
        ]);
        self.installed_packages.contains(package_name)
    }

    fn path_exists(&mut self, path: &str) -> bool {
        let privileged = py_bool(root_requirements::is_app_private_path(path));
        self.commands.push(vec![
            "path_exists".to_string(),
            path.to_string(),
            privileged.to_string(),
        ]);
        self.remote_paths.contains(path)
    }

    fn path_is_dir(&mut self, path: &str) -> bool {
        let privileged = py_bool(root_requirements::is_app_private_path(path));
        self.commands.push(vec![
            "path_is_dir".to_string(),
            path.to_string(),
            privileged.to_string(),
        ]);
        self.remote_dirs.contains(path)
    }

    fn apply_permission_operation(&mut self, operation: PermissionOperation) -> Result<(), String> {
        let permission_set = match operation.permission_set() {
            PermissionSetKind::Baseline => "baseline",
            PermissionSetKind::Elevated => "elevated",
        };
        let mut recorded = vec![
            "permission_operation".to_string(),
            permission_set.to_string(),
        ];
        recorded.extend(operation.adb_shell_arguments());
        self.commands.push(recorded);
        if let Some(message) = self.permission_failures.get(&operation) {
            return Err(message.clone());
        }
        Ok(())
    }

    fn launch_app(&mut self, package_name: &str, activity: Option<&str>) -> Result<(), String> {
        let activity = activity.unwrap_or("");
        self.commands.push(vec![
            "launch_app".to_string(),
            package_name.to_string(),
            activity.to_string(),
        ]);
        if let Some(message) = self
            .launch_failures
            .get(&(package_name.to_string(), activity.to_string()))
        {
            return Err(message.clone());
        }
        Ok(())
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), String> {
        self.commands
            .push(vec!["force_stop_app".to_string(), package_name.to_string()]);
        if let Some(message) = self.force_stop_failures.get(package_name) {
            return Err(message.clone());
        }
        Ok(())
    }
}

impl ExecutorDevice for FakeDryRunDevice {
    /// Reports an empty on-device tree because the dry-run adapter records
    /// commands without modeling file contents on the device.
    fn copy_commit_tree_manifest(
        &mut self,
        _root: &str,
    ) -> Result<Vec<CopyManifestEntry>, DeviceOperationError> {
        Ok(Vec::new())
    }

    fn revalidate_root(&mut self) -> Result<(), DeviceOperationError> {
        self.root_preflight_calls = self.root_preflight_calls.saturating_add(1);
        self.commands.push(vec!["root_preflight".to_string()]);
        self.root_preflight_result
            .clone()
            .unwrap_or(Ok(()))
            .map_err(DeviceOperationError::from)
    }

    fn uses_fake_device_filesystem(&self) -> bool {
        true
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        FakeDryRunDevice::install_apk(self, apk_path, replace_existing)
            .map_err(DeviceOperationError::from)
    }

    fn record_installed_package(&mut self, package_name: &str) {
        self.installed_packages.insert(package_name.to_string());
    }

    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError> {
        self.commands.push(vec![
            if sync { "push_sync" } else { "push" }.to_string(),
            source.to_string_lossy().to_string(),
            dest.to_string(),
        ]);
        self.remote_paths.insert(dest.to_string());
        if source.is_dir() {
            self.remote_dirs.insert(dest.to_string());
        }
        Ok(())
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.commands
            .push(vec!["mkdir_p".to_string(), path.to_string()]);
        self.remote_paths.insert(path.to_string());
        self.remote_dirs.insert(path.to_string());
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.commands
            .push(vec!["remove_file".to_string(), path.to_string()]);
        self.remote_paths.remove(path);
        self.remote_dirs.remove(path);
        Ok(())
    }

    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.commands
            .push(vec!["remove_tree".to_string(), path.to_string()]);
        let prefix = format!("{}/", path.trim_end_matches('/'));
        self.remote_paths
            .retain(|candidate| candidate != path && !candidate.starts_with(&prefix));
        self.remote_dirs
            .retain(|candidate| candidate != path && !candidate.starts_with(&prefix));
        Ok(())
    }

    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError> {
        self.commands.push(vec![
            "copy_on_device".to_string(),
            source.to_string(),
            dest.to_string(),
            py_bool(recursive).to_string(),
            py_bool(privileged).to_string(),
        ]);
        self.remote_paths.insert(dest.to_string());
        if recursive {
            self.remote_dirs.insert(dest.to_string());
        }
        Ok(())
    }

    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError> {
        Ok(FakeDryRunDevice::package_installed(self, package_name))
    }

    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        Ok(FakeDryRunDevice::path_exists(self, path))
    }

    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        Ok(FakeDryRunDevice::path_is_dir(self, path))
    }

    fn apply_permission_operation(
        &mut self,
        operation: PermissionOperation,
    ) -> Result<(), DeviceOperationError> {
        #[cfg(test)]
        if let Some(error) = self.permission_operation_errors.get(&operation) {
            return Err(error.clone());
        }
        FakeDryRunDevice::apply_permission_operation(self, operation)
            .map_err(DeviceOperationError::from)
    }

    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError> {
        FakeDryRunDevice::launch_app(self, package_name, activity)
            .map_err(DeviceOperationError::from)
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError> {
        FakeDryRunDevice::force_stop_app(self, package_name).map_err(DeviceOperationError::from)
    }
}

pub(crate) fn map_adb_error(error: adb::AdbCommandError) -> DeviceOperationError {
    match error {
        adb::AdbCommandError::TimedOut { cleanup, .. } => DeviceOperationError::timed_out(cleanup),
        adb::AdbCommandError::CommandFailed { .. } => DeviceOperationError {
            kind: DeviceOperationKind::CommandFailed,
            message: adb::ADB_COMMAND_FAILURE_MESSAGE.to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::DeviceOffline => {
            DeviceOperationError::transport(DeviceOperationKind::DeviceOffline)
        }
        adb::AdbCommandError::DeviceUnauthorized => {
            DeviceOperationError::transport(DeviceOperationKind::DeviceUnauthorized)
        }
        adb::AdbCommandError::DeviceDisconnected => {
            DeviceOperationError::transport(DeviceOperationKind::DeviceDisconnected)
        }
        adb::AdbCommandError::DeviceStorageExhausted => DeviceOperationError::storage_exhausted(),
        adb::AdbCommandError::AdbServerUnavailable => {
            DeviceOperationError::transport(DeviceOperationKind::AdbServerUnavailable)
        }
        adb::AdbCommandError::TransportReset => {
            DeviceOperationError::transport(DeviceOperationKind::TransportReset)
        }
        adb::AdbCommandError::TransportFailure => {
            DeviceOperationError::transport(DeviceOperationKind::TransportFailure)
        }
        adb::AdbCommandError::DeviceIdentityChanged { phase } => {
            DeviceOperationError::identity(DeviceOperationKind::DeviceIdentityChanged(phase), phase)
        }
        adb::AdbCommandError::DeviceIdentityUnverified { phase } => DeviceOperationError::identity(
            DeviceOperationKind::DeviceIdentityUnverified(phase),
            phase,
        ),
        adb::AdbCommandError::ProcessFailed { cleanup, .. } => DeviceOperationError {
            kind: DeviceOperationKind::ProcessFailed,
            message: "The device process failed before a trustworthy result was available."
                .to_string(),
            cleanup: Some(cleanup),
        },
        adb::AdbCommandError::RootDenied => DeviceOperationError {
            kind: DeviceOperationKind::RootDenied,
            message: "Root access was denied by the device.".to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::RootUnavailable => DeviceOperationError {
            kind: DeviceOperationKind::RootUnavailable,
            message: "Root access is unavailable on this device.".to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::RootAuthorityRevoked => DeviceOperationError {
            kind: DeviceOperationKind::RootAuthorityRevoked,
            message: adb::ROOT_AUTHORITY_REVOKED_MESSAGE.to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::RootAuthorityUnverified => DeviceOperationError {
            kind: DeviceOperationKind::RootAuthorityUnverified,
            message: "Continued root authority could not be confirmed safely.".to_string(),
            cleanup: None,
        },
        adb::AdbCommandError::Resolution(message)
            if message.starts_with("android_signing_failed:") =>
        {
            DeviceOperationError {
                kind: DeviceOperationKind::InstallSigningRejected,
                message: "The device operation could not be started or completed safely."
                    .to_string(),
                cleanup: None,
            }
        }
        adb::AdbCommandError::Resolution(message)
            if message.starts_with("android_version_failed:") =>
        {
            DeviceOperationError {
                kind: DeviceOperationKind::InstallVersionRejected,
                message: "The device operation could not be started or completed safely."
                    .to_string(),
                cleanup: None,
            }
        }
        adb::AdbCommandError::Resolution(message)
            if message.starts_with("android_install_failed:") =>
        {
            DeviceOperationError {
                kind: DeviceOperationKind::InstallRejected,
                message: "The device operation could not be started or completed safely."
                    .to_string(),
                cleanup: None,
            }
        }
        adb::AdbCommandError::Resolution(_) | adb::AdbCommandError::RootCheckFailed => {
            DeviceOperationError::other(
                "The device operation could not be started or completed safely.",
            )
        }
    }
}

fn real_adb_error<E: adb::AdbCommandExecutor>(
    device: &mut adb::RealAdbDevice<E>,
    fallback: String,
) -> DeviceOperationError {
    let mutation_precedes_root_failure = device.root_authority_failure_after_mutation();
    device
        .take_last_error()
        .map(|error| {
            let root_failure = matches!(
                error,
                adb::AdbCommandError::RootAuthorityRevoked
                    | adb::AdbCommandError::RootAuthorityUnverified
            );
            let mut mapped = map_adb_error(error);
            if root_failure && mutation_precedes_root_failure {
                mapped.message =
                    crate::executor::root_authority::ROOT_AUTHORITY_FAILURE_AFTER_MUTATION_MARKER
                        .to_string();
            }
            mapped
        })
        .unwrap_or_else(|| DeviceOperationError::other(fallback))
}

impl<E: adb::AdbCommandExecutor> ExecutorDevice for adb::RealAdbDevice<E> {
    fn configure_identity_guard(&mut self, target: Option<&TargetDeviceBinding>) {
        adb::RealAdbDevice::configure_identity_guard(self, target);
    }

    fn configure_root_authority(&mut self, reviewed_root_authorized: bool) {
        adb::RealAdbDevice::configure_root_authority(self, reviewed_root_authorized);
    }
    fn configure_root_shell_route(&mut self, route: Option<crate::planner::RootShellRoute>) {
        adb::RealAdbDevice::configure_root_shell_route(self, route);
    }

    fn owns_per_command_root_authority(&self) -> bool {
        true
    }

    fn revalidate_root(&mut self) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::check_root(self).map_err(|message| real_adb_error(self, message))
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::install_apk(self, apk_path, replace_existing)
            .map_err(|message| real_adb_error(self, message))
    }

    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::push(self, source, dest, sync)
            .map_err(|message| real_adb_error(self, message))
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::mkdir_p(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::remove_file(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::remove_tree(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::copy_on_device(self, source, dest, recursive, privileged)
            .map_err(|message| real_adb_error(self, message))
    }

    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError> {
        adb::RealAdbDevice::package_installed(self, package_name)
            .map_err(|message| real_adb_error(self, message))
    }

    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        adb::RealAdbDevice::path_exists(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        adb::RealAdbDevice::path_is_dir(self, path).map_err(|message| real_adb_error(self, message))
    }

    fn copy_commit_tree_manifest(
        &mut self,
        root: &str,
    ) -> Result<Vec<CopyManifestEntry>, DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_tree_manifest(self, root).map_err(map_adb_error)
    }

    fn copy_commit_path_binding(&mut self, path: &str) -> Result<String, DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_path_binding(self, path).map_err(map_adb_error)
    }

    fn copy_commit_path_is_symlink(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_path_is_symlink(self, path).map_err(map_adb_error)
    }

    fn copy_commit_verify_setup_location(
        &mut self,
        binding: &StorageTargetBinding,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_verify_setup_location(self, binding).map_err(map_adb_error)
    }

    fn copy_commit_create_directory(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_create_directory(self, path).map_err(map_adb_error)
    }

    fn copy_commit_remove_empty_directory(
        &mut self,
        path: &str,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_remove_empty_directory(self, path).map_err(map_adb_error)
    }

    fn copy_commit_prepare_mutation_helper(
        &mut self,
        stage_root: &str,
        helper_path: &str,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_prepare_mutation_helper(self, stage_root, helper_path)
            .map_err(map_adb_error)
    }

    fn copy_commit_apply_selected_root_mutation(
        &mut self,
        selected_root: &str,
        destination_binding: &str,
        stage_root: &str,
        helper_path: &str,
        expected_entry: &str,
        mutation: &CopyCommitMutation,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_apply_selected_root_mutation(
            self,
            selected_root,
            destination_binding,
            stage_root,
            helper_path,
            expected_entry,
            mutation,
        )
        .map_err(map_adb_error)
    }

    fn copy_commit_cleanup_mutation_helper(
        &mut self,
        helper_path: &str,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::copy_commit_cleanup_mutation_helper(self, helper_path)
            .map_err(map_adb_error)
    }

    fn apply_permission_operation(
        &mut self,
        operation: PermissionOperation,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::apply_permission_operation(self, operation)
            .map_err(|message| real_adb_error(self, message))
    }

    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::launch_app(self, package_name, activity)
            .map_err(|message| real_adb_error(self, message))
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError> {
        adb::RealAdbDevice::force_stop_app(self, package_name)
            .map_err(|message| real_adb_error(self, message))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SandboxRoots {
    pub(crate) runtime_root: PathBuf,
    pub(crate) cache_root: PathBuf,
    pub(crate) fake_device_root: PathBuf,
    pub(crate) read_only_roots: Vec<PathBuf>,
}

impl SandboxRoots {
    pub(crate) fn ensure_read_allowed(&self, path: &Path) -> Result<(), StepFailure> {
        let normalized = path
            .canonicalize()
            .map(|path| normalize_path(&path))
            .unwrap_or_else(|_| normalize_path(path));
        if self
            .allowed_read_roots()
            .iter()
            .any(|root| normalized.starts_with(root))
        {
            return Ok(());
        }
        Err(StepFailure::new(format!(
            "sandbox read rejected outside allowed roots: {}",
            path.display()
        )))
    }

    pub(crate) fn ensure_runtime_or_cache_write(&self, path: &Path) -> Result<(), StepFailure> {
        let normalized = normalize_path(path);
        let runtime_root = normalize_path(&self.runtime_root);
        let cache_root = normalize_path(&self.cache_root);
        if normalized.starts_with(&runtime_root) {
            reject_symlink_ancestor(&self.runtime_root, path)?;
            reject_existing_symlink(path)?;
            return Ok(());
        }
        if normalized.starts_with(&cache_root) {
            reject_symlink_ancestor(&self.cache_root, path)?;
            reject_existing_symlink(path)?;
            return Ok(());
        }
        Err(StepFailure::new(format!(
            "sandbox write rejected outside runtime/cache roots: {}",
            path.display()
        )))
    }

    fn fake_device_path(&self, device_path: &str) -> Result<PathBuf, StepFailure> {
        if !device_path.starts_with('/') {
            return Err(StepFailure::new(format!(
                "fake device path must be absolute: {device_path}"
            )));
        }
        let relative = fake_device_relative_components(Path::new(device_path))?;
        let path = normalize_path(&self.fake_device_root.join(relative));
        let fake_device_root = normalize_path(&self.fake_device_root);
        if path.starts_with(&fake_device_root) {
            Ok(path)
        } else {
            Err(StepFailure::new(format!(
                "path traversal rejected for fake device path: {device_path}"
            )))
        }
    }

    fn extract_zip_to_directory(
        &self,
        archive_path: &Path,
        dest_dir: &Path,
    ) -> Result<Vec<PathBuf>, StepFailure> {
        let operation = self.extract_zip_to_directory_inner(archive_path, dest_dir);
        if operation.is_err() {
            return combine_operation_and_cleanup(operation, remove_host_staging(dest_dir));
        }
        operation
    }

    fn extract_zip_to_directory_inner(
        &self,
        archive_path: &Path,
        dest_dir: &Path,
    ) -> Result<Vec<PathBuf>, StepFailure> {
        self.ensure_runtime_or_cache_write(dest_dir)?;
        let file =
            fs::File::open(archive_path).map_err(|error| StepFailure::new(error.to_string()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|_| StepFailure::new("File is not a zip file".to_string()))?;
        let mut entries = Vec::new();
        let mut declared_paths = HashMap::new();
        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .map_err(|error| StepFailure::new(error.to_string()))?;
            let entry_name = file.name().to_string();
            if file.is_symlink() {
                return Err(StepFailure::new(format!(
                    "symlink archive entry rejected: {entry_name}"
                )));
            }
            let relative = archive_relative_components(Path::new(&entry_name))?;
            let is_dir = file.is_dir() || entry_name.ends_with('/');
            if declared_paths.insert(relative.clone(), is_dir).is_some() {
                return Err(StepFailure::new(format!(
                    "duplicate archive entry rejected: {entry_name}"
                )));
            }
            let target = normalize_path(&dest_dir.join(relative));
            if !target.starts_with(normalize_path(dest_dir)) {
                return Err(StepFailure::new(format!(
                    "unsafe archive entry rejected: {entry_name}"
                )));
            }
            self.ensure_runtime_or_cache_write(&target)?;
            entries.push((index, entry_name, target, is_dir));
        }
        for path in declared_paths.keys() {
            for ancestor in path.ancestors().skip(1) {
                if ancestor.as_os_str().is_empty() {
                    break;
                }
                if declared_paths.get(ancestor) == Some(&false) {
                    return Err(StepFailure::new(format!(
                        "archive path conflict rejected: {}",
                        path.display()
                    )));
                }
            }
        }
        fs::create_dir_all(dest_dir).map_err(|error| StepFailure::new(error.to_string()))?;
        for (index, entry_name, target, is_dir) in entries {
            let mut file = archive
                .by_index(index)
                .map_err(|error| StepFailure::new(error.to_string()))?;
            reject_symlink_ancestor(dest_dir, &target)?;
            reject_existing_symlink(&target)?;
            if is_dir || entry_name.ends_with('/') {
                fs::create_dir_all(&target).map_err(|error| StepFailure::new(error.to_string()))?;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| StepFailure::new(error.to_string()))?;
                }
                let mut output = fs::File::create(&target)
                    .map_err(|error| StepFailure::new(error.to_string()))?;
                io::copy(&mut file, &mut output)
                    .map_err(|error| StepFailure::new(error.to_string()))?;
            }
        }
        let mut children = fs::read_dir(dest_dir)
            .map_err(|error| StepFailure::new(error.to_string()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| StepFailure::new(error.to_string()))?;
        children.sort();
        if children.is_empty() {
            Ok(vec![dest_dir.to_path_buf()])
        } else {
            Ok(children)
        }
    }

    fn copy_host_source_to_fake_device(
        &self,
        source: &RuntimeValue,
        dest: &str,
        copy_policy: &str,
        dest_kind: Option<&str>,
    ) -> Result<Vec<String>, StepFailure> {
        match source.type_name.as_str() {
            "file_path" => {
                let source_path = source_path_from_value(source)?;
                self.ensure_read_allowed(&source_path)?;
                let (target, dest_is_dir) = self.file_target_path(&source_path, dest, dest_kind)?;
                if copy_policy == "replace" && target.exists() {
                    self.remove_fake_device_path(&target)?;
                }
                self.copy_path(&source_path, &target)?;
                Ok(vec![device_file_target(&source_path, dest, dest_is_dir)])
            }
            "directory_path" => {
                let source_path = source_path_from_value(source)?;
                self.ensure_read_allowed(&source_path)?;
                let target_dir = self.fake_device_path(dest)?;
                // Directory-style `sync` mirrors the source. File and path-list
                // policy handling intentionally remains in its own branch.
                if matches!(copy_policy, "replace" | "sync") && target_dir.exists() {
                    self.remove_fake_device_path(&target_dir)?;
                }
                reject_symlink_ancestor(&self.fake_device_root, &target_dir)?;
                reject_existing_symlink(&target_dir)?;
                fs::create_dir_all(&target_dir)
                    .map_err(|error| StepFailure::new(error.to_string()))?;
                let mut copied = Vec::new();
                for child in sorted_children(&source_path)? {
                    let child_name = child
                        .file_name()
                        .expect("source child should have a basename")
                        .to_string_lossy()
                        .to_string();
                    let target = target_dir.join(&child_name);
                    self.copy_path(&child, &target)?;
                    copied.push(join_device_path(dest, &child_name));
                }
                Ok(copied)
            }
            "path_list" => {
                let target_dir = self.fake_device_path(dest)?;
                if copy_policy == "replace" && target_dir.exists() {
                    self.remove_fake_device_path(&target_dir)?;
                }
                reject_symlink_ancestor(&self.fake_device_root, &target_dir)?;
                reject_existing_symlink(&target_dir)?;
                fs::create_dir_all(&target_dir)
                    .map_err(|error| StepFailure::new(error.to_string()))?;
                let mut copied = Vec::new();
                for item in source.value.as_array().cloned().unwrap_or_default() {
                    let Some(item) = item.as_str() else {
                        return Err(StepFailure::new(
                            "copy_files path_list source values must be strings".to_string(),
                        ));
                    };
                    let source_path = PathBuf::from(item);
                    self.ensure_read_allowed(&source_path)?;
                    let child_name = source_path
                        .file_name()
                        .ok_or_else(|| {
                            StepFailure::new(format!(
                                "copy_files source path has no basename: {item}"
                            ))
                        })?
                        .to_string_lossy()
                        .to_string();
                    let target = target_dir.join(&child_name);
                    self.copy_path(&source_path, &target)?;
                    copied.push(join_device_path(dest, &child_name));
                }
                Ok(copied)
            }
            other => Err(StepFailure::new(format!(
                "copy_files does not support source runtime type {other:?}."
            ))),
        }
    }

    fn file_target_path(
        &self,
        source_path: &Path,
        dest: &str,
        dest_kind: Option<&str>,
    ) -> Result<(PathBuf, bool), StepFailure> {
        let dest_path = self.fake_device_path(dest)?;
        if dest_kind == Some("file") && dest_path.is_dir() {
            return Err(StepFailure::new(
                "copy_files exact file target is an existing directory".to_string(),
            ));
        }
        if dest_kind == Some("directory") {
            reject_symlink_ancestor(&self.fake_device_root, &dest_path)?;
            reject_existing_symlink(&dest_path)?;
            fs::create_dir_all(&dest_path).map_err(|error| StepFailure::new(error.to_string()))?;
        }
        if dest_path.is_dir() {
            Ok((
                dest_path.join(
                    source_path
                        .file_name()
                        .expect("source file should have basename"),
                ),
                true,
            ))
        } else {
            Ok((dest_path, false))
        }
    }

    fn copy_path(&self, source: &Path, target: &Path) -> Result<(), StepFailure> {
        reject_existing_symlink(source)?;
        reject_symlink_ancestor(&self.fake_device_root, target)?;
        reject_existing_symlink(target)?;
        let fake_device_root = normalize_path(&self.fake_device_root);
        let normalized_target = normalize_path(target);
        if !normalized_target.starts_with(&fake_device_root) {
            return Err(StepFailure::new(format!(
                "sandbox write rejected outside fake device root: {}",
                target.display()
            )));
        }
        if source.is_dir() {
            copy_dir_recursive(source, target, &self.fake_device_root)
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| StepFailure::new(error.to_string()))?;
            }
            fs::copy(source, target)
                .map(|_| ())
                .map_err(|error| StepFailure::new(error.to_string()))
        }
    }

    fn remove_fake_device_path(&self, path: &Path) -> Result<(), StepFailure> {
        let normalized = normalize_path(path);
        if !normalized.starts_with(normalize_path(&self.fake_device_root)) {
            return Err(StepFailure::new(format!(
                "delete rejected outside fake device root: {}",
                path.display()
            )));
        }
        reject_symlink_ancestor(&self.fake_device_root, path)?;
        reject_existing_symlink(path)?;
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|error| StepFailure::new(error.to_string()))
        } else if path.exists() {
            fs::remove_file(path).map_err(|error| StepFailure::new(error.to_string()))
        } else {
            Ok(())
        }
    }

    fn allowed_read_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![
            normalize_path(&self.runtime_root),
            normalize_path(&self.cache_root),
            normalize_path(&self.fake_device_root),
        ];
        roots.extend(
            [&self.runtime_root, &self.cache_root, &self.fake_device_root]
                .into_iter()
                .filter_map(|root| root.canonicalize().ok())
                .map(|root| normalize_path(&root)),
        );
        roots.extend(self.read_only_roots.iter().map(|root| normalize_path(root)));
        roots.extend(
            self.read_only_roots
                .iter()
                .filter_map(|root| root.canonicalize().ok())
                .map(|root| normalize_path(&root)),
        );
        roots
    }
}

#[derive(Debug, Default)]
struct ExecutionState {
    steps: HashMap<String, StepRuntimeState>,
    artifacts: HashMap<String, ArtifactRuntimeState>,
    inputs: HashMap<String, RuntimeValue>,
}

impl ExecutionState {
    fn from_plan(plan: &ExecutionPlan) -> Self {
        Self {
            steps: HashMap::new(),
            inputs: plan
                .inputs
                .iter()
                .map(|input| (input.id.clone(), input.value.clone()))
                .collect(),
            artifacts: plan
                .artifacts
                .iter()
                .map(|artifact| {
                    (
                        artifact.id.clone(),
                        ArtifactRuntimeState {
                            status: ArtifactRuntimeStatus::Pending,
                            local_path: None,
                            resolved_url: None,
                            filename: None,
                            cache_hit: false,
                            error: None,
                            resolved_release: None,
                            resolved_artifact: None,
                            verified_path_guard: None,
                        },
                    )
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug)]
struct StepRuntimeState {
    status: StepRuntimeStatus,
    outputs: OrderedMap<RuntimeValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StepRuntimeStatus {
    Skipped,
    Blocked,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ArtifactRuntimeStatus {
    Pending,
    Resolved,
    Failed,
}

impl ArtifactRuntimeStatus {
    fn as_status_value(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Resolved => "resolved",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
struct ArtifactRuntimeState {
    status: ArtifactRuntimeStatus,
    local_path: Option<String>,
    resolved_url: Option<String>,
    filename: Option<String>,
    cache_hit: bool,
    error: Option<String>,
    resolved_release: Option<ExecutionResolvedRelease>,
    resolved_artifact: Option<ExecutionResolvedArtifact>,
    verified_path_guard: Option<Arc<TempPath>>,
}

#[derive(Clone, Debug)]
pub(crate) struct StepFailure {
    pub(crate) message: String,
    outputs: OrderedMap<RuntimeValue>,
    pub(crate) failure_kind: Option<StepFailureKind>,
    pub(crate) cleanup: Option<ProcessCleanup>,
}

impl StepFailure {
    fn new(message: String) -> Self {
        Self {
            message,
            outputs: OrderedMap::new(),
            failure_kind: None,
            cleanup: None,
        }
    }
}

impl From<DeviceOperationError> for StepFailure {
    fn from(error: DeviceOperationError) -> Self {
        let failure_kind = match error.kind {
            DeviceOperationKind::TimedOut => Some(StepFailureKind::OperationTimedOut),
            DeviceOperationKind::CommandFailed => Some(StepFailureKind::OperationCommandFailed),
            DeviceOperationKind::ProcessFailed => Some(StepFailureKind::OperationProcessFailed),
            DeviceOperationKind::DeviceOffline => Some(StepFailureKind::DeviceOffline),
            DeviceOperationKind::DeviceUnauthorized => Some(StepFailureKind::DeviceUnauthorized),
            DeviceOperationKind::DeviceDisconnected => Some(StepFailureKind::DeviceDisconnected),
            DeviceOperationKind::DeviceStorageExhausted => {
                Some(StepFailureKind::DeviceStorageExhausted)
            }
            DeviceOperationKind::AdbServerUnavailable => {
                Some(StepFailureKind::AdbServerUnavailable)
            }
            DeviceOperationKind::TransportReset => Some(StepFailureKind::TransportReset),
            DeviceOperationKind::TransportFailure => Some(StepFailureKind::TransportFailure),
            DeviceOperationKind::DeviceIdentityChanged(phase) => {
                Some(StepFailureKind::DeviceIdentityChanged(phase))
            }
            DeviceOperationKind::DeviceIdentityUnverified(phase) => {
                Some(StepFailureKind::DeviceIdentityUnverified(phase))
            }
            DeviceOperationKind::RootAuthorityRevoked => {
                Some(StepFailureKind::RootAuthorityRevoked)
            }
            DeviceOperationKind::RootAuthorityUnverified => {
                Some(StepFailureKind::RootAuthorityUnverified)
            }
            DeviceOperationKind::RootDenied => Some(StepFailureKind::RootDenied),
            DeviceOperationKind::RootUnavailable => Some(StepFailureKind::RootUnavailable),
            DeviceOperationKind::InstallSigningRejected => {
                Some(StepFailureKind::InstallSigningRejected)
            }
            DeviceOperationKind::InstallVersionRejected => {
                Some(StepFailureKind::InstallVersionRejected)
            }
            DeviceOperationKind::InstallRejected => Some(StepFailureKind::InstallRejected),
            DeviceOperationKind::Other => Some(StepFailureKind::OperationFailed),
        };
        Self {
            message: error.message,
            outputs: OrderedMap::new(),
            failure_kind,
            cleanup: error.cleanup,
        }
    }
}

impl From<CopyCommitError> for StepFailure {
    fn from(error: CopyCommitError) -> Self {
        match error {
            CopyCommitError::DeviceOperation(error) => error.into(),
            other => Self::new(other.to_string()),
        }
    }
}

fn copy_commit_report_failure(
    report: crate::executor::copy_commit::CopyCommitReport,
) -> StepFailure {
    let message = format!(
        "copy commit did not complete: {}",
        report
            .error
            .as_deref()
            .unwrap_or("selected destination state is incomplete or uncertain")
    );
    let mut failure = report
        .internal_device_error
        .clone()
        .map(StepFailure::from)
        .unwrap_or_else(|| StepFailure::new(message.clone()));
    failure.message = message;
    let value = serde_json::to_value(&report).unwrap_or_else(|error| {
        json!({
            "succeeded": false,
            "outcome": "uncertain",
            "error": format!("copy commit report serialization failed: {error}"),
        })
    });
    failure.outputs.insert(
        "copy_commit_report".to_string(),
        RuntimeValue {
            type_name: "copy_commit_report".to_string(),
            value,
            location: Some("device".to_string()),
        },
    );
    failure
}

fn host_staging_cleanup_failure() -> StepFailure {
    let mut outputs = OrderedMap::new();
    outputs.insert(
        "cleanup_report".to_string(),
        RuntimeValue {
            type_name: "object".to_string(),
            value: json!({
                "classification": "host_staging_cleanup_failed",
                "status": "failed",
            }),
            location: None,
        },
    );
    StepFailure {
        message: "host_staging_cleanup_failed: temporary extraction staging could not be removed."
            .to_string(),
        outputs,
        failure_kind: None,
        cleanup: None,
    }
}

fn remove_host_staging(path: &Path) -> Result<(), StepFailure> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(host_staging_cleanup_failure()),
    }
}

fn combine_operation_and_cleanup<T>(
    operation: Result<T, StepFailure>,
    cleanup: Result<(), StepFailure>,
) -> Result<T, StepFailure> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_failure)) => Err(cleanup_failure),
        (Err(operation_failure), Ok(())) => Err(operation_failure),
        (Err(mut operation_failure), Err(cleanup_failure)) => {
            operation_failure.outputs.extend(cleanup_failure.outputs);
            Err(operation_failure)
        }
    }
}

#[cfg(test)]
mod cleanup_combination_tests {
    use super::*;

    #[test]
    fn cleanup_failure_is_separate_and_preserves_the_original_operation_failure() {
        let operation = Err::<(), _>(StepFailure::new(
            "verification_failed: controlled original failure".to_string(),
        ));
        let failure = combine_operation_and_cleanup(operation, Err(host_staging_cleanup_failure()))
            .unwrap_err();

        assert_eq!(
            failure.message,
            "verification_failed: controlled original failure"
        );
        assert_eq!(
            failure.outputs["cleanup_report"].value,
            json!({
                "classification": "host_staging_cleanup_failed",
                "status": "failed",
            })
        );
    }

    #[test]
    fn cleanup_failure_after_success_has_its_own_fixed_classification() {
        let failure = combine_operation_and_cleanup(
            Ok::<(), StepFailure>(()),
            Err(host_staging_cleanup_failure()),
        )
        .unwrap_err();

        assert_eq!(
            failure.outputs["cleanup_report"].value["classification"],
            "host_staging_cleanup_failed"
        );
        assert!(!failure.message.contains('/'));
    }
}

#[derive(Clone, Debug)]
enum RootPreflightState {
    NotRun,
    Granted,
    Failed(String),
}

impl From<String> for StepFailure {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

fn record(
    step: &ExecutionStep,
    status: StepRunStatus,
    message: Option<String>,
    outputs: OrderedMap<RuntimeValue>,
) -> StepRunRecord {
    record_with_failure(step, status, message, outputs, None, None)
}

fn record_with_failure(
    step: &ExecutionStep,
    status: StepRunStatus,
    message: Option<String>,
    outputs: OrderedMap<RuntimeValue>,
    failure_kind: Option<StepFailureKind>,
    cleanup: Option<ProcessCleanup>,
) -> StepRunRecord {
    StepRunRecord {
        step_id: step.id.clone(),
        status,
        message,
        outputs,
        failure_kind,
        cleanup,
    }
}

fn progress_event(
    step: &ExecutionStep,
    total_steps: usize,
    plan: &ExecutionPlan,
    phase: ProgressPhase,
    status: Option<ProgressStatus>,
    message: Option<String>,
) -> ExecutionProgressEvent {
    let step_index = plan
        .steps
        .iter()
        .position(|candidate| candidate.id == step.id)
        .map(|index| index + 1)
        .unwrap_or(0);
    ExecutionProgressEvent {
        step_index,
        total_steps,
        step_id: step.id.clone(),
        recipe_ref: step.recipe_ref.clone(),
        step_name: step.name.clone(),
        note: crate::planner::normalized_plan_step_note(
            Some(&step.note),
            &step.name,
            &step.type_name,
            &step.id,
        ),
        phase,
        status,
        message,
    }
}

fn blocking_dependencies(state: &ExecutionState, dependencies: &[String]) -> Vec<String> {
    dependencies
        .iter()
        .filter(|dependency_id| {
            state.steps.get(*dependency_id).is_some_and(|step_state| {
                matches!(
                    step_state.status,
                    StepRuntimeStatus::Failed | StepRuntimeStatus::Blocked
                )
            })
        })
        .cloned()
        .collect()
}

fn missing_capabilities_message(plan: &ExecutionPlan, step: &ExecutionStep) -> Option<String> {
    let missing = step
        .constraints
        .capabilities
        .iter()
        .filter(|capability| !runtime_capability_enabled(plan, capability))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "missing required capabilities: {}",
            missing.join(", ")
        ))
    }
}

fn runtime_capability_enabled(plan: &ExecutionPlan, capability: &str) -> bool {
    match capability {
        "adb_available" => plan.runtime_capabilities.adb_available,
        "apk_install" => plan.runtime_capabilities.apk_install,
        "shared_storage_write" => plan.runtime_capabilities.shared_storage_write,
        "app_launch" => plan.runtime_capabilities.app_launch,
        "shell_command" => plan.runtime_capabilities.shell_command,
        "package_remove_for_user" => plan.runtime_capabilities.package_remove_for_user,
        "root_shell" => plan.runtime_capabilities.root_shell,
        "app_data_write" => plan.runtime_capabilities.app_data_write,
        _ => false,
    }
}

fn resolve_step_params(
    state: &ExecutionState,
    params: &OrderedMap<ExecutionParamValue>,
) -> Result<OrderedMap<Value>, StepFailure> {
    let mut resolved = OrderedMap::new();
    for (name, value) in params {
        match value {
            ExecutionParamValue::Literal { value } => {
                resolved.insert(name.clone(), value.clone());
            }
            ExecutionParamValue::Ref { ref_value } => {
                resolved.insert(name.clone(), resolve_runtime_ref(state, ref_value)?);
            }
        }
    }
    Ok(resolved)
}

/// Project a runtime reference into the JSON shape expected by executor params.
///
/// Path-aware consumers require the type and location envelope for `file_path`,
/// `directory_path`, and `path_list`. Every other runtime type is consumed as
/// its payload. Step-output and artifact-field references share this contract so
/// an equivalent value does not change shape based on where it came from.
fn project_runtime_value(value: &RuntimeValue) -> Result<Value, StepFailure> {
    if matches!(
        value.type_name.as_str(),
        "file_path" | "directory_path" | "path_list" | "setup_location_ref"
    ) {
        serde_json::to_value(value).map_err(|error| StepFailure::new(error.to_string()))
    } else {
        Ok(value.value.clone())
    }
}

fn resolve_runtime_ref(state: &ExecutionState, ref_value: &str) -> Result<Value, StepFailure> {
    if let Some(input_ref) = ref_value.strip_prefix("inputs.") {
        let Some(input) = state.inputs.get(input_ref) else {
            return Err(StepFailure::new(format!(
                "unknown_input_ref: Unknown input ref: {ref_value:?}."
            )));
        };
        if matches!(
            input.type_name.as_str(),
            "file_path" | "directory_path" | "path_list" | "setup_location_ref"
        ) {
            return serde_json::to_value(input)
                .map_err(|error| StepFailure::new(error.to_string()));
        }
        return Ok(input.value.clone());
    }
    if let Some(artifact_ref) = ref_value.strip_prefix("artifacts.") {
        let Some((artifact_id, field)) = artifact_ref.rsplit_once('.') else {
            return Err(StepFailure::new(format!(
                "invalid_ref_format: Invalid runtime ref: {ref_value:?}."
            )));
        };
        let Some(artifact) = state.artifacts.get(artifact_id) else {
            return Err(StepFailure::new(format!(
                "unknown_artifact_ref: Unknown artifact ref: {ref_value:?}."
            )));
        };
        if artifact.status != ArtifactRuntimeStatus::Resolved {
            return Err(StepFailure::new(format!(
                "artifact_not_resolved: Artifact is not resolved: {ref_value:?}."
            )));
        }
        let runtime_value = match field {
            "status" => RuntimeValue {
                type_name: "string".to_string(),
                value: json!(artifact.status.as_status_value()),
                location: None,
            },
            "local_path" => RuntimeValue {
                type_name: "file_path".to_string(),
                value: json!(artifact.local_path),
                location: Some("host".to_string()),
            },
            "filename" => RuntimeValue {
                type_name: "string".to_string(),
                value: json!(artifact.filename),
                location: None,
            },
            "resolved_url" => RuntimeValue {
                type_name: "string".to_string(),
                value: json!(artifact.resolved_url),
                location: None,
            },
            "cache_hit" => RuntimeValue {
                type_name: "boolean".to_string(),
                value: json!(artifact.cache_hit),
                location: None,
            },
            "error" => RuntimeValue {
                type_name: "string".to_string(),
                value: json!(artifact.error.clone().unwrap_or_default()),
                location: None,
            },
            _ => {
                return Err(StepFailure::new(format!(
                    "unknown_artifact_field: Unknown artifact field in ref: {ref_value:?}."
                )))
            }
        };
        return project_runtime_value(&runtime_value);
    }
    let Some(step_ref) = ref_value.strip_prefix("steps.") else {
        return Err(StepFailure::new(format!(
            "invalid_ref_format: Invalid runtime ref: {ref_value:?}."
        )));
    };
    let Some((step_id, output_name)) = step_ref.split_once(".outputs.") else {
        return Err(StepFailure::new(format!(
            "invalid_ref_format: Invalid runtime ref: {ref_value:?}."
        )));
    };
    let Some(step_state) = state.steps.get(step_id) else {
        return Err(StepFailure::new(format!(
            "unknown_step_ref: Unknown step ref: {ref_value:?}."
        )));
    };
    if step_state.status != StepRuntimeStatus::Succeeded {
        return Err(StepFailure::new(format!(
            "step_output_unavailable: Step output is unavailable because step {step_id:?} did not succeed."
        )));
    }
    let Some(value) = step_state.outputs.get(output_name) else {
        return Err(StepFailure::new(format!(
            "unknown_step_output: Unknown step output in ref: {ref_value:?}."
        )));
    };
    project_runtime_value(value)
}

fn string_list_param(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn runtime_value_param(
    params: &OrderedMap<Value>,
    name: &str,
) -> Result<RuntimeValue, StepFailure> {
    let Some(Value::Object(value)) = params.get(name) else {
        return Err(StepFailure::new(format!(
            "{name} must be a runtime value object"
        )));
    };
    Ok(RuntimeValue {
        type_name: value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        value: value.get("value").cloned().unwrap_or(Value::Null),
        location: value
            .get("location")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn source_path_from_value(source: &RuntimeValue) -> Result<PathBuf, StepFailure> {
    source.value.as_str().map(PathBuf::from).ok_or_else(|| {
        StepFailure::new("copy_files source runtime value must contain a string path".to_string())
    })
}

fn fake_device_relative_components(path: &Path) -> Result<PathBuf, StepFailure> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(StepFailure::new(format!(
                    "path traversal rejected: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(relative)
}

fn archive_relative_components(path: &Path) -> Result<PathBuf, StepFailure> {
    let raw = path.to_string_lossy();
    let portable = raw.replace('\\', "/");
    let rooted_or_drive = portable.starts_with('/')
        || portable
            .as_bytes()
            .get(1)
            .is_some_and(|separator| *separator == b':')
            && portable
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic);
    if rooted_or_drive {
        return Err(StepFailure::new(format!(
            "unsafe archive entry rejected: {}",
            path.display()
        )));
    }
    let mut relative = PathBuf::new();
    for component in Path::new(&portable).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::RootDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(StepFailure::new(format!(
                    "unsafe archive entry rejected: {}",
                    path.display()
                )));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(StepFailure::new(format!(
            "unsafe archive entry rejected: {}",
            path.display()
        )));
    }
    Ok(relative)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn sanitize_step_id(step_id: &str) -> String {
    step_id.replace('/', "_")
}

fn sorted_children(path: &Path) -> Result<Vec<PathBuf>, StepFailure> {
    let mut children = fs::read_dir(path)
        .map_err(|error| StepFailure::new(error.to_string()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| StepFailure::new(error.to_string()))?;
    children.sort();
    Ok(children)
}

fn reject_existing_symlink(path: &Path) -> Result<(), StepFailure> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(StepFailure::new(format!(
            "symlink paths are not supported in sandbox operations: {}",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink_ancestor(root: &Path, path: &Path) -> Result<(), StepFailure> {
    reject_existing_symlink(root)?;
    let root = normalize_path(root);
    let path = normalize_path(path);
    let relative = path.strip_prefix(&root).map_err(|_| {
        StepFailure::new(format!(
            "sandbox path is outside root: {}",
            path.to_string_lossy()
        ))
    })?;
    let mut current = root;
    for component in relative.components() {
        current.push(component.as_os_str());
        reject_existing_symlink(&current)?;
    }
    Ok(())
}

fn copy_dir_recursive(
    source: &Path,
    target: &Path,
    fake_device_root: &Path,
) -> Result<(), StepFailure> {
    reject_existing_symlink(source)?;
    reject_symlink_ancestor(fake_device_root, target)?;
    reject_existing_symlink(target)?;
    fs::create_dir_all(target).map_err(|error| StepFailure::new(error.to_string()))?;
    for child in sorted_children(source)? {
        reject_existing_symlink(&child)?;
        let child_target = target.join(
            child
                .file_name()
                .expect("directory child should have a basename"),
        );
        reject_symlink_ancestor(fake_device_root, &child_target)?;
        reject_existing_symlink(&child_target)?;
        if child.is_dir() {
            copy_dir_recursive(&child, &child_target, fake_device_root)?;
        } else {
            if let Some(parent) = child_target.parent() {
                fs::create_dir_all(parent).map_err(|error| StepFailure::new(error.to_string()))?;
            }
            fs::copy(&child, &child_target)
                .map(|_| ())
                .map_err(|error| StepFailure::new(error.to_string()))?;
        }
    }
    Ok(())
}

fn join_device_path(parent: &str, child: &str) -> String {
    format!("{}/{}", parent.trim_end_matches('/'), child)
}

fn device_file_target(source_path: &Path, dest: &str, dest_is_dir: bool) -> String {
    if dest_is_dir {
        join_device_path(
            dest,
            &source_path
                .file_name()
                .expect("source file should have basename")
                .to_string_lossy(),
        )
    } else {
        dest.to_string()
    }
}

fn required_string_param(condition: &ExecutionStepCondition, name: &str) -> Result<String, String> {
    condition
        .params
        .get(name)
        .map(|value| match value {
            Value::String(value) => value.clone(),
            other => match other {
                Value::Null => "None".to_string(),
                Value::Bool(value) => py_bool(*value).to_string(),
                Value::Number(value) => value.to_string(),
                _ => other.to_string(),
            },
        })
        .ok_or_else(|| format!("Unsupported condition params for {}", condition.type_name))
}

fn permission_operation(
    permission_set: PermissionSetKind,
    action: &PermissionPlanAction,
) -> PermissionOperation {
    match action {
        PermissionPlanAction::RuntimePermission {
            package_name,
            permission,
            ..
        } => PermissionOperation::GrantRuntimePermission {
            package_name: package_name.clone(),
            permission: permission.clone(),
            permission_set,
        },
        PermissionPlanAction::AppOp {
            package_name,
            op,
            mode: crate::authored_models::AppOpMode::Allow,
            ..
        } => PermissionOperation::AllowAppOp {
            package_name: package_name.clone(),
            operation: op.clone(),
            permission_set,
        },
    }
}

fn permission_not_applicable_outputs(
    step: &ExecutionStep,
    permission_plan: &PermissionPlan,
) -> OrderedMap<RuntimeValue> {
    let action_results = permission_plan
        .actions
        .iter()
        .map(|action| {
            permission_plan_action_result(step, permission_plan, action, "not_applicable", None)
        })
        .collect();
    permission_action_outputs(action_results)
}

fn permission_plan_action_result(
    step: &ExecutionStep,
    permission_plan: &PermissionPlan,
    action: &PermissionPlanAction,
    status: &str,
    message: Option<String>,
) -> Value {
    let mut result = serde_json::Map::new();
    result.insert("step_id".to_string(), json!(step.id));
    result.insert(
        "kind".to_string(),
        json!(match action {
            PermissionPlanAction::RuntimePermission { .. } => "runtime_permission",
            PermissionPlanAction::AppOp { .. } => "appop",
        }),
    );
    result.insert("package_name".to_string(), json!(action.package_name()));
    result.insert("source_recipe_id".to_string(), json!(step.recipe_ref));
    result.insert("source_section".to_string(), json!(action.source_section()));
    result.insert(
        "permission_set".to_string(),
        json!(permission_plan.permission_set),
    );
    result.insert("on_failure".to_string(), json!(permission_plan.on_failure));
    result.insert("status".to_string(), json!(status));
    result.insert(
        "applicability".to_string(),
        serde_json::to_value(action.applicability())
            .expect("permission applicability is serializable"),
    );
    match action {
        PermissionPlanAction::RuntimePermission {
            permission,
            android_api_min,
            android_api_max,
            applicability,
            ..
        } => {
            result.insert("permission".to_string(), json!(permission));
            if let Some(minimum) = android_api_min {
                result.insert("android_api_min".to_string(), json!(minimum));
            }
            if let Some(maximum) = android_api_max {
                result.insert("android_api_max".to_string(), json!(maximum));
            }
            if let PermissionActionApplicability::NotApplicable { reason } = applicability {
                result.insert("reason".to_string(), json!(reason.description()));
            }
        }
        PermissionPlanAction::AppOp {
            op,
            mode: crate::authored_models::AppOpMode::Allow,
            android_api_min,
            android_api_max,
            applicability,
            ..
        } => {
            result.insert("op".to_string(), json!(op));
            result.insert("mode".to_string(), json!("allow"));
            if let Some(minimum) = android_api_min {
                result.insert("android_api_min".to_string(), json!(minimum));
            }
            if let Some(maximum) = android_api_max {
                result.insert("android_api_max".to_string(), json!(maximum));
            }
            if let PermissionActionApplicability::NotApplicable { reason } = applicability {
                result.insert("reason".to_string(), json!(reason.description()));
            }
        }
    }
    if let Some(message) = message {
        result.insert("message".to_string(), json!(message));
    }
    Value::Object(result)
}

fn permission_action_outputs(actions: Vec<Value>) -> OrderedMap<RuntimeValue> {
    let mut outputs = OrderedMap::new();
    outputs.insert(
        "permission_results".to_string(),
        RuntimeValue {
            type_name: "object".to_string(),
            value: json!({ "actions": actions }),
            location: None,
        },
    );
    outputs
}

fn step_failure_with_outputs(
    error: DeviceOperationError,
    outputs: OrderedMap<RuntimeValue>,
) -> StepFailure {
    let mut failure: StepFailure = error.into();
    failure.outputs = outputs;
    failure
}

fn py_bool(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

fn value_is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => py_bool(*value).to_string(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn stable_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(value) => py_bool(*value).to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("{value:?}").replace('"', "'"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod runtime_ref_projection_tests {
    use super::*;

    fn runtime_value(type_name: &str, value: Value, location: Option<&str>) -> RuntimeValue {
        RuntimeValue {
            type_name: type_name.to_string(),
            value,
            location: location.map(str::to_string),
        }
    }

    fn resolved_step(outputs: OrderedMap<RuntimeValue>) -> ExecutionState {
        let mut state = ExecutionState::default();
        state.steps.insert(
            "producer".to_string(),
            StepRuntimeState {
                status: StepRuntimeStatus::Succeeded,
                outputs,
            },
        );
        state
    }

    #[test]
    fn step_output_refs_project_payloads_and_preserve_required_path_envelopes() {
        let mut outputs = OrderedMap::new();
        outputs.insert(
            "url".to_string(),
            runtime_value("string", json!("https://example.test/app.apk"), None),
        );
        outputs.insert(
            "size".to_string(),
            runtime_value("integer", json!(17), None),
        );
        outputs.insert(
            "available".to_string(),
            runtime_value("boolean", json!(true), None),
        );
        outputs.insert(
            "metadata".to_string(),
            runtime_value("object", json!({"channel": "stable"}), None),
        );
        outputs.insert(
            "names".to_string(),
            runtime_value("string_list", json!(["one", "two"]), None),
        );
        outputs.insert(
            "path".to_string(),
            runtime_value("path", json!("/tmp/path"), Some("host")),
        );
        outputs.insert(
            "device_path".to_string(),
            runtime_value("device_path", json!("/sdcard/app.apk"), Some("device")),
        );
        outputs.insert(
            "apk".to_string(),
            runtime_value("file_path", json!("/tmp/app.apk"), Some("host")),
        );
        outputs.insert(
            "directory".to_string(),
            runtime_value("directory_path", json!("/tmp/apps"), Some("host")),
        );
        outputs.insert(
            "paths".to_string(),
            runtime_value("path_list", json!(["/tmp/one", "/tmp/two"]), Some("host")),
        );
        let state = resolved_step(outputs);

        for (field, expected) in [
            ("url", json!("https://example.test/app.apk")),
            ("size", json!(17)),
            ("available", json!(true)),
            ("metadata", json!({"channel": "stable"})),
            ("names", json!(["one", "two"])),
            ("path", json!("/tmp/path")),
            ("device_path", json!("/sdcard/app.apk")),
            (
                "apk",
                json!({"type": "file_path", "value": "/tmp/app.apk", "location": "host"}),
            ),
            (
                "directory",
                json!({"type": "directory_path", "value": "/tmp/apps", "location": "host"}),
            ),
            (
                "paths",
                json!({"type": "path_list", "value": ["/tmp/one", "/tmp/two"], "location": "host"}),
            ),
        ] {
            assert_eq!(
                resolve_runtime_ref(&state, &format!("steps.producer.outputs.{field}"))
                    .expect("step output ref should resolve"),
                expected,
                "unexpected projection for {field}"
            );
        }
    }

    #[test]
    fn artifact_field_refs_project_payloads_and_preserve_file_path_envelope() {
        let mut state = ExecutionState::default();
        state.artifacts.insert(
            "app".to_string(),
            ArtifactRuntimeState {
                status: ArtifactRuntimeStatus::Resolved,
                local_path: Some("/tmp/app.apk".to_string()),
                resolved_url: Some("https://example.test/app.apk".to_string()),
                filename: Some("app.apk".to_string()),
                cache_hit: true,
                error: None,
                resolved_release: None,
                resolved_artifact: None,
                verified_path_guard: None,
            },
        );

        assert_eq!(
            resolve_runtime_ref(&state, "artifacts.app.filename")
                .expect("artifact filename ref should resolve"),
            json!("app.apk")
        );
        assert_eq!(
            resolve_runtime_ref(&state, "artifacts.app.cache_hit")
                .expect("artifact cache-hit ref should resolve"),
            json!(true)
        );
        assert_eq!(
            resolve_runtime_ref(&state, "artifacts.app.local_path")
                .expect("artifact local-path ref should resolve"),
            json!({"type": "file_path", "value": "/tmp/app.apk", "location": "host"})
        );
    }

    #[test]
    fn input_refs_keep_their_existing_type_specific_projection() {
        let mut state = ExecutionState::default();
        for (id, value) in [
            (
                "file",
                runtime_value("file_path", json!("/tmp/app.apk"), Some("host")),
            ),
            (
                "directory",
                runtime_value("directory_path", json!("/tmp/apps"), Some("host")),
            ),
            (
                "paths",
                runtime_value("path_list", json!(["/tmp/one"]), Some("host")),
            ),
            (
                "path",
                runtime_value("path", json!("/tmp/path"), Some("host")),
            ),
            (
                "device",
                runtime_value("device_path", json!("/sdcard/app.apk"), Some("device")),
            ),
            ("label", runtime_value("string", json!("stable"), None)),
        ] {
            state.inputs.insert(id.to_string(), value);
        }

        for (reference, expected) in [
            (
                "inputs.file",
                json!({"type": "file_path", "value": "/tmp/app.apk", "location": "host"}),
            ),
            (
                "inputs.directory",
                json!({"type": "directory_path", "value": "/tmp/apps", "location": "host"}),
            ),
            (
                "inputs.paths",
                json!({"type": "path_list", "value": ["/tmp/one"], "location": "host"}),
            ),
            ("inputs.path", json!("/tmp/path")),
            ("inputs.device", json!("/sdcard/app.apk")),
            ("inputs.label", json!("stable")),
        ] {
            assert_eq!(
                resolve_runtime_ref(&state, reference).expect("input ref should resolve"),
                expected,
                "unexpected projection for {reference}"
            );
        }
    }
}

#[cfg(test)]
mod setup_location_writer_tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        validate_active_copy_writer_collisions, validate_copy_commit_snapshot_coverage,
        validate_supported_copy_plan,
    };
    use crate::executor::copy_commit::{
        CopyCommitDestinationKind, CopyCommitOperation, CopyEntryKind, CopyManifestEntry,
        ExecutionCopyCommitSnapshot,
    };
    use crate::storage_targets::{
        SetupDestinationPrecondition, StorageTargetBinding, StorageTargetVolumeKind,
    };
    use crate::{
        model::OrderedMap,
        planner::{
            DeviceContext, ExecutionDevicePlanSnapshot, ExecutionInputValue, ExecutionParamValue,
            ExecutionPlan, ExecutionPlanSource, ExecutionStep, ExecutionStepConstraints,
            RuntimeCapabilities, RuntimeValue,
        },
    };

    fn binding(id: &str, relative_path: &str) -> StorageTargetBinding {
        StorageTargetBinding {
            id: id.to_string(),
            device_plan: "plan.test".to_string(),
            session_epoch: 1,
            name: id.to_string(),
            relative_path: relative_path.to_string(),
            volume_handle: "1".repeat(32),
            volume_kind: StorageTargetVolumeKind::RemovableSd,
            mount_root: "/storage/ABCD-1234".to_string(),
            volume_identity: "2".repeat(32),
            volume_continuity: "3".repeat(32),
            observation_token: "4".repeat(32),
        }
    }

    fn file(path: &str) -> CopyManifestEntry {
        CopyManifestEntry {
            relative_path: path.to_string(),
            kind: CopyEntryKind::File,
            sha256: Some("a".repeat(64)),
            entry_identity: None,
        }
    }

    fn plan_with_runtime_source(
        destination: ExecutionParamValue,
        inputs: Vec<ExecutionInputValue>,
    ) -> ExecutionPlan {
        let mut params = OrderedMap::new();
        params.insert(
            "source".to_string(),
            ExecutionParamValue::Ref {
                ref_value: "steps.extract.outputs.path".to_string(),
            },
        );
        params.insert("dest".to_string(), destination);
        params.insert(
            "dest_kind".to_string(),
            ExecutionParamValue::Literal {
                value: json!("directory"),
            },
        );
        params.insert(
            "copy_policy".to_string(),
            ExecutionParamValue::Literal {
                value: json!("merge"),
            },
        );

        ExecutionPlan {
            id: "plan.test".to_string(),
            source: ExecutionPlanSource {
                device_profile_ref: "example.device_profile".to_string(),
                device_plan_ref: "example.device_plan".to_string(),
                selected_recipe_refs: vec!["example.recipe".to_string()],
                expanded_recipe_refs: vec!["example.recipe".to_string()],
                catalog: None,
            },
            device_plan: None,
            recipes: Vec::new(),
            apps: Vec::new(),
            resolved_inputs: Vec::new(),
            copy_commit_snapshots: Vec::new(),
            target_device: None,
            device_context: DeviceContext {
                manufacturer: "Example".to_string(),
                model: "Example".to_string(),
                android_version: 13,
                android_api_level: Some(33),
                device_tags: Vec::new(),
                root_shell_mode: None,
            },
            runtime_capabilities: RuntimeCapabilities {
                adb_available: true,
                apk_install: true,
                shared_storage_write: true,
                app_launch: true,
                shell_command: true,
                package_remove_for_user: false,
                root_shell: true,
                app_data_write: true,
                root_shell_route: None,
            },
            inputs,
            artifacts: Vec::new(),
            steps: vec![ExecutionStep {
                id: "example.recipe/copy".to_string(),
                recipe_ref: "example.recipe".to_string(),
                app_id: None,
                type_name: "copy_files".to_string(),
                name: "Copy".to_string(),
                note: "Copy".to_string(),
                dependencies: Vec::new(),
                constraints: ExecutionStepConstraints {
                    capabilities: Vec::new(),
                    conflicts_with: Vec::new(),
                },
                permission_plan: None,
                params,
                skip_if: Vec::new(),
                verify: Vec::new(),
            }],
            schema_version: 1,
            kind: "execution_plan",
        }
    }

    fn snapshot(
        step_id: &str,
        location_id: &str,
        relative_location: &str,
        canonical_scope: &str,
        source_manifest: Vec<CopyManifestEntry>,
        destination_before_manifest: Vec<CopyManifestEntry>,
        reviewed_deletion_set: Vec<&str>,
    ) -> ExecutionCopyCommitSnapshot {
        ExecutionCopyCommitSnapshot {
            step_id: step_id.to_string(),
            operation: CopyCommitOperation::AddUpdate,
            destination_kind: CopyCommitDestinationKind::SetupLocation,
            source_path: PathBuf::from("/tmp/source"),
            source_root: PathBuf::from("/tmp/source"),
            source_manifest,
            source_digest_sha256: String::new(),
            destination_identity: canonical_scope.to_string(),
            fake_device_root: PathBuf::from("/tmp/fake-device"),
            selected_root: PathBuf::from("/tmp/fake-device/selected"),
            destination_binding: None,
            setup_location_binding: Some(binding(location_id, relative_location)),
            setup_destination_precondition: Some(SetupDestinationPrecondition {
                canonical_scope: canonical_scope.to_string(),
                existing_anchor_path: "/storage/ABCD-1234".to_string(),
                existing_anchor_binding: "anchor".to_string(),
                missing_components: Vec::new(),
            }),
            destination_before_manifest,
            destination_before_digest_sha256: String::new(),
            reviewed_deletion_set: reviewed_deletion_set
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }

    #[test]
    fn rejects_writers_when_reviewed_file_effects_overlap_across_nested_scopes() {
        let parent = snapshot(
            "copy-parent",
            "parent-location",
            "Games",
            "/storage/ABCD-1234/Games",
            vec![file("Emulator")],
            Vec::new(),
            Vec::new(),
        );
        let nested = snapshot(
            "copy-nested",
            "nested-location",
            "Games/Emulator",
            "/storage/ABCD-1234/Games/Emulator",
            vec![file("settings.cfg")],
            Vec::new(),
            Vec::new(),
        );

        let error = validate_active_copy_writer_collisions(&[parent, nested])
            .expect_err("writing a file over a nested destination scope must conflict");
        assert!(error.contains("copy-parent"));
        assert!(error.contains("copy-nested"));
    }

    #[test]
    fn rejects_distinct_location_ids_that_resolve_to_the_same_canonical_scope() {
        let first = snapshot(
            "copy-first",
            "first-location",
            "Games",
            "/storage/ABCD-1234/Games",
            vec![file("same.rom")],
            Vec::new(),
            Vec::new(),
        );
        let second = snapshot(
            "copy-second",
            "second-location",
            "Games",
            "/storage/ABCD-1234/Games",
            vec![file("same.rom")],
            Vec::new(),
            Vec::new(),
        );

        assert!(validate_active_copy_writer_collisions(&[first, second]).is_err());
    }

    #[test]
    fn rejects_mixed_setup_and_legacy_writers_that_resolve_to_the_same_scope() {
        let mut setup = snapshot(
            "copy-setup",
            "internal-location",
            "Games",
            "/storage/emulated/0/Games",
            vec![file("same.rom")],
            Vec::new(),
            Vec::new(),
        );
        let binding = setup
            .setup_location_binding
            .as_mut()
            .expect("Setup Location binding");
        binding.mount_root = "/storage/emulated/0".to_string();
        binding.volume_kind = StorageTargetVolumeKind::InternalShared;

        let mut legacy = snapshot(
            "copy-legacy",
            "legacy-location",
            "Games",
            "/storage/emulated/0/Games",
            vec![file("same.rom")],
            Vec::new(),
            Vec::new(),
        );
        legacy.destination_kind = CopyCommitDestinationKind::SharedDevicePath;
        legacy.destination_identity = "/storage/self/primary/Games".to_string();
        legacy.destination_binding =
            Some("/storage/emulated/0/Games|parent:2:10|selected:2:11".to_string());
        legacy.setup_location_binding = None;
        legacy.setup_destination_precondition = None;

        let error = validate_active_copy_writer_collisions(&[setup, legacy])
            .expect_err("a legacy alias and Setup Location cannot write the same reviewed file");

        assert!(error.contains("copy-setup"));
        assert!(error.contains("copy-legacy"));
    }

    #[test]
    fn rejects_setup_and_legacy_writers_across_removable_mount_aliases() {
        let setup = snapshot(
            "copy-setup",
            "removable-location",
            "Games",
            "/storage/ABCD-1234/Games",
            vec![file("same.rom")],
            Vec::new(),
            Vec::new(),
        );
        let mut legacy = snapshot(
            "copy-legacy",
            "legacy-location",
            "Games",
            "/mnt/media_rw/ABCD-1234/Games",
            vec![file("same.rom")],
            Vec::new(),
            Vec::new(),
        );
        legacy.destination_kind = CopyCommitDestinationKind::SharedDevicePath;
        legacy.destination_binding =
            Some("/mnt/media_rw/ABCD-1234/Games|parent:2:10|selected:2:11".to_string());
        legacy.setup_location_binding = None;
        legacy.setup_destination_precondition = None;

        let error = validate_active_copy_writer_collisions(&[setup, legacy])
            .expect_err("removable mount aliases must share one active writer scope");

        assert!(error.contains("copy-setup"));
        assert!(error.contains("copy-legacy"));
    }

    #[test]
    fn rejects_review_of_setup_location_copy_with_runtime_step_output_source() {
        let setup_location_plan = plan_with_runtime_source(
            ExecutionParamValue::Ref {
                ref_value: "inputs.setup-location".to_string(),
            },
            vec![ExecutionInputValue {
                id: "setup-location".to_string(),
                value: RuntimeValue {
                    type_name: "setup_location_ref".to_string(),
                    value: json!("location-one"),
                    location: None,
                },
            }],
        );
        let error = validate_supported_copy_plan(&setup_location_plan)
            .expect_err("Setup Location source outputs must be reviewable before execution");
        assert!(error.contains("Setup Location"), "{error}");
        let error = validate_copy_commit_snapshot_coverage(&setup_location_plan)
            .expect_err("snapshot coverage must reject an unreviewable Setup Location source");
        assert!(error.contains("Setup Location"), "{error}");

        let legacy_plan = plan_with_runtime_source(
            ExecutionParamValue::Literal {
                value: json!({
                    "type": "directory_path",
                    "value": "/sdcard/roms",
                    "location": "device"
                }),
            },
            Vec::new(),
        );
        assert!(validate_supported_copy_plan(&legacy_plan).is_ok());
    }

    #[test]
    fn rejects_disjoint_writers_that_share_a_reviewed_missing_parent() {
        let mut first = snapshot(
            "copy-first",
            "first-location",
            "Games/A",
            "/storage/ABCD-1234/Games/A",
            vec![file("first.rom")],
            Vec::new(),
            Vec::new(),
        );
        first
            .setup_destination_precondition
            .as_mut()
            .expect("first precondition")
            .missing_components = vec!["Games".to_string(), "A".to_string()];

        let mut second = snapshot(
            "copy-second",
            "second-location",
            "Games/B",
            "/storage/ABCD-1234/Games/B",
            vec![file("second.rom")],
            Vec::new(),
            Vec::new(),
        );
        second
            .setup_destination_precondition
            .as_mut()
            .expect("second precondition")
            .missing_components = vec!["Games".to_string(), "B".to_string()];

        let error = validate_active_copy_writer_collisions(&[first, second])
            .expect_err("the reviewed writers must not share a directory-creation precondition");

        assert!(error.contains("copy-first"));
        assert!(error.contains("copy-second"));
    }

    #[test]
    fn allows_overlapping_location_scopes_when_reviewed_effects_are_disjoint() {
        let parent = snapshot(
            "copy-parent",
            "parent-location",
            "Games",
            "/storage/ABCD-1234/Games",
            vec![file("arcade.rom")],
            Vec::new(),
            Vec::new(),
        );
        let nested = snapshot(
            "copy-nested",
            "nested-location",
            "Games/Emulator",
            "/storage/ABCD-1234/Games/Emulator",
            vec![file("settings.cfg")],
            Vec::new(),
            Vec::new(),
        );

        validate_active_copy_writer_collisions(&[parent, nested])
            .expect("overlapping scopes alone do not create a writer collision");
    }

    #[test]
    fn allows_nested_creation_below_an_already_existing_directory_effect() {
        let existing_directory = CopyManifestEntry {
            relative_path: "Emulator".to_string(),
            kind: CopyEntryKind::Directory,
            sha256: None,
            entry_identity: None,
        };
        let parent = snapshot(
            "copy-parent",
            "parent-location",
            "Games",
            "/storage/ABCD-1234/Games",
            vec![existing_directory.clone(), file("arcade.rom")],
            vec![existing_directory],
            Vec::new(),
        );
        let mut nested = snapshot(
            "copy-nested",
            "nested-location",
            "Games/Emulator/Settings",
            "/storage/ABCD-1234/Games/Emulator/Settings",
            vec![file("settings.cfg")],
            Vec::new(),
            Vec::new(),
        );
        nested
            .setup_destination_precondition
            .as_mut()
            .expect("nested precondition")
            .missing_components = vec!["Settings".to_string()];

        validate_active_copy_writer_collisions(&[parent, nested])
            .expect("an existing parent directory does not conflict with nested creation");
    }

    #[test]
    fn rejects_reviewed_directory_deletion_that_contains_another_writer() {
        let parent = snapshot(
            "replace-parent",
            "parent-location",
            "Games",
            "/storage/ABCD-1234/Games",
            Vec::new(),
            vec![CopyManifestEntry {
                relative_path: "Emulator".to_string(),
                kind: CopyEntryKind::Directory,
                sha256: None,
                entry_identity: None,
            }],
            vec!["Emulator"],
        );
        let nested = snapshot(
            "copy-nested",
            "nested-location",
            "Games/Emulator",
            "/storage/ABCD-1234/Games/Emulator",
            vec![file("settings.cfg")],
            Vec::new(),
            Vec::new(),
        );

        assert!(validate_active_copy_writer_collisions(&[parent, nested]).is_err());
    }

    #[test]
    fn named_locations_without_active_copy_snapshots_do_not_collide() {
        validate_active_copy_writer_collisions(&[])
            .expect("unreferenced named locations are not destination writers");
    }

    #[test]
    fn rejects_legacy_host_file_writer_over_setup_effect_before_device_push() {
        let temporary = tempfile::tempdir().unwrap();
        let source_root = temporary.path().join("source");
        std::fs::create_dir(&source_root).unwrap();
        std::fs::write(source_root.join("settings.cfg"), b"reviewed setup bytes").unwrap();
        let legacy_source = temporary.path().join("legacy-settings.cfg");
        std::fs::write(&legacy_source, b"legacy bytes").unwrap();
        let runtime_root = temporary.path().join("runtime");
        let cache_root = temporary.path().join("cache");
        let fake_device_root = temporary.path().join("fake-device");
        std::fs::create_dir_all(&runtime_root).unwrap();
        std::fs::create_dir_all(&cache_root).unwrap();
        std::fs::create_dir_all(&fake_device_root).unwrap();

        let inputs = vec![
            ExecutionInputValue {
                id: "setup-location".to_string(),
                value: RuntimeValue {
                    type_name: "setup_location_ref".to_string(),
                    value: json!("location-one"),
                    location: None,
                },
            },
            ExecutionInputValue {
                id: "setup-source".to_string(),
                value: RuntimeValue {
                    type_name: "directory_path".to_string(),
                    value: json!(source_root.to_string_lossy()),
                    location: Some("host".to_string()),
                },
            },
            ExecutionInputValue {
                id: "legacy-source".to_string(),
                value: RuntimeValue {
                    type_name: "file_path".to_string(),
                    value: json!(legacy_source.to_string_lossy()),
                    location: Some("host".to_string()),
                },
            },
        ];
        let mut plan = plan_with_runtime_source(
            ExecutionParamValue::Ref {
                ref_value: "inputs.setup-location".to_string(),
            },
            inputs,
        );
        plan.device_plan = Some(ExecutionDevicePlanSnapshot {
            id: "plan.test".to_string(),
            name: None,
            description: None,
        });
        let setup_step = &mut plan.steps[0];
        setup_step.id = "copy-setup".to_string();
        setup_step.params.insert(
            "source".to_string(),
            ExecutionParamValue::Ref {
                ref_value: "inputs.setup-source".to_string(),
            },
        );
        let mut legacy_step = setup_step.clone();
        legacy_step.id = "copy-legacy".to_string();
        legacy_step.params.insert(
            "source".to_string(),
            ExecutionParamValue::Ref {
                ref_value: "inputs.legacy-source".to_string(),
            },
        );
        legacy_step.params.insert(
            "dest".to_string(),
            ExecutionParamValue::Literal {
                value: json!("/storage/ABCD-1234/Other/settings.cfg"),
            },
        );
        legacy_step.params.insert(
            "dest_kind".to_string(),
            ExecutionParamValue::Literal {
                value: json!("file"),
            },
        );
        plan.steps.push(legacy_step);

        let adapters = super::ExecutorAdapters::with_sandbox_roots(
            runtime_root,
            cache_root,
            fake_device_root,
            vec![temporary.path().to_path_buf()],
        );
        let mut runner = super::ExecutorRunner::new(adapters);

        runner
            .prepare_copy_commit_snapshots_with_storage_targets(
                &mut plan,
                &[binding("location-one", "Games")],
            )
            .expect("disjoint legacy file effects may coexist with Setup Location writes");
        plan.steps[1].params.insert(
            "dest".to_string(),
            ExecutionParamValue::Literal {
                value: json!("/storage/ABCD-1234/Games/settings.cfg"),
            },
        );

        let error = runner
            .prepare_copy_commit_snapshots_with_storage_targets(
                &mut plan,
                &[binding("location-one", "Games")],
            )
            .expect_err("an unsnapshotted legacy file writer must join collision review");

        assert!(error.contains("copy-setup"), "{error}");
        assert!(error.contains("copy-legacy"), "{error}");
        assert!(runner
            .adapters()
            .device()
            .commands()
            .iter()
            .all(|command| command.first().is_none_or(|name| name != "push")));
    }

    fn prepare_legacy_copy_collision(
        source_type: &str,
        source_value: serde_json::Value,
        source_location: &str,
        destination: &str,
        destination_kind: &str,
        setup_source_filename: &str,
        copy_policy: Option<&str>,
        source_step_output_ref: Option<&str>,
        extract_destination: Option<&str>,
        extract_on: Option<&str>,
    ) -> Result<(), String> {
        let temporary = tempfile::tempdir().unwrap();
        let source_root = temporary.path().join("source");
        std::fs::create_dir(&source_root).unwrap();
        std::fs::write(
            source_root.join(setup_source_filename),
            b"reviewed setup bytes",
        )
        .unwrap();
        let runtime_root = temporary.path().join("runtime");
        let cache_root = temporary.path().join("cache");
        let fake_device_root = temporary.path().join("fake-device");
        std::fs::create_dir_all(&runtime_root).unwrap();
        std::fs::create_dir_all(&cache_root).unwrap();
        std::fs::create_dir_all(&fake_device_root).unwrap();
        std::fs::create_dir_all(fake_device_root.join("data/local/tmp")).unwrap();

        let inputs = vec![
            ExecutionInputValue {
                id: "setup-location".to_string(),
                value: RuntimeValue {
                    type_name: "setup_location_ref".to_string(),
                    value: json!("location-one"),
                    location: None,
                },
            },
            ExecutionInputValue {
                id: "setup-source".to_string(),
                value: RuntimeValue {
                    type_name: "directory_path".to_string(),
                    value: json!(source_root.to_string_lossy()),
                    location: Some("host".to_string()),
                },
            },
            ExecutionInputValue {
                id: "legacy-source".to_string(),
                value: RuntimeValue {
                    type_name: source_type.to_string(),
                    value: source_value,
                    location: Some(source_location.to_string()),
                },
            },
        ];
        let mut plan = plan_with_runtime_source(
            ExecutionParamValue::Ref {
                ref_value: "inputs.setup-location".to_string(),
            },
            inputs,
        );
        plan.device_plan = Some(ExecutionDevicePlanSnapshot {
            id: "plan.test".to_string(),
            name: None,
            description: None,
        });
        let setup_step = &mut plan.steps[0];
        setup_step.id = "copy-setup".to_string();
        setup_step.params.insert(
            "source".to_string(),
            ExecutionParamValue::Ref {
                ref_value: "inputs.setup-source".to_string(),
            },
        );
        let mut legacy_step = setup_step.clone();
        legacy_step.id = "copy-legacy".to_string();
        legacy_step.params.insert(
            "source".to_string(),
            source_step_output_ref
                .map(|ref_value| ExecutionParamValue::Ref {
                    ref_value: ref_value.to_string(),
                })
                .unwrap_or_else(|| ExecutionParamValue::Ref {
                    ref_value: "inputs.legacy-source".to_string(),
                }),
        );
        legacy_step.params.insert(
            "dest".to_string(),
            ExecutionParamValue::Literal {
                value: json!(destination),
            },
        );
        legacy_step.params.insert(
            "dest_kind".to_string(),
            ExecutionParamValue::Literal {
                value: json!(destination_kind),
            },
        );
        if let Some(copy_policy) = copy_policy {
            legacy_step.params.insert(
                "copy_policy".to_string(),
                ExecutionParamValue::Literal {
                    value: json!(copy_policy),
                },
            );
        }
        if source_step_output_ref.is_some() {
            legacy_step
                .dependencies
                .push("example.recipe/unpack".to_string());
        }
        plan.steps.push(legacy_step);
        if source_step_output_ref.is_some() || extract_destination.is_some() {
            let archive_path = temporary.path().join("archive.zip");
            std::fs::write(&archive_path, b"archive fixture").unwrap();
            plan.inputs.push(ExecutionInputValue {
                id: "example.recipe/archive".to_string(),
                value: RuntimeValue {
                    type_name: "file_path".to_string(),
                    value: json!(archive_path.to_string_lossy()),
                    location: Some("host".to_string()),
                },
            });
            let mut unpack_params = OrderedMap::new();
            unpack_params.insert(
                "archive".to_string(),
                if extract_destination.is_some() {
                    plan.artifacts.push(crate::planner::ExecutionArtifact {
                        id: "pending-archive".to_string(),
                        type_name: "file_path".to_string(),
                        source: crate::planner::ExecutionArtifactSource::LocalInput {
                            input_id: "example.recipe/archive".to_string(),
                        },
                        cache: "none".to_string(),
                        app_provenance: None,
                    });
                    ExecutionParamValue::Ref {
                        ref_value: "artifacts.pending-archive.local_path".to_string(),
                    }
                } else {
                    ExecutionParamValue::Ref {
                        ref_value: "inputs.example.recipe/archive".to_string(),
                    }
                },
            );
            unpack_params.insert(
                "dest".to_string(),
                ExecutionParamValue::Literal {
                    value: json!(extract_destination.unwrap_or("/data/local/tmp/extracted")),
                },
            );
            unpack_params.insert(
                "extract_on".to_string(),
                ExecutionParamValue::Literal {
                    value: json!(extract_on.unwrap_or("device")),
                },
            );
            plan.steps.insert(
                0,
                ExecutionStep {
                    id: "example.recipe/unpack".to_string(),
                    recipe_ref: "example.recipe".to_string(),
                    app_id: None,
                    type_name: "extract_archive".to_string(),
                    name: "Unpack on device".to_string(),
                    note: String::new(),
                    dependencies: Vec::new(),
                    constraints: ExecutionStepConstraints {
                        capabilities: Vec::new(),
                        conflicts_with: Vec::new(),
                    },
                    permission_plan: None,
                    params: unpack_params,
                    skip_if: Vec::new(),
                    verify: Vec::new(),
                },
            );
        }

        let adapters = super::ExecutorAdapters::with_sandbox_roots(
            runtime_root,
            cache_root,
            fake_device_root,
            vec![temporary.path().to_path_buf()],
        );
        let mut runner = super::ExecutorRunner::new(adapters);
        let result = runner.prepare_copy_commit_snapshots_with_storage_targets(
            &mut plan,
            &[binding("location-one", "Games")],
        );
        let mutating_commands = ["cp", "mkdir", "mv", "push", "rm"];
        assert!(runner.adapters().device().commands().iter().all(|command| {
            command
                .iter()
                .all(|part| !mutating_commands.contains(&part.as_str()))
        }));
        result
    }

    #[test]
    fn rejects_unreviewed_device_path_list_collision_with_setup_location_writer() {
        let error = prepare_legacy_copy_collision(
            "path_list",
            json!(["/data/local/tmp/extracted/settings.cfg"]),
            "device",
            "/storage/ABCD-1234/Games",
            "directory",
            "settings.cfg",
            None,
            None,
            None,
            None,
        )
        .expect_err("device path_list writes must participate in destination collision review");

        assert!(error.contains("copy-setup"), "{error}");
        assert!(error.contains("copy-legacy"), "{error}");
    }

    #[test]
    fn rejects_unreviewed_host_path_list_collision_with_trailing_slash() {
        let error = prepare_legacy_copy_collision(
            "path_list",
            json!(["/data/local/tmp/extracted/settings.cfg"]),
            "host",
            "/storage/ABCD-1234/Games/",
            "directory",
            "settings.cfg",
            None,
            None,
            None,
            None,
        )
        .expect_err("trailing separators must resolve to the reviewed destination scope");

        assert!(error.contains("copy-setup"), "{error}");
        assert!(error.contains("copy-legacy"), "{error}");
    }

    #[test]
    fn rejects_unreviewed_host_path_list_collision_with_repeated_trailing_slashes() {
        let error = prepare_legacy_copy_collision(
            "path_list",
            json!(["/data/local/tmp/extracted/settings.cfg"]),
            "host",
            "/storage/ABCD-1234/Games//",
            "directory",
            "settings.cfg",
            None,
            None,
            None,
            None,
        )
        .expect_err("host path_list writers must collide after separator normalization");

        assert!(error.contains("copy-setup"), "{error}");
        assert!(error.contains("copy-legacy"), "{error}");
    }

    #[test]
    fn rejects_unreviewed_device_file_path_collision_with_setup_location_writer() {
        let error = prepare_legacy_copy_collision(
            "file_path",
            json!("/sdcard/game.rom"),
            "device",
            "/storage/ABCD-1234/Games/game.rom",
            "file",
            "game.rom",
            Some("replace"),
            None,
            None,
            None,
        )
        .expect_err("device file_path writes must participate in destination collision review");

        assert!(error.contains("copy-setup"), "{error}");
        assert!(error.contains("copy-legacy"), "{error}");
    }

    #[test]
    fn rejects_unreviewed_device_writer_across_removable_mount_aliases() {
        let error = prepare_legacy_copy_collision(
            "directory_path",
            json!("/data/local/tmp/extracted"),
            "device",
            "/mnt/media_rw/ABCD-1234/Games/settings.cfg",
            "file",
            "settings.cfg",
            None,
            Some("steps.example.recipe/unpack.outputs.extracted_path"),
            None,
            None,
        )
        .expect_err("a deferred device writer must collide through removable mount aliases");

        assert!(error.contains("copy-setup"), "{error}");
        assert!(error.contains("copy-legacy"), "{error}");
    }

    #[test]
    fn rejects_unreviewed_device_extract_directory_collision_with_repeated_separators() {
        let error = prepare_legacy_copy_collision(
            "directory_path",
            json!("/data/local/tmp/extracted"),
            "device",
            "/storage/ABCD-1234/Games//",
            "directory",
            "settings.cfg",
            None,
            Some("steps.example.recipe/unpack.outputs.extracted_path"),
            None,
            None,
        )
        .expect_err("a deferred device directory writer must collide after path canonicalization");

        assert!(error.contains("copy-setup"), "{error}");
        assert!(error.contains("copy-legacy"), "{error}");
    }

    #[test]
    fn rejects_unreviewed_device_extract_writer_overlapping_setup_location() {
        let error = prepare_legacy_copy_collision(
            "file_path",
            json!("/tmp/legacy.cfg"),
            "host",
            "/storage/ABCD-1234/Other",
            "directory",
            "settings.cfg",
            None,
            None,
            Some("/storage/ABCD-1234/Games//"),
            None,
        )
        .expect_err("device archive extraction must join active Setup Location collision review");

        assert!(error.contains("copy-setup"), "{error}");
        assert!(error.contains("unpack"), "{error}");
    }

    #[test]
    fn allows_device_extract_writer_with_non_overlapping_reviewed_destination() {
        let result = prepare_legacy_copy_collision(
            "file_path",
            json!("/tmp/legacy.cfg"),
            "host",
            "/data/local/tmp/copy",
            "directory",
            "settings.cfg",
            None,
            None,
            Some("/data/local/tmp/extracted"),
            None,
        );

        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn host_extract_with_pending_archive_artifact_does_not_block_setup_location_plan() {
        let result = prepare_legacy_copy_collision(
            "file_path",
            json!("/tmp/legacy.cfg"),
            "host",
            "/data/local/tmp/copy",
            "directory",
            "settings.cfg",
            None,
            None,
            Some("/tmp/extracted"),
            Some("host"),
        );

        assert!(result.is_ok(), "{result:?}");
    }
}
