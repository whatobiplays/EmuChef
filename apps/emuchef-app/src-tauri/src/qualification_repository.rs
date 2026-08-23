//! Fixed-root persistence and bounded access to the canonical qualification tool.
//!
//! Candidate files are deliberately treated as non-authoritative handoff data.
//! Rust owns only the local storage boundary and the small amount of integrity
//! checking needed to recover that data safely. The Node tool remains the
//! authority for candidate semantics, canonical digests, and repository state.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(target_os = "macos")]
use std::{ffi::CString, os::unix::ffi::OsStrExt};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::qualification_build::{embedded_build_identity, QualificationBuildIdentity};

/// Prefix shared by every opaque candidate handle.
pub(crate) const CANDIDATE_HANDLE_PREFIX: &str = "qualification-candidate-";
const CANDIDATE_HANDLE_HEX_LENGTH: usize = 32;
const CANDIDATE_SCHEMA_VERSION: u64 = 1;
const CANDIDATE_DIRECTORY: &str = ".emuchef_runtime/qualification-candidates";
const QUALIFICATION_TOOL: &str = "tools/device-qualification.mjs";
const CANDIDATE_FILE: &str = "candidate.json";
const EXECUTION_REPORT_FILE: &str = "execution-report.json";
const CANDIDATE_STAGING_PREFIX: &str = ".qualification-candidate-tmp-";
const MAX_TOOL_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// The two candidate kinds understood by the repository qualification tool.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    TargetRegistration,
    QualificationRun,
}

impl CandidateKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::TargetRegistration => "target_registration",
            Self::QualificationRun => "qualification_run",
        }
    }
}

/// Safe, presentation-ready information about a persisted candidate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationCandidateSummary {
    pub(crate) candidate_handle: String,
    pub(crate) kind: CandidateKind,
    pub(crate) captured_at: String,
    pub(crate) promotable: bool,
    pub(crate) non_promotable_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) run_validity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qualification_outcome: Option<String>,
}

/// Metadata for the one optional report file owned by a candidate directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QualificationReportMetadata {
    pub(crate) path: String,
    pub(crate) byte_length: u64,
    pub(crate) sha256: String,
}

/// The Rust-owned envelope stored around a Node-owned candidate payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CandidateFileEnvelope {
    candidate_handle: String,
    kind: CandidateKind,
    captured_at: Option<String>,
    build: Option<QualificationBuildIdentity>,
    payload: Value,
    report: Option<QualificationReportMetadata>,
}

/// A candidate loaded from the fixed runtime directory.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredQualificationCandidate {
    pub(crate) candidate_handle: String,
    pub(crate) kind: CandidateKind,
    pub(crate) captured_at: Option<String>,
    pub(crate) build: Option<QualificationBuildIdentity>,
    pub(crate) payload: Value,
    pub(crate) report: Option<QualificationReportMetadata>,
    #[serde(skip)]
    pub(crate) report_bytes: Option<Vec<u8>>,
    pub(crate) promotable: bool,
    pub(crate) non_promotable_reason: Option<String>,
}

/// The bounded operation envelope emitted by the canonical Node tool.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationOperation {
    RegisterTarget,
    RecordRun,
}

/// Rust-owned shape for a canonical target/run operation result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationOperationResult {
    pub(crate) operation: QualificationOperation,
    pub(crate) candidate_handle: String,
    pub(crate) candidate_kind: CandidateKind,
    pub(crate) payload: Value,
}

/// The machine-readable repository description returned by Node `--describe`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryQualificationDescription {
    pub(crate) schema_version: u64,
    pub(crate) runtime_contract: String,
    pub(crate) qualification_contract: u32,
    pub(crate) build: QualificationBuildIdentity,
    pub(crate) workflow_catalog: Value,
    pub(crate) device_targets: Value,
}

/// The trusted source/worktree facts used to decide whether qualification may
/// record or promote anything from the current application build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QualificationSourceState {
    pub(crate) head: String,
    pub(crate) tracked_worktree_clean: bool,
}

/// A repository status snapshot is read while the repository operation gate is
/// held so candidate projection cannot observe a half-completed mutation.
pub(crate) struct QualificationRepositoryStatus {
    pub(crate) description: RepositoryQualificationDescription,
    pub(crate) candidates: Vec<QualificationCandidateSummary>,
    pub(crate) recordable: bool,
}

/// Narrow seam used to test the repository without starting Node.
pub trait QualificationToolRunner: Send + Sync {
    fn run(&self, repo_root: &Path, args: &[String]) -> Result<Vec<u8>, String>;
}

/// Candidate persistence and bounded access to the canonical Node tool.
pub struct QualificationRepository {
    repo_root: PathBuf,
    candidate_root: PathBuf,
    runner: Box<dyn QualificationToolRunner>,
    embedded_build_identity: Option<QualificationBuildIdentity>,
    operation_gate: Mutex<()>,
    lifecycle_dirty: AtomicBool,
    #[cfg(test)]
    source_state_override: Option<std::sync::Arc<Mutex<QualificationSourceState>>>,
}

/// Lazily resolves the trusted qualification repository only when the mode is
/// enabled and a valid source checkout is available.
pub struct QualificationRepositoryProvider {
    repository: OnceLock<Option<QualificationRepository>>,
}

impl Default for QualificationRepositoryProvider {
    fn default() -> Self {
        Self {
            repository: OnceLock::new(),
        }
    }
}

impl QualificationRepositoryProvider {
    pub(crate) fn get(&self) -> Option<&QualificationRepository> {
        self.repository
            .get_or_init(QualificationRepository::production)
            .as_ref()
    }

    #[cfg(test)]
    pub(crate) fn is_initialized_for_test(&self) -> bool {
        self.repository.get().is_some()
    }

    #[cfg(test)]
    pub(crate) fn unavailable_for_test() -> Self {
        let provider = Self::default();
        let _ = provider.repository.set(None);
        provider
    }

    #[cfg(test)]
    pub(crate) fn for_test(repository: QualificationRepository) -> Self {
        let provider = Self::default();
        let _ = provider.repository.set(Some(repository));
        provider
    }
}

impl QualificationRepository {
    /// Builds the production repository using the compile-time trusted root.
    ///
    /// Packaged or ordinary builds may not contain the development source
    /// checkout. In that case construction returns `None` without resolving a
    /// path or invoking Node.
    pub fn production() -> Option<Self> {
        let repo_root = production_repo_root();
        validate_repository_root(&repo_root).ok()?;
        qualification_tool_path(&repo_root).ok()?;
        Some(Self::with_root(
            repo_root,
            Box::new(ProcessQualificationToolRunner),
        ))
    }

    /// Builds a repository with an injected runner for unit tests.
    #[cfg(test)]
    pub fn new_for_test(repo_root: PathBuf, runner: Box<dyn QualificationToolRunner>) -> Self {
        Self::with_root(repo_root, runner)
    }

    #[cfg(test)]
    fn new_for_test_with_embedded_build(
        repo_root: PathBuf,
        runner: Box<dyn QualificationToolRunner>,
        build: QualificationBuildIdentity,
    ) -> Self {
        let mut repository = Self::with_root(repo_root, runner);
        repository.embedded_build_identity = Some(build);
        repository.source_state_override =
            Some(std::sync::Arc::new(Mutex::new(QualificationSourceState {
                head: repository
                    .embedded_build_identity
                    .as_ref()
                    .expect("test build identity should be present")
                    .git_commit
                    .clone(),
                tracked_worktree_clean: true,
            })));
        repository
    }

    #[cfg(test)]
    pub(crate) fn new_for_test_with_source_state(
        repo_root: PathBuf,
        runner: Box<dyn QualificationToolRunner>,
        build: QualificationBuildIdentity,
        source_state: QualificationSourceState,
    ) -> Self {
        let repository = Self::new_for_test_with_embedded_build(repo_root, runner, build);
        *repository
            .source_state_override
            .as_ref()
            .expect("test source state should be present")
            .lock()
            .expect("test source state should not be poisoned") = source_state;
        repository
    }

