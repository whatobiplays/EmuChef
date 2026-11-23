//! Fixed-root persistence and bounded access to the canonical qualification tool.
//!
//! Candidate files are untrusted handoff data. The checked-in execution-report
//! schema defines the report structure Rust validates during recovery and
//! candidate promotion. Rust checks terminal status, candidate-to-report
//! identity, and ROM Merge artifact bindings before exposing a candidate as
//! promotable. Node independently validates canonical evidence semantics and
//! owns the evidence matrix.

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
use crate::qualification_session::{PersistedQualificationSession, QualificationSession};

/// Prefix shared by every opaque candidate handle.
pub(crate) const CANDIDATE_HANDLE_PREFIX: &str = "qualification-candidate-";
const CANDIDATE_HANDLE_HEX_LENGTH: usize = 32;
const CANDIDATE_SCHEMA_VERSION: u64 = 2;
const CANDIDATE_DIRECTORY: &str = ".emuchef_runtime/qualification-candidates";
const QUALIFICATION_TOOL: &str = "tools/device-qualification.mjs";
const CANDIDATE_FILE: &str = "candidate.json";
const CANDIDATE_PUBLICATION_FILE: &str = ".qualification-candidate-publication.json";
const CANDIDATE_PUBLICATION_STAGING_DIR: &str = ".qualification-candidate-publication";
const CANDIDATE_PUBLICATION_STAGING_PREFIX: &str = ".qualification-candidate-publication-tmp-";
const CANDIDATE_PUBLICATION_SCHEMA_VERSION: u64 = 2;
const SESSION_FILE: &str = "session.json";
const SESSION_POISON_FILE: &str = "session-poison.json";
const SESSION_AUDIT_ONLY_FILE: &str = "session-audit-only";
const SESSION_AUDIT_ONLY_STAGING_PREFIX: &str = ".qualification-session-audit-only-tmp-";
const EXECUTION_REPORT_FILE: &str = "execution-report.json";
const ROM_MERGE_EVIDENCE_FILE: &str = "rom-merge-evidence.json";
const ROM_MERGE_APK_PIN_FILE: &str = "docs/testing/device-qualification/rom-merge-apk-pin.json";
const MAX_ROM_MERGE_EVIDENCE_BYTES: usize = 1024 * 1024;
/// Effective capability names the current restricted session can report.
const ROM_MERGE_UNPRIVILEGED_CAPABILITIES: [&str; 6] = [
    "adb_available",
    "apk_install",
    "shared_storage_write",
    "app_launch",
    "shell_command",
    "package_remove_for_user",
];
const SESSION_REPORT_FILE: &str = "session-terminal-report.json";
const CANDIDATE_STAGING_PREFIX: &str = ".qualification-candidate-tmp-";
const SESSION_STAGING_PREFIX: &str = ".qualification-session-tmp-";
const SESSION_POISON_STAGING_PREFIX: &str = ".qualification-session-poison-tmp-";
const SESSION_REPORT_STAGING_PREFIX: &str = ".qualification-session-report-tmp-";
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
    pub(crate) rom_merge_evidence_required: bool,
    pub(crate) rom_merge_evidence_attached: bool,
}

/// Metadata for the one optional report file owned by a candidate directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QualificationReportMetadata {
    pub(crate) path: String,
    pub(crate) byte_length: u64,
    pub(crate) sha256: String,
}

/// Rust-owned metadata for the bounded ROM Merge attachment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RomMergeEvidenceMetadata {
    path: String,
    byte_length: u64,
    sha256: String,
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
    #[serde(default)]
    rom_merge_evidence: Option<RomMergeEvidenceMetadata>,
}

