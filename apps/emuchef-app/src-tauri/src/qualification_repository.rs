//! Fixed-root persistence and bounded access to the canonical qualification tool.
//!
//! Candidate files are deliberately treated as non-authoritative handoff data.
//! Rust owns only the local storage boundary and the small amount of integrity
//! checking needed to recover that data safely. The Node tool remains the
//! authority for candidate semantics, canonical digests, and repository state.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::qualification_build::{embedded_build_identity, QualificationBuildIdentity};

/// Prefix shared by every opaque candidate handle.
pub(crate) const CANDIDATE_HANDLE_PREFIX: &str = "qualification-candidate-";
const CANDIDATE_HANDLE_HEX_LENGTH: usize = 32;
const CANDIDATE_SCHEMA_VERSION: u64 = 1;
const CANDIDATE_DIRECTORY: &str = ".emuchef_runtime/qualification-candidates";
const QUALIFICATION_TOOL: &str = "tools/device-qualification.mjs";
const EXECUTION_REPORT_FILE: &str = "execution-report.json";
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

/// A candidate loaded from the fixed runtime directory.
#[derive(Clone, Debug)]
pub struct StoredQualificationCandidate {
    pub(crate) handle: String,
    pub(crate) kind: CandidateKind,
    pub(crate) json: Value,
    pub(crate) report_bytes: Option<Vec<u8>>,
    pub(crate) promotable: bool,
    pub(crate) non_promotable_reason: Option<String>,
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

/// Narrow seam used to test the repository without starting Node.
pub trait QualificationToolRunner: Send + Sync {
    fn run(&self, repo_root: &Path, args: &[String]) -> Result<Vec<u8>, String>;
}

/// Candidate persistence and bounded access to the canonical Node tool.
pub struct QualificationRepository {
    repo_root: PathBuf,
    candidate_root: PathBuf,
    runner: Box<dyn QualificationToolRunner>,
}

impl QualificationRepository {
    /// Builds the production repository using the compile-time trusted root.
    pub fn production() -> Self {
        Self::with_root(
            production_repo_root(),
            Box::new(ProcessQualificationToolRunner),
        )
    }

    /// Builds a repository with an injected runner for unit tests.
    #[cfg(test)]
    pub fn new_for_test(repo_root: PathBuf, runner: Box<dyn QualificationToolRunner>) -> Self {
        Self::with_root(repo_root, runner)
    }