    fn with_root(repo_root: PathBuf, runner: Box<dyn QualificationToolRunner>) -> Self {
        let repo_root = absolute_normalized_path(&repo_root);
        Self {
            candidate_root: repo_root.join(CANDIDATE_DIRECTORY),
            repo_root,
            runner,
            embedded_build_identity: embedded_build_identity(),
            operation_gate: Mutex::new(()),
            lifecycle_dirty: AtomicBool::new(false),
            #[cfg(test)]
            source_state_override: None,
        }
    }

    fn lock_operation(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.operation_gate
            .lock()
            .map_err(|_| "qualification repository operation is unavailable".to_string())
    }

    fn current_source_state(&self) -> Result<QualificationSourceState, String> {
        #[cfg(test)]
        if let Some(source_state) = &self.source_state_override {
            return source_state
                .lock()
                .map(|state| state.clone())
                .map_err(|_| "qualification source state is unavailable".to_string());
        }
        read_git_source_state(&self.repo_root)
    }

    #[cfg(test)]
    fn set_source_state_for_test(&self, state: QualificationSourceState) {
        if let Some(source_state) = &self.source_state_override {
            *source_state
                .lock()
                .expect("test source state should not be poisoned") = state;
        }
    }

    /// Rechecks the trusted source/worktree lifecycle before capture begins.
    pub(crate) fn require_recordable(&self) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        self.ensure_recordable_unlocked()
    }

    /// Returns the normalized trusted repository root used for every operation.
    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Returns the fixed ignored candidate root beneath the repository.
    pub(crate) fn candidate_root(&self) -> &Path {
        &self.candidate_root
    }

    /// Stores a candidate and returns its opaque handle.
    pub fn create_candidate(
        &self,
        kind: CandidateKind,
        json: &Value,
        report_bytes: Option<&[u8]>,
    ) -> Result<String, String> {
        let _operation = self.lock_operation()?;
        let mut candidate = json
            .as_object()
            .cloned()
            .ok_or_else(|| "qualification candidate must be a JSON object".to_string())?;
        let candidate_value = Value::Object(candidate.clone());
        let captured_at = optional_string_field(&candidate_value, "capturedAt")?;
        let build = candidate_build_identity(&candidate_value)?;
        let report = report_metadata_for_candidate(&candidate_value, report_bytes)?;
        ensure_candidate_root(&self.repo_root, &self.candidate_root)?;

        for _ in 0..4 {
            let handle = new_candidate_handle();
            let staging = self.candidate_root.join(format!(
                "{CANDIDATE_STAGING_PREFIX}{}",
                Uuid::new_v4().simple()
            ));
            match fs::create_dir(&staging) {
                Ok(()) => {
                    candidate.insert(
                        "candidateSchemaVersion".to_string(),
                        Value::from(CANDIDATE_SCHEMA_VERSION),
                    );
                    candidate.insert("candidateId".to_string(), Value::from(handle.clone()));
                    candidate.insert("kind".to_string(), Value::from(kind.as_str()));

                    let envelope = CandidateFileEnvelope {
                        candidate_handle: handle.clone(),
                        kind,
                        captured_at: captured_at.clone(),
                        build: build.clone(),
                        payload: Value::Object(candidate.clone()),
                        report: report.clone(),
                    };
                    let mut bytes = serde_json::to_vec_pretty(&envelope).map_err(|_| {
                        "qualification candidate could not be serialized".to_string()
                    })?;
                    bytes.push(b'\n');
                    if let Err(error) = write_synced_new_file(&staging.join(CANDIDATE_FILE), &bytes)
                    {
                        cleanup_owned_staging(&staging);
                        return Err(error);
                    }
                    if let Some(report_bytes) = report_bytes {
                        if let Err(error) = write_synced_new_file(
                            &staging.join(EXECUTION_REPORT_FILE),
                            report_bytes,
                        ) {
                            cleanup_owned_staging(&staging);
                            return Err(error);
                        }
                    }

                    if let Err(error) = sync_directory(&staging) {
                        cleanup_owned_staging(&staging);
                        return Err(error);
                    }
                    let final_directory = self.candidate_root.join(&handle);
                    match publish_staged_candidate(&staging, &final_directory) {
                        Ok(()) => {
                            sync_directory(&self.candidate_root)?;
                            return Ok(handle);
                        }
                        Err(PublishCandidateError::Collision) => {
                            cleanup_owned_staging(&staging);
                        }
                        Err(PublishCandidateError::Io(error)) => {
                            cleanup_owned_staging(&staging);
                            return Err(error);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err("qualification candidate directory could not be created".to_string())
                }
            }
        }
        Err("qualification candidate handle could not be allocated".to_string())
    }

    /// Lists stored candidates after local integrity checks.
    pub fn list_candidates(&self) -> Result<Vec<QualificationCandidateSummary>, String> {
        let _operation = self.lock_operation()?;
        self.list_candidates_unlocked()
    }

    fn list_candidates_unlocked(&self) -> Result<Vec<QualificationCandidateSummary>, String> {
        let Some(root) = existing_candidate_root(&self.repo_root, &self.candidate_root)? else {
            return Ok(Vec::new());
        };
        let mut handles = Vec::new();
        for entry in fs::read_dir(root)
            .map_err(|_| "qualification candidates could not be listed".to_string())?
        {
            let entry =
                entry.map_err(|_| "qualification candidates could not be listed".to_string())?;
            let file_type = entry
                .file_type()
                .map_err(|_| "qualification candidate metadata could not be read".to_string())?;
            let handle = entry.file_name().to_string_lossy().into_owned();
            if file_type.is_symlink() && validate_candidate_handle(&handle).is_ok() {
                return Err("qualification candidate directory is a symlink".to_string());
            }
            if file_type.is_dir() && validate_candidate_handle(&handle).is_ok() {
                handles.push(handle);
            }
        }
        handles.sort();

        handles
            .into_iter()
            .map(|handle| {
                self.load_candidate_unlocked(&handle)
                    .map(|candidate| candidate.summary())
            })
            .collect()
    }

    /// Loads one candidate and rechecks only Rust-owned local integrity.
    pub fn load_candidate(&self, handle: &str) -> Result<StoredQualificationCandidate, String> {
        let _operation = self.lock_operation()?;
        self.load_candidate_unlocked(handle)
    }

    fn load_candidate_unlocked(
        &self,
        handle: &str,
    ) -> Result<StoredQualificationCandidate, String> {
        let directory = self.candidate_directory_unlocked(handle)?;
        let candidate_bytes =
            read_regular_file(&directory.join(CANDIDATE_FILE), "qualification candidate")?;
        let envelope = decode_candidate_envelope(&candidate_bytes)?;
        validate_candidate_binding(&envelope, handle)?;
        let payload_build = candidate_build_identity(&envelope.payload)?;
        if payload_build != envelope.build {
            return Err("qualification candidate build metadata is inconsistent".to_string());
        }
        if optional_string_field(&envelope.payload, "capturedAt")? != envelope.captured_at {
            return Err("qualification candidate capture metadata is inconsistent".to_string());
        }
        let report_path = directory.join(EXECUTION_REPORT_FILE);
        let declared_report_sha256 = candidate_report_sha256(&envelope.payload)?;
        let report_bytes = load_report_bytes(
            &report_path,
            envelope.report.as_ref(),
            declared_report_sha256.as_deref(),
        )?;
        let (promotable, non_promotable_reason) =
            self.candidate_promotion_status(envelope.build.as_ref());

        Ok(StoredQualificationCandidate {
            candidate_handle: handle.to_string(),
            kind: envelope.kind,
            captured_at: envelope.captured_at,
            build: envelope.build,
            payload: envelope.payload,
            report: envelope.report,
            report_bytes,
            promotable,
            non_promotable_reason,
        })
    }

    /// Removes one validated candidate directory beneath the fixed root.
    pub fn discard_candidate(&self, handle: &str) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(handle)?;
        validate_candidate_files(&directory)?;
        fs::remove_dir_all(directory)
            .map_err(|_| "qualification candidate could not be discarded".to_string())
    }

    /// Invokes the canonical tool's bounded repository description operation.
    pub fn describe(&self) -> Result<RepositoryQualificationDescription, String> {
        let _operation = self.lock_operation()?;
        self.describe_unlocked()
    }

    fn describe_unlocked(&self) -> Result<RepositoryQualificationDescription, String> {
        let output = self.invoke_unlocked(vec!["--describe".to_string()])?;
        serde_json::from_slice(&output)
            .map_err(|_| "qualification repository description is invalid".to_string())
    }

    /// Reads Node's repository projection and local candidates under the same
    /// operation gate used by candidate mutations.
    pub(crate) fn describe_and_list_candidates(
        &self,
    ) -> Result<QualificationRepositoryStatus, String> {
        let _operation = self.lock_operation()?;
        let description = self.describe_unlocked()?;
        let candidates = self.list_candidates_unlocked()?;
        let recordable = self.recordable_for_build_unlocked(&description.build);
        Ok(QualificationRepositoryStatus {
            description,
            candidates,
            recordable,
        })
    }

    /// Invokes the canonical tool for a target-registration candidate.
    pub fn register_target(&self, handle: &str) -> Result<QualificationOperationResult, String> {
        let _operation = self.lock_operation()?;
        self.ensure_recordable_unlocked()?;
        self.require_kind_unlocked(handle, CandidateKind::TargetRegistration)?;
        let output = self.invoke_candidate_unlocked(
            handle,
            vec!["--register-target".to_string(), handle.to_string()],
        )?;
        let result = parse_operation_result(
            &output,
            QualificationOperation::RegisterTarget,
            handle,
            CandidateKind::TargetRegistration,
        )?;
        self.lifecycle_dirty.store(true, Ordering::Release);
        Ok(result)
    }

    /// Invokes the canonical tool for a qualification-run candidate.
    pub fn record_run(&self, handle: &str) -> Result<QualificationOperationResult, String> {
        let _operation = self.lock_operation()?;
        self.ensure_recordable_unlocked()?;
        self.require_kind_unlocked(handle, CandidateKind::QualificationRun)?;
        let output = self.invoke_candidate_unlocked(
            handle,
            vec!["--record-run".to_string(), handle.to_string()],
        )?;
        parse_operation_result(
            &output,
            QualificationOperation::RecordRun,
            handle,
            CandidateKind::QualificationRun,
        )
    }

    fn require_kind_unlocked(&self, handle: &str, expected: CandidateKind) -> Result<(), String> {
        let candidate = self.load_candidate_unlocked(handle)?;
        if candidate.kind != expected {
            return Err(
                "qualification candidate kind does not match the requested operation".to_string(),
            );
        }
        if !candidate.promotable {
            return Err(candidate
                .non_promotable_reason
                .unwrap_or_else(|| "qualification candidate is not promotable".to_string()));
        }
        Ok(())
    }

    fn candidate_directory_unlocked(&self, handle: &str) -> Result<PathBuf, String> {
        validate_candidate_handle(handle)?;
        let root = existing_candidate_root(&self.repo_root, &self.candidate_root)?
            .ok_or_else(|| "qualification candidate root does not exist".to_string())?;
        let directory = root.join(handle);
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|_| "qualification candidate does not exist".to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("qualification candidate directory is a symlink".to_string());
        }
        if !metadata.is_dir() {
            return Err("qualification candidate directory is invalid".to_string());
        }
        Ok(directory)
    }

    fn invoke_unlocked(&self, args: Vec<String>) -> Result<Vec<u8>, String> {
        if !allowlisted_tool_args(&args) {
            return Err("qualification tool operation is not allowlisted".to_string());
        }
        validate_repository_root(&self.repo_root)?;
        let _ = existing_candidate_root(&self.repo_root, &self.candidate_root)?;
        self.runner.run(&self.repo_root, &args)
    }

    fn invoke_candidate_unlocked(
        &self,
        handle: &str,
        args: Vec<String>,
    ) -> Result<Vec<u8>, String> {
        let directory = self.candidate_directory_unlocked(handle)?;
        validate_candidate_files(&directory)?;
        self.invoke_unlocked(args)
    }

    fn candidate_promotion_status(
        &self,
        candidate_build: Option<&QualificationBuildIdentity>,
    ) -> (bool, Option<String>) {
        let Some(candidate_build) = candidate_build else {
            return (
                false,
                Some("qualification candidate build identity is missing".to_string()),
            );
        };
        let Some(embedded_build) = self.embedded_build_identity.as_ref() else {
            return (
                false,
                Some("this application has no recordable qualification build identity".to_string()),
            );
        };
        if candidate_build != embedded_build {
            return (
                false,
                Some(
                    "qualification candidate build identity does not match this build".to_string(),
                ),
            );
        }
        if self.lifecycle_dirty.load(Ordering::Acquire) {
            return (
                false,
                Some("qualification source state is dirty after a canonical mutation".to_string()),
            );
        }
        let Ok(source_state) = self.current_source_state() else {
            return (
                false,
                Some("qualification source state is unavailable".to_string()),
            );
        };
        if !source_state.tracked_worktree_clean {
            return (
                false,
                Some("qualification source state is not clean".to_string()),
            );
        }
        if source_state.head != embedded_build.git_commit {
            return (
                false,
                Some("qualification source state no longer matches this build".to_string()),
            );
        }
        (true, None)
    }

    fn recordable_for_build_unlocked(&self, current_build: &QualificationBuildIdentity) -> bool {
        self.candidate_promotion_status(Some(current_build)).0
    }

    fn ensure_recordable_unlocked(&self) -> Result<(), String> {
        let Some(embedded_build) = self.embedded_build_identity.as_ref() else {
            return Err("qualification build identity is unavailable".to_string());
        };
        if self.lifecycle_dirty.load(Ordering::Acquire) {
            return Err(
                "qualification source state is dirty after a canonical mutation".to_string(),
            );
        }
        let source_state = self.current_source_state()?;
        if !source_state.tracked_worktree_clean || source_state.head != embedded_build.git_commit {
            return Err(
                "qualification source state changed and requires a clean rebuild".to_string(),
            );
        }
        Ok(())
    }
}