/// Durable intent to publish the candidate, report, and optional ROM Merge
/// artifact as one recoverable update. Staged files remain until every final
/// path has been synced.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CandidatePublicationTransaction {
    schema_version: u64,
    report_present: bool,
    #[serde(default)]
    rom_merge_evidence_present: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeManifestEntry {
    relative_path: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeSourceSnapshot {
    source_identity: String,
    entries: Vec<RomMergeManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeDestinationSnapshot {
    path: String,
    setup_location: String,
    volume_identity: String,
    before: Vec<RomMergeManifestEntry>,
    after: Vec<RomMergeManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeTerminalOperation {
    recipe_id: String,
    step_id: String,
    outcome: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeSentinel {
    relative_path: String,
    expected_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeRetroArchIdentity {
    package_id: String,
    installed_version: String,
    pinned_source: String,
    pinned_version: String,
    installed_apk_sha256: String,
    pinned_artifact_sha256: String,
}

/// Strict, candidate-bound evidence for the reviewed ROM Merge operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeEvidence {
    schema_version: u64,
    candidate_handle: String,
    session_id: String,
    execution_report_sha256: String,
    device_target_id: String,
    device_profile_id: String,
    android_api: u64,
    firmware_build: String,
    abi_soc_class: String,
    connection_type: String,
    observed_root_state: String,
    build_identity: QualificationBuildIdentity,
    reviewed_plan_id: String,
    reviewed_plan_digest: String,
    terminal_operation: RomMergeTerminalOperation,
    copy_policy: String,
    strategy_digest: String,
    source_snapshot: RomMergeSourceSnapshot,
    destination: RomMergeDestinationSnapshot,
    sentinel: RomMergeSentinel,
    retro_arch: RomMergeRetroArchIdentity,
    root_authority_used: bool,
    oem_compiler_used: bool,
    oem_stage_used: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeApkPinManifest {
    schema_version: u64,
    retroarch: Option<RomMergeApkPin>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomMergeApkPin {
    package_id: String,
    source: String,
    version: String,
    sha256: String,
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
    pub(crate) rom_merge_evidence: Option<RomMergeEvidenceMetadata>,
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

/// Content identity for an authored recipe captured when a trusted session
/// starts and carried with that qualification session across restart.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AuthoredRecipeDigest {
    pub(crate) id: String,
    pub(crate) sha256: String,
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
    begin_gate: Mutex<()>,
    lifecycle_dirty: AtomicBool,
    #[cfg(test)]
    source_state_override: Option<std::sync::Arc<Mutex<QualificationSourceState>>>,
    #[cfg(test)]
    source_state_read_hook: Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    authored_recipe_digest_capture_hook: Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    fail_next_finalize: AtomicBool,
    #[cfg(test)]
    fail_next_audit_only_marker: AtomicBool,
    #[cfg(test)]
    fail_after_candidate_envelope_publication: AtomicBool,
    #[cfg(test)]
    fail_candidate_publication_staging_cleanup: AtomicBool,
}

/// Lazily resolves the trusted qualification repository only when the mode is
/// enabled and a valid source checkout is available.
pub struct QualificationRepositoryProvider {
    repository: OnceLock<Option<QualificationRepository>>,
    #[cfg(test)]
    test_mode_build: Option<QualificationBuildIdentity>,
}

impl Default for QualificationRepositoryProvider {
    fn default() -> Self {
        Self {
            repository: OnceLock::new(),
            #[cfg(test)]
            test_mode_build: None,
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
        let test_mode_build = repository.embedded_build_identity.clone();
        let _ = provider.repository.set(Some(repository));
        Self {
            repository: provider.repository,
            test_mode_build,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_mode_build(&self) -> Option<&QualificationBuildIdentity> {
        self.test_mode_build.as_ref()
    }
}

impl QualificationRepository {
    /// Builds the qualification repository from the compile-time trusted root
    /// used by debug qualification runs. Release bundles do not contain the
    /// source checkout and therefore never resolve this repository.
    ///
    /// Packaged or ordinary builds may not contain the development source
    /// checkout. In that case construction returns `None` without resolving a
    /// path or invoking Node.
    pub fn production() -> Option<Self> {
        #[cfg(not(debug_assertions))]
        {
            None
        }
        #[cfg(debug_assertions)]
        {
            let repo_root = production_repo_root();
            validate_repository_root(&repo_root).ok()?;
            qualification_tool_path(&repo_root).ok()?;
            Some(Self::with_root(
                repo_root,
                Box::new(ProcessQualificationToolRunner),
            ))
        }
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
            begin_gate: Mutex::new(()),
            lifecycle_dirty: AtomicBool::new(false),
            #[cfg(test)]
            source_state_override: None,
            #[cfg(test)]
            source_state_read_hook: Mutex::new(None),
            #[cfg(test)]
            authored_recipe_digest_capture_hook: Mutex::new(None),
            #[cfg(test)]
            fail_next_finalize: AtomicBool::new(false),
            #[cfg(test)]
            fail_next_audit_only_marker: AtomicBool::new(false),
            #[cfg(test)]
            fail_after_candidate_envelope_publication: AtomicBool::new(false),
            #[cfg(test)]
            fail_candidate_publication_staging_cleanup: AtomicBool::new(false),
        }
    }

    fn lock_operation(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.operation_gate
            .lock()
            .map_err(|_| "qualification repository operation is unavailable".to_string())
    }

    /// Serialize qualification starts before any device capture or observation
    /// side effect can occur.
    pub(crate) fn lock_begin(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.begin_gate
            .lock()
            .map_err(|_| "qualification session start is unavailable".to_string())
    }

    /// Capture canonical authored recipe content identities at a trusted
    /// session or review boundary. Candidate finalization uses these retained
    /// digests instead of re-reading mutable working-tree files.
    pub(crate) fn capture_authored_recipe_digests(
        &self,
        recipe_ids: &[String],
    ) -> Result<Vec<AuthoredRecipeDigest>, String> {
        let mut digests = Vec::with_capacity(recipe_ids.len());
        for id in recipe_ids {
            if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
                return Err("qualification authored source identity is invalid".to_string());
            }
            let path = self
                .repo_root
                .join("authored/recipes")
                .join(format!("{id}.yaml"));
            let bytes = fs::read(path)
                .map_err(|_| "qualification authored source could not be read".to_string())?;
            #[cfg(test)]
            if let Some(hook) = self
                .authored_recipe_digest_capture_hook
                .lock()
                .map_err(|_| "qualification authored source is unavailable".to_string())?
                .clone()
            {
                hook();
            }
            digests.push(AuthoredRecipeDigest {
                id: id.clone(),
                sha256: hex::encode(Sha256::digest(bytes)),
            });
        }
        Ok(digests)
    }

    /// Capture session-start recipe identities only while the checkout remains
    /// the clean source for this embedded qualification build. The second
    /// source check catches filesystem changes that occur during the reads.
    pub(crate) fn capture_recordable_authored_recipe_digests(
        &self,
        recipe_ids: &[String],
    ) -> Result<Vec<AuthoredRecipeDigest>, String> {
        let _operation = self.lock_operation()?;
        self.ensure_recordable_unlocked()?;
        let digests = self.capture_authored_recipe_digests(recipe_ids);
        self.ensure_recordable_unlocked()?;
        digests
    }

    /// Revalidate the canonical source bound to a session before valid evidence
    /// is materialized. A dirty, changed, or unavailable checkout leaves the
    /// completed session pending so a later source change cannot seal a
    /// mismatched fingerprint into a candidate.
    pub(crate) fn authored_recipe_digests_match(&self, expected: &[AuthoredRecipeDigest]) -> bool {
        let Ok(_operation) = self.lock_operation() else {
            return false;
        };
        if self.ensure_recordable_unlocked().is_err() {
            return false;
        }
        let recipe_ids = expected
            .iter()
            .map(|digest| digest.id.clone())
            .collect::<Vec<_>>();
        let current = self.capture_authored_recipe_digests(&recipe_ids);
        let source_remains_recordable = self.ensure_recordable_unlocked().is_ok();
        source_remains_recordable && current.is_ok_and(|current| current == expected)
    }

    fn current_source_state(&self) -> Result<QualificationSourceState, String> {
        #[cfg(test)]
        if let Some(hook) = self
            .source_state_read_hook
            .lock()
            .map_err(|_| "qualification source state is unavailable".to_string())?
            .clone()
        {
            hook();
        }
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
    pub(crate) fn set_source_state_for_test(&self, state: QualificationSourceState) {
        if let Some(source_state) = &self.source_state_override {
            *source_state
                .lock()
                .expect("test source state should not be poisoned") = state;
        }
    }

    #[cfg(test)]
    pub(crate) fn source_state_handle_for_test(
        &self,
    ) -> Option<std::sync::Arc<Mutex<QualificationSourceState>>> {
        self.source_state_override.clone()
    }

    #[cfg(test)]
    pub(crate) fn set_source_state_read_hook_for_test(
        &self,
        hook: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) {
        *self
            .source_state_read_hook
            .lock()
            .expect("source-state read hook should not be poisoned") = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_authored_recipe_digest_capture_hook_for_test(
        &self,
        hook: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) {
        *self
            .authored_recipe_digest_capture_hook
            .lock()
            .expect("authored recipe digest hook should not be poisoned") = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_finalize_for_test(&self) {
        self.fail_next_finalize.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_audit_only_marker_for_test(&self) {
        self.fail_next_audit_only_marker
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn fail_after_candidate_envelope_publication_for_test(&self) {
        self.fail_after_candidate_envelope_publication
            .store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn fail_candidate_publication_staging_cleanup_for_test(&self) {
        self.fail_candidate_publication_staging_cleanup
            .store(true, Ordering::SeqCst);
    }

    /// Rechecks the trusted source/worktree lifecycle before capture begins.
    pub(crate) fn require_recordable(&self) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        self.ensure_recordable_unlocked()
    }

    /// Returns the normalized trusted repository root used for every operation.
    #[cfg(test)]
    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Returns the fixed ignored candidate root beneath the repository.
    #[cfg(test)]
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
                        rom_merge_evidence: None,
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

    /// Persists strict Rust-owned session state beside, but separately from,
    /// the Node-owned candidate payload. A save is immediately visible after a
    /// process restart and never changes canonical candidate data.
    pub(crate) fn save_session(
        &self,
        candidate_handle: &str,
        persisted: &PersistedQualificationSession,
    ) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        match fs::symlink_metadata(directory.join(SESSION_POISON_FILE)) {
            Ok(_) => return Err("qualification session has durable fail-closed state".to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("qualification poison marker could not be inspected".to_string()),
        }
        if persisted.candidate_handle != candidate_handle {
            return Err("qualification session candidate binding is inconsistent".to_string());
        }
        let _ = QualificationSession::from_persisted(persisted.clone())?;
        let mut bytes = serde_json::to_vec_pretty(persisted)
            .map_err(|_| "qualification session could not be serialized".to_string())?;
        bytes.push(b'\n');
        write_synced_replaced_file(&directory, SESSION_FILE, SESSION_STAGING_PREFIX, &bytes)
    }

    /// Loads and validates strict session state associated with a candidate.
    pub(crate) fn load_session(
        &self,
        candidate_handle: &str,
    ) -> Result<PersistedQualificationSession, String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        let bytes = read_regular_file(&directory.join(SESSION_FILE), "qualification session")?;
        let persisted: PersistedQualificationSession = serde_json::from_slice(&bytes)
            .map_err(|_| "qualification session JSON is invalid".to_string())?;
        if persisted.candidate_handle != candidate_handle {
            return Err("qualification session candidate binding is inconsistent".to_string());
        }
        let _ = QualificationSession::from_persisted(persisted.clone())?;
        Ok(persisted)
    }

    /// Reads the raw session document so recovery can tell a session written by
    /// an older build apart from missing or corrupt state. Version checks stay
    /// with the session owner; this boundary only returns the stored JSON.
    pub(crate) fn load_session_json(&self, candidate_handle: &str) -> Result<Value, String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        let bytes = read_regular_file(&directory.join(SESSION_FILE), "qualification session")?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "qualification session JSON is invalid".to_string())?;
        if value.get("candidateHandle").and_then(Value::as_str) != Some(candidate_handle) {
            return Err("qualification session candidate binding is inconsistent".to_string());
        }
        Ok(value)
    }

    /// Removes authoritative session state once an attempt has closed. The
    /// immutable candidate and any recorded evidence remain untouched.
    pub(crate) fn remove_session(&self, candidate_handle: &str) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        remove_optional_regular_file(&directory, SESSION_FILE, "qualification session")?;
        sync_directory(&directory)
    }

    /// Durably record that a session transition failed. Recovery must never
    /// resume a session while this marker remains in its candidate directory.
    pub(crate) fn mark_session_poisoned(&self, candidate_handle: &str) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        write_synced_replaced_file(
            &directory,
            SESSION_POISON_FILE,
            SESSION_POISON_STAGING_PREFIX,
            b"Qualification session evidence could not be retained.\n",
        )?;
        sync_directory(&directory)
    }

    /// Durably prevent a recovery-invalid attempt from being promoted. This
    /// candidate-local marker remains after resumable session files are removed.
    pub(crate) fn mark_candidate_audit_only(&self, candidate_handle: &str) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        #[cfg(test)]
        if self
            .fail_next_audit_only_marker
            .swap(false, Ordering::SeqCst)
        {
            return Err("qualification audit-only marker could not be retained".to_string());
        }
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        write_synced_replaced_file(
            &directory,
            SESSION_AUDIT_ONLY_FILE,
            SESSION_AUDIT_ONLY_STAGING_PREFIX,
            b"Recovered invalid qualification attempts cannot be promoted.\n",
        )?;
        sync_directory(&directory)
    }

    /// Report whether durable fail-closed state exists for one session.
    pub(crate) fn session_is_poisoned(&self, candidate_handle: &str) -> Result<bool, String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        match fs::symlink_metadata(directory.join(SESSION_POISON_FILE)) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
            Ok(_) => Err("qualification poison marker is not a regular file".to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err("qualification poison marker could not be inspected".to_string()),
        }
    }

    /// Persists the authoritative terminal execution report captured by the
    /// product terminal transition, before the immutable candidate exists.
    /// The report is stored beside the session and published into the candidate
    /// artifact set when the candidate is materialized.
    pub(crate) fn save_session_report(
        &self,
        candidate_handle: &str,
        report_bytes: &[u8],
    ) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        write_synced_replaced_file(
            &directory,
            SESSION_REPORT_FILE,
            SESSION_REPORT_STAGING_PREFIX,
            report_bytes,
        )
    }

    /// Loads the retained terminal execution report, when one was captured.
    pub(crate) fn load_session_report(
        &self,
        candidate_handle: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        let path = directory.join(SESSION_REPORT_FILE);
        match fs::symlink_metadata(&path) {
            Ok(_) => read_regular_file(&path, "qualification terminal report").map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("qualification terminal report could not be inspected".to_string()),
        }
    }

    /// Removes a retained terminal execution report that will never be
    /// published by an immutable candidate.
    pub(crate) fn remove_session_report(&self, candidate_handle: &str) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        remove_optional_regular_file(
            &directory,
            SESSION_REPORT_FILE,
            "qualification terminal report",
        )?;
        sync_directory(&directory)
    }

    /// Replaces a provisional candidate payload with the terminal run
    /// candidate while retaining its opaque directory and resumable session.
    pub(crate) fn finalize_candidate(
        &self,
        candidate_handle: &str,
        kind: CandidateKind,
        payload: &Value,
        report_bytes: Option<&[u8]>,
    ) -> Result<StoredQualificationCandidate, String> {
        let _operation = self.lock_operation()?;
        #[cfg(test)]
        if self.fail_next_finalize.swap(false, Ordering::SeqCst) {
            return Err("injected qualification candidate finalization failure".to_string());
        }
        let directory = self.candidate_directory_unlocked(candidate_handle)?;
        validate_candidate_files(&directory)?;
        let existing = self.load_candidate_unlocked(candidate_handle)?;
        if existing.kind != kind {
            return Err("qualification candidate kind does not match the session".to_string());
        }
        let mut candidate = payload
            .as_object()
            .cloned()
            .ok_or_else(|| "qualification candidate must be a JSON object".to_string())?;
        candidate.insert(
            "candidateSchemaVersion".to_string(),
            Value::from(CANDIDATE_SCHEMA_VERSION),
        );
        candidate.insert("candidateId".to_string(), Value::from(candidate_handle));
        candidate.insert("kind".to_string(), Value::from(kind.as_str()));
        let candidate = Value::Object(candidate);
        let captured_at = optional_string_field(&candidate, "capturedAt")?;
        let build = candidate_build_identity(&candidate)?;
        let report = report_metadata_for_candidate(&candidate, report_bytes)?;
        let rom_merge_evidence_bytes =
            load_rom_merge_evidence_bytes(&directory, existing.rom_merge_evidence.as_ref())?;
        let envelope = CandidateFileEnvelope {
            candidate_handle: candidate_handle.to_string(),
            kind,
            captured_at,
            build,
            payload: candidate,
            report,
            rom_merge_evidence: existing.rom_merge_evidence.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&envelope)
            .map_err(|_| "qualification candidate could not be serialized".to_string())?;
        bytes.push(b'\n');
        stage_candidate_publication(
            &directory,
            &bytes,
            report_bytes,
            rom_merge_evidence_bytes.as_deref(),
        )?;
        #[cfg(test)]
        let fail_after_envelope = self
            .fail_after_candidate_envelope_publication
            .swap(false, Ordering::SeqCst);
        #[cfg(not(test))]
        let fail_after_envelope = false;
        #[cfg(test)]
        let fail_staging_cleanup = self
            .fail_candidate_publication_staging_cleanup
            .swap(false, Ordering::SeqCst);
        #[cfg(not(test))]
        let fail_staging_cleanup = false;
        publish_candidate_publication(&directory, fail_after_envelope, fail_staging_cleanup)?;
        self.load_candidate_unlocked(candidate_handle)
    }

    fn load_candidate_unlocked(
        &self,
        handle: &str,
    ) -> Result<StoredQualificationCandidate, String> {
        let directory = self.candidate_directory_unlocked(handle)?;
        recover_candidate_publication(&directory)?;
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
        let rom_merge_evidence_bytes =
            load_rom_merge_evidence_bytes(&directory, envelope.rom_merge_evidence.as_ref())?;
        let rom_merge_evidence = rom_merge_evidence_bytes
            .as_deref()
            .map(|bytes| {
                serde_json::from_slice::<RomMergeEvidence>(bytes)
                    .map_err(|_| "ROM Merge evidence is malformed".to_string())
            })
            .transpose()?;
        let trusted_target_profile = registered_target_profile_id(
            &self.repo_root,
            envelope
                .payload
                .get("deviceTargetId")
                .and_then(Value::as_str),
        );
        let report_status = report_bytes
            .as_deref()
            .and_then(validated_execution_report_v2_status);
        let report_has_plan_identity = report_bytes
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            .is_some_and(|report| report_has_plan_identity(&report));
        let report_claim_matches_candidate = report_bytes.as_deref().is_none_or(|bytes| {
            execution_report_claim_matches_candidate(
                bytes,
                &envelope.payload,
                trusted_target_profile.as_deref(),
            )
        });
        let report_is_invalid = report_bytes.is_some() && report_status.is_none();
        let valid_run_report_is_ineligible = envelope.kind == CandidateKind::QualificationRun
            && envelope.payload.get("runValidity").and_then(Value::as_str) == Some("valid")
            && !report_bytes.as_deref().is_some_and(|bytes| {
                valid_real_plan_bound_execution_report(bytes)
                    && report_has_complete_restricted_operation_coverage(bytes)
            });
        let passing_rom_library_sync = is_passing_rom_library_sync_candidate(&envelope.payload);
        let rom_library_target_matches_fingerprint = !passing_rom_library_sync
            || candidate_profile_matches_registered_target(
                &envelope.payload,
                trusted_target_profile.as_deref(),
            );
        let passing_konkr_rom_library_sync = is_passing_konkr_rom_library_sync_candidate(
            &envelope.payload,
            trusted_target_profile.as_deref(),
        );
        if rom_merge_evidence.is_some() && !passing_konkr_rom_library_sync {
            return Err(
                "ROM Merge evidence is only supported for a passing KONKR ROM Merge candidate"
                    .to_string(),
            );
        }
        let rom_merge_evidence_error = if passing_konkr_rom_library_sync {
            match (rom_merge_evidence.as_ref(), report_bytes.as_deref()) {
                (Some(evidence), Some(report_bytes)) => validate_rom_merge_evidence(
                    &envelope.payload,
                    report_bytes,
                    evidence,
                    &self.repo_root,
                )
                .err(),
                (Some(_), None) => {
                    Some("ROM Merge evidence requires a retained execution report".to_string())
                }
                (None, _) => {
                    Some("ROM Merge evidence is required for a passing KONKR candidate".to_string())
                }
            }
        } else {
            None
        };
        let poisoned = match fs::symlink_metadata(directory.join(SESSION_POISON_FILE)) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => true,
            Ok(_) => return Err("qualification poison marker is not a regular file".to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(_) => return Err("qualification poison marker could not be inspected".to_string()),
        };
        let audit_only = match fs::symlink_metadata(directory.join(SESSION_AUDIT_ONLY_FILE)) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => true,
            Ok(_) => {
                return Err("qualification audit-only marker is not a regular file".to_string())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(_) => {
                return Err("qualification audit-only marker could not be inspected".to_string())
            }
        };
        let (promotable, non_promotable_reason) = if poisoned {
            (
                false,
                Some("qualification session persistence failed and cannot be promoted".to_string()),
            )
        } else if audit_only {
            (
                false,
                Some("recovered invalid qualification attempts cannot be promoted".to_string()),
            )
        } else if envelope.kind == CandidateKind::QualificationRun
            && !is_terminal_run_candidate(&envelope.payload)
        {
            (
                false,
                Some(
                    "qualification run candidate has not reached terminal finalization".to_string(),
                ),
            )
        } else if envelope.kind == CandidateKind::QualificationRun && report_is_invalid {
            (
                false,
                Some(
                    "qualification execution report does not satisfy schema version 2".to_string(),
                ),
            )
        } else if envelope.kind == CandidateKind::QualificationRun
            && report_bytes.is_some()
            && !report_has_plan_identity
        {
            (
                false,
                Some(
                    "qualification execution report does not contain reviewed plan identity"
                        .to_string(),
                ),
            )
        } else if envelope.kind == CandidateKind::QualificationRun
            && !report_claim_matches_candidate
        {
            (
                false,
                Some(
                    "qualification report identity does not match the retained candidate target"
                        .to_string(),
                ),
            )
        } else if valid_run_report_is_ineligible {
            (
                false,
                Some(
                    "valid qualification evidence requires a real plan-bound report with complete restricted operation coverage"
                        .to_string(),
                ),
            )
        } else if !rom_library_target_matches_fingerprint {
            (
                false,
                Some(
                    "passing ROM-library-sync candidate does not match a unique registered target"
                        .to_string(),
                ),
            )
        } else if let Some(reason) = rom_merge_evidence_error {
            (false, Some(reason))
        } else {
            self.candidate_promotion_status(envelope.build.as_ref())
        };

        Ok(StoredQualificationCandidate {
            candidate_handle: handle.to_string(),
            kind: envelope.kind,
            captured_at: envelope.captured_at,
            build: envelope.build,
            payload: envelope.payload,
            report: envelope.report,
            rom_merge_evidence: envelope.rom_merge_evidence,
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

    /// Remove one explicitly selected untrusted candidate without parsing its
    /// contents or replaying its publication transaction. The handle and both
    /// fixed-root and candidate-directory identities are still validated, so
    /// this path can remove corrupt state without following a substituted path.
    pub(crate) fn discard_untrusted_candidate(&self, handle: &str) -> Result<(), String> {
        let _operation = self.lock_operation()?;
        let directory = self.candidate_directory_unlocked(handle)?;
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|_| "qualification candidate could not be inspected".to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("qualification candidate directory is a symlink".to_string());
        }
        if !metadata.is_dir() {
            return Err("qualification candidate directory is invalid".to_string());
        }
        // Re-resolve the validated handle immediately before deletion. This
        // rejects a directory replaced after the first path validation.
        if self.candidate_directory_unlocked(handle)? != directory {
            return Err("qualification candidate directory changed".to_string());
        }
        let current = fs::symlink_metadata(&directory)
            .map_err(|_| "qualification candidate could not be inspected".to_string())?;
        if current.file_type().is_symlink() {
            return Err("qualification candidate directory is a symlink".to_string());
        }
        if !current.is_dir() {
            return Err("qualification candidate directory is invalid".to_string());
        }
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

    /// Attaches one validated ROM Merge artifact to its immutable attempt.
    pub fn attach_rom_merge_evidence(
        &self,
        handle: &str,
        evidence_bytes: &[u8],
    ) -> Result<QualificationCandidateSummary, String> {
        let _operation = self.lock_operation()?;
        if evidence_bytes.is_empty() || evidence_bytes.len() > MAX_ROM_MERGE_EVIDENCE_BYTES {
            return Err("ROM Merge evidence size is outside the supported bound".to_string());
        }
        let directory = self.candidate_directory_unlocked(handle)?;
        validate_candidate_files(&directory)?;
        let stored = self.load_candidate_unlocked(handle)?;
        if stored.kind != CandidateKind::QualificationRun {
            return Err("ROM Merge evidence requires a qualification-run candidate".to_string());
        }
        let trusted_target_profile = registered_target_profile_id(
            &self.repo_root,
            stored.payload.get("deviceTargetId").and_then(Value::as_str),
        );
        if !is_passing_konkr_rom_library_sync_candidate(
            &stored.payload,
            trusted_target_profile.as_deref(),
        ) {
            return Err(
                "ROM Merge evidence requires a passing KONKR ROM Merge candidate".to_string(),
            );
        }
        if stored.rom_merge_evidence.is_some() {
            return Err("ROM Merge evidence is already attached to this candidate".to_string());
        }

        let candidate_bytes =
            read_regular_file(&directory.join(CANDIDATE_FILE), "qualification candidate")?;
        let mut envelope = decode_candidate_envelope(&candidate_bytes)?;
        if envelope.rom_merge_evidence.is_some() {
            return Err("ROM Merge evidence is already attached to this candidate".to_string());
        }
        let report_metadata = envelope
            .report
            .as_ref()
            .ok_or_else(|| "ROM Merge evidence requires a retained execution report".to_string())?;
        let report_bytes = read_regular_file(
            &directory.join(EXECUTION_REPORT_FILE),
            "qualification execution report",
        )?;
        load_report_bytes(
            &directory.join(EXECUTION_REPORT_FILE),
            Some(report_metadata),
            Some(&report_metadata.sha256),
        )?;
        let evidence: RomMergeEvidence = serde_json::from_slice(evidence_bytes)
            .map_err(|_| "ROM Merge evidence is malformed".to_string())?;
        validate_rom_merge_evidence(&envelope.payload, &report_bytes, &evidence, &self.repo_root)?;
        envelope.rom_merge_evidence = Some(RomMergeEvidenceMetadata {
            path: ROM_MERGE_EVIDENCE_FILE.to_string(),
            byte_length: evidence_bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(evidence_bytes)),
        });
        let mut updated_candidate_bytes = serde_json::to_vec_pretty(&envelope)
            .map_err(|_| "qualification candidate could not be serialized".to_string())?;
        updated_candidate_bytes.push(b'\n');
        stage_candidate_publication(
            &directory,
            &updated_candidate_bytes,
            Some(&report_bytes),
            Some(evidence_bytes),
        )?;
        #[cfg(test)]
        let fail_after_envelope = self
            .fail_after_candidate_envelope_publication
            .swap(false, Ordering::SeqCst);
        #[cfg(not(test))]
        let fail_after_envelope = false;
        #[cfg(test)]
        let fail_staging_cleanup = self
            .fail_candidate_publication_staging_cleanup
            .swap(false, Ordering::SeqCst);
        #[cfg(not(test))]
        let fail_staging_cleanup = false;
        publish_candidate_publication(&directory, fail_after_envelope, fail_staging_cleanup)?;
        self.load_candidate_unlocked(handle)
            .map(|candidate| candidate.summary())
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
    pub(crate) fn summary(&self) -> QualificationCandidateSummary {
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
            rom_merge_evidence_required: is_passing_konkr_rom_library_sync_candidate(
                &self.payload,
                Some("konkr.pocket_advance"),
            ),
            rom_merge_evidence_attached: self.rom_merge_evidence.is_some(),
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
#[cfg(debug_assertions)]
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

fn is_terminal_run_candidate(candidate: &Value) -> bool {
    matches!(
        (
            candidate.get("runValidity").and_then(Value::as_str),
            candidate
                .get("qualificationOutcome")
                .and_then(Value::as_str),
        ),
        (Some("invalid"), Some("not_observed")) | (Some("valid"), Some("passed" | "failed"))
    )
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
    const LEGACY_ENVELOPE_FIELDS: [&str; 6] = [
        "candidateHandle",
        "kind",
        "capturedAt",
        "build",
        "payload",
        "report",
    ];
    const ENVELOPE_FIELDS: [&str; 7] = [
        "candidateHandle",
        "kind",
        "capturedAt",
        "build",
        "payload",
        "report",
        "romMergeEvidence",
    ];
    let legacy_shape = object.len() == LEGACY_ENVELOPE_FIELDS.len()
        && LEGACY_ENVELOPE_FIELDS
            .iter()
            .all(|field| object.contains_key(*field));
    let current_shape = object.len() == ENVELOPE_FIELDS.len()
        && ENVELOPE_FIELDS
            .iter()
            .all(|field| object.contains_key(*field));
    if !legacy_shape && !current_shape {
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
    recover_candidate_publication(directory)?;
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
    validate_regular_file_path(
        &directory.join(ROM_MERGE_EVIDENCE_FILE),
        false,
        "ROM Merge evidence",
    )?;
    validate_regular_file_path(
        &directory.join(SESSION_AUDIT_ONLY_FILE),
        false,
        "qualification audit-only marker",
    )?;
    Ok(())
}

/// Stage the complete candidate/report/artifact set before recording durable
/// publication intent. A reader can replay the staged files after interruption.
fn stage_candidate_publication(
    directory: &Path,
    candidate_bytes: &[u8],
    report_bytes: Option<&[u8]>,
    rom_merge_evidence_bytes: Option<&[u8]>,
) -> Result<(), String> {
    remove_candidate_publication_staging(directory)?;
    let staging = directory.join(CANDIDATE_PUBLICATION_STAGING_DIR);
    fs::create_dir(&staging)
        .map_err(|_| "qualification candidate publication could not be staged".to_string())?;

    let result = (|| {
        write_synced_new_file(&staging.join(CANDIDATE_FILE), candidate_bytes)?;
        if let Some(report_bytes) = report_bytes {
            write_synced_new_file(&staging.join(EXECUTION_REPORT_FILE), report_bytes)?;
        }
        if let Some(evidence_bytes) = rom_merge_evidence_bytes {
            write_synced_new_file(&staging.join(ROM_MERGE_EVIDENCE_FILE), evidence_bytes)?;
        }
        sync_directory(&staging)?;
        sync_directory(directory)?;

        let transaction = CandidatePublicationTransaction {
            schema_version: CANDIDATE_PUBLICATION_SCHEMA_VERSION,
            report_present: report_bytes.is_some(),
            rom_merge_evidence_present: rom_merge_evidence_bytes.is_some(),
        };
        let mut transaction_bytes = serde_json::to_vec_pretty(&transaction).map_err(|_| {
            "qualification candidate publication could not be serialized".to_string()
        })?;
        transaction_bytes.push(b'\n');
        write_synced_replaced_file(
            directory,
            CANDIDATE_PUBLICATION_FILE,
            CANDIDATE_PUBLICATION_STAGING_PREFIX,
            &transaction_bytes,
        )
    })();

    if result.is_err()
        && matches!(
            fs::symlink_metadata(directory.join(CANDIDATE_PUBLICATION_FILE)),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        )
    {
        let _ = remove_candidate_publication_staging(directory);
    }
    result
}

/// Replay a durable candidate update. The complete synced file set remains
/// staged until publication commits, so partially written destinations can be
/// replaced safely during recovery.
fn publish_candidate_publication(
    directory: &Path,
    fail_after_candidate_envelope: bool,
    fail_staging_cleanup: bool,
) -> Result<(), String> {
    let transaction_path = directory.join(CANDIDATE_PUBLICATION_FILE);
    let transaction_bytes =
        read_regular_file(&transaction_path, "qualification publication transaction")?;
    let transaction: CandidatePublicationTransaction =
        serde_json::from_slice(&transaction_bytes)
            .map_err(|_| "qualification publication transaction is invalid".to_string())?;
    if !matches!(
        transaction.schema_version,
        1 | CANDIDATE_PUBLICATION_SCHEMA_VERSION
    ) || (transaction.schema_version == 1 && transaction.rom_merge_evidence_present)
    {
        return Err("qualification publication transaction version is unsupported".to_string());
    }

    let staging = directory.join(CANDIDATE_PUBLICATION_STAGING_DIR);
    let staging_metadata = fs::symlink_metadata(&staging)
        .map_err(|_| "qualification candidate publication staging is unavailable".to_string())?;
    if staging_metadata.file_type().is_symlink() || !staging_metadata.is_dir() {
        return Err("qualification candidate publication staging is invalid".to_string());
    }
    let candidate_bytes = read_regular_file(
        &staging.join(CANDIDATE_FILE),
        "staged qualification candidate",
    )?;
    let envelope = decode_candidate_envelope(&candidate_bytes)?;
    let handle = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "qualification candidate directory name is invalid".to_string())?;
    validate_candidate_binding(&envelope, handle)?;
    let declared_sha256 = candidate_report_sha256(&envelope.payload)?;
    let staged_report = staging.join(EXECUTION_REPORT_FILE);
    load_report_bytes(
        &staged_report,
        envelope.report.as_ref(),
        declared_sha256.as_deref(),
    )?;
    if transaction.report_present != envelope.report.is_some() {
        return Err(
            "qualification publication transaction report state is inconsistent".to_string(),
        );
    }
    if transaction.rom_merge_evidence_present != envelope.rom_merge_evidence.is_some() {
        return Err(
            "qualification publication transaction ROM Merge evidence state is inconsistent"
                .to_string(),
        );
    }
    let staged_evidence = staging.join(ROM_MERGE_EVIDENCE_FILE);
    if transaction.rom_merge_evidence_present {
        let evidence_bytes =
            read_rom_merge_evidence_bytes(&staged_evidence, envelope.rom_merge_evidence.as_ref())?;
        let _: RomMergeEvidence = serde_json::from_slice(&evidence_bytes)
            .map_err(|_| "staged ROM Merge evidence is malformed".to_string())?;
    }

    write_synced_replaced_file(
        directory,
        CANDIDATE_FILE,
        CANDIDATE_STAGING_PREFIX,
        &candidate_bytes,
    )?;
    if fail_after_candidate_envelope {
        return Err("injected interruption after candidate envelope publication".to_string());
    }
    if transaction.report_present {
        let report_bytes =
            read_regular_file(&staged_report, "staged qualification execution report")?;
        write_synced_replaced_file(
            directory,
            EXECUTION_REPORT_FILE,
            CANDIDATE_STAGING_PREFIX,
            &report_bytes,
        )?;
    } else {
        remove_optional_regular_file(
            directory,
            EXECUTION_REPORT_FILE,
            "qualification execution report",
        )?;
    }
    if transaction.rom_merge_evidence_present {
        let evidence_bytes = read_regular_file(&staged_evidence, "staged ROM Merge evidence")?;
        write_synced_replaced_file(
            directory,
            ROM_MERGE_EVIDENCE_FILE,
            CANDIDATE_STAGING_PREFIX,
            &evidence_bytes,
        )?;
    } else {
        remove_optional_regular_file(directory, ROM_MERGE_EVIDENCE_FILE, "ROM Merge evidence")?;
    }
    sync_directory(directory)?;
    remove_optional_regular_file(
        directory,
        CANDIDATE_PUBLICATION_FILE,
        "qualification publication transaction",
    )?;
    sync_directory(directory)?;
    let _cleanup = if fail_staging_cleanup {
        Err("injected staging cleanup failure".to_string())
    } else {
        remove_candidate_publication_staging(directory)
    };
    // The candidate/report pair is committed once the transaction is durably
    // removed and the candidate directory is synced. Staging cleanup can be
    // retried on a later repository operation and cannot roll back publication.
    Ok(())
}

fn recover_candidate_publication(directory: &Path) -> Result<(), String> {
    match fs::symlink_metadata(directory.join(CANDIDATE_PUBLICATION_FILE)) {
        Ok(_) => publish_candidate_publication(directory, false, false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A previous publication may have committed before its best-effort
            // staging cleanup completed. Retry that housekeeping without
            // changing the already-published candidate authority.
            let _ = remove_candidate_publication_staging(directory);
            Ok(())
        }
        Err(_) => Err("qualification publication transaction could not be inspected".to_string()),
    }
}

fn remove_candidate_publication_staging(directory: &Path) -> Result<(), String> {
    let staging = directory.join(CANDIDATE_PUBLICATION_STAGING_DIR);
    match fs::symlink_metadata(&staging) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err("qualification candidate publication staging is invalid".to_string())
        }
        Ok(_) => {
            fs::remove_dir_all(&staging).map_err(|_| {
                "qualification candidate publication staging could not be removed".to_string()
            })?;
            sync_directory(directory)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => {
            Err("qualification candidate publication staging could not be inspected".to_string())
        }
    }
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

/// Removes one optional regular file from a candidate directory. Missing files
/// are already in the desired state; symlinks and directories are refused so a
/// removal can never follow a substituted path.
fn remove_optional_regular_file(directory: &Path, file: &str, label: &str) -> Result<(), String> {
    let path = directory.join(file);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(format!("{label} file is not regular"))
        }
        Ok(_) => fs::remove_file(&path).map_err(|_| format!("{label} file could not be removed")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
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

/// Validate the declared execution report against the retained candidate
/// metadata. The bytes themselves stay on disk; the candidate envelope as
/// loaded never carries them, so callers cannot accidentally treat a report as
/// authoritative without going through this check.
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

/// Returns the terminal status for a report that passes the checked-in v2 schema and native bindings.
pub(crate) fn validated_execution_report_v2_status(report_bytes: &[u8]) -> Option<String> {
    let report = serde_json::from_slice::<Value>(report_bytes).ok()?;
    let validator = execution_report_v2_schema_validator()?;
    if !validator.is_valid(&report)
        || !valid_redacted_report_urls(&report)
        || !valid_restricted_report_recipe_step_ids(&report)
    {
        return None;
    }

    if let Some(qualification) = report.get("qualification") {
        let plan = report.get("plan")?;
        if report["app"]["version"] != qualification["buildIdentity"]["appVersion"]
            || plan["planId"] != qualification["reviewedPlanId"]
            || plan["planDigest"] != qualification["reviewedPlanDigest"]
        {
            return None;
        }
    }

    validated_report_execution_status(&report).map(str::to_string)
}

fn execution_report_v2_schema_validator() -> Option<&'static jsonschema::Validator> {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema = serde_json::from_str::<Value>(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../docs/testing/device-qualification/execution-report.schema.json"
            )))
            .ok()?;
            jsonschema::draft202012::options()
                .should_validate_formats(true)
                .offline()
                .build(&schema)
                .ok()
        })
        .as_ref()
}

fn valid_redacted_report_urls(report: &Value) -> bool {
    let Some(releases) = report
        .pointer("/execution/resolvedReleases")
        .and_then(Value::as_array)
    else {
        return false;
    };
    releases.iter().all(|release| {
        let Some(value) = release.get("redactedFinalUrl") else {
            return false;
        };
        if value.is_null() {
            return true;
        }
        let Some(raw_url) = value.as_str() else {
            return false;
        };
        let Ok(url) = tauri::Url::parse(raw_url) else {
            return false;
        };
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none_or(str::is_empty)
            && url.fragment().is_none_or(str::is_empty)
    })
}

fn valid_restricted_report_recipe_step_ids(report: &Value) -> bool {
    if report.get("qualification").is_none() {
        return true;
    }
    let Some(recipes) = report
        .pointer("/execution/recipes")
        .and_then(Value::as_array)
    else {
        return false;
    };
    let mut recipe_ids = std::collections::HashSet::new();
    let mut step_ids = std::collections::HashSet::new();
    for recipe in recipes {
        let Some(recipe_id) = recipe.get("recipeId").and_then(Value::as_str) else {
            return false;
        };
        if !recipe_ids.insert(recipe_id) {
            return false;
        }
        let Some(steps) = recipe.get("steps").and_then(Value::as_array) else {
            return false;
        };
        for step in steps {
            let Some(step_recipe_id) = step.get("recipeId").and_then(Value::as_str) else {
                return false;
            };
            let Some(step_id) = step.get("stepId").and_then(Value::as_str) else {
                return false;
            };
            if step_recipe_id != recipe_id || !step_ids.insert((step_recipe_id, step_id)) {
                return false;
            }
        }
    }
    true
}

fn validated_report_execution_status(report: &Value) -> Option<&str> {
    let execution = report.get("execution")?;
    let simulated = execution.get("simulated")?.as_bool()?;
    let expected_scope = if simulated {
        "simulation_only"
    } else {
        "real_device"
    };
    if execution.get("verificationScope").and_then(Value::as_str) != Some(expected_scope) {
        return None;
    }
    match (simulated, execution.get("target")?) {
        (true, Value::Null) | (false, Value::Object(_)) => {}
        _ => return None,
    }

    let status = execution.get("status")?.as_str()?;
    if execution
        .pointer("/completion/classification")
        .and_then(Value::as_str)
        != Some(expected_report_classification(status)?)
    {
        return None;
    }
    let warning_count = execution
        .pointer("/completion/warningCount")
        .and_then(Value::as_u64)?;
    let warning_length = u64::try_from(execution.get("warnings")?.as_array()?.len()).ok()?;
    (warning_count == warning_length).then_some(status)
}

fn expected_report_classification(status: &str) -> Option<&'static str> {
    match status {
        "succeeded" => Some("success"),
        "succeeded_with_warnings" => Some("success_with_warnings"),
        "failed" => Some("failed"),
        "cancelled" => Some("cancelled"),
        _ => None,
    }
}

fn nonempty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
}

fn report_has_plan_identity(report: &Value) -> bool {
    nonempty_string(report.pointer("/plan/planId"))
        && report
            .pointer("/plan/planDigest")
            .and_then(Value::as_str)
            .is_some_and(is_sha256_hex)
}

fn valid_real_plan_bound_execution_report(report_bytes: &[u8]) -> bool {
    if validated_execution_report_v2_status(report_bytes).is_none() {
        return false;
    }
    let Ok(report) = serde_json::from_slice::<Value>(report_bytes) else {
        return false;
    };
    report
        .pointer("/execution/simulated")
        .and_then(Value::as_bool)
        == Some(false)
        && report
            .pointer("/execution/verificationScope")
            .and_then(Value::as_str)
            == Some("real_device")
        && report_has_plan_identity(&report)
}

fn report_has_complete_restricted_operation_coverage(report_bytes: &[u8]) -> bool {
    let Ok(report) = serde_json::from_slice::<Value>(report_bytes) else {
        return false;
    };
    let Some(qualification) = report.get("qualification") else {
        return true;
    };
    let Some(required_operations) = qualification
        .get("requiredOperationIds")
        .and_then(Value::as_array)
    else {
        return false;
    };
    let Some(operation_outcomes) = qualification
        .get("operationOutcomes")
        .and_then(Value::as_array)
    else {
        return false;
    };
    let required_ids = required_operations
        .iter()
        .filter_map(Value::as_str)
        .collect::<std::collections::HashSet<_>>();
    let outcome_ids = operation_outcomes
        .iter()
        .filter_map(|outcome| outcome.get("operationId").and_then(Value::as_str))
        .collect::<std::collections::HashSet<_>>();

    !required_ids.is_empty()
        && required_ids.len() == required_operations.len()
        && outcome_ids.len() == operation_outcomes.len()
        && outcome_ids == required_ids
}

fn execution_report_claim_matches_candidate(
    report_bytes: &[u8],
    candidate: &Value,
    trusted_target_profile: Option<&str>,
) -> bool {
    let Ok(report) = serde_json::from_slice::<Value>(report_bytes) else {
        return false;
    };
    let Some(qualification) = report.get("qualification") else {
        return true;
    };
    let Some(trusted_target_profile) = trusted_target_profile else {
        return false;
    };
    qualification.get("deviceTargetId") == candidate.get("deviceTargetId")
        && qualification.get("deviceProfileId").and_then(Value::as_str)
            == Some(trusted_target_profile)
        && candidate
            .get("fingerprint")
            .and_then(|fingerprint| fingerprint.get("deviceProfile"))
            .and_then(Value::as_str)
            == Some(trusted_target_profile)
}

fn registered_target_profile_id(repo_root: &Path, target_id: Option<&str>) -> Option<String> {
    let target_id = target_id?;
    let path = repo_root.join("docs/testing/device-qualification/device-targets.json");
    let bytes = read_regular_file(&path, "registered device targets").ok()?;
    let registry = serde_json::from_slice::<Value>(&bytes).ok()?;
    let targets = registry.get("targets")?.as_array()?;
    let mut matching = targets
        .iter()
        .filter(|target| target.get("id").and_then(Value::as_str) == Some(target_id));
    let target = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    target
        .get("profileId")?
        .get("value")?
        .as_str()
        .filter(|profile| !profile.is_empty())
        .map(str::to_string)
}

/// Loads registered target facts and verifies their canonical identity hash.
fn load_registered_rom_merge_target(repo_root: &Path, target_id: &str) -> Result<Value, String> {
    const TARGET_IDENTITY_FIELDS: [&str; 9] = [
        "profileId",
        "manufacturer",
        "model",
        "androidVersion",
        "androidApi",
        "abiSocClass",
        "rootState",
        "connectionType",
        "firmwareBuild",
    ];
    let path = repo_root.join("docs/testing/device-qualification/device-targets.json");
    let bytes = read_regular_file(&path, "registered device targets")?;
    let registry: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "registered device targets are malformed".to_string())?;
    let targets = registry
        .get("targets")
        .and_then(Value::as_array)
        .ok_or_else(|| "registered device targets are malformed".to_string())?;
    let mut matching = targets
        .iter()
        .filter(|target| target.get("id").and_then(Value::as_str) == Some(target_id));
    let target = matching
        .next()
        .ok_or_else(|| "ROM Merge target is not registered".to_string())?;
    if matching.next().is_some() {
        return Err("ROM Merge target registration is ambiguous".to_string());
    }
    let mut identity = serde_json::Map::new();
    for field in TARGET_IDENTITY_FIELDS {
        let value = target
            .get(field)
            .and_then(|fact| fact.get("value"))
            .cloned()
            .ok_or_else(|| "ROM Merge registered target identity is incomplete".to_string())?;
        if field == "androidApi" {
            if value.as_u64().is_none_or(|api| api == 0) {
                return Err("ROM Merge registered target API is invalid".to_string());
            }
        } else if value.as_str().is_none_or(str::is_empty) {
            return Err("ROM Merge registered target identity is invalid".to_string());
        }
        identity.insert(field.to_string(), value);
    }
    let expected_id = format!(
        "device-target-{}",
        canonical_sha256(&Value::Object(identity))?
    );
    if expected_id != target_id {
        return Err("ROM Merge registered target id does not match its identity".to_string());
    }
    Ok(target.clone())
}