    fn with_root(repo_root: PathBuf, runner: Box<dyn QualificationToolRunner>) -> Self {
        let repo_root = fs::canonicalize(&repo_root).unwrap_or(repo_root);
        Self {
            candidate_root: repo_root.join(CANDIDATE_DIRECTORY),
            repo_root,
            runner,
        }
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
        let mut candidate = json
            .as_object()
            .cloned()
            .ok_or_else(|| "qualification candidate must be a JSON object".to_string())?;
        ensure_candidate_root(&self.repo_root, &self.candidate_root)?;

        for _ in 0..4 {
            let handle = new_candidate_handle();
            let directory = self.candidate_root.join(&handle);
            match fs::create_dir(&directory) {
                Ok(()) => {
                    candidate.insert(
                        "candidateSchemaVersion".to_string(),
                        Value::from(CANDIDATE_SCHEMA_VERSION),
                    );
                    candidate.insert("candidateId".to_string(), Value::from(handle.clone()));
                    candidate.insert("kind".to_string(), Value::from(kind.as_str()));

                    let mut bytes =
                        serde_json::to_vec_pretty(&Value::Object(candidate)).map_err(|_| {
                            "qualification candidate could not be serialized".to_string()
                        })?;
                    bytes.push(b'\n');
                    if let Err(error) = atomic_write(&directory.join("candidate.json"), &bytes) {
                        let _ = fs::remove_dir_all(&directory);
                        return Err(error);
                    }
                    if let Some(report_bytes) = report_bytes {
                        if let Err(error) =
                            atomic_write(&directory.join(EXECUTION_REPORT_FILE), report_bytes)
                        {
                            let _ = fs::remove_dir_all(&directory);
                            return Err(error);
                        }
                    }
                    return Ok(handle);
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
        if !self.candidate_root.exists() {
            return Ok(Vec::new());
        }
        let root = existing_candidate_root(&self.repo_root, &self.candidate_root)?;
        let mut handles = Vec::new();
        for entry in fs::read_dir(root)
            .map_err(|_| "qualification candidates could not be listed".to_string())?
        {
            let entry =
                entry.map_err(|_| "qualification candidates could not be listed".to_string())?;
            if entry
                .file_type()
                .map_err(|_| "qualification candidate metadata could not be read".to_string())?
                .is_dir()
            {
                let handle = entry.file_name().to_string_lossy().into_owned();
                if validate_candidate_handle(&handle).is_ok() {
                    handles.push(handle);
                }
            }
        }
        handles.sort();

        handles
            .into_iter()
            .map(|handle| {
                self.load_candidate(&handle)
                    .map(|candidate| candidate.summary())
            })
            .collect()
    }

    /// Loads one candidate and rechecks only Rust-owned local integrity.
    pub fn load_candidate(&self, handle: &str) -> Result<StoredQualificationCandidate, String> {
        let directory = self.candidate_directory(handle)?;
        let candidate_bytes = fs::read(directory.join("candidate.json"))
            .map_err(|_| "qualification candidate could not be read".to_string())?;
        let candidate: Value = serde_json::from_slice(&candidate_bytes)
            .map_err(|_| "qualification candidate JSON is invalid".to_string())?;
        let candidate_object = candidate
            .as_object()
            .ok_or_else(|| "qualification candidate must be a JSON object".to_string())?;
        if candidate_object.get("candidateId").and_then(Value::as_str) != Some(handle) {
            return Err(
                "qualification candidate identity does not match its directory".to_string(),
            );
        }
        if candidate_object
            .get("candidateSchemaVersion")
            .and_then(Value::as_u64)
            != Some(CANDIDATE_SCHEMA_VERSION)
        {
            return Err("qualification candidate schema version is unsupported".to_string());
        }
        let kind = serde_json::from_value::<CandidateKind>(
            candidate_object
                .get("kind")
                .cloned()
                .ok_or_else(|| "qualification candidate kind is missing".to_string())?,
        )
        .map_err(|_| "qualification candidate kind is invalid".to_string())?;

        let report_path = directory.join(EXECUTION_REPORT_FILE);
        let report_metadata = candidate_declares_report(&candidate)?;
        let report_bytes = match fs::symlink_metadata(&report_path) {
            Ok(metadata) if !metadata.is_file() => {
                return Err("qualification execution report is not a regular file".to_string());
            }
            Ok(_) if !report_metadata => {
                return Err(
                    "qualification execution report is not declared by the candidate".to_string(),
                );
            }
            Ok(_) => Some(
                fs::read(report_path)
                    .map_err(|_| "qualification execution report could not be read".to_string())?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && report_metadata => {
                return Err("qualification execution report is missing".to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err("qualification execution report could not be inspected".to_string())
            }
        };
        let (promotable, non_promotable_reason) = candidate_promotion_status(&candidate);

        Ok(StoredQualificationCandidate {
            handle: handle.to_string(),
            kind,
            json: candidate,
            report_bytes,
            promotable,
            non_promotable_reason,
        })
    }

    /// Removes one validated candidate directory beneath the fixed root.
    pub fn discard_candidate(&self, handle: &str) -> Result<(), String> {
        let directory = self.candidate_directory(handle)?;
        fs::remove_dir_all(directory)
            .map_err(|_| "qualification candidate could not be discarded".to_string())
    }

    /// Invokes the canonical tool's bounded repository description operation.
    pub fn describe(&self) -> Result<RepositoryQualificationDescription, String> {
        let output = self.invoke(vec!["--describe".to_string()])?;
        serde_json::from_slice(&output)
            .map_err(|_| "qualification repository description is invalid".to_string())
    }

    /// Invokes the canonical tool for a target-registration candidate.
    pub fn register_target(&self, handle: &str) -> Result<Value, String> {
        self.require_kind(handle, CandidateKind::TargetRegistration)?;
        let output = self.invoke(vec!["--register-target".to_string(), handle.to_string()])?;
        parse_tool_json(&output)
    }

    /// Invokes the canonical tool for a qualification-run candidate.
    pub fn record_run(&self, handle: &str) -> Result<Value, String> {
        self.require_kind(handle, CandidateKind::QualificationRun)?;
        let output = self.invoke(vec!["--record-run".to_string(), handle.to_string()])?;
        parse_tool_json(&output)
    }

    fn require_kind(&self, handle: &str, expected: CandidateKind) -> Result<(), String> {
        let candidate = self.load_candidate(handle)?;
        if candidate.kind != expected {
            return Err(
                "qualification candidate kind does not match the requested operation".to_string(),
            );
        }
        Ok(())
    }

    fn candidate_directory(&self, handle: &str) -> Result<PathBuf, String> {
        validate_candidate_handle(handle)?;
        let root = existing_candidate_root(&self.repo_root, &self.candidate_root)?;
        let directory = self.candidate_root.join(handle);
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|_| "qualification candidate does not exist".to_string())?;
        if !metadata.is_dir() {
            return Err("qualification candidate directory is invalid".to_string());
        }
        let canonical_directory = fs::canonicalize(&directory)
            .map_err(|_| "qualification candidate directory is invalid".to_string())?;
        if !canonical_directory.starts_with(&root) {
            return Err("qualification candidate directory is outside the fixed root".to_string());
        }
        Ok(directory)
    }

    fn invoke(&self, args: Vec<String>) -> Result<Vec<u8>, String> {
        if !allowlisted_tool_args(&args) {
            return Err("qualification tool operation is not allowlisted".to_string());
        }
        self.runner.run(&self.repo_root, &args)
    }
}

impl StoredQualificationCandidate {
    fn summary(&self) -> QualificationCandidateSummary {
        QualificationCandidateSummary {
            candidate_handle: self.handle.clone(),
            kind: self.kind,
            captured_at: self
                .json
                .get("capturedAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            promotable: self.promotable,
            non_promotable_reason: self.non_promotable_reason.clone(),
            target: self.json.get("target").cloned(),
            run_validity: self
                .json
                .get("runValidity")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            qualification_outcome: self
                .json
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
        let tool = repo_root.join(QUALIFICATION_TOOL);
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
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    fs::canonicalize(&root).expect("the EmuChef repository root must exist")
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
    fs::create_dir_all(candidate_root)
        .map_err(|_| "qualification candidate root could not be created".to_string())?;
    let canonical_repo = fs::canonicalize(repo_root)
        .map_err(|_| "qualification repository root is unavailable".to_string())?;
    let canonical_root = fs::canonicalize(candidate_root)
        .map_err(|_| "qualification candidate root is unavailable".to_string())?;
    if !canonical_root.starts_with(&canonical_repo) {
        return Err("qualification candidate root is outside the repository".to_string());
    }
    Ok(())
}

fn existing_candidate_root(repo_root: &Path, candidate_root: &Path) -> Result<PathBuf, String> {
    if !candidate_root.exists() {
        return Err("qualification candidate root does not exist".to_string());
    }
    ensure_candidate_root(repo_root, candidate_root)?;
    fs::canonicalize(candidate_root)
        .map_err(|_| "qualification candidate root is unavailable".to_string())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "qualification candidate file name is invalid".to_string())?;
    let temporary = path.with_file_name(format!(".{file_name}.tmp-{}", Uuid::new_v4().simple()));
    fs::write(&temporary, bytes)
        .map_err(|_| "qualification candidate file could not be written".to_string())?;
    if fs::rename(&temporary, path).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("qualification candidate file could not be committed".to_string());
    }
    Ok(())
}

fn candidate_declares_report(candidate: &Value) -> Result<bool, String> {
    let Some(artifacts) = candidate.get("artifacts") else {
        return Ok(false);
    };
    let artifacts = artifacts
        .as_array()
        .ok_or_else(|| "qualification candidate artifacts are invalid".to_string())?;
    Ok(artifacts.iter().any(|artifact| {
        artifact.get("path").and_then(Value::as_str) == Some(EXECUTION_REPORT_FILE)
    }))
}

fn candidate_promotion_status(candidate: &Value) -> (bool, Option<String>) {
    let Some(build) = candidate.get("build") else {
        return (
            false,
            Some("qualification candidate build identity is missing".to_string()),
        );
    };
    let candidate_build = match serde_json::from_value::<QualificationBuildIdentity>(build.clone())
    {
        Ok(build) => build,
        Err(_) => {
            return (
                false,
                Some("qualification candidate build identity is invalid".to_string()),
            )
        }
    };
    let Some(embedded_build) = embedded_build_identity() else {
        return (
            false,
            Some("this application has no recordable qualification build identity".to_string()),
        );
    };
    if candidate_build != embedded_build {
        return (
            false,
            Some("qualification candidate build identity does not match this build".to_string()),
        );
    }
    (true, None)
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

fn parse_tool_json(output: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(output)
        .map_err(|_| "qualification tool returned invalid JSON".to_string())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use serde_json::{json, Value};
    use tempfile::TempDir;

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
                .handle,
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
            std::fs::canonicalize(temp.path())
                .expect("temporary repository should canonicalize")
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
        let json = json!({
            "capturedAt": "2026-08-23T12:00:00Z",
            "build": build_identity_json(),
            "artifacts": [{ "path": "execution-report.json" }],
        });
        let report = b"{\"status\":\"completed\"}";

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
            "artifacts": [{ "path": "execution-report.json" }],
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
        let response = json!({
            "schemaVersion": 1,
            "runtimeContract": "real-execution-v1",
            "qualificationContract": 1,
            "build": build_identity_json(),
            "workflowCatalog": { "schemaVersion": 1, "workflows": [] },
            "deviceTargets": { "schemaVersion": 2, "targets": [] },
        });
        let runner = FakeQualificationToolRunner::with_response(response);
        let calls = runner.clone();
        let repository =
            QualificationRepository::new_for_test(temp.path().to_path_buf(), Box::new(runner));

        let target_handle = repository
            .create_candidate(CandidateKind::TargetRegistration, &json!({}), None)
            .expect("target candidate should be stored");
        let run_handle = repository
            .create_candidate(CandidateKind::QualificationRun, &json!({}), None)
            .expect("run candidate should be stored");

        repository
            .describe()
            .expect("describe should parse the fake response");
        repository
            .register_target(&target_handle)
            .expect("target registration should invoke the tool");
        repository
            .record_run(&run_handle)
            .expect("run recording should invoke the tool");

        let calls = calls.calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, repository.repo_root());
        assert_eq!(calls[0].1, vec!["--describe"]);
        assert_eq!(
            calls[1].1,
            vec!["--register-target".to_string(), target_handle]
        );
        assert_eq!(calls[2].1, vec!["--record-run".to_string(), run_handle]);
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