impl StoredQualificationCandidate {
    fn summary(&self) -> QualificationCandidateSummary {
        QualificationCandidateSummary {
            candidate_handle: self.candidate_handle.clone(),
            kind: self.kind,
            captured_at: self.captured_at.clone().unwrap_or_default(),
            promotable: self.promotable,
            non_promotable_reason: self.non_promotable_reason.clone(),
            target: self.payload.get("target").cloned(),
            run_validity: self
                .payload
                .get("runValidity")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            qualification_outcome: self
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        }
    }
}

struct ProcessQualificationToolRunner;

impl QualificationToolRunner for ProcessQualificationToolRunner {
    fn run(&self, repo_root: &Path, args: &[String]) -> Result<Vec<u8>, String> {
        if !allowlisted_tool_args(args) {
            return Err("qualification tool operation is not allowlisted".to_string());
        }
        let tool = qualification_tool_path(repo_root)?;
        let output = Command::new("node")
            .arg(tool)
            .args(args)
            .current_dir(repo_root)
            .output()
            .map_err(|_| "qualification tool could not be started".to_string())?;
        if output.stdout.len() > MAX_TOOL_OUTPUT_BYTES
            || output.stderr.len() > MAX_TOOL_OUTPUT_BYTES
        {
            return Err("qualification tool output exceeded its limit".to_string());
        }
        if !output.status.success() {
            return Err("qualification tool operation failed".to_string());
        }
        Ok(output.stdout)
    }
}

/// Derives the only production repository root from the trusted manifest path.
pub fn production_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn read_git_source_state(repo_root: &Path) -> Result<QualificationSourceState, String> {
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .map_err(|_| "qualification source state is unavailable".to_string())?;
    if !head.status.success() {
        return Err("qualification source state is unavailable".to_string());
    }
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .current_dir(repo_root)
        .output()
        .map_err(|_| "qualification source state is unavailable".to_string())?;
    if !status.status.success() {
        return Err("qualification source state is unavailable".to_string());
    }
    let head = String::from_utf8(head.stdout)
        .map_err(|_| "qualification source state is unavailable".to_string())?
        .trim()
        .to_string();
    Ok(QualificationSourceState {
        head,
        tracked_worktree_clean: status.stdout.is_empty(),
    })
}