fn registered_target_fact<'a>(target: &'a Value, field: &str) -> Option<&'a Value> {
    target.get(field)?.get("value")
}

fn candidate_profile_matches_registered_target(
    candidate: &Value,
    trusted_target_profile: Option<&str>,
) -> bool {
    trusted_target_profile.is_some_and(|profile| {
        candidate
            .get("fingerprint")
            .and_then(|fingerprint| fingerprint.get("deviceProfile"))
            .and_then(Value::as_str)
            == Some(profile)
    })
}

fn is_passing_rom_library_sync_candidate(candidate: &Value) -> bool {
    candidate.get("workflowId").and_then(Value::as_str) == Some("rom-library-sync")
        && candidate.get("runValidity").and_then(Value::as_str) == Some("valid")
        && candidate
            .get("qualificationOutcome")
            .and_then(Value::as_str)
            == Some("passed")
}

fn is_passing_konkr_rom_library_sync_candidate(
    candidate: &Value,
    trusted_target_profile: Option<&str>,
) -> bool {
    let profile = candidate
        .get("fingerprint")
        .and_then(|fingerprint| fingerprint.get("deviceProfile"))
        .and_then(Value::as_str);
    is_passing_rom_library_sync_candidate(candidate)
        && trusted_target_profile == Some("konkr.pocket_advance")
        && profile == Some("konkr.pocket_advance")
}

fn load_rom_merge_evidence_bytes(
    directory: &Path,
    metadata: Option<&RomMergeEvidenceMetadata>,
) -> Result<Option<Vec<u8>>, String> {
    let path = directory.join(ROM_MERGE_EVIDENCE_FILE);
    let Some(metadata) = metadata else {
        return match fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Ok(_) => Err("unbound ROM Merge evidence file is not allowed".to_string()),
            Err(_) => Err("ROM Merge evidence could not be inspected".to_string()),
        };
    };
    let bytes = read_rom_merge_evidence_bytes(&path, Some(metadata))?;
    Ok(Some(bytes))
}

fn read_rom_merge_evidence_bytes(
    path: &Path,
    metadata: Option<&RomMergeEvidenceMetadata>,
) -> Result<Vec<u8>, String> {
    let metadata = metadata.ok_or_else(|| "ROM Merge evidence metadata is missing".to_string())?;
    if metadata.path != ROM_MERGE_EVIDENCE_FILE
        || metadata.byte_length == 0
        || metadata.byte_length as usize > MAX_ROM_MERGE_EVIDENCE_BYTES
        || !is_sha256_hex(&metadata.sha256)
    {
        return Err("ROM Merge evidence metadata is invalid".to_string());
    }
    let file_metadata = fs::symlink_metadata(path)
        .map_err(|_| "ROM Merge evidence could not be inspected".to_string())?;
    if file_metadata.file_type().is_symlink()
        || !file_metadata.is_file()
        || file_metadata.len() > MAX_ROM_MERGE_EVIDENCE_BYTES as u64
    {
        return Err("ROM Merge evidence file is not a bounded regular file".to_string());
    }
    let bytes = read_regular_file(path, "ROM Merge evidence")?;
    if bytes.len() as u64 != metadata.byte_length
        || hex::encode(Sha256::digest(&bytes)) != metadata.sha256
    {
        return Err("ROM Merge evidence bytes do not match their Rust-owned digest".to_string());
    }
    Ok(bytes)
}