fn new_candidate_handle() -> String {
    format!("{}{}", CANDIDATE_HANDLE_PREFIX, Uuid::new_v4().simple())
}

fn validate_candidate_handle(handle: &str) -> Result<(), String> {
    let suffix = handle
        .strip_prefix(CANDIDATE_HANDLE_PREFIX)
        .ok_or_else(|| "qualification candidate handle is invalid".to_string())?;
    if suffix.len() != CANDIDATE_HANDLE_HEX_LENGTH
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("qualification candidate handle is invalid".to_string());
    }
    Ok(())
}

fn ensure_candidate_root(repo_root: &Path, candidate_root: &Path) -> Result<(), String> {
    validate_candidate_root_path(repo_root, candidate_root, true).map(|_| ())
}

fn existing_candidate_root(
    repo_root: &Path,
    candidate_root: &Path,
) -> Result<Option<PathBuf>, String> {
    validate_candidate_root_path(repo_root, candidate_root, false)
}

fn validate_repository_root(repo_root: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(repo_root)
        .map_err(|_| "qualification repository root is unavailable".to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("qualification repository root is a symlink".to_string());
    }
    if !metadata.is_dir() {
        return Err("qualification repository root is not a directory".to_string());
    }
    Ok(())
}

fn validate_candidate_root_path(
    repo_root: &Path,
    candidate_root: &Path,
    create_missing: bool,
) -> Result<Option<PathBuf>, String> {
    validate_repository_root(repo_root)?;
    let relative = candidate_root
        .strip_prefix(repo_root)
        .map_err(|_| "qualification candidate root is outside the repository".to_string())?;
    let mut current = repo_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err("qualification candidate root contains an invalid path".to_string());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("qualification candidate root contains a symlink".to_string());
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("qualification candidate root contains a non-directory".to_string());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_missing => {
                fs::create_dir(&current)
                    .map_err(|_| "qualification candidate root could not be created".to_string())?;
                let metadata = fs::symlink_metadata(&current).map_err(|_| {
                    "qualification candidate root could not be inspected".to_string()
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(
                        "qualification candidate root is not a regular directory".to_string()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("qualification candidate root could not be inspected".to_string()),
        }
    }
    Ok(Some(candidate_root.to_path_buf()))
}

fn absolute_normalized_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn optional_string_field(candidate: &Value, field: &str) -> Result<Option<String>, String> {
    match candidate.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!(
            "qualification candidate {field} metadata is invalid"
        )),
    }
}

fn candidate_build_identity(
    candidate: &Value,
) -> Result<Option<QualificationBuildIdentity>, String> {
    match candidate.get("build") {
        None | Some(Value::Null) => Ok(None),
        Some(build) => serde_json::from_value(build.clone())
            .map(Some)
            .map_err(|_| "qualification candidate build identity is invalid".to_string()),
    }
}

fn candidate_report_sha256(candidate: &Value) -> Result<Option<String>, String> {
    let Some(artifacts) = candidate.get("artifacts") else {
        return Ok(None);
    };
    let artifacts = artifacts
        .as_array()
        .ok_or_else(|| "qualification candidate artifacts are invalid".to_string())?;
    let mut report_sha256 = None;
    for artifact in artifacts {
        if artifact.get("path").and_then(Value::as_str) != Some(EXECUTION_REPORT_FILE) {
            continue;
        }
        if report_sha256.is_some() {
            return Err(
                "qualification candidate declares the execution report more than once".to_string(),
            );
        }
        let sha256 = artifact
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                "qualification execution report metadata is missing its digest".to_string()
            })?;
        if !is_sha256_hex(sha256) {
            return Err(
                "qualification execution report metadata has an invalid digest".to_string(),
            );
        }
        report_sha256 = Some(sha256.to_string());
    }
    Ok(report_sha256)
}

fn report_metadata_for_candidate(
    candidate: &Value,
    report_bytes: Option<&[u8]>,
) -> Result<Option<QualificationReportMetadata>, String> {
    let declared_sha256 = candidate_report_sha256(candidate)?;
    match (declared_sha256, report_bytes) {
        (Some(expected), Some(bytes)) => {
            let actual = hex::encode(Sha256::digest(bytes));
            if actual != expected {
                return Err(
                    "qualification execution report metadata does not match its bytes".to_string(),
                );
            }
            Ok(Some(QualificationReportMetadata {
                path: EXECUTION_REPORT_FILE.to_string(),
                byte_length: bytes.len() as u64,
                sha256: actual,
            }))
        }
        (Some(_), None) => {
            Err("qualification execution report is declared but its bytes are missing".to_string())
        }
        (None, Some(_)) => Err(
            "qualification execution report bytes are not declared by the candidate".to_string(),
        ),
        (None, None) => Ok(None),
    }
}

fn decode_candidate_envelope(bytes: &[u8]) -> Result<CandidateFileEnvelope, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| "qualification candidate JSON is invalid".to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "qualification candidate envelope must be a JSON object".to_string())?;
    const ENVELOPE_FIELDS: [&str; 6] = [
        "candidateHandle",
        "kind",
        "capturedAt",
        "build",
        "payload",
        "report",
    ];
    if object.len() != ENVELOPE_FIELDS.len()
        || ENVELOPE_FIELDS
            .iter()
            .any(|field| !object.contains_key(*field))
    {
        return Err("qualification candidate envelope has the wrong shape".to_string());
    }
    serde_json::from_value(value)
        .map_err(|_| "qualification candidate envelope is invalid".to_string())
}

fn validate_candidate_binding(
    envelope: &CandidateFileEnvelope,
    handle: &str,
) -> Result<(), String> {
    if envelope.candidate_handle != handle {
        return Err("qualification candidate identity does not match its directory".to_string());
    }
    let candidate = envelope
        .payload
        .as_object()
        .ok_or_else(|| "qualification candidate payload must be a JSON object".to_string())?;
    if candidate.get("candidateId").and_then(Value::as_str) != Some(handle) {
        return Err(
            "qualification candidate payload identity does not match its directory".to_string(),
        );
    }
    let payload_kind = serde_json::from_value::<CandidateKind>(
        candidate
            .get("kind")
            .cloned()
            .ok_or_else(|| "qualification candidate payload kind is missing".to_string())?,
    )
    .map_err(|_| "qualification candidate payload kind is invalid".to_string())?;
    if payload_kind != envelope.kind {
        return Err("qualification candidate kind metadata is inconsistent".to_string());
    }
    Ok(())
}

fn validate_candidate_files(directory: &Path) -> Result<(), String> {
    validate_regular_file_path(
        &directory.join(CANDIDATE_FILE),
        true,
        "qualification candidate",
    )?;
    validate_regular_file_path(
        &directory.join(EXECUTION_REPORT_FILE),
        false,
        "qualification execution report",
    )?;
    Ok(())
}

fn validate_regular_file_path(path: &Path, required: bool, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("{label} file is a symlink"))
        }
        Ok(metadata) if !metadata.is_file() => Err(format!("{label} file is not regular")),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => Ok(()),
        Err(_) if required => Err(format!("{label} file could not be inspected")),
        Err(_) => Err(format!("{label} file could not be inspected")),
    }
}

fn read_regular_file(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    validate_regular_file_path(path, true, label)?;
    let mut options = OpenOptions::new();
    options.read(true);
    // Keep the final component no-following even if it changes after lstat.
    #[cfg(target_os = "macos")]
    options.custom_flags(0x100);
    #[cfg(target_os = "linux")]
    options.custom_flags(0x20_000);
    let mut file = options
        .open(path)
        .map_err(|_| format!("{label} file could not be read"))?;
    let metadata = file
        .metadata()
        .map_err(|_| format!("{label} file could not be inspected"))?;
    if !metadata.is_file() {
        return Err(format!("{label} file is not regular"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| format!("{label} file could not be read"))?;
    Ok(bytes)
}

fn load_report_bytes(
    report_path: &Path,
    metadata: Option<&QualificationReportMetadata>,
    declared_sha256: Option<&str>,
) -> Result<Option<Vec<u8>>, String> {
    match metadata {
        Some(metadata) => {
            if metadata.path != EXECUTION_REPORT_FILE
                || declared_sha256 != Some(metadata.sha256.as_str())
                || !is_sha256_hex(&metadata.sha256)
            {
                return Err("qualification execution report metadata is inconsistent".to_string());
            }
            let bytes = read_regular_file(report_path, "qualification execution report")?;
            if metadata.byte_length != bytes.len() as u64
                || metadata.sha256 != hex::encode(Sha256::digest(&bytes))
            {
                return Err(
                    "qualification execution report bytes do not match metadata".to_string()
                );
            }
            Ok(Some(bytes))
        }
        None => match fs::symlink_metadata(report_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err("qualification execution report is a symlink".to_string())
            }
            Ok(_) => {
                Err("qualification execution report is not declared by the candidate".to_string())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("qualification execution report could not be inspected".to_string()),
        },
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn qualification_tool_path(repo_root: &Path) -> Result<PathBuf, String> {
    validate_repository_root(repo_root)?;
    let tools_directory = repo_root.join("tools");
    let tools_metadata = fs::symlink_metadata(&tools_directory)
        .map_err(|_| "qualification tool directory is unavailable".to_string())?;
    if tools_metadata.file_type().is_symlink() || !tools_metadata.is_dir() {
        return Err("qualification tool directory is not trusted".to_string());
    }
    let tool = repo_root.join(QUALIFICATION_TOOL);
    let metadata =
        fs::symlink_metadata(&tool).map_err(|_| "qualification tool is unavailable".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("qualification tool is not a regular file".to_string());
    }
    Ok(tool)
}

fn write_synced_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "qualification candidate file could not be created".to_string())?;
    file.write_all(bytes)
        .map_err(|_| "qualification candidate file could not be written".to_string())?;
    file.sync_all()
        .map_err(|_| "qualification candidate file could not be synchronized".to_string())?;
    drop(file);
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "qualification candidate directory could not be synchronized".to_string())
}

enum PublishCandidateError {
    Collision,
    Io(String),
}

fn publish_staged_candidate(
    staging: &Path,
    final_directory: &Path,
) -> Result<(), PublishCandidateError> {
    publish_staged_candidate_with_hook(staging, final_directory, || {})
}

fn publish_staged_candidate_with_hook<F>(
    staging: &Path,
    final_directory: &Path,
    after_absence_check: F,
) -> Result<(), PublishCandidateError>
where
    F: FnOnce(),
{
    if fs::symlink_metadata(final_directory).is_ok() {
        cleanup_owned_staging(staging);
        return Err(PublishCandidateError::Collision);
    }

    after_absence_check();
    match rename_without_replacing(staging, final_directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            cleanup_owned_staging(staging);
            Err(PublishCandidateError::Collision)
        }
        Err(_) => {
            cleanup_owned_staging(staging);
            Err(PublishCandidateError::Io(
                "qualification candidate directory could not be committed".to_string(),
            ))
        }
    }
}

/// Renames a complete staged candidate directory without replacing a destination.
///
/// The macOS primitive evaluates the destination-exists condition as part of the
/// filesystem operation, so a competing publisher cannot win the check/rename
/// interval and have its candidate replaced.
fn rename_without_replacing(staging: &Path, final_directory: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let staging = CString::new(staging.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid staging path"))?;
        let final_directory = CString::new(final_directory.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid candidate path"))?;
        let result = unsafe {
            libc::renamex_np(
                staging.as_ptr(),
                final_directory.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (staging, final_directory);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic no-replace candidate publication is unsupported on this platform",
        ))
    }
}

fn cleanup_owned_staging(path: &Path) {
    let owned = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(CANDIDATE_STAGING_PREFIX));
    if !owned {
        return;
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let _ = fs::remove_file(path);
        }
        Ok(_) => {
            let _ = fs::remove_dir_all(path);
        }
        Err(_) => {}
    }
}

fn allowlisted_tool_args(args: &[String]) -> bool {
    match args {
        [operation] => operation == "--describe",
        [operation, handle] if operation == "--register-target" || operation == "--record-run" => {
            validate_candidate_handle(handle).is_ok()
        }
        _ => false,
    }
}