fn load_rom_merge_apk_pin(repo_root: &Path) -> Result<Option<RomMergeApkPin>, String> {
    let path = repo_root.join(ROM_MERGE_APK_PIN_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("RetroArch APK pin manifest could not be inspected".to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("RetroArch APK pin manifest is not a regular file".to_string());
    }
    let bytes = read_regular_file(&path, "RetroArch APK pin manifest")?;
    let manifest: RomMergeApkPinManifest = serde_json::from_slice(&bytes)
        .map_err(|_| "RetroArch APK pin manifest is malformed".to_string())?;
    if manifest.schema_version != 1 {
        return Err("RetroArch APK pin manifest version is unsupported".to_string());
    }
    let Some(pin) = manifest.retroarch else {
        return Ok(None);
    };
    if pin.package_id != "com.retroarch.aarch64"
        || !pin.source.starts_with("https://")
        || pin.version.trim().is_empty()
        || !is_sha256_hex(&pin.sha256)
    {
        return Err("RetroArch APK pin does not identify a fixed reviewed artifact".to_string());
    }
    Ok(Some(pin))
}

/// Validates candidate-bound ROM Merge evidence against its report and target.
fn validate_rom_merge_evidence(
    candidate: &Value,
    report_bytes: &[u8],
    evidence: &RomMergeEvidence,
    repo_root: &Path,
) -> Result<(), String> {
    if evidence.schema_version != 1 {
        return Err("ROM Merge evidence schema version is unsupported".to_string());
    }
    if candidate
        .get("candidateSchemaVersion")
        .and_then(Value::as_u64)
        != Some(CANDIDATE_SCHEMA_VERSION)
    {
        return Err("ROM Merge evidence requires candidate schema version 2".to_string());
    }
    let report_sha256 = hex::encode(Sha256::digest(report_bytes));
    if !is_sha256_hex(&evidence.execution_report_sha256)
        || evidence.execution_report_sha256 != report_sha256
    {
        return Err("ROM Merge evidence does not bind the retained execution report".to_string());
    }
    let report: Value = serde_json::from_slice(report_bytes)
        .map_err(|_| "ROM Merge evidence execution report is malformed".to_string())?;
    let report_status = report.pointer("/execution/status").and_then(Value::as_str);
    let candidate_report_outcome = candidate
        .get("automatedObservations")
        .and_then(Value::as_array)
        .and_then(|observations| {
            observations.iter().find(|observation| {
                observation.get("id").and_then(Value::as_str) == Some("execution-report")
            })
        })
        .and_then(|observation| observation.get("outcome"))
        .and_then(Value::as_str);
    if !matches!(report_status, Some("succeeded" | "succeeded_with_warnings"))
        || candidate
            .get("qualificationOutcome")
            .and_then(Value::as_str)
            != Some("passed")
        || candidate_report_outcome != Some("passed")
    {
        return Err(
            "ROM Merge report execution status does not match the passing candidate outcome"
                .to_string(),
        );
    }
    let qualification = report
        .get("qualification")
        .and_then(Value::as_object)
        .ok_or_else(|| "ROM Merge evidence requires a restricted execution report".to_string())?;
    let report_plan = report
        .get("plan")
        .and_then(Value::as_object)
        .ok_or_else(|| "ROM Merge evidence report plan is missing".to_string())?;
    let report_attempt = qualification.get("attemptId").and_then(Value::as_str);
    let candidate_handle = candidate.get("candidateId").and_then(Value::as_str);
    if report_attempt.is_none()
        || report_attempt != candidate_handle
        || evidence.candidate_handle != candidate_handle.unwrap_or_default()
    {
        return Err("ROM Merge evidence candidate and report attempt do not match".to_string());
    }
    let session_id = candidate.get("sessionId").and_then(Value::as_str);
    if session_id.is_none()
        || session_id.is_some_and(str::is_empty)
        || !session_id.is_some_and(is_qualification_session_handle)
        || evidence.session_id != session_id.unwrap_or_default()
    {
        return Err(
            "ROM Merge evidence session does not match the qualification candidate".to_string(),
        );
    }
    let fingerprint = candidate
        .get("fingerprint")
        .and_then(Value::as_object)
        .ok_or_else(|| "ROM Merge evidence candidate fingerprint is missing".to_string())?;
    let target_id = candidate.get("deviceTargetId").and_then(Value::as_str);
    let profile_id = fingerprint.get("deviceProfile").and_then(Value::as_str);
    let firmware_build = fingerprint.get("firmwareBuild").and_then(Value::as_str);
    let abi_soc_class = fingerprint.get("abiSocClass").and_then(Value::as_str);
    let connection_type = fingerprint.get("connectionType").and_then(Value::as_str);
    let observed_root_state = fingerprint.get("rootState").and_then(Value::as_str);
    let android_api = fingerprint.get("androidApi").and_then(Value::as_u64);
    let registered_target =
        load_registered_rom_merge_target(repo_root, evidence.device_target_id.as_str())?;
    if target_id != Some(evidence.device_target_id.as_str())
        || profile_id != Some("konkr.pocket_advance")
        || profile_id != Some(evidence.device_profile_id.as_str())
        || android_api != Some(evidence.android_api)
        || firmware_build != Some(evidence.firmware_build.as_str())
        || abi_soc_class != Some(evidence.abi_soc_class.as_str())
        || connection_type != Some(evidence.connection_type.as_str())
        || observed_root_state != Some(evidence.observed_root_state.as_str())
        || registered_target_fact(&registered_target, "profileId").and_then(Value::as_str)
            != profile_id
        || registered_target_fact(&registered_target, "androidApi").and_then(Value::as_u64)
            != android_api
        || registered_target_fact(&registered_target, "firmwareBuild").and_then(Value::as_str)
            != firmware_build
        || registered_target_fact(&registered_target, "abiSocClass").and_then(Value::as_str)
            != abi_soc_class
        || registered_target_fact(&registered_target, "connectionType").and_then(Value::as_str)
            != connection_type
        || registered_target_fact(&registered_target, "rootState").and_then(Value::as_str)
            != observed_root_state
        || qualification.get("deviceTargetId").and_then(Value::as_str) != target_id
        || qualification.get("deviceProfileId").and_then(Value::as_str) != profile_id
        || qualification
            .get("observedRootState")
            .and_then(Value::as_str)
            != observed_root_state
    {
        return Err(
            "ROM Merge evidence target identity does not match the candidate and report"
                .to_string(),
        );
    }
    let candidate_build = candidate
        .get("build")
        .ok_or_else(|| "ROM Merge evidence candidate build identity is missing".to_string())?;
    let report_build = qualification
        .get("buildIdentity")
        .ok_or_else(|| "ROM Merge evidence report build identity is missing".to_string())?;
    let artifact_build = serde_json::to_value(&evidence.build_identity)
        .map_err(|_| "ROM Merge evidence build identity is invalid".to_string())?;
    let runtime_build = report
        .pointer("/runtime/executionRestriction/buildIdentity")
        .ok_or_else(|| "ROM Merge runtime restriction build identity is missing".to_string())?;
    if candidate_build != report_build
        || report_build != &artifact_build
        || report_build != runtime_build
    {
        return Err(
            "ROM Merge evidence build identity does not match the report and candidate".to_string(),
        );
    }
    let plan_id = report_plan.get("planId").and_then(Value::as_str);
    let plan_digest = report_plan.get("planDigest").and_then(Value::as_str);
    if plan_id != Some(evidence.reviewed_plan_id.as_str())
        || plan_digest != Some(evidence.reviewed_plan_digest.as_str())
        || !is_sha256_hex(&evidence.reviewed_plan_digest)
        || qualification.get("reviewedPlanId").and_then(Value::as_str) != plan_id
        || qualification
            .get("reviewedPlanDigest")
            .and_then(Value::as_str)
            != plan_digest
    {
        return Err(
            "ROM Merge evidence reviewed plan does not match the execution report".to_string(),
        );
    }
    validate_rom_merge_terminal_operation(&report, evidence)?;
    validate_rom_merge_strategy(&report, evidence)?;
    validate_rom_merge_manifests(evidence)?;
    validate_rom_merge_authority(&report, evidence)?;
    let pin = load_rom_merge_apk_pin(repo_root)?
        .ok_or_else(|| "authoritative fixed RetroArch APK pin is unavailable".to_string())?;
    if evidence.retro_arch.package_id != pin.package_id
        || evidence.retro_arch.pinned_source != pin.source
        || evidence.retro_arch.pinned_version != pin.version
        || evidence.retro_arch.installed_version != pin.version
        || evidence.retro_arch.installed_apk_sha256 != pin.sha256
        || evidence.retro_arch.pinned_artifact_sha256 != pin.sha256
    {
        return Err(
            "ROM Merge evidence RetroArch package does not match the repository APK pin"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_rom_merge_terminal_operation(
    report: &Value,
    evidence: &RomMergeEvidence,
) -> Result<(), String> {
    const RECIPE_ID: &str = "feature.copy_roms";
    const STEP_ID: &str = "feature.copy_roms/copy_rom_library";
    if evidence.terminal_operation.recipe_id != RECIPE_ID
        || evidence.terminal_operation.step_id != STEP_ID
        || evidence.terminal_operation.outcome != "succeeded"
        || evidence.copy_policy != "merge"
    {
        return Err("ROM Merge evidence terminal operation or copy policy is invalid".to_string());
    }
    let qualification = report
        .get("qualification")
        .and_then(Value::as_object)
        .ok_or_else(|| "ROM Merge execution report qualification is missing".to_string())?;
    let required = qualification
        .get("requiredOperationIds")
        .and_then(Value::as_array)
        .ok_or_else(|| "ROM Merge report required operations are missing".to_string())?;
    let outcomes = qualification
        .get("operationOutcomes")
        .and_then(Value::as_array)
        .ok_or_else(|| "ROM Merge report operation outcomes are missing".to_string())?;
    let required_ids = required
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| "ROM Merge report operation id is invalid".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if required_ids.is_empty() || required_ids.iter().filter(|id| **id == RECIPE_ID).count() != 1 {
        return Err("ROM Merge report does not require the reviewed copy operation".to_string());
    }
    let mut seen = std::collections::BTreeSet::new();
    for outcome in outcomes {
        let id = outcome
            .get("operationId")
            .and_then(Value::as_str)
            .ok_or_else(|| "ROM Merge report operation outcome is malformed".to_string())?;
        if !required_ids.contains(&id) || !seen.insert(id) {
            return Err(
                "ROM Merge report operation outcomes are duplicate or unexpected".to_string(),
            );
        }
        if outcome.get("outcome").and_then(Value::as_str) != Some("executed") {
            return Err(
                "ROM Merge report contains a skipped or unsuccessful required operation"
                    .to_string(),
            );
        }
    }
    if seen.len() != required_ids.len() {
        return Err("ROM Merge report is missing a required operation outcome".to_string());
    }
    let recipes = report
        .pointer("/execution/recipes")
        .and_then(Value::as_array)
        .ok_or_else(|| "ROM Merge report recipe results are missing".to_string())?;
    let matching = recipes
        .iter()
        .filter(|recipe| recipe.get("recipeId").and_then(Value::as_str) == Some(RECIPE_ID))
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err("ROM Merge report must contain one terminal copy recipe result".to_string());
    }
    let recipe = matching[0];
    if !matches!(
        recipe.get("status").and_then(Value::as_str),
        Some("succeeded" | "succeeded_with_warnings")
    ) {
        return Err("ROM Merge copy recipe did not succeed".to_string());
    }
    let steps = recipe
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "ROM Merge copy recipe step results are missing".to_string())?;
    let matching_steps = steps
        .iter()
        .filter(|step| step.get("stepId").and_then(Value::as_str) == Some(STEP_ID))
        .collect::<Vec<_>>();
    if matching_steps.len() != 1
        || matching_steps[0].get("recipeId").and_then(Value::as_str) != Some(RECIPE_ID)
        || matching_steps[0].get("status").and_then(Value::as_str) != Some("succeeded")
    {
        return Err(
            "ROM Merge report lacks the exact successful copy_rom_library step".to_string(),
        );
    }
    Ok(())
}

fn validate_rom_merge_strategy(report: &Value, evidence: &RomMergeEvidence) -> Result<(), String> {
    let qualification = report
        .get("qualification")
        .and_then(Value::as_object)
        .ok_or_else(|| "ROM Merge report qualification is missing".to_string())?;
    let runtime_restriction = report
        .pointer("/runtime/executionRestriction")
        .ok_or_else(|| "ROM Merge report execution restriction is missing".to_string())?;
    let capabilities = qualification
        .get("effectiveCapabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| "ROM Merge report effective capabilities are missing".to_string())?;
    let capability_names = capabilities
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "ROM Merge report effective capability is invalid".to_string())?;
    if capability_names
        .iter()
        .any(|capability| !ROM_MERGE_UNPRIVILEGED_CAPABILITIES.contains(capability))
    {
        return Err("ROM Merge report contains an unsupported effective capability".to_string());
    }
    let strategy = qualification
        .get("strategy")
        .and_then(Value::as_str)
        .ok_or_else(|| "ROM Merge report strategy is missing".to_string())?;
    let restriction = qualification
        .get("restriction")
        .and_then(Value::as_str)
        .ok_or_else(|| "ROM Merge report restriction is missing".to_string())?;
    if strategy != "adb_unprivileged" || restriction != "privileged_execution_disabled" {
        return Err("ROM Merge report strategy or restriction is unsupported".to_string());
    }
    if runtime_restriction.get("enabled").and_then(Value::as_bool) != Some(true)
        || runtime_restriction.get("strategy").and_then(Value::as_str) != Some("adb_shell")
        || runtime_restriction
            .get("rootShell")
            .and_then(Value::as_bool)
            != Some(false)
        || runtime_restriction
            .get("appDataWrite")
            .and_then(Value::as_bool)
            != Some(false)
    {
        return Err(
            "ROM Merge report does not establish the restricted execution strategy".to_string(),
        );
    }
    let digest_payload = serde_json::json!({
        "domain": "emuchef.rom-merge.strategy",
        "version": 1,
        "strategy": strategy,
        "restriction": restriction,
        "runtimeRestriction": runtime_restriction,
        "effectiveCapabilities": capabilities,
    });
    let digest = canonical_sha256(&digest_payload)?;
    if evidence.strategy_digest != digest {
        return Err(
            "ROM Merge evidence strategy digest does not match the execution report".to_string(),
        );
    }
    Ok(())
}

fn validate_rom_merge_manifests(evidence: &RomMergeEvidence) -> Result<(), String> {
    if evidence.source_snapshot.source_identity.trim().is_empty()
        || evidence.destination.path.trim().is_empty()
        || evidence.destination.setup_location.trim().is_empty()
        || evidence.destination.volume_identity.trim().is_empty()
    {
        return Err("ROM Merge source or destination binding is incomplete".to_string());
    }
    let source = rom_merge_manifest_map(&evidence.source_snapshot.entries, "source", false)?;
    let before = rom_merge_manifest_map(&evidence.destination.before, "destination-before", true)?;
    let after = rom_merge_manifest_map(&evidence.destination.after, "destination-after", true)?;
    for (path, digest) in &source {
        if after.get(path) != Some(digest) {
            return Err(format!(
                "ROM Merge source path {path} is missing or changed after copying"
            ));
        }
    }
    for (path, digest) in &before {
        if !source.contains_key(path) && after.get(path) != Some(digest) {
            return Err(format!(
                "ROM Merge destination-only path {path} was not preserved"
            ));
        }
    }
    if after
        .keys()
        .any(|path| !source.contains_key(path) && !before.contains_key(path))
    {
        return Err("ROM Merge destination-after contains an unexpected path".to_string());
    }
    if evidence.sentinel.relative_path != "qualification-sentinel.keep"
        || !is_sha256_hex(&evidence.sentinel.expected_sha256)
        || source.contains_key(&evidence.sentinel.relative_path)
        || before.get(&evidence.sentinel.relative_path) != Some(&evidence.sentinel.expected_sha256)
        || after.get(&evidence.sentinel.relative_path) != Some(&evidence.sentinel.expected_sha256)
    {
        return Err(
            "ROM Merge destination sentinel is missing, changed, or present in the source"
                .to_string(),
        );
    }
    Ok(())
}

fn rom_merge_manifest_map(
    entries: &[RomMergeManifestEntry],
    label: &str,
    allow_empty: bool,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    if !allow_empty && entries.is_empty() {
        return Err("ROM Merge source manifest must not be empty".to_string());
    }
    let mut result = std::collections::BTreeMap::new();
    for entry in entries {
        let normalized = !entry.relative_path.is_empty()
            && !entry.relative_path.starts_with('/')
            && !entry.relative_path.contains('\\')
            && !entry.relative_path.contains('\0')
            && entry
                .relative_path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..");
        if !normalized || !is_sha256_hex(&entry.sha256) {
            return Err(format!(
                "ROM Merge {label} manifest contains an invalid path or digest"
            ));
        }
        if result
            .insert(entry.relative_path.clone(), entry.sha256.clone())
            .is_some()
        {
            return Err(format!(
                "ROM Merge {label} manifest contains a duplicate path"
            ));
        }
    }
    Ok(result)
}

fn validate_rom_merge_authority(report: &Value, evidence: &RomMergeEvidence) -> Result<(), String> {
    if evidence.root_authority_used || evidence.oem_compiler_used || evidence.oem_stage_used {
        return Err("ROM Merge evidence claims unsupported root or OEM authority".to_string());
    }
    let observed_root_state = report
        .pointer("/qualification/observedRootState")
        .and_then(Value::as_str);
    if observed_root_state != Some(evidence.observed_root_state.as_str()) {
        return Err("ROM Merge observed root state is missing".to_string());
    }
    Ok(())
}

fn canonical_sha256(value: &Value) -> Result<String, String> {
    fn canonicalize(value: &Value) -> Value {
        match value {
            Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                let mut sorted = serde_json::Map::new();
                for key in keys {
                    sorted.insert(key.clone(), canonicalize(&object[key]));
                }
                Value::Object(sorted)
            }
            _ => value.clone(),
        }
    }
    let bytes = serde_json::to_vec(&canonicalize(value))
        .map_err(|_| "ROM Merge strategy digest could not be serialized".to_string())?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}

fn is_qualification_session_handle(value: &str) -> bool {
    value
        .strip_prefix("qualification-session-")
        .is_some_and(|suffix| {
            suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
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

fn write_synced_replaced_file(
    directory: &Path,
    filename: &str,
    staging_prefix: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let path = directory.join(filename);
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("qualification session file is not regular".to_string());
        }
    }
    let staging = directory.join(format!("{staging_prefix}{}", Uuid::new_v4().simple()));
    let result = (|| {
        write_synced_new_file(&staging, bytes)?;
        fs::rename(&staging, &path)
            .map_err(|_| "qualification session file could not be committed".to_string())?;
        sync_directory(directory)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
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

    fn write_registered_targets_for_test(temp: &TempDir, targets: &[(&str, &str)]) {
        let directory = temp.path().join("docs/testing/device-qualification");
        std::fs::create_dir_all(&directory).expect("target registry directory should be created");
        let targets = targets
            .iter()
            .map(|(id, profile)| {
                json!({
                    "id": id,
                    "profileId": { "value": profile }
                })
            })
            .collect::<Vec<_>>();
        std::fs::write(
            directory.join("device-targets.json"),
            serde_json::to_vec(&json!({ "schemaVersion": 2, "targets": targets }))
                .expect("target registry should serialize"),
        )
        .expect("target registry should be written");
    }

    fn write_registered_konkr_target_for_test(temp: &TempDir) {
        const TARGET_ID: &str =
            "device-target-sha256:ee1d7cbe10b21866fb39ab6aff94d43f29c76ab3f10d083d7e304b6022e82bf9";
        let directory = temp.path().join("docs/testing/device-qualification");
        std::fs::create_dir_all(&directory).expect("target registry directory should be created");
        let target = json!({
            "id": TARGET_ID,
            "profileId": { "value": "konkr.pocket_advance", "source": "production_observation" },
            "manufacturer": { "value": "KONKR", "source": "production_observation" },
            "model": { "value": "Pocket Advance", "source": "production_observation" },
            "androidVersion": { "value": "15", "source": "production_observation" },
            "androidApi": { "value": 35, "source": "production_observation" },
            "abiSocClass": { "value": "arm64", "source": "production_observation" },
            "rootState": { "value": "non_root", "source": "explicit_root_check" },
            "connectionType": { "value": "usb3", "source": "operator_attestation" },
            "firmwareBuild": { "value": "konkr-test-build", "source": "production_observation" },
            "capabilities": [
                "adb_available",
                "apk_install",
                "shared_storage_write",
                "app_launch",
                "shell_command",
                "package_remove_for_user"
            ]
        });
        std::fs::write(
            directory.join("device-targets.json"),
            serde_json::to_vec(&json!({ "schemaVersion": 2, "targets": [target] }))
                .expect("target registry should serialize"),
        )
        .expect("target registry should be written");
    }

    fn rust_execution_report_fixture_for_target(target_id: &str, profile_id: &str) -> Vec<u8> {
        const FIXTURE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/testing/device-qualification/fixtures/execution-report-v2-real.json"
        ));
        let mut report = serde_json::from_slice::<Value>(FIXTURE)
            .expect("Rust-produced fixture should be valid JSON");
        report["qualification"]["deviceTargetId"] = Value::String(target_id.to_string());
        report["qualification"]["deviceProfileId"] = Value::String(profile_id.to_string());
        serde_json::to_vec(&report).expect("report fixture should serialize")
    }

    fn rust_ordinary_execution_report_fixture_for_target() -> Vec<u8> {
        const FIXTURE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/testing/device-qualification/fixtures/execution-report-v2-real.json"
        ));
        let mut report = serde_json::from_slice::<Value>(FIXTURE)
            .expect("Rust-produced fixture should be valid JSON");
        report.as_object_mut().unwrap().remove("qualification");
        for recipe in report["execution"]["recipes"].as_array_mut().unwrap() {
            let recipe = recipe.as_object_mut().unwrap();
            recipe.remove("recipeId");
            for step in recipe["steps"].as_array_mut().unwrap() {
                let step = step.as_object_mut().unwrap();
                step.remove("recipeId");
                step.remove("stepId");
            }
        }
        serde_json::to_vec(&report).expect("ordinary report fixture should serialize")
    }

    #[test]
    fn native_v2_validation_matches_nested_schema_and_consistency_rules() {
        const FIXTURE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/testing/device-qualification/fixtures/execution-report-v2-real.json"
        ));
        let fixture = serde_json::from_slice::<Value>(FIXTURE)
            .expect("Rust-produced fixture should be valid JSON");
        assert_eq!(
            validated_execution_report_v2_status(FIXTURE).as_deref(),
            Some("succeeded")
        );
        assert_eq!(
            validated_execution_report_v2_status(
                &rust_ordinary_execution_report_fixture_for_target()
            )
            .as_deref(),
            Some("succeeded")
        );

        macro_rules! reject_mutation {
            ($label:literal, $mutation:expr) => {{
                let mut report = fixture.clone();
                ($mutation)(&mut report);
                let bytes = serde_json::to_vec(&report).expect("mutated report should serialize");
                assert!(
                    validated_execution_report_v2_status(&bytes).is_none(),
                    concat!("native validator accepted ", $label)
                );
            }};
        }

        reject_mutation!("simulation/scope mismatch", |report: &mut Value| {
            report["execution"]["verificationScope"] = json!("simulation_only");
        });
        reject_mutation!("real report without a target", |report: &mut Value| {
            report["execution"]["target"] = Value::Null;
        });
        reject_mutation!("simulated report with a target", |report: &mut Value| {
            report["execution"]["simulated"] = json!(true);
            report["execution"]["verificationScope"] = json!("simulation_only");
        });
        reject_mutation!("recipe with an unknown field", |report: &mut Value| {
            report["execution"]["recipes"][0]["extra"] = json!(true);
        });
        reject_mutation!("step bound to a different recipe", |report: &mut Value| {
            report["execution"]["recipes"][0]["steps"][0]["recipeId"] = json!("required.second");
        });
        reject_mutation!("invalid issue remediation", |report: &mut Value| {
            report["execution"]["warnings"] = json!([{
                "message": "Warning",
                "remediation": {
                    "kind": "execute_shell",
                    "title": "Review",
                    "message": "Review the warning"
                }
            }]);
        });
        reject_mutation!("malformed target", |report: &mut Value| {
            report["execution"]["target"]["label"] = json!(null);
        });
        reject_mutation!("malformed resolved release", |report: &mut Value| {
            report["execution"]["resolvedReleases"] = json!([{"provider": "github"}]);
        });
        reject_mutation!(
            "completion classification mismatch",
            |report: &mut Value| {
                report["execution"]["completion"]["classification"] = json!("failed");
            }
        );
        reject_mutation!("warning count mismatch", |report: &mut Value| {
            report["execution"]["completion"]["warningCount"] = json!(1);
        });
        reject_mutation!("unsafe JSON integer", |report: &mut Value| {
            report["execution"]["completion"]["counts"]["total"] = json!(9_007_199_254_740_992_u64);
        });
        reject_mutation!("invalid timestamp", |report: &mut Value| {
            report["execution"]["startedAt"] = json!("yesterday");
        });
        reject_mutation!("release URL with credentials", |report: &mut Value| {
            report["execution"]["resolvedReleases"] = json!([{
                "recipeArtifactId": "retroarch.apk",
                "provider": "github",
                "appId": "org.example.app",
                "artifactId": "apk",
                "kind": "apk",
                "releaseTag": "v1",
                "assetName": "app.apk",
                "publishedAt": null,
                "createdAt": null,
                "timestampSource": "created_at",
                "size": null,
                "calculatedSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "cacheHit": false,
                "redactedFinalUrl": "https://user:secret@example.test/app.apk"
            }]);
        });
        reject_mutation!("release URL with a query", |report: &mut Value| {
            report["execution"]["resolvedReleases"] = json!([{
                "recipeArtifactId": "retroarch.apk",
                "provider": "github",
                "appId": "org.example.app",
                "artifactId": "apk",
                "kind": "apk",
                "releaseTag": "v1",
                "assetName": "app.apk",
                "publishedAt": null,
                "createdAt": null,
                "timestampSource": "created_at",
                "size": null,
                "calculatedSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "cacheHit": false,
                "redactedFinalUrl": "https://example.test/app.apk?token=secret"
            }]);
        });
    }

    #[test]
    fn restricted_claim_outcome_coverage_is_a_promotion_rule_not_schema_shape() {
        const TARGET_ID: &str =
            "device-target-sha256:7db2d1094b12e20600c2c9bf920c41bef07700b2131f9648fe2fa1f7e9cebafa";
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        write_registered_targets_for_test(&temp, &[(TARGET_ID, "ayaneo.pocket_air_mini")]);

        let incomplete_outcomes = [
            json!([]),
            json!([
                { "operationId": "required.first", "outcome": "executed" },
                { "operationId": "required.first", "outcome": "executed" }
            ]),
            json!([{ "operationId": "required.first", "outcome": "executed" }]),
        ];
        for operation_outcomes in incomplete_outcomes {
            let mut report = serde_json::from_slice::<Value>(
                &rust_execution_report_fixture_for_target(TARGET_ID, "ayaneo.pocket_air_mini"),
            )
            .expect("restricted report fixture should be valid JSON");
            report["qualification"]["operationOutcomes"] = operation_outcomes;
            let report_bytes = serde_json::to_vec(&report).expect("report should serialize");

            assert_eq!(
                validated_execution_report_v2_status(&report_bytes).as_deref(),
                Some("succeeded"),
                "operation coverage is a native promotion rule, not a schema shape rule"
            );

            let candidate = repository
                .create_candidate(
                    CandidateKind::QualificationRun,
                    &report_backed_candidate(
                        "retroarch-plus-bios",
                        TARGET_ID,
                        "ayaneo.pocket_air_mini",
                        &report_bytes,
                    ),
                    Some(&report_bytes),
                )
                .expect("incomplete restricted report should remain retained for inspection");
            let stored = repository
                .load_candidate(&candidate)
                .expect("candidate should remain inspectable");

            assert!(
                !stored.promotable,
                "a valid run needs every claimed operation outcome"
            );
            assert!(repository.record_run(&candidate).is_err());
        }
        assert!(
            calls.calls().is_empty(),
            "incomplete claims must not reach Node"
        );
    }

    #[test]
    fn simulated_report_is_valid_audit_data_but_not_promotable_canonical_evidence() {
        const TARGET_ID: &str =
            "device-target-sha256:7db2d1094b12e20600c2c9bf920c41bef07700b2131f9648fe2fa1f7e9cebafa";
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let mut report =
            serde_json::from_slice::<Value>(&rust_ordinary_execution_report_fixture_for_target())
                .expect("ordinary fixture should be valid JSON");
        report["execution"]["simulated"] = json!(true);
        report["execution"]["verificationScope"] = json!("simulation_only");
        report["execution"]["target"] = Value::Null;
        let report_bytes = serde_json::to_vec(&report).expect("simulated report should serialize");
        assert_eq!(
            validated_execution_report_v2_status(&report_bytes).as_deref(),
            Some("succeeded")
        );

        let candidate = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "retroarch-plus-bios",
                    TARGET_ID,
                    "ayaneo.pocket_air_mini",
                    &report_bytes,
                ),
                Some(&report_bytes),
            )
            .expect("simulated report should remain retained for audit");
        assert!(
            !repository
                .load_candidate(&candidate)
                .expect("simulated candidate should remain inspectable")
                .promotable
        );
        assert!(repository.record_run(&candidate).is_err());
        assert!(
            calls.calls().is_empty(),
            "simulated evidence must not reach Node"
        );

        let mut audit_payload = report_backed_candidate(
            "retroarch-plus-bios",
            TARGET_ID,
            "ayaneo.pocket_air_mini",
            &report_bytes,
        );
        audit_payload["runValidity"] = json!("invalid");
        audit_payload["qualificationOutcome"] = json!("not_observed");
        let audit_candidate = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &audit_payload,
                Some(&report_bytes),
            )
            .expect("simulated report should be retained for invalid audit recording");
        assert!(
            repository
                .load_candidate(&audit_candidate)
                .expect("invalid audit candidate should load")
                .promotable
        );
        calls.set_response(json!({
            "operation": "record_run",
            "candidateHandle": audit_candidate,
            "candidateKind": "qualification_run",
            "payload": { "runId": "simulated-invalid-audit" },
        }));
        repository
            .record_run(&audit_candidate)
            .expect("report-bearing invalid not_observed audit remains recordable");
        assert_eq!(calls.calls().len(), 1);
    }

    #[test]
    fn malformed_v2_report_cannot_reach_native_record_runner() {
        const TARGET_ID: &str =
            "device-target-sha256:7db2d1094b12e20600c2c9bf920c41bef07700b2131f9648fe2fa1f7e9cebafa";
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let mut report =
            serde_json::from_slice::<Value>(&rust_ordinary_execution_report_fixture_for_target())
                .expect("ordinary fixture should be valid JSON");
        report["execution"]["recipes"][0]["steps"][0]["extra"] = json!(true);
        let report_bytes = serde_json::to_vec(&report).expect("malformed report should serialize");
        let candidate = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "retroarch-plus-bios",
                    TARGET_ID,
                    "ayaneo.pocket_air_mini",
                    &report_bytes,
                ),
                Some(&report_bytes),
            )
            .expect("malformed report should remain retained for inspection");

        let stored = repository
            .load_candidate(&candidate)
            .expect("candidate should remain inspectable");
        assert!(!stored.promotable);
        assert!(repository.record_run(&candidate).is_err());
        assert!(
            calls.calls().is_empty(),
            "malformed evidence must not reach Node"
        );
    }

    fn report_backed_candidate(
        workflow_id: &str,
        target_id: &str,
        profile_id: &str,
        report_bytes: &[u8],
    ) -> Value {
        json!({
            "capturedAt": "2026-08-23T12:00:00Z",
            "build": build_identity_json(),
            "workflowId": workflow_id,
            "deviceTargetId": target_id,
            "fingerprint": { "deviceProfile": profile_id },
            "runValidity": "valid",
            "qualificationOutcome": "passed",
            "artifacts": [{
                "path": "execution-report.json",
                "sha256": hex::encode(Sha256::digest(report_bytes))
            }]
        })
    }

    fn rom_merge_candidate_payload(target_id: &str, session_id: &str) -> Value {
        json!({
            "candidateSchemaVersion": 2,
            "kind": "qualification_run",
            "capturedAt": "2026-08-23T12:00:00Z",
            "sessionId": session_id,
            "build": build_identity_json(),
            "workflowId": "rom-library-sync",
            "workflowVersion": 1,
            "deviceTargetId": target_id,
            "fingerprint": {
                "deviceProfile": "konkr.pocket_advance",
                "androidApi": 35,
                "firmwareBuild": "konkr-test-build",
                "abiSocClass": "arm64",
                "rootState": "non_root",
                "connectionType": "usb3"
            },
            "runValidity": "valid",
            "qualificationOutcome": "passed",
            "automatedObservations": [{
                "id": "execution-report",
                "outcome": "passed",
                "observedAt": "2026-08-23T12:00:00Z"
            }],
            "humanCheckpoints": [],
            "targetWideFailure": null,
            "limitations": [],
            "artifacts": []
        })
    }

    fn rom_merge_report_for_candidate(target_id: &str, candidate_handle: &str) -> Vec<u8> {
        const FIXTURE: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/testing/device-qualification/fixtures/execution-report-v2-real.json"
        ));
        const PLAN_ID: &str = "konkr.pocket_advance.base";
        const PLAN_DIGEST: &str =
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let mut report = serde_json::from_slice::<Value>(FIXTURE)
            .expect("execution report fixture should be valid");
        report["qualification"]["attemptId"] = json!(candidate_handle);
        report["qualification"]["deviceTargetId"] = json!(target_id);
        report["qualification"]["deviceProfileId"] = json!("konkr.pocket_advance");
        report["qualification"]["reviewedPlanId"] = json!(PLAN_ID);
        report["qualification"]["reviewedPlanDigest"] = json!(PLAN_DIGEST);
        report["qualification"]["requiredOperationIds"] = json!(["feature.copy_roms"]);
        report["qualification"]["operationOutcomes"] = json!([{
            "operationId": "feature.copy_roms",
            "outcome": "executed"
        }]);
        report["plan"]["planId"] = json!(PLAN_ID);
        report["plan"]["planDigest"] = json!(PLAN_DIGEST);
        let mut recipe = report["execution"]["recipes"][0].clone();
        recipe["recipeId"] = json!("feature.copy_roms");
        recipe["status"] = json!("succeeded");
        let mut step = recipe["steps"][0].clone();
        step["recipeId"] = json!("feature.copy_roms");
        step["stepId"] = json!("feature.copy_roms/copy_rom_library");
        step["status"] = json!("succeeded");
        recipe["steps"] = json!([step]);
        report["execution"]["recipes"] = json!([recipe]);
        serde_json::to_vec(&report).expect("ROM Merge report should serialize")
    }

    fn rom_merge_evidence_for_candidate(
        candidate_handle: &str,
        session_id: &str,
        target_id: &str,
        candidate_build: &Value,
        report_bytes: &[u8],
    ) -> Vec<u8> {
        const SENTINEL_SHA: &str =
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        const APK_SHA: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let report: Value = serde_json::from_slice(report_bytes).unwrap();
        let qualification = &report["qualification"];
        let strategy_digest = canonical_sha256(&json!({
            "domain": "emuchef.rom-merge.strategy",
            "version": 1,
            "strategy": qualification["strategy"],
            "restriction": qualification["restriction"],
            "runtimeRestriction": report["runtime"]["executionRestriction"],
            "effectiveCapabilities": qualification["effectiveCapabilities"]
        }))
        .unwrap();
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "candidateHandle": candidate_handle,
            "sessionId": session_id,
            "executionReportSha256": hex::encode(Sha256::digest(report_bytes)),
            "deviceTargetId": target_id,
            "deviceProfileId": "konkr.pocket_advance",
            "androidApi": 35,
            "firmwareBuild": "konkr-test-build",
            "abiSocClass": "arm64",
            "connectionType": "usb3",
            "observedRootState": qualification["observedRootState"],
            "buildIdentity": candidate_build,
            "reviewedPlanId": qualification["reviewedPlanId"],
            "reviewedPlanDigest": qualification["reviewedPlanDigest"],
            "terminalOperation": {
                "recipeId": "feature.copy_roms",
                "stepId": "feature.copy_roms/copy_rom_library",
                "outcome": "succeeded"
            },
            "copyPolicy": "merge",
            "strategyDigest": strategy_digest,
            "sourceSnapshot": {
                "sourceIdentity": "reviewed-source-snapshot-1",
                "entries": [{ "relativePath": "games/one.gba", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }]
            },
            "destination": {
                "path": "/storage/emulated/0/ROMs",
                "setupLocation": "internal_shared_storage",
                "volumeIdentity": "volume-sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "before": [
                    { "relativePath": "games/one.gba", "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" },
                    { "relativePath": "existing/readme.txt", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
                    { "relativePath": "qualification-sentinel.keep", "sha256": SENTINEL_SHA }
                ],
                "after": [
                    { "relativePath": "games/one.gba", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
                    { "relativePath": "existing/readme.txt", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
                    { "relativePath": "qualification-sentinel.keep", "sha256": SENTINEL_SHA }
                ]
            },
            "sentinel": {
                "relativePath": "qualification-sentinel.keep",
                "expectedSha256": SENTINEL_SHA
            },
            "retroArch": {
                "packageId": "com.retroarch.aarch64",
                "installedVersion": "synthetic-1",
                "pinnedSource": "https://example.invalid/retroarch.apk",
                "pinnedVersion": "synthetic-1",
                "installedApkSha256": APK_SHA,
                "pinnedArtifactSha256": APK_SHA
            },
            "rootAuthorityUsed": false,
            "oemCompilerUsed": false,
            "oemStageUsed": false
        }))
        .unwrap()
    }

    fn write_rom_merge_apk_pin_for_test(temp: &TempDir) {
        let path = temp.path().join(ROM_MERGE_APK_PIN_FILE);
        std::fs::create_dir_all(path.parent().unwrap())
            .expect("pin manifest directory should be created");
        std::fs::write(
            path,
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "retroarch": {
                    "packageId": "com.retroarch.aarch64",
                    "source": "https://example.invalid/retroarch.apk",
                    "version": "synthetic-1",
                    "sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
                }
            }))
            .unwrap(),
        )
        .expect("synthetic pin manifest should be written");
    }

    fn finalized_rom_merge_candidate(
        repository: &QualificationRepository,
        target_id: &str,
        session_id: &str,
    ) -> (String, Vec<u8>, Vec<u8>) {
        let mut candidate = rom_merge_candidate_payload(target_id, session_id);
        let handle = repository
            .create_candidate(CandidateKind::QualificationRun, &candidate, None)
            .expect("provisional ROM Merge candidate should be stored");
        let report_bytes = rom_merge_report_for_candidate(target_id, &handle);
        candidate["artifacts"] = json!([{
            "id": "execution-report",
            "kind": "production_execution_report",
            "path": "execution-report.json",
            "sha256": hex::encode(Sha256::digest(&report_bytes))
        }]);
        repository
            .finalize_candidate(
                &handle,
                CandidateKind::QualificationRun,
                &candidate,
                Some(&report_bytes),
            )
            .expect("ROM Merge candidate should retain its report");
        let evidence_bytes = rom_merge_evidence_for_candidate(
            &handle,
            session_id,
            target_id,
            &candidate["build"],
            &report_bytes,
        );
        (handle, report_bytes, evidence_bytes)
    }

    #[test]
    fn valid_rom_merge_attachment_is_bound_retained_and_passes_the_rust_gate() {
        const TARGET_ID: &str =
            "device-target-sha256:ee1d7cbe10b21866fb39ab6aff94d43f29c76ab3f10d083d7e304b6022e82bf9";
        const SESSION_ID: &str = "qualification-session-cccccccccccccccccccccccccccccccc";
        let temp = TempDir::new().expect("temporary repository should be created");
        write_registered_konkr_target_for_test(&temp);
        write_rom_merge_apk_pin_for_test(&temp);
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let (handle, report_bytes, evidence_bytes) =
            finalized_rom_merge_candidate(&repository, TARGET_ID, SESSION_ID);
        let before = repository
            .load_candidate(&handle)
            .expect("candidate without supplemental evidence should remain inspectable");
        assert!(!before.promotable);
        assert!(before
            .non_promotable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("ROM Merge evidence is required")));

        let attached = repository
            .attach_rom_merge_evidence(&handle, &evidence_bytes)
            .expect("complete evidence should attach");
        assert!(attached.rom_merge_evidence_required);
        assert!(attached.rom_merge_evidence_attached);
        assert!(
            attached.promotable,
            "the test pin and complete evidence satisfy the Rust gate"
        );
        let directory = temp.path().join(CANDIDATE_DIRECTORY).join(&handle);
        assert_eq!(
            std::fs::read(directory.join(ROM_MERGE_EVIDENCE_FILE))
                .expect("attached bytes should remain in the candidate"),
            evidence_bytes,
            "the producer must preserve the supplied bytes"
        );
        assert_eq!(
            std::fs::read(directory.join(EXECUTION_REPORT_FILE))
                .expect("report bytes should remain in the candidate"),
            report_bytes,
            "attachment must preserve the report bytes"
        );

        let finalized_payload = repository
            .load_candidate(&handle)
            .expect("attached candidate should remain readable")
            .payload;
        let finalized_again = repository
            .finalize_candidate(
                &handle,
                CandidateKind::QualificationRun,
                &finalized_payload,
                Some(&report_bytes),
            )
            .expect("recovery finalization should preserve the attachment");
        assert!(finalized_again.summary().rom_merge_evidence_attached);
        assert!(finalized_again.promotable);

        calls.set_response(json!({
            "operation": "record_run",
            "candidateHandle": handle.clone(),
            "candidateKind": "qualification_run",
            "payload": { "runId": "qualification-run-opaque" }
        }));
        repository
            .record_run(&handle)
            .expect("complete ROM Merge evidence should pass the Rust gate");
        assert_eq!(calls.calls().len(), 1);
        assert_eq!(
            calls.calls()[0].1,
            vec!["--record-run".to_string(), handle.clone()]
        );
        assert!(repository
            .attach_rom_merge_evidence(&handle, &evidence_bytes)
            .expect_err("duplicate attachments must be rejected")
            .contains("already attached"));
    }

    #[test]
    fn rom_merge_evidence_rejects_runtime_build_identity_mismatch() {
        const TARGET_ID: &str =
            "device-target-sha256:ee1d7cbe10b21866fb39ab6aff94d43f29c76ab3f10d083d7e304b6022e82bf9";
        const SESSION_ID: &str = "qualification-session-cccccccccccccccccccccccccccccccc";
        let temp = TempDir::new().expect("temporary repository should be created");
        write_registered_konkr_target_for_test(&temp);
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let (handle, report_bytes, _) =
            finalized_rom_merge_candidate(&repository, TARGET_ID, SESSION_ID);
        let candidate = repository
            .load_candidate(&handle)
            .expect("candidate should remain inspectable")
            .payload;
        let mut report: Value = serde_json::from_slice(&report_bytes).unwrap();
        report["runtime"]["executionRestriction"]["buildIdentity"]["gitCommit"] =
            json!("ffffffffffffffffffffffffffffffffffffffff");
        let mismatched_report = serde_json::to_vec(&report).unwrap();
        let evidence_bytes = rom_merge_evidence_for_candidate(
            &handle,
            SESSION_ID,
            TARGET_ID,
            &candidate["build"],
            &mismatched_report,
        );
        let evidence: RomMergeEvidence = serde_json::from_slice(&evidence_bytes).unwrap();

        assert!(validate_rom_merge_evidence(
            &candidate,
            &mismatched_report,
            &evidence,
            temp.path(),
        )
        .expect_err("runtime restriction must use the same build identity")
        .contains("build identity"));
    }

    #[test]
    fn rom_merge_evidence_binds_report_outcome_target_and_capabilities() {
        const TARGET_ID: &str =
            "device-target-sha256:ee1d7cbe10b21866fb39ab6aff94d43f29c76ab3f10d083d7e304b6022e82bf9";
        const SESSION_ID: &str = "qualification-session-cccccccccccccccccccccccccccccccc";
        let temp = TempDir::new().expect("temporary repository should be created");
        write_registered_konkr_target_for_test(&temp);
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let (handle, report_bytes, _) =
            finalized_rom_merge_candidate(&repository, TARGET_ID, SESSION_ID);
        let candidate = repository
            .load_candidate(&handle)
            .expect("candidate should remain inspectable")
            .payload;

        let mut failed_report: Value = serde_json::from_slice(&report_bytes).unwrap();
        failed_report["execution"]["status"] = json!("failed");
        let failed_report_bytes = serde_json::to_vec(&failed_report).unwrap();
        let failed_evidence: RomMergeEvidence =
            serde_json::from_slice(&rom_merge_evidence_for_candidate(
                &handle,
                SESSION_ID,
                TARGET_ID,
                &candidate["build"],
                &failed_report_bytes,
            ))
            .unwrap();
        assert!(validate_rom_merge_evidence(
            &candidate,
            &failed_report_bytes,
            &failed_evidence,
            temp.path(),
        )
        .expect_err("failed report must not qualify a passed candidate")
        .contains("passing candidate outcome"));

        let mut stale_candidate = candidate.clone();
        stale_candidate["fingerprint"]["androidApi"] = json!(34);
        let mut stale_evidence: Value = serde_json::from_slice(&rom_merge_evidence_for_candidate(
            &handle,
            SESSION_ID,
            TARGET_ID,
            &candidate["build"],
            &report_bytes,
        ))
        .unwrap();
        stale_evidence["androidApi"] = json!(34);
        let stale_evidence: RomMergeEvidence = serde_json::from_value(stale_evidence).unwrap();
        assert!(validate_rom_merge_evidence(
            &stale_candidate,
            &report_bytes,
            &stale_evidence,
            temp.path(),
        )
        .expect_err("candidate target facts must match the registered target")
        .contains("target identity"));

        let mut capability_report: Value = serde_json::from_slice(&report_bytes).unwrap();
        capability_report["qualification"]["effectiveCapabilities"]
            .as_array_mut()
            .unwrap()
            .push(json!("future_privileged_capability"));
        let capability_report_bytes = serde_json::to_vec(&capability_report).unwrap();
        let capability_evidence: RomMergeEvidence =
            serde_json::from_slice(&rom_merge_evidence_for_candidate(
                &handle,
                SESSION_ID,
                TARGET_ID,
                &candidate["build"],
                &capability_report_bytes,
            ))
            .unwrap();
        assert!(validate_rom_merge_evidence(
            &candidate,
            &capability_report_bytes,
            &capability_evidence,
            temp.path(),
        )
        .expect_err("unknown capabilities must fail closed")
        .contains("unsupported effective capability"));
    }

    #[test]
    fn rom_merge_attachment_rejects_missing_pin_malformed_bytes_and_replacement() {
        const TARGET_ID: &str =
            "device-target-sha256:ee1d7cbe10b21866fb39ab6aff94d43f29c76ab3f10d083d7e304b6022e82bf9";
        const SESSION_ID: &str = "qualification-session-cccccccccccccccccccccccccccccccc";
        let temp = TempDir::new().expect("temporary repository should be created");
        write_registered_konkr_target_for_test(&temp);
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let (handle, _, evidence_bytes) =
            finalized_rom_merge_candidate(&repository, TARGET_ID, SESSION_ID);
        let oversized_evidence = vec![b' '; MAX_ROM_MERGE_EVIDENCE_BYTES + 1];
        assert!(repository
            .attach_rom_merge_evidence(&handle, &oversized_evidence)
            .expect_err("oversized evidence must be rejected")
            .contains("size"));
        assert!(repository
            .attach_rom_merge_evidence(&handle, &evidence_bytes)
            .expect_err("missing production pin must fail closed")
            .contains("APK pin is unavailable"));
        assert!(repository
            .attach_rom_merge_evidence(&handle, b"{}")
            .expect_err("malformed artifacts must be rejected")
            .contains("malformed"));
        assert!(
            !repository
                .load_candidate(&handle)
                .expect("candidate should remain inspectable")
                .summary()
                .rom_merge_evidence_attached
        );
        assert!(repository.record_run(&handle).is_err());
        assert!(
            calls.calls().is_empty(),
            "rejected evidence must not reach Node"
        );

        write_rom_merge_apk_pin_for_test(&temp);
        repository
            .attach_rom_merge_evidence(&handle, &evidence_bytes)
            .expect("complete evidence should attach after adding the test pin");
        let directory = temp.path().join(CANDIDATE_DIRECTORY).join(&handle);
        let outside = temp.path().join("outside-rom-merge-evidence.json");
        std::fs::write(&outside, &evidence_bytes).expect("outside bytes should be written");
        std::fs::remove_file(directory.join(ROM_MERGE_EVIDENCE_FILE))
            .expect("candidate artifact should be replaced for the symlink case");
        std::os::unix::fs::symlink(&outside, directory.join(ROM_MERGE_EVIDENCE_FILE))
            .expect("candidate artifact symlink should be created");
        assert!(repository.load_candidate(&handle).is_err());
        assert!(repository.record_run(&handle).is_err());
        assert!(
            calls.calls().is_empty(),
            "replaced evidence must not reach Node"
        );
    }

    #[test]
    fn interrupted_rom_merge_publication_recovers_the_exact_three_file_set() {
        const TARGET_ID: &str =
            "device-target-sha256:ee1d7cbe10b21866fb39ab6aff94d43f29c76ab3f10d083d7e304b6022e82bf9";
        const SESSION_ID: &str = "qualification-session-cccccccccccccccccccccccccccccccc";
        let temp = TempDir::new().expect("temporary repository should be created");
        write_registered_konkr_target_for_test(&temp);
        write_rom_merge_apk_pin_for_test(&temp);
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let (handle, report_bytes, evidence_bytes) =
            finalized_rom_merge_candidate(&repository, TARGET_ID, SESSION_ID);
        repository
            .fail_after_candidate_envelope_publication
            .store(true, Ordering::SeqCst);
        assert!(repository
            .attach_rom_merge_evidence(&handle, &evidence_bytes)
            .expect_err("injected interruption should leave recoverable intent")
            .contains("injected interruption"));
        let recovered = repository
            .load_candidate(&handle)
            .expect("the next read should recover the staged publication");
        assert!(recovered.summary().rom_merge_evidence_attached);
        assert!(recovered.promotable);
        let directory = temp.path().join(CANDIDATE_DIRECTORY).join(&handle);
        assert_eq!(
            std::fs::read(directory.join(ROM_MERGE_EVIDENCE_FILE)).unwrap(),
            evidence_bytes
        );
        assert_eq!(
            std::fs::read(directory.join(EXECUTION_REPORT_FILE)).unwrap(),
            report_bytes
        );
        assert!(directory.join(CANDIDATE_FILE).is_file());
        assert!(!directory.join(CANDIDATE_PUBLICATION_FILE).exists());
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
        #[cfg(debug_assertions)]
        assert!(production_repo_root()
            .join("tools/device-qualification.mjs")
            .is_file());
    }

    #[test]
    fn release_build_does_not_resolve_the_source_repository() {
        if !cfg!(debug_assertions) {
            assert!(QualificationRepository::production().is_none());
        }
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
        repository
            .load_candidate(&handle)
            .expect("report candidate should reload");

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
    fn interrupted_candidate_and_report_publication_recovers_before_listing() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository = QualificationRepository::new_for_test(
            temp.path().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let captured_at = "2026-08-23T12:00:00Z";
        let initial_payload = json!({
            "capturedAt": captured_at,
            "build": build_identity_json(),
        });
        let handle = repository
            .create_candidate(CandidateKind::QualificationRun, &initial_payload, None)
            .expect("provisional candidate should be stored");
        let report = b"{\"status\":\"completed\"}";
        let final_payload = json!({
            "capturedAt": captured_at,
            "build": build_identity_json(),
            "artifacts": [{
                "path": "execution-report.json",
                "sha256": hex::encode(sha2::Sha256::digest(report)),
            }],
        });

        repository.fail_after_candidate_envelope_publication_for_test();
        assert!(repository
            .finalize_candidate(
                &handle,
                CandidateKind::QualificationRun,
                &final_payload,
                Some(report),
            )
            .is_err());

        let reopened = QualificationRepository::new_for_test(
            repository.repo_root().to_path_buf(),
            Box::new(FakeQualificationToolRunner::default()),
        );
        let listed = reopened
            .list_candidates()
            .expect("restart should complete the pending candidate publication");
        assert_eq!(listed.len(), 1);
        let stored = reopened
            .load_candidate(&handle)
            .expect("the recovered candidate should remain readable");
        assert_eq!(
            std::fs::read(
                reopened
                    .candidate_root()
                    .join(&handle)
                    .join(EXECUTION_REPORT_FILE),
            )
            .expect("execution report should be retained"),
            report
        );
        assert!(stored.report.is_some());
        assert!(!reopened
            .candidate_root()
            .join(&handle)
            .join(CANDIDATE_PUBLICATION_FILE)
            .exists());
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
    fn provisional_run_candidate_is_not_promotable_or_recorded() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::with_response(json!({
            "operation": "record_run",
            "candidateHandle": "unexpected",
            "candidateKind": "qualification_run",
            "payload": { "runId": "unexpected" },
        }));
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let handle = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": build_identity_json(),
                    "workflowId": "retroarch-plus-bios",
                }),
                None,
            )
            .expect("provisional run candidate should be stored");
        let candidate_path = repository
            .candidate_root()
            .join(&handle)
            .join(CANDIDATE_FILE);
        let before = std::fs::read(&candidate_path).expect("candidate should be readable");

        let error = repository
            .record_run(&handle)
            .expect_err("provisional run candidates must not be recorded");

        assert!(error.contains("terminal"));
        assert!(calls.calls().is_empty());
        assert_eq!(
            std::fs::read(&candidate_path).expect("candidate should remain readable"),
            before
        );
        let summary = repository
            .list_candidates()
            .expect("provisional candidate should remain inspectable");
        assert!(!summary[0].promotable);
        assert!(summary[0]
            .non_promotable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("terminal")));
    }

    #[test]
    fn terminal_invalid_not_observed_run_candidate_can_be_recorded() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let handle = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": build_identity_json(),
                    "runValidity": "invalid",
                    "qualificationOutcome": "not_observed",
                }),
                None,
            )
            .expect("terminal invalid candidate should be stored");
        calls.set_response(json!({
            "operation": "record_run",
            "candidateHandle": handle,
            "candidateKind": "qualification_run",
            "payload": { "runId": "qualification-run-opaque" },
        }));

        repository
            .record_run(&handle)
            .expect("terminal invalid candidates remain recordable");
        assert_eq!(calls.calls()[0].1, vec!["--record-run".to_string(), handle]);
    }

    #[test]
    fn restricted_report_binding_uses_registered_target_and_konkr_veto_is_scoped() {
        const AYANEO_TARGET_ID: &str =
            "device-target-sha256:7db2d1094b12e20600c2c9bf920c41bef07700b2131f9648fe2fa1f7e9cebafa";
        const KONKR_TARGET_ID: &str =
            "device-target-sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        write_registered_targets_for_test(
            &temp,
            &[
                (AYANEO_TARGET_ID, "ayaneo.pocket_air_mini"),
                (KONKR_TARGET_ID, "konkr.pocket_advance"),
            ],
        );

        let ayaneo_report =
            rust_execution_report_fixture_for_target(AYANEO_TARGET_ID, "ayaneo.pocket_air_mini");
        let ayaneo_candidate = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "rom-library-sync",
                    AYANEO_TARGET_ID,
                    "ayaneo.pocket_air_mini",
                    &ayaneo_report,
                ),
                Some(&ayaneo_report),
            )
            .expect("registered non-KONKR report should be stored");
        assert!(
            repository
                .load_candidate(&ayaneo_candidate)
                .expect("registered non-KONKR candidate should load")
                .promotable
        );

        let konkre_report =
            rust_execution_report_fixture_for_target(KONKR_TARGET_ID, "konkr.pocket_advance");
        let konkre_candidate = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "rom-library-sync",
                    KONKR_TARGET_ID,
                    "konkr.pocket_advance",
                    &konkre_report,
                ),
                Some(&konkre_report),
            )
            .expect("KONKR candidate should be stored for policy validation");
        let stored = repository
            .load_candidate(&konkre_candidate)
            .expect("KONKR candidate should remain inspectable without merge evidence");
        assert!(!stored.promotable);
        assert!(stored
            .non_promotable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("ROM Merge evidence is required")));
        let error = repository
            .record_run(&konkre_candidate)
            .expect_err("passing KONKR ROM-library-sync evidence must be vetoed");
        assert!(error.contains("ROM Merge evidence is required"));
        assert!(calls.calls().is_empty(), "the veto must run before Node");

        let mismatched_report =
            rust_execution_report_fixture_for_target(KONKR_TARGET_ID, "ayaneo.pocket_air_mini");
        let mismatched_candidate = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "rom-library-sync",
                    KONKR_TARGET_ID,
                    "ayaneo.pocket_air_mini",
                    &mismatched_report,
                ),
                Some(&mismatched_report),
            )
            .expect("mismatched candidate should be retained for inspection");
        let stored = repository
            .load_candidate(&mismatched_candidate)
            .expect("mismatched candidate should remain inspectable");
        assert!(!stored.promotable);
        assert!(stored
            .non_promotable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("identity")));
        assert!(calls.calls().is_empty(), "the mismatch must not reach Node");

        let non_rom_candidate = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "retroarch-plus-bios",
                    KONKR_TARGET_ID,
                    "konkr.pocket_advance",
                    &konkre_report,
                ),
                Some(&konkre_report),
            )
            .expect("KONKR non-ROM candidate should be stored");
        assert!(
            repository
                .load_candidate(&non_rom_candidate)
                .expect("KONKR non-ROM candidate should remain promotable")
                .promotable
        );

        let ordinary_report = rust_ordinary_execution_report_fixture_for_target();
        let mismatched_ordinary = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "rom-library-sync",
                    KONKR_TARGET_ID,
                    "ayaneo.pocket_air_mini",
                    &ordinary_report,
                ),
                Some(&ordinary_report),
            )
            .expect("ordinary report with mismatched target should remain inspectable");
        let error = repository
            .record_run(&mismatched_ordinary)
            .expect_err("ordinary evidence must not bypass trusted target binding");
        assert!(error.contains("unique registered target"));
        assert!(
            calls.calls().is_empty(),
            "mismatched ordinary evidence must not reach Node"
        );

        let ordinary_konkr = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "rom-library-sync",
                    KONKR_TARGET_ID,
                    "konkr.pocket_advance",
                    &ordinary_report,
                ),
                Some(&ordinary_report),
            )
            .expect("ordinary KONKR report should remain inspectable");
        let error = repository
            .record_run(&ordinary_konkr)
            .expect_err("the no-claim path must still apply the KONKR veto");
        assert!(error.contains("ROM Merge evidence"));
        assert!(
            calls.calls().is_empty(),
            "KONKR ordinary evidence must not reach Node"
        );

        let ordinary_ayaneo = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "rom-library-sync",
                    AYANEO_TARGET_ID,
                    "ayaneo.pocket_air_mini",
                    &ordinary_report,
                ),
                Some(&ordinary_report),
            )
            .expect("ordinary AYANEO report should remain recordable");
        calls.set_response(json!({
            "operation": "record_run",
            "candidateHandle": ordinary_ayaneo,
            "candidateKind": "qualification_run",
            "payload": { "runId": "ordinary-allowed" },
        }));
        repository
            .record_run(&ordinary_ayaneo)
            .expect("a matching registered non-KONKR ROM-sync target remains recordable");
        assert_eq!(calls.calls().len(), 1);

        let ordinary_konkr_non_rom = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &report_backed_candidate(
                    "retroarch-plus-bios",
                    KONKR_TARGET_ID,
                    "konkr.pocket_advance",
                    &ordinary_report,
                ),
                Some(&ordinary_report),
            )
            .expect("ordinary KONKR non-ROM report should remain recordable");
        assert!(
            repository
                .load_candidate(&ordinary_konkr_non_rom)
                .expect("ordinary KONKR non-ROM candidate should load")
                .promotable
        );
    }

    #[test]
    fn recovery_audit_only_marker_survives_session_cleanup_and_blocks_recording() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let runner = FakeQualificationToolRunner::default();
        let calls = runner.clone();
        let repository = repository_with_embedded_build(&temp, runner);
        let handle = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": build_identity_json(),
                    "runValidity": "invalid",
                    "qualificationOutcome": "not_observed",
                }),
                None,
            )
            .expect("provisional run candidate should be stored");

        repository
            .mark_candidate_audit_only(&handle)
            .expect("recovery audit-only status should be retained durably");
        repository
            .remove_session(&handle)
            .expect("session state should be removable");
        repository
            .remove_session_report(&handle)
            .expect("session report should be removable");

        let restarted = repository_with_embedded_build(&temp, calls.clone());
        let stored = restarted
            .load_candidate(&handle)
            .expect("audit-only candidate should remain readable after restart");
        assert!(!stored.promotable);
        assert_eq!(
            stored.non_promotable_reason.as_deref(),
            Some("recovered invalid qualification attempts cannot be promoted")
        );
        assert_eq!(
            restarted.record_run(&handle).unwrap_err(),
            "recovered invalid qualification attempts cannot be promoted"
        );
        assert!(
            calls.calls().is_empty(),
            "Node must not be invoked for audit-only evidence"
        );
        restarted
            .discard_candidate(&handle)
            .expect("audit-only candidates must remain discardable");
        assert!(restarted.load_candidate(&handle).is_err());
    }

    #[test]
    fn recordable_recipe_digest_capture_accepts_clean_source_and_rejects_dirty_source() {
        let temp = TempDir::new().expect("temporary repository should be created");
        fs::create_dir_all(temp.path().join("authored/recipes"))
            .expect("recipe directory should be created");
        fs::write(
            temp.path().join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\n",
        )
        .expect("recipe should be written");
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let recipe_ids = vec!["test.recipe".to_string()];

        let digests = repository
            .capture_recordable_authored_recipe_digests(&recipe_ids)
            .expect("clean embedded source should produce a trusted digest");
        assert_eq!(digests.len(), 1);
        assert_eq!(digests[0].id, "test.recipe");

        repository.set_source_state_for_test(QualificationSourceState {
            head: "2".repeat(40),
            tracked_worktree_clean: true,
        });
        assert!(repository
            .capture_recordable_authored_recipe_digests(&recipe_ids)
            .is_err());
    }

    #[test]
    fn recordable_recipe_digest_capture_rechecks_source_after_recipe_read() {
        let temp = TempDir::new().expect("temporary repository should be created");
        fs::create_dir_all(temp.path().join("authored/recipes"))
            .expect("recipe directory should be created");
        let recipe_path = temp.path().join("authored/recipes/test.recipe.yaml");
        fs::write(&recipe_path, b"id: test.recipe\n").expect("recipe should be written");
        let repository = std::sync::Arc::new(repository_with_embedded_build(
            &temp,
            FakeQualificationToolRunner::default(),
        ));
        let repository_for_hook = std::sync::Arc::downgrade(&repository);
        let path_for_hook = recipe_path.clone();
        repository.set_authored_recipe_digest_capture_hook_for_test(std::sync::Arc::new(
            move || {
                fs::write(
                    &path_for_hook,
                    b"id: test.recipe\n# changed during capture\n",
                )
                .expect("recipe mutation should be injected during digest capture");
                repository_for_hook
                    .upgrade()
                    .expect("repository should remain alive during capture")
                    .set_source_state_for_test(QualificationSourceState {
                        head: "2".repeat(40),
                        tracked_worktree_clean: false,
                    });
            },
        ));

        let error = repository
            .capture_recordable_authored_recipe_digests(&["test.recipe".to_string()])
            .expect_err("the second source check must reject a mutation during digest capture");
        assert!(error.contains("source state changed"));
    }

    #[test]
    fn authored_recipe_digest_match_requires_clean_source_and_exact_recipe_bytes() {
        let temp = TempDir::new().expect("temporary repository should be created");
        fs::create_dir_all(temp.path().join("authored/recipes"))
            .expect("recipe directory should be created");
        let recipe_path = temp.path().join("authored/recipes/test.recipe.yaml");
        fs::write(&recipe_path, b"id: test.recipe\n").expect("recipe should be written");
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let recipe_ids = vec!["test.recipe".to_string()];
        let expected = repository
            .capture_authored_recipe_digests(&recipe_ids)
            .expect("expected recipe digest should be captured");

        assert!(repository.authored_recipe_digests_match(&expected));

        fs::write(&recipe_path, b"id: test.recipe\n# changed\n")
            .expect("changed recipe should be written");
        assert!(!repository.authored_recipe_digests_match(&expected));

        fs::write(&recipe_path, b"id: test.recipe\n").expect("recipe should be restored");
        repository.set_source_state_for_test(QualificationSourceState {
            head: "2".repeat(40),
            tracked_worktree_clean: false,
        });
        assert!(!repository.authored_recipe_digests_match(&expected));
    }

    #[test]
    fn authored_recipe_digest_match_rechecks_source_after_recipe_capture() {
        let temp = TempDir::new().expect("temporary repository should be created");
        fs::create_dir_all(temp.path().join("authored/recipes"))
            .expect("recipe directory should be created");
        fs::write(
            temp.path().join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\n",
        )
        .expect("recipe should be written");
        let repository = std::sync::Arc::new(repository_with_embedded_build(
            &temp,
            FakeQualificationToolRunner::default(),
        ));
        let expected = repository
            .capture_authored_recipe_digests(&["test.recipe".to_string()])
            .expect("expected recipe digest should be captured");
        let repository_for_hook = std::sync::Arc::downgrade(&repository);
        let source_checks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let checks_for_hook = source_checks.clone();
        repository.set_source_state_read_hook_for_test(std::sync::Arc::new(move || {
            if checks_for_hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                repository_for_hook
                    .upgrade()
                    .expect("repository should remain alive during digest matching")
                    .set_source_state_for_test(QualificationSourceState {
                        head: "2".repeat(40),
                        tracked_worktree_clean: false,
                    });
            }
        }));

        assert!(!repository.authored_recipe_digests_match(&expected));
        assert_eq!(source_checks.load(std::sync::atomic::Ordering::SeqCst), 2);
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
        assert!(repository.discard_untrusted_candidate(&handle).is_err());
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

    #[test]
    fn persisted_session_survives_repository_restart_without_becoming_candidate_data() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let candidate_handle = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": build_identity_json(),
                }),
                None,
            )
            .expect("session candidate should be stored");
        let mut persisted =
            QualificationSession::for_test(&["device_behavior_verified"]).to_persisted();
        persisted.candidate_handle = candidate_handle.clone();
        repository
            .save_session(&candidate_handle, &persisted)
            .expect("session should be persisted");

        let restarted =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let restored = restarted
            .load_session(&candidate_handle)
            .expect("session should survive restart");
        assert_eq!(restored, persisted);
        assert!(restarted
            .candidate_root()
            .join(&candidate_handle)
            .join("session.json")
            .is_file());
        let candidate = restarted
            .load_candidate(&candidate_handle)
            .expect("candidate should remain loadable");
        assert!(candidate.payload.get("sessionHandle").is_none());
    }

    #[test]
    fn raw_session_load_rejects_a_session_substituted_from_another_candidate() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let create = || {
            repository
                .create_candidate(
                    CandidateKind::QualificationRun,
                    &json!({
                        "capturedAt": "2026-08-23T12:00:00Z",
                        "build": build_identity_json(),
                    }),
                    None,
                )
                .expect("candidate should be stored")
        };
        let source = create();
        let destination = create();
        let mut persisted =
            QualificationSession::for_test(&["device_behavior_verified"]).to_persisted();
        persisted.candidate_handle = source.clone();
        repository
            .save_session(&source, &persisted)
            .expect("source session should be persisted");
        let source_path = repository.candidate_root().join(&source).join(SESSION_FILE);
        let destination_path = repository
            .candidate_root()
            .join(&destination)
            .join(SESSION_FILE);
        std::fs::copy(source_path, destination_path)
            .expect("test should substitute one candidate session for another");

        assert!(repository.load_session_json(&destination).is_err());
    }

    #[test]
    fn removing_session_and_terminal_report_survives_repository_restart() {
        let temp = TempDir::new().expect("temporary repository should be created");
        let repository =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        let candidate_handle = repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": "2026-08-23T12:00:00Z",
                    "build": build_identity_json(),
                }),
                None,
            )
            .expect("session candidate should be stored");
        let mut persisted =
            QualificationSession::for_test(&["device_behavior_verified"]).to_persisted();
        persisted.candidate_handle = candidate_handle.clone();
        repository
            .save_session(&candidate_handle, &persisted)
            .expect("session should be persisted");
        repository
            .save_session_report(&candidate_handle, b"{\"status\":\"succeeded\"}")
            .expect("terminal report should be persisted");

        repository
            .remove_session(&candidate_handle)
            .expect("session removal should be durable");
        repository
            .remove_session_report(&candidate_handle)
            .expect("report removal should be durable");

        let restarted =
            repository_with_embedded_build(&temp, FakeQualificationToolRunner::default());
        assert!(restarted.load_session(&candidate_handle).is_err());
        assert_eq!(
            restarted
                .load_session_report(&candidate_handle)
                .expect("removed report should remain absent"),
            None
        );
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