fn parse_operation_result(
    output: &[u8],
    expected_operation: QualificationOperation,
    handle: &str,
    expected_kind: CandidateKind,
) -> Result<QualificationOperationResult, String> {
    let result: QualificationOperationResult = serde_json::from_slice(output)
        .map_err(|_| "qualification tool returned an invalid operation envelope".to_string())?;
    if result.operation != expected_operation
        || result.candidate_handle != handle
        || result.candidate_kind != expected_kind
        || !result.payload.is_object()
    {
        return Err("qualification tool returned the wrong operation result shape".to_string());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier, Mutex,
    };
    use std::thread;

    use serde_json::{json, Value};
    use sha2::Digest;
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;

    #[derive(Clone, Default)]
    struct FakeQualificationToolRunner {
        calls: Arc<Mutex<Vec<RunnerCall>>>,
        response: Arc<Mutex<Vec<u8>>>,
    }

    type RunnerCall = (PathBuf, Vec<String>);

    impl FakeQualificationToolRunner {
        fn with_response(response: Value) -> Self {
            Self {
                response: Arc::new(Mutex::new(
                    serde_json::to_vec(&response).expect("fake response should serialize"),
                )),
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<(PathBuf, Vec<String>)> {
            self.calls
                .lock()
                .expect("fake calls should not be poisoned")
                .clone()
        }

        fn set_response(&self, response: Value) {
            *self
                .response
                .lock()
                .expect("fake response should not be poisoned") =
                serde_json::to_vec(&response).expect("fake response should serialize");
        }
    }

    fn repository_with_embedded_build(
        temp: &TempDir,
        runner: FakeQualificationToolRunner,
    ) -> QualificationRepository {
        QualificationRepository::new_for_test_with_embedded_build(
            temp.path().to_path_buf(),
            Box::new(runner),
            build_identity(),
        )
    }

    impl QualificationToolRunner for FakeQualificationToolRunner {
        fn run(&self, repo_root: &Path, args: &[String]) -> Result<Vec<u8>, String> {
            self.calls
                .lock()
                .expect("fake calls should not be poisoned")
                .push((repo_root.to_path_buf(), args.to_vec()));
            Ok(self
                .response
                .lock()
                .expect("fake response should not be poisoned")
                .clone())
        }
    }

    #[test]
    fn candidates_use_opaque_handles_and_survive_restart() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let json = json!({
            "capturedAt": "2026-08-23T12:00:00Z",
            "build": build_identity_json(),
        });

        let handle = repository
            .create_candidate(CandidateKind::TargetRegistration, &json, None)
            .expect("candidate should be stored");
        assert!(handle.starts_with(CANDIDATE_HANDLE_PREFIX));
        assert!(!handle.contains('/'));
        assert!(!handle.contains(".."));

        let candidate_path = temp
            .path()
            .join(".emuchef_runtime/qualification-candidates")
            .join(&handle)
            .join("candidate.json");
        assert!(candidate_path.is_file());
        let stored_entries = std::fs::read_dir(
            temp.path()
                .join(".emuchef_runtime/qualification-candidates"),
        )
        .expect("candidate root should be readable")
        .map(|entry| {
            entry
                .expect("candidate entry should be readable")
                .file_name()
        })
        .collect::<Vec<_>>();
        assert_eq!(stored_entries.len(), 1);
        assert_eq!(stored_entries[0].to_string_lossy(), handle);

        let reopened = QualificationRepository::new_for_test(
            repository.repo_root().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        assert_eq!(
            reopened
                .list_candidates()
                .expect("candidates should reload")
                .len(),
            1
        );
        assert_eq!(
            reopened
                .load_candidate(&handle)
                .expect("candidate should reload")
                .candidate_handle,
            handle
        );
        assert!(reopened.load_candidate("../../etc/passwd").is_err());
    }

    #[test]
    fn candidate_root_is_fixed_beneath_the_repository_root() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );

        assert_eq!(
            repository.candidate_root(),
            temp.path()
                .join(".emuchef_runtime/qualification-candidates")
        );
        assert!(production_repo_root()
            .join("tools/device-qualification.mjs")
            .is_file());
    }

    #[test]
    fn report_bytes_are_persisted_only_when_the_candidate_declares_the_report() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let report = b"{\"status\":\"completed\"}";
        let json = json!({
            "capturedAt": "2026-08-23T12:00:00Z",
            "build": build_identity_json(),
            "artifacts": [{
                "path": "execution-report.json",
                "sha256": hex::encode(sha2::Sha256::digest(report)),
            }],
        });

        let handle = repository
            .create_candidate(CandidateKind::QualificationRun, &json, Some(report))
            .expect("report candidate should be stored");
        let loaded = repository
            .load_candidate(&handle)
            .expect("report candidate should reload");
        assert_eq!(loaded.report_bytes.as_deref(), Some(report.as_slice()));

        let report_path = repository
            .candidate_root()
            .join(&handle)
            .join("execution-report.json");
        assert_eq!(
            std::fs::read(report_path).expect("report should exist"),
            report
        );
    }

    #[test]
    fn report_presence_must_match_candidate_metadata_on_reload() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let json = json!({
            "capturedAt": "2026-08-23T12:00:00Z",
            "build": build_identity_json(),
            "artifacts": [{
                "path": "execution-report.json",
                "sha256": hex::encode(sha2::Sha256::digest(b"report")),
            }],
        });
        let handle = repository
            .create_candidate(CandidateKind::QualificationRun, &json, Some(b"report"))
            .expect("report candidate should be stored");
        std::fs::remove_file(
            repository
                .candidate_root()
                .join(&handle)
                .join("execution-report.json"),
        )
        .expect("report should be removable for the regression setup");

        assert!(repository.load_candidate(&handle).is_err());
    }

    #[test]
    fn invalid_handles_are_rejected_before_any_path_is_joined() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository =
            QualificationRepository::new_for_test(temp.path().to_path_buf(), Box::new(runner));

        for handle in [
            "../../etc/passwd",
            "qualification-candidate-0123456789ABCDEF0123456789abcdef",
            "qualification-candidate-0123456789abcdef",
            "qualification-candidate-0123456789abcdef0123456789abcdef-extra",
        ] {
            assert!(repository.load_candidate(handle).is_err(), "{handle}");
            assert!(repository.discard_candidate(handle).is_err(), "{handle}");
            assert!(repository.register_target(handle).is_err(), "{handle}");
            assert!(repository.record_run(handle).is_err(), "{handle}");
        }
        assert!(calls.calls().is_empty());
    }

    #[test]
    fn node_operations_are_allowlisted_and_receive_only_opaque_handles() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);

        let target_handle = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": build_identity_json() }),
                None,
            )
            .expect("target candidate should be stored");
        let run_handle = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &json!({ "build": build_identity_json() }),
                None,
            )
            .expect("run candidate should be stored");

        repository
            .describe()
            .expect_err("the operation-shaped response is not a description");
        calls.set_response(json!({
            "operation": "register_target",
            "candidateHandle": target_handle,
            "candidateKind": "target_registration",
            "payload": {},
        }));
        repository
            .register_target(&target_handle)
            .expect("target registration should invoke the tool");
        calls.set_response(json!({
            "operation": "record_run",
            "candidateHandle": run_handle,
            "candidateKind": "qualification_run",
            "payload": {},
        }));
        repository
            .record_run(&run_handle)
            .expect_err("run recording must wait for a clean rebuild after registration");

        let calls = calls.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, repository.repo_root());
        assert_eq!(calls[0].1, vec!["--describe"]);
        assert_eq!(
            calls[1].1,
            vec!["--register-target".to_string(), target_handle]
        );
    }

    #[test]
    fn node_description_is_decoded_without_reimplementing_its_semantics() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let description = json!({
            "schemaVersion": 1,
            "runtimeContract": "real-execution-v1",
            "qualificationContract": 1,
            "build": build_identity_json(),
            "workflowCatalog": { "schemaVersion": 1, "workflows": [] },
            "deviceTargets": { "schemaVersion": 2, "targets": [] },
        });
        let runner = FakeQualificationToolRunner::with_response(description.clone());
        let repository =
            QualificationRepository::new_for_test(temp.path().to_path_buf(), Box::new(runner));

        let decoded = repository.describe().expect("description should decode");
        assert_eq!(decoded.schema_version, 1);
        assert_eq!(decoded.runtime_contract, "real-execution-v1");
        assert_eq!(decoded.workflow_catalog, description["workflowCatalog"]);
    }

    #[test]
    fn discarded_candidates_are_removed_from_the_fixed_root() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let handle = repository
            .create_candidate(CandidateKind::TargetRegistration, &json!({}), None)
            .expect("candidate should be stored");

        repository
            .discard_candidate(&handle)
            .expect("candidate should be discarded");
        assert!(repository.load_candidate(&handle).is_err());
        assert!(repository
            .list_candidates()
            .expect("candidate list should load")
            .is_empty());
    }

    #[test]
    fn listing_an_uninitialized_repository_has_no_side_effects() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );

        assert!(repository
            .list_candidates()
            .expect("empty candidate root should list successfully")
            .is_empty());
        assert!(!repository.candidate_root().exists());
    }

    #[test]
    fn stale_build_candidates_remain_inspectable_but_are_not_promotable() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let handle = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": {
                        "appVersion": "0.1.0",
                        "gitCommit": "2222222222222222222222222222222222222222",
                        "materialBuildDigest": format!("sha256:{}", "b".repeat(64)),
                        "realExecutionEnabled": true,
                        "qualificationContract": 1,
                    },
                    "target": { "model": "Pocket S2" },
                }),
                None,
            )
            .expect("stale candidate should be stored");

        let summaries = repository
            .list_candidates()
            .expect("stale candidate should remain inspectable");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].candidate_handle, handle);
        assert!(!summaries[0].promotable);
        assert!(summaries[0]
            .non_promotable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("build identity")));
        assert_eq!(summaries[0].target, Some(json!({ "model": "Pocket S2" })));
    }

    #[test]
    fn an_unlisted_report_file_is_rejected_during_reload() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let handle = repository
            .create_candidate(CandidateKind::TargetRegistration, &json!({}), None)
            .expect("candidate should be stored");
        std::fs::write(
            repository
                .candidate_root()
                .join(&handle)
                .join(EXECUTION_REPORT_FILE),
            b"unexpected report",
        )
        .expect("test report should be written");

        assert!(repository.load_candidate(&handle).is_err());
    }

    #[test]
    fn candidate_kind_is_bound_to_the_requested_tool_operation() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::with_response(json!({ "status": "ok" }));
        let calls = runner.clone();
        let repository =
            QualificationRepository::new_for_test(temp.path().to_path_buf(), Box::new(runner));
        let handle = repository
            .create_candidate(CandidateKind::TargetRegistration, &json!({}), None)
            .expect("candidate should be stored");

        assert!(repository.record_run(&handle).is_err());
        assert!(calls.calls().is_empty());
    }

    #[test]
    fn promotion_does_not_invoke_node_for_missing_or_stale_build_identity() {
        let missing_temp = TempDir::new().expect("temporary repository should be created");
        let missing_runner = FakeQualificationToolRunner::default();
        let missing_calls = missing_runner.clone();
        let missing_repository = QualificationRepository::new_for_test(
            missing_temp.path().to_path_buf(),
            Box::new(missing_runner),
        );
        let missing_handle = missing_repository
            .create_candidate(CandidateKind::TargetRegistration, &json!({}), None)
            .expect("candidate should be stored");
        assert!(missing_repository.register_target(&missing_handle).is_err());
        assert!(missing_calls.calls().is_empty());

        let stale_temp = TempDir::new().expect("temporary repository should be created");
        let stale_runner = FakeQualificationToolRunner::default();
        let stale_calls = stale_runner.clone();
        let stale_repository = repository_with_embedded_build(&stale_temp, stale_runner);
        let stale_handle = stale_repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({
                    "build": {
                        "appVersion": "0.1.0",
                        "gitCommit": "2".repeat(40),
                        "materialBuildDigest": format!("sha256:{}", "b".repeat(64)),
                        "realExecutionEnabled": true,
                        "qualificationContract": 1,
                    },
                }),
                None,
            )
            .expect("stale candidate should be stored");
        assert!(stale_repository.register_target(&stale_handle).is_err());
        assert!(stale_calls.calls().is_empty());
    }

    #[test]
    fn candidate_preview_recomputes_promotability_against_current_source_state() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let handle = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": build_identity_json(),
                    "target": {
                        "model": {
                            "value": "Pocket S2",
                            "source": "production_observation"
                        }
                    }
                }),
                None,
            )
            .expect("candidate should be stored");

        assert!(
            repository
                .load_candidate(&handle)
                .expect("candidate should load")
                .promotable
        );

        repository.set_source_state_for_test(QualificationSourceState {
            head: "2".repeat(40),
            tracked_worktree_clean: true,
        });
        let stale = repository
            .load_candidate(&handle)
            .expect("stale candidate should remain inspectable");
        assert!(!stale.promotable);
        assert!(stale
            .non_promotable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("source")));
        assert_eq!(stale.payload["target"]["model"]["value"], "Pocket S2");

        repository.set_source_state_for_test(QualificationSourceState {
            head: build_identity().git_commit,
            tracked_worktree_clean: false,
        });
        let dirty = repository
            .load_candidate(&handle)
            .expect("dirty candidate should remain inspectable");
        assert!(!dirty.promotable);
        assert!(dirty
            .non_promotable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("clean")));
    }

    #[test]
    fn successful_registration_blocks_lifecycle_until_a_clean_rebuild() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::with_response(json!({
            "operation": "register_target",
            "candidateHandle": "placeholder",
            "candidateKind": "target_registration",
            "payload": { "id": "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }
        }));
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let handle = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": build_identity_json() }),
                None,
            )
            .expect("candidate should be stored");
        calls.set_response(json!({
            "operation": "register_target",
            "candidateHandle": handle,
            "candidateKind": "target_registration",
            "payload": { "id": "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }
        }));

        repository
            .register_target(&handle)
            .expect("canonical registration should succeed");
        let calls_after_registration = calls.calls().len();

        let second_handle = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": build_identity_json() }),
                None,
            )
            .expect("second candidate should remain storable for inspection");
        assert!(repository.register_target(&second_handle).is_err());
        assert_eq!(calls.calls().len(), calls_after_registration);
        repository
            .discard_candidate(&second_handle)
            .expect("stale candidates must remain discardable");
        assert!(repository.load_candidate(&second_handle).is_err());
    }

    #[test]
    fn registration_discard_and_status_share_one_operation_gate() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = BlockingQualificationToolRunner::new();
        let status_started = runner.status_started.clone();
        let release_status = runner.release_status.clone();
        let repository = Arc::new(QualificationRepository::new_for_test_with_source_state(
            temp.path().to_path_buf(),
            Box::new(runner.clone()),
            build_identity(),
            QualificationSourceState {
                head: build_identity().git_commit,
                tracked_worktree_clean: true,
            },
        ));
        let status_handle = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": build_identity_json() }),
                None,
            )
            .expect("candidate should be stored");

        let registration_handle = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": build_identity_json() }),
                None,
            )
            .expect("registration candidate should be stored");

        let status_repository = Arc::clone(&repository);
        let status = thread::spawn(move || status_repository.describe_and_list_candidates());
        status_started.wait();

        let discard_done = Arc::new(AtomicBool::new(false));
        let discard_repository = Arc::clone(&repository);
        let discard_handle = status_handle.clone();
        let discard_done_for_thread = Arc::clone(&discard_done);
        let discard = thread::spawn(move || {
            let result = discard_repository.discard_candidate(&discard_handle);
            discard_done_for_thread.store(true, Ordering::Release);
            result
        });
        for _ in 0..128 {
            thread::yield_now();
        }
        assert!(repository
            .candidate_root()
            .join(&status_handle)
            .join(CANDIDATE_FILE)
            .exists());
        assert!(!discard_done.load(Ordering::Acquire));

        release_status.wait();
        let status = status
            .join()
            .expect("status thread should join")
            .expect("status should succeed");
        assert_eq!(status.candidates.len(), 2);
        discard
            .join()
            .expect("discard thread should join")
            .expect("discard should be serialized after status");

        let registration_repository = Arc::clone(&repository);
        let registration_handle_for_thread = registration_handle.clone();
        let registration = thread::spawn(move || {
            registration_repository.register_target(&registration_handle_for_thread)
        });
        runner.registration_started.wait();

        let registration_discard_done = Arc::new(AtomicBool::new(false));
        let registration_discard_repository = Arc::clone(&repository);
        let registration_discard_handle = registration_handle.clone();
        let registration_discard_done_for_thread = Arc::clone(&registration_discard_done);
        let registration_discard = thread::spawn(move || {
            let result =
                registration_discard_repository.discard_candidate(&registration_discard_handle);
            registration_discard_done_for_thread.store(true, Ordering::Release);
            result
        });
        for _ in 0..128 {
            thread::yield_now();
        }
        assert!(!registration_discard_done.load(Ordering::Acquire));

        runner.release_registration.wait();
        registration
            .join()
            .expect("registration thread should join")
            .expect("registration should succeed");
        registration_discard
            .join()
            .expect("registration discard thread should join")
            .expect("discard should be serialized after registration");
        let status = repository
            .describe_and_list_candidates()
            .expect("post-mutation status should succeed");
        assert!(!status.recordable);
        assert!(status.candidates.is_empty());
        assert!(!repository
            .candidate_root()
            .join(&registration_handle)
            .join(CANDIDATE_FILE)
            .exists());
    }

    #[derive(Clone)]
    struct BlockingQualificationToolRunner {
        calls: Arc<Mutex<Vec<RunnerCall>>>,
        status_started: Arc<Barrier>,
        release_status: Arc<Barrier>,
        registration_started: Arc<Barrier>,
        release_registration: Arc<Barrier>,
        status_blocked: Arc<AtomicBool>,
    }

    impl BlockingQualificationToolRunner {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                status_started: Arc::new(Barrier::new(2)),
                release_status: Arc::new(Barrier::new(2)),
                registration_started: Arc::new(Barrier::new(2)),
                release_registration: Arc::new(Barrier::new(2)),
                status_blocked: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl QualificationToolRunner for BlockingQualificationToolRunner {
        fn run(&self, repo_root: &Path, args: &[String]) -> Result<Vec<u8>, String> {
            self.calls
                .lock()
                .expect("blocking calls should not be poisoned")
                .push((repo_root.to_path_buf(), args.to_vec()));
            if args.first().map(String::as_str) == Some("--register-target") {
                self.registration_started.wait();
                self.release_registration.wait();
                return Ok(serde_json::to_vec(&json!({
                    "operation": "register_target",
                    "candidateHandle": args[1],
                    "candidateKind": "target_registration",
                    "payload": { "id": "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }
                }))
                .expect("registration response should serialize"));
            }
            if args.first().map(String::as_str) == Some("--describe")
                && !self.status_blocked.swap(true, Ordering::AcqRel)
            {
                self.status_started.wait();
                self.release_status.wait();
            }
            Ok(serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "runtimeContract": "real-execution-v1",
                "qualificationContract": 1,
                "build": build_identity_json(),
                "workflowCatalog": { "schemaVersion": 1, "workflows": [] },
                "deviceTargets": { "schemaVersion": 2, "targets": [] }
            }))
            .expect("description response should serialize"))
        }
    }

    #[test]
    fn strict_candidate_envelope_rejects_valid_json_with_the_wrong_shape() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let handle = "qualification-candidate-0123456789abcdef0123456789abcdef";
        let directory = repository.candidate_root().join(handle);
        std::fs::create_dir_all(&directory).expect("candidate directory should be created");
        std::fs::write(
            directory.join("candidate.json"),
            serde_json::to_vec(&json!({
                "candidateHandle": handle,
                "kind": "target_registration",
                "capturedAt": null,
                "build": null,
                "payload": {},
                "report": null,
                "unexpected": true,
            }))
            .expect("wrong-shaped envelope should serialize"),
        )
        .expect("wrong-shaped envelope should be written");

        assert!(repository.load_candidate(handle).is_err());
    }

    #[test]
    fn strict_operation_envelope_rejects_valid_json_with_the_wrong_shape() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let handle = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": build_identity_json() }),
                None,
            )
            .expect("candidate should be stored");

        calls.set_response(json!({
            "operation": "register_target",
            "candidateHandle": handle,
            "candidateKind": "target_registration",
        }));
        assert!(repository.register_target(&handle).is_err());
        assert_eq!(calls.calls().len(), 1);

        calls.set_response(json!({
            "operation": "register_target",
            "candidateHandle": handle,
            "candidateKind": "target_registration",
            "payload": {},
            "unexpected": true,
        }));
        assert!(repository.register_target(&handle).is_err());
        assert_eq!(calls.calls().len(), 2);
    }

    #[test]
    fn report_metadata_must_match_bytes_before_candidate_creation_writes_anything() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let result = repository.create_candidate(
            CandidateKind::QualificationRun,
            &json!({
                "artifacts": [{
                    "path": "execution-report.json",
                    "sha256": "0".repeat(64),
                }],
            }),
            Some(b"report"),
        );

        assert!(result.is_err());
        assert!(!repository.candidate_root().exists());
    }

    #[test]
    fn publishing_a_duplicate_handle_preserves_the_existing_candidate() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let root = temp
            .path()
            .join(".emuchef_runtime/qualification-candidates");
        std::fs::create_dir_all(&root).expect("candidate root should be created");
        let handle = "qualification-candidate-0123456789abcdef0123456789abcdef";
        let existing = root.join(handle);
        std::fs::create_dir(&existing).expect("existing candidate should be created");
        std::fs::write(existing.join("candidate.json"), b"existing")
            .expect("existing candidate should be written");
        let staging = root.join(".qualification-candidate-tmp-test");
        std::fs::create_dir(&staging).expect("staging directory should be created");
        std::fs::write(staging.join("candidate.json"), b"replacement")
            .expect("staging candidate should be written");

        assert!(publish_staged_candidate(&staging, &existing).is_err());
        assert_eq!(
            std::fs::read(existing.join("candidate.json"))
                .expect("existing candidate should remain"),
            b"existing"
        );
        assert!(!staging.exists());
    }

    #[test]
    fn competing_destination_after_absence_check_is_not_replaced_and_staging_is_cleaned() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let root = temp
            .path()
            .join(".emuchef_runtime/qualification-candidates");
        std::fs::create_dir_all(&root).expect("candidate root should be created");
        let handle = "qualification-candidate-fedcba9876543210fedcba9876543210";
        let destination = root.join(handle);
        let staging = root.join(".qualification-candidate-tmp-race");
        std::fs::create_dir(&staging).expect("staging directory should be created");
        std::fs::write(staging.join(CANDIDATE_FILE), b"replacement")
            .expect("staging candidate should be written");

        let result = publish_staged_candidate_with_hook(&staging, &destination, || {
            std::fs::create_dir(&destination).expect("competing destination should be created");
            std::fs::write(destination.join(CANDIDATE_FILE), b"competitor")
                .expect("competing candidate should be written");
        });

        assert!(matches!(result, Err(PublishCandidateError::Collision)));
        assert_eq!(
            std::fs::read(destination.join(CANDIDATE_FILE))
                .expect("competing candidate should remain readable"),
            b"competitor"
        );
        assert!(!staging.exists());
    }

    #[test]
    fn incomplete_candidate_staging_is_not_recovered_after_restart() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        ensure_candidate_root(repository.repo_root(), repository.candidate_root())
            .expect("candidate root should be created");
        let staging = repository
            .candidate_root()
            .join(format!("{CANDIDATE_STAGING_PREFIX}crash"));
        std::fs::create_dir(&staging).expect("partial staging directory should be created");
        std::fs::write(staging.join(CANDIDATE_FILE), b"partial")
            .expect("partial candidate should be written");

        assert!(repository
            .list_candidates()
            .expect("partial staging should be ignored")
            .is_empty());
        assert!(staging.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_candidate_root_components_are_rejected() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let outside = TempDir::new().expect("outside directory should be created");
        let runtime_root = temp.path().join(".emuchef_runtime");
        symlink(outside.path(), &runtime_root).expect("runtime root symlink should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository =
            QualificationRepository::new_for_test(temp.path().to_path_buf(), Box::new(runner));

        assert!(repository
            .create_candidate(CandidateKind::TargetRegistration, &json!({}), None)
            .is_err());
        assert!(repository.list_candidates().is_err());
        assert!(repository.describe().is_err());
        assert!(calls.calls().is_empty());
        assert!(outside
            .path()
            .read_dir()
            .expect("outside should be readable")
            .next()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_candidate_files_are_rejected_before_read_or_discard() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let outside = TempDir::new().expect("outside directory should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let handle = repository
            .create_candidate(CandidateKind::TargetRegistration, &json!({}), None)
            .expect("candidate should be stored");
        let candidate_path = repository
            .candidate_root()
            .join(&handle)
            .join("candidate.json");
        let outside_candidate = outside.path().join("candidate.json");
        std::fs::write(&outside_candidate, b"outside")
            .expect("outside candidate should be written");
        std::fs::remove_file(&candidate_path).expect("candidate should be removed");
        symlink(&outside_candidate, &candidate_path).expect("candidate symlink should be created");

        assert!(repository.load_candidate(&handle).is_err());
        assert!(repository.discard_candidate(&handle).is_err());
        assert!(candidate_path.exists());
        assert_eq!(
            std::fs::read(outside_candidate).expect("outside candidate should remain"),
            b"outside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_candidate_directories_are_rejected_during_listing() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let outside = TempDir::new().expect("outside directory should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let handle = repository
            .create_candidate(CandidateKind::TargetRegistration, &json!({}), None)
            .expect("candidate should be stored");
        let directory = repository.candidate_root().join(&handle);
        let outside_directory = outside.path().join(&handle);
        std::fs::rename(&directory, &outside_directory).expect("candidate should be moved");
        symlink(&outside_directory, &directory)
            .expect("candidate directory symlink should be created");

        assert!(repository.list_candidates().is_err());
        assert!(outside_directory.join(CANDIDATE_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_execution_reports_are_rejected_before_read_or_discard() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let outside = TempDir::new().expect("outside directory should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let report = b"report";
        let handle = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &json!({
                    "artifacts": [{
                        "path": EXECUTION_REPORT_FILE,
                        "sha256": hex::encode(sha2::Sha256::digest(report)),
                    }],
                }),
                Some(report),
            )
            .expect("candidate should be stored");
        let report_path = repository
            .candidate_root()
            .join(&handle)
            .join(EXECUTION_REPORT_FILE);
        let outside_report = outside.path().join(EXECUTION_REPORT_FILE);
        std::fs::write(&outside_report, report).expect("outside report should be written");
        std::fs::remove_file(&report_path).expect("report should be removed");
        symlink(&outside_report, &report_path).expect("report symlink should be created");

        assert!(repository.load_candidate(&handle).is_err());
        assert!(repository.discard_candidate(&handle).is_err());
        assert!(report_path.exists());
        assert_eq!(
            std::fs::read(outside_report).expect("outside report should remain"),
            report
        );
    }

    fn build_identity() -> QualificationBuildIdentity {
        serde_json::from_value(build_identity_json()).expect("test build identity should decode")
    }

    #[test]
    fn malformed_tool_output_is_rejected_after_the_allowlisted_call() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository =
            QualificationRepository::new_for_test(temp.path().to_path_buf(), Box::new(runner));

        assert!(repository.describe().is_err());
        assert_eq!(calls.calls().len(), 1);
        assert_eq!(calls.calls()[0].1, vec!["--describe"]);
        assert!(!allowlisted_tool_args(&[
            "--describe".to_string(),
            "extra".to_string()
        ]));
        assert!(!allowlisted_tool_args(&[
            "--record-run".to_string(),
            "not-a-handle".to_string()
        ]));
    }

    fn build_identity_json() -> Value {
        json!({
            "appVersion": "0.1.0",
            "gitCommit": "1111111111111111111111111111111111111111",
            "materialBuildDigest": format!("sha256:{}", "a".repeat(64)),
            "realExecutionEnabled": true,
            "qualificationContract": 1,
        })
    }
}
