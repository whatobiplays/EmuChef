//! Development-only Phase 6D.6 UI-smoke binding and capture bridge.
//!
//! This module lets a later operator run the mandatory development-build UI
//! smoke against already accepted physical backend evidence without replaying
//! a fault through the GUI. It is inert unless every gate is present: a debug
//! build, the `real-execution` Cargo feature, `EMUCHEF_RUN_REAL_ADB_TESTS=1`,
//! and `EMUCHEF_PHASE_6D6_UI_SMOKE=1`.
//!
//! Trusted Rust owns every filesystem decision here. React only receives
//! opaque handles and sanitized labels, and only ever sends opaque handles and
//! a UI repetition back. The bridge verifies the checked-in binding index, the
//! raw evidence/trace bytes, and the parsed record itself before projecting a
//! fixed terminal report through the production `execution.rs` projection.
//! Capture copies the production-authored terminal policy from the projected
//! DTO and writes one canonical, create-new `ui_state_capture` artifact under
//! the fixed `docs/testing/phase-6d6/evidence/ui/` directory. It never starts
//! an executor, allocates an execution slot, sends an ADB command, resumes or
//! replays a run, or creates a final `ui_smoke_composite` record.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::commands::safe_error;
use crate::execution;

const UI_BINDING_INDEX_RELATIVE_PATH: &str = "docs/testing/phase-6d6/ui-binding-index.json";
const EVIDENCE_RELATIVE_DIR: &str = "docs/testing/phase-6d6/evidence";
const INDEX_SCHEMA_VERSION: u64 = 1;
const EVIDENCE_PATH_PREFIX: &str = "docs/testing/phase-6d6/evidence/";
const UI_BINDING_INDEX_SOURCES: [&str; 4] = [
    "docs/testing/phase-6d6/scenario-manifest.json",
    "docs/testing/phase-6d6/evidence-schema.json",
    "apps/emuchef-app/src-tauri/src/execution.rs",
    "tools/phase-6d6-evidence.mjs",
];
const SUBCASES: [&str; 5] = ["cancellation", "transport", "root", "storage", "host_sleep"];
const MAX_CANDIDATES: usize = 32;
const MAX_PROJECTIONS: usize = 16;
const SCENARIOS_BY_SUBCASE: [&[&str]; 5] = [
    &["cancellation_active", "cancellation_boundary"],
    &["usb_disconnect_active", "usb_disconnect_boundary"],
    &["root_revocation"],
    &["low_storage"],
    &["host_sleep_before_deadline", "host_sleep_after_deadline"],
];

/// One binding resolved from the verified checked-in index.
#[derive(Clone, Debug)]
struct VerifiedBinding {
    subcase: String,
    repetition: u8,
    run_id: String,
    scenario: String,
    commit: String,
    issue_code: Option<String>,
    evidence_path: String,
    trace_path: String,
    record_digest: String,
    trace_digest: String,
    evidence_sha256: String,
    trace_sha256: String,
}

/// A projection loaded for one qualification session, retaining the exact
/// backend binding and the trusted DTO that was presented to React.
#[derive(Clone, Debug)]
struct LoadedProjection {
    session: u64,
    binding: VerifiedBinding,
    snapshot: Value,
}

#[derive(Default)]
struct StoreState {
    session: u64,
    bindings: HashMap<String, VerifiedBinding>,
    projections: HashMap<String, LoadedProjection>,
}

/// Session-scoped opaque-handle state for the qualification bridge.
///
/// Every successful status/candidate refresh starts a new session, clears all
/// prior candidate and projection handles, and enforces explicit bounds on
/// both maps. Capture accepts only projections from the current session.
#[derive(Default)]
pub struct Phase6d6UiSmokeStore {
    state: Mutex<StoreState>,
}

fn next_session(state: &mut StoreState) -> u64 {
    state.session = state.session.wrapping_add(1);
    state.bindings.clear();
    state.projections.clear();
    state.session
}

fn insert_binding_locked(
    state: &mut StoreState,
    handle: String,
    binding: VerifiedBinding,
) -> Result<(), String> {
    if state.bindings.len() >= MAX_CANDIDATES {
        return Err("qualification_capacity_exceeded".to_string());
    }
    state.bindings.insert(handle, binding);
    Ok(())
}

fn insert_projection_locked(
    state: &mut StoreState,
    handle: String,
    projection: LoadedProjection,
) -> Result<(), String> {
    if state.projections.len() >= MAX_PROJECTIONS {
        return Err("qualification_capacity_exceeded".to_string());
    }
    state.projections.insert(handle, projection);
    Ok(())
}

/// Resolved qualification roots. Every path is derived inside trusted code;
/// React never supplies a filesystem path.
#[derive(Clone, Debug)]
pub(crate) struct QualificationRoots {
    pub(crate) repo_root: PathBuf,
    pub(crate) index_path: PathBuf,
    pub(crate) evidence_root: PathBuf,
    pub(crate) ui_capture_root: PathBuf,
}

impl QualificationRoots {
    /// Resolve the repository, index, evidence, and UI-capture roots from the
    /// compile-time development location. The evidence root must canonicalize
    /// to a real directory beneath the canonical repository root.
    pub(crate) fn compile_time() -> Result<Self, String> {
        let repo_root =
            fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."))
                .map_err(|_| "qualification_repository_unavailable".to_string())?;
        let evidence_root = fs::canonicalize(repo_root.join(EVIDENCE_RELATIVE_DIR))
            .map_err(|_| "qualification_repository_unavailable".to_string())?;
        if !evidence_root.starts_with(&repo_root) {
            return Err("qualification_repository_unavailable".to_string());
        }
        Ok(Self {
            index_path: repo_root.join(UI_BINDING_INDEX_RELATIVE_PATH),
            evidence_root: evidence_root.clone(),
            ui_capture_root: evidence_root.join("ui"),
            repo_root,
        })
    }
}

/// Pure four-gate decision shared by the Tauri commands and their tests. Both
/// opt-ins must equal the exact string `"1"`; values such as `"true"` or `"0"`
/// never enable qualification.
pub(crate) fn ui_smoke_requested(
    debug_build: bool,
    real_execution: bool,
    global_opt_in: Option<&str>,
    ui_smoke_opt_in: Option<&str>,
) -> bool {
    debug_build && real_execution && global_opt_in == Some("1") && ui_smoke_opt_in == Some("1")
}

/// All four qualification gates must be present before any qualification file
/// is read or written.
pub(crate) fn ui_smoke_enabled() -> bool {
    ui_smoke_requested(
        cfg!(debug_assertions),
        cfg!(feature = "real-execution"),
        std::env::var("EMUCHEF_RUN_REAL_ADB_TESTS").ok().as_deref(),
        std::env::var("EMUCHEF_PHASE_6D6_UI_SMOKE").ok().as_deref(),
    )
}

/// Trusted sanitized candidate label. Distinguishes the physical scenario
/// variant when a subcase can contain more than one, and never exposes paths,
/// run IDs, issue codes, hashes, serials, or raw evidence.
fn candidate_label(subcase: &str, scenario: &str, repetition: u8) -> String {
    let display = match subcase {
        "cancellation" => "Cancellation",
        "transport" => "Transport",
        "root" => "Root",
        "storage" => "Storage",
        "host_sleep" => "Host sleep",
        _ => subcase,
    };
    let variant = match scenario {
        "cancellation_active" | "usb_disconnect_active" => Some("active interruption"),
        "cancellation_boundary" | "usb_disconnect_boundary" => Some("safe boundary"),
        "host_sleep_before_deadline" => Some("before deadline"),
        "host_sleep_after_deadline" => Some("after deadline"),
        _ => None,
    };
    match variant {
        Some(variant) => format!("{display} — {variant} — physical repetition {repetition}"),
        None => format!("{display} — physical repetition {repetition}"),
    }
}

fn qualification_unavailable() -> String {
    safe_error(
        "qualification_unavailable",
        "Phase 6D.6 UI-smoke qualification is not enabled for this build.",
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sha256_digest(text: &str) -> Option<&str> {
    text.strip_prefix("sha256:")
        .filter(|rest| rest.len() == 64 && rest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn run_id_digest(text: &str) -> Option<&str> {
    text.strip_prefix("physical-run-sha256:")
        .filter(|rest| rest.len() == 64 && rest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn is_evidence_json_path(value: &str) -> bool {
    value.starts_with(EVIDENCE_PATH_PREFIX)
        && value.ends_with(".json")
        && !value.contains("..")
        && !value.contains('\\')
}

fn scenario_index(subcase: &str) -> Option<usize> {
    SUBCASES.iter().position(|candidate| *candidate == subcase)
}

/// Parse the authoritative `uiSmokeContracts` table from the source-digested
/// scenario manifest. Values are used only for fail-closed comparison, never
/// to populate a capture.
fn ui_smoke_contracts(repo_root: &Path) -> Result<Value, String> {
    let manifest: Value = serde_json::from_slice(
        &fs::read(repo_root.join("docs/testing/phase-6d6/scenario-manifest.json"))
            .map_err(|_| "ui_binding_index_invalid")?,
    )
    .map_err(|_| "ui_binding_index_invalid")?;
    manifest
        .get("uiSmokeContracts")
        .cloned()
        .ok_or_else(|| "ui_binding_index_invalid".to_string())
}

fn binding_from_index_entry(subcase: &str, entry: &Value) -> Result<VerifiedBinding, String> {
    let index_invalid = || "ui_binding_index_invalid".to_string();
    let get = |key: &str| {
        entry
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(index_invalid)
    };
    let run_id = get("runId")?;
    if run_id_digest(run_id).is_none() {
        return Err(index_invalid());
    }
    let repetition = entry
        .get("repetition")
        .and_then(Value::as_u64)
        .filter(|value| matches!(value, 1 | 2))
        .ok_or_else(index_invalid)?;
    let scenario = get("scenario")?;
    let scenario_set = scenario_index(subcase)
        .and_then(|position| SCENARIOS_BY_SUBCASE.get(position))
        .ok_or_else(index_invalid)?;
    if !scenario_set.contains(&scenario) {
        return Err(index_invalid());
    }
    for digest_key in [
        "recordDigest",
        "traceDigest",
        "evidenceSha256",
        "traceSha256",
    ] {
        if sha256_digest(get(digest_key)?).is_none() {
            return Err(index_invalid());
        }
    }
    let issue_code = match entry.get("issueCode") {
        None | Some(Value::Null) => None,
        Some(Value::String(code)) => Some(code.clone()),
        Some(_) => return Err(index_invalid()),
    };
    let evidence_path = get("evidencePath")?;
    let trace_path = get("tracePath")?;
    if !is_evidence_json_path(evidence_path) || !is_evidence_json_path(trace_path) {
        return Err(index_invalid());
    }
    Ok(VerifiedBinding {
        subcase: subcase.to_string(),
        repetition: repetition as u8,
        run_id: run_id.to_string(),
        scenario: scenario.to_string(),
        commit: get("commit")?.to_string(),
        issue_code,
        evidence_path: evidence_path.to_string(),
        trace_path: trace_path.to_string(),
        record_digest: get("recordDigest")?.to_string(),
        trace_digest: get("traceDigest")?.to_string(),
        evidence_sha256: get("evidenceSha256")?.to_string(),
        trace_sha256: get("traceSha256")?.to_string(),
    })
}

fn index_without_digest(index: &Value) -> Value {
    let mut copy = index.clone();
    if let Some(object) = copy.as_object_mut() {
        object.remove("digest");
    }
    copy
}

/// Verify the checked-in index's self digest, source digests, exact subcase
/// shape, scenario/issue eligibility, and duplicate-free binding identities.
fn verify_index_shape_and_digests(
    index: &Value,
    roots: &QualificationRoots,
    contracts: &Value,
) -> Result<(), String> {
    let index_invalid = || "ui_binding_index_invalid".to_string();
    if index.get("schemaVersion").and_then(Value::as_u64) != Some(INDEX_SCHEMA_VERSION) {
        return Err(index_invalid());
    }
    let expected_digest = format!(
        "sha256:{}",
        execution::canonical_json_digest(&index_without_digest(index))
            .map_err(|_| index_invalid())?
    );
    if index.get("digest").and_then(Value::as_str) != Some(expected_digest.as_str()) {
        return Err(index_invalid());
    }
    let sources = index
        .get("sourceDigests")
        .and_then(Value::as_object)
        .ok_or_else(index_invalid)?;
    if sources.len() != UI_BINDING_INDEX_SOURCES.len() {
        return Err(index_invalid());
    }
    for relative in UI_BINDING_INDEX_SOURCES {
        let raw = fs::read(roots.repo_root.join(relative)).map_err(|_| index_invalid())?;
        let expected = format!("sha256:{}", sha256_hex(&raw));
        if sources.get(relative).and_then(Value::as_str) != Some(expected.as_str()) {
            return Err(index_invalid());
        }
    }
    let bindings = index
        .get("bindings")
        .and_then(Value::as_object)
        .ok_or_else(index_invalid)?;
    if bindings.len() != SUBCASES.len() {
        return Err(index_invalid());
    }
    for subcase in SUBCASES {
        let entries = bindings
            .get(subcase)
            .and_then(Value::as_array)
            .ok_or_else(index_invalid)?;
        let contract = contracts.get(subcase).ok_or_else(index_invalid)?;
        let allowed = contract
            .get("allowedIssueCodes")
            .and_then(Value::as_array)
            .ok_or_else(index_invalid)?;
        let mut seen_runs = HashSet::new();
        let mut seen_paths = HashSet::new();
        for entry in entries {
            let binding = binding_from_index_entry(subcase, entry)?;
            let issue_eligible =
                allowed
                    .iter()
                    .any(|allowed| match (allowed, &binding.issue_code) {
                        (Value::Null, None) => true,
                        (Value::String(code), Some(issue)) => code == issue,
                        _ => false,
                    });
            if !issue_eligible {
                return Err(index_invalid());
            }
            if !seen_runs.insert(binding.run_id.clone()) {
                return Err(index_invalid());
            }
            if !seen_paths.insert(binding.evidence_path.clone())
                || !seen_paths.insert(binding.trace_path.clone())
            {
                return Err(index_invalid());
            }
        }
    }
    Ok(())
}

fn load_verified_index(roots: &QualificationRoots, contracts: &Value) -> Result<Value, String> {
    let index: Value = serde_json::from_slice(
        &fs::read(&roots.index_path).map_err(|_| "ui_binding_index_invalid")?,
    )
    .map_err(|_| "ui_binding_index_invalid")?;
    verify_index_shape_and_digests(&index, roots, contracts)?;
    Ok(index)
}

/// Convert one record-relative evidence path to a path beneath the canonical
/// evidence root, rejecting any escape or unsupported component.
fn evidence_relative_path(record_path: &str) -> Result<&str, String> {
    record_path
        .strip_prefix(EVIDENCE_PATH_PREFIX)
        .filter(|rest| !rest.contains("..") && !rest.starts_with('/') && !rest.contains('\\'))
        .ok_or_else(|| "path_escape".to_string())
}

/// Read one evidence or trace file, rejecting symlinks and path escapes, and
/// return the exact raw bytes for digest comparison.
fn confined_file_bytes(roots: &QualificationRoots, record_path: &str) -> Result<Vec<u8>, String> {
    let canonical_evidence =
        fs::canonicalize(&roots.evidence_root).map_err(|_| "evidence_unavailable")?;
    let relative = Path::new(evidence_relative_path(record_path)?);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("path_escape".to_string());
    }
    let candidate = canonical_evidence.join(relative);
    if candidate.extension().and_then(|value| value.to_str()) != Some("json") {
        return Err("unsupported_file".to_string());
    }
    let metadata = fs::symlink_metadata(&candidate).map_err(|_| "evidence_unavailable")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("evidence_unavailable".to_string());
    }
    let canonical = fs::canonicalize(&candidate).map_err(|_| "evidence_unavailable")?;
    if !canonical.starts_with(&canonical_evidence) {
        return Err("path_escape".to_string());
    }
    fs::read(&canonical).map_err(|_| "evidence_unavailable".to_string())
}

/// Independently recheck the exact indexed evidence/trace raw bytes and the
/// parsed record's run, digest, scenario, repetition, outcome, and issue.
fn verify_binding_files_and_record(
    binding: &VerifiedBinding,
    index: &Value,
    roots: &QualificationRoots,
    contracts: &Value,
) -> Result<(), String> {
    let index_invalid = || "ui_binding_index_invalid".to_string();
    let entries = index["bindings"][binding.subcase.as_str()]
        .as_array()
        .ok_or_else(index_invalid)?;
    let matching = entries
        .iter()
        .find(|entry| entry.get("runId").and_then(Value::as_str) == Some(binding.run_id.as_str()))
        .ok_or_else(index_invalid)?;
    let from_index = binding_from_index_entry(&binding.subcase, matching)?;
    if from_index.repetition != binding.repetition
        || from_index.scenario != binding.scenario
        || from_index.commit != binding.commit
        || from_index.issue_code != binding.issue_code
        || from_index.evidence_path != binding.evidence_path
        || from_index.trace_path != binding.trace_path
        || from_index.record_digest != binding.record_digest
        || from_index.trace_digest != binding.trace_digest
        || from_index.evidence_sha256 != binding.evidence_sha256
        || from_index.trace_sha256 != binding.trace_sha256
    {
        return Err(index_invalid());
    }

    let evidence_bytes = confined_file_bytes(roots, &binding.evidence_path)?;
    if format!("sha256:{}", sha256_hex(&evidence_bytes)) != binding.evidence_sha256 {
        return Err("evidence_digest_mismatch".to_string());
    }
    let trace_bytes = confined_file_bytes(roots, &binding.trace_path)?;
    if format!("sha256:{}", sha256_hex(&trace_bytes)) != binding.trace_sha256 {
        return Err("evidence_digest_mismatch".to_string());
    }

    let record: Value = serde_json::from_slice(&evidence_bytes).map_err(|_| "record_invalid")?;
    verify_parsed_record(binding, &record, contracts)?;

    let trace: Value = serde_json::from_slice(&trace_bytes).map_err(|_| "trace_invalid")?;
    if trace.get("runId").and_then(Value::as_str) != Some(binding.run_id.as_str()) {
        return Err("trace_run_mismatch".to_string());
    }
    let trace_digest = format!(
        "sha256:{}",
        execution::canonical_json_digest(&trace).map_err(|_| "trace_invalid")?
    );
    if trace_digest != binding.trace_digest {
        return Err("trace_digest_mismatch".to_string());
    }
    let embedded_trace = record
        .get("trace")
        .ok_or_else(|| "record_invalid".to_string())?;
    let embedded_digest = format!(
        "sha256:{}",
        execution::canonical_json_digest(embedded_trace).map_err(|_| "record_invalid")?
    );
    if embedded_digest != binding.trace_digest {
        return Err("trace_digest_mismatch".to_string());
    }
    Ok(())
}

/// Recheck every parsed record field the qualification contract depends on.
/// The canonical record digest excludes the record's own `recordDigest` field,
/// matching the Phase 6D.6 evidence validator exactly.
fn verify_parsed_record(
    binding: &VerifiedBinding,
    record: &Value,
    contracts: &Value,
) -> Result<(), String> {
    if record.get("runId").and_then(Value::as_str) != Some(binding.run_id.as_str()) {
        return Err("record_run_mismatch".to_string());
    }
    if record.get("scenario").and_then(Value::as_str) != Some(binding.scenario.as_str()) {
        return Err("record_scenario_mismatch".to_string());
    }
    if record.get("repetition").and_then(Value::as_u64) != Some(u64::from(binding.repetition)) {
        return Err("record_repetition_mismatch".to_string());
    }
    if record.get("outcome").and_then(Value::as_str) != Some("passed") {
        return Err("record_outcome_mismatch".to_string());
    }
    let observed = match record.get("observedIssueCode") {
        None | Some(Value::Null) => None,
        Some(Value::String(code)) => Some(code.as_str()),
        Some(_) => return Err("record_issue_invalid".to_string()),
    };
    if observed != binding.issue_code.as_deref() {
        return Err("record_issue_mismatch".to_string());
    }
    let contract = contracts
        .get(binding.subcase.as_str())
        .ok_or_else(|| "ui_contract_unavailable".to_string())?;
    let allowed = contract
        .get("allowedIssueCodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "ui_contract_unavailable".to_string())?;
    let issue_eligible = allowed.iter().any(|entry| match (entry, observed) {
        (Value::Null, None) => true,
        (Value::String(code), Some(issue)) => code == issue,
        _ => false,
    });
    if !issue_eligible {
        return Err("record_issue_mismatch".to_string());
    }
    let mut without_digest = record.clone();
    if let Some(object) = without_digest.as_object_mut() {
        object.remove("recordDigest");
    }
    let canonical =
        execution::canonical_json_digest(&without_digest).map_err(|_| "record_digest_invalid")?;
    let expected = format!("sha256:{canonical}");
    if expected != binding.record_digest {
        return Err("record_digest_mismatch".to_string());
    }
    if record.get("recordDigest").and_then(Value::as_str) != Some(binding.record_digest.as_str()) {
        return Err("record_digest_mismatch".to_string());
    }
    Ok(())
}

/// Build the fixed, authority-free terminal report for one eligible binding.
///
/// Possible-partial-change subcases project one earlier succeeded step, the
/// interrupted terminal step, and one later pending step so the normal
/// terminal UI shows **Not attempted**. The host-sleep indeterminate branch
/// projects only the failed step and the pending later step and never claims
/// an earlier completed change.
fn fixed_terminal_report(binding: &VerifiedBinding) -> Result<Value, String> {
    let terminal_status = if binding.subcase == "cancellation" {
        "cancelled"
    } else {
        "failed"
    };
    let mut steps = vec![
        json!({"name": "Prepare reviewed setup", "status": "succeeded"}),
        json!({"name": "Apply reviewed changes", "status": terminal_status}),
        json!({"name": "Verify completed setup", "status": "pending"}),
    ];
    if binding.subcase == "host_sleep" {
        steps.remove(0);
    }
    let mut errors = Vec::new();
    if let Some(code) = &binding.issue_code {
        errors.push(json!({ "code": code }));
    }
    Ok(json!({
        "status": terminal_status,
        "startedAt": null,
        "finishedAt": null,
        "latestSequence": 0,
        "recipes": [{
            "recipeId": "phase6d6-qualification",
            "name": "Reviewed device setup",
            "status": terminal_status,
            "steps": steps,
        }],
        "warnings": [],
        "errors": errors,
    }))
}

/// Derive the canonical sanitized `uiState` from the stored trusted DTO, never
/// from the manifest or from React input.
fn derive_ui_state(binding: &VerifiedBinding, snapshot: &Value) -> Result<Value, String> {
    let inconsistent = || "projection_inconsistent".to_string();
    let status = snapshot
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(inconsistent)?;
    let counts = snapshot
        .get("completion")
        .and_then(|completion| completion.get("counts"))
        .ok_or_else(inconsistent)?;
    let not_attempted = counts
        .get("pending")
        .and_then(Value::as_u64)
        .ok_or_else(inconsistent)?;
    let policy = snapshot.get("terminalPolicy").ok_or_else(inconsistent)?;
    let authority_invalidated = policy
        .get("authorityInvalidated")
        .and_then(Value::as_bool)
        .ok_or_else(inconsistent)?;
    let recovery_state = policy
        .get("recoveryState")
        .and_then(Value::as_str)
        .ok_or_else(inconsistent)?;
    let partial_presentation = policy
        .get("partialChangePresentation")
        .and_then(Value::as_str)
        .ok_or_else(inconsistent)?;
    let available_controls = policy
        .get("availableControls")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));

    let (authored_title, authored_issue_text, authored_remediation, terminal_projection) =
        if status == "cancelled" {
            let cancellation = snapshot.get("cancellation").ok_or_else(inconsistent)?;
            let remediation = cancellation.get("remediation").ok_or_else(inconsistent)?;
            (
                cancellation
                    .get("title")
                    .and_then(Value::as_str)
                    .ok_or_else(inconsistent)?
                    .to_string(),
                cancellation
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or_else(inconsistent)?
                    .to_string(),
                remediation
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or_else(inconsistent)?
                    .to_string(),
                "cancelled".to_string(),
            )
        } else if status == "failed" {
            let issue = snapshot
                .get("errors")
                .and_then(Value::as_array)
                .and_then(|errors| errors.first())
                .ok_or_else(inconsistent)?;
            let remediation = issue.get("remediation").ok_or_else(inconsistent)?;
            (
                remediation
                    .get("title")
                    .and_then(Value::as_str)
                    .ok_or_else(inconsistent)?
                    .to_string(),
                issue
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or_else(inconsistent)?
                    .to_string(),
                remediation
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or_else(inconsistent)?
                    .to_string(),
                "failed".to_string(),
            )
        } else {
            return Err(inconsistent());
        };

    Ok(json!({
        "backendRunId": binding.run_id,
        "authoredTitle": authored_title,
        "authoredIssueText": authored_issue_text,
        "authoredRemediation": authored_remediation,
        "terminalStepProjection": terminal_projection,
        "notAttempted": not_attempted,
        "partialChangePresentation": partial_presentation,
        "authorityInvalidated": authority_invalidated,
        "recoveryState": recovery_state,
        "availableControls": available_controls,
    }))
}

/// Fail-closed comparison of the complete derived UI state against the
/// authoritative `uiSmokeContracts` entry. The values themselves still
/// originate from the production terminal projection.
fn verify_ui_state_contract(
    subcase: &str,
    ui_state: &Value,
    contracts: &Value,
) -> Result<(), String> {
    let contract_mismatch = || {
        safe_error(
            "ui_contract_mismatch",
            "The projected terminal state no longer matches the authoritative UI-smoke contract.",
        )
    };
    let contract = contracts.get(subcase).ok_or_else(contract_mismatch)?;
    for field in [
        "authoredTitle",
        "authoredIssueText",
        "authoredRemediation",
        "terminalStepProjection",
        "partialChangePresentation",
        "recoveryState",
    ] {
        if ui_state.get(field).and_then(Value::as_str)
            != contract.get(field).and_then(Value::as_str)
        {
            return Err(contract_mismatch());
        }
    }
    if ui_state
        .get("authorityInvalidated")
        .and_then(Value::as_bool)
        != contract
            .get("authorityInvalidated")
            .and_then(Value::as_bool)
    {
        return Err(contract_mismatch());
    }
    if contract
        .get("notAttemptedRequired")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && ui_state
            .get("notAttempted")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            < 1
    {
        return Err(contract_mismatch());
    }
    let forbidden = contract
        .get("forbiddenControls")
        .and_then(Value::as_array)
        .ok_or_else(contract_mismatch)?;
    let controls = ui_state
        .get("availableControls")
        .and_then(Value::as_array)
        .ok_or_else(contract_mismatch)?;
    if controls.iter().any(|control| forbidden.contains(control)) {
        return Err(contract_mismatch());
    }
    Ok(())
}

/// Trusted development-build identity from package metadata and the running
/// executable bytes. The executable path is never exposed.
fn development_build() -> Result<Value, String> {
    let executable =
        std::env::current_exe().map_err(|_| "build_identity_unavailable".to_string())?;
    let bytes = fs::read(&executable).map_err(|_| "build_identity_unavailable".to_string())?;
    Ok(json!({
        "identity": format!("{}:development-ui-smoke", env!("CARGO_PKG_NAME")),
        "version": env!("CARGO_PKG_VERSION"),
        "digest": format!("sha256:{}", sha256_hex(&bytes)),
    }))
}

/// Resolve a create-new destination beneath a safely created, canonical
/// UI-capture directory that remains inside the canonical evidence root.
fn secure_ui_capture_path(roots: &QualificationRoots, filename: &str) -> Result<PathBuf, String> {
    let body = filename
        .strip_suffix(".json")
        .ok_or_else(|| "capture_destination_invalid".to_string())?;
    if body.is_empty()
        || !body.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
    {
        return Err("capture_destination_invalid".to_string());
    }
    let canonical_evidence =
        fs::canonicalize(&roots.evidence_root).map_err(|_| "capture_root_unavailable")?;
    let ui = &roots.ui_capture_root;
    match fs::symlink_metadata(ui) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(safe_error(
                    "capture_root_invalid",
                    "The UI-state capture directory is not a real directory.",
                ));
            }
        }
        Err(_) => {
            fs::create_dir_all(ui).map_err(|_| "capture_root_unavailable")?;
        }
    }
    let canonical_ui = fs::canonicalize(ui).map_err(|_| "capture_root_unavailable")?;
    if !canonical_ui.starts_with(&canonical_evidence) {
        return Err("capture_root_escape".to_string());
    }
    let destination = canonical_ui.join(filename);
    if let Ok(metadata) = fs::symlink_metadata(&destination) {
        if metadata.file_type().is_symlink() {
            return Err("capture_destination_invalid".to_string());
        }
    }
    Ok(destination)
}

/// Report qualification status and the sanitized candidate set. When enabled
/// but the index/evidence is invalid, this returns `enabled: true,
/// ready: false` with a sanitized message instead of falling back to normal
/// app startup.
fn status_inner(store: &Phase6d6UiSmokeStore, roots: &QualificationRoots) -> Result<Value, String> {
    if !ui_smoke_enabled() {
        return Ok(json!({
            "enabled": false,
            "ready": false,
            "message": null,
            "candidates": [],
        }));
    }
    let contracts = match ui_smoke_contracts(&roots.repo_root) {
        Ok(contracts) => contracts,
        Err(code) => {
            return Ok(json!({
                "enabled": true,
                "ready": false,
                "message": code,
                "candidates": [],
            }));
        }
    };
    let index = match load_verified_index(roots, &contracts) {
        Ok(index) => index,
        Err(code) => {
            return Ok(json!({
                "enabled": true,
                "ready": false,
                "message": code,
                "candidates": [],
            }));
        }
    };
    let mut state = store
        .state
        .lock()
        .map_err(|_| qualification_unavailable())?;
    let session = next_session(&mut state);
    let mut candidates = Vec::new();
    for subcase in SUBCASES {
        let entries = index["bindings"][subcase]
            .as_array()
            .map(|entries| entries.iter())
            .into_iter()
            .flatten();
        for entry in entries {
            let binding = match binding_from_index_entry(subcase, entry) {
                Ok(binding) => binding,
                Err(code) => {
                    return Ok(json!({
                        "enabled": true,
                        "ready": false,
                        "message": code,
                        "candidates": [],
                    }));
                }
            };
            if let Err(code) = verify_binding_files_and_record(&binding, &index, roots, &contracts)
            {
                return Ok(json!({
                    "enabled": true,
                    "ready": false,
                    "message": code,
                    "candidates": [],
                }));
            }
            let handle = format!("phase6d6-binding-{}", Uuid::new_v4().simple());
            if let Err(code) = insert_binding_locked(&mut state, handle.clone(), binding.clone()) {
                return Ok(json!({
                    "enabled": true,
                    "ready": false,
                    "message": code,
                    "candidates": [],
                }));
            }
            candidates.push(json!({
                "subcase": subcase,
                "handle": handle,
                "label": candidate_label(subcase, &binding.scenario, binding.repetition),
                "repetition": binding.repetition,
            }));
        }
    }
    debug_assert_eq!(state.session, session);
    Ok(json!({
        "enabled": true,
        "ready": true,
        "message": null,
        "candidates": candidates,
    }))
}

/// Load one verified binding as a qualification terminal projection. This
/// performs no ADB call, no executor start, no execution-slot acquisition, and
/// no authority allocation.
fn load_inner(
    store: &Phase6d6UiSmokeStore,
    roots: &QualificationRoots,
    binding_handle: &str,
) -> Result<Value, String> {
    if !ui_smoke_enabled() {
        return Err(qualification_unavailable());
    }
    if !binding_handle.starts_with("phase6d6-binding-")
        || !binding_handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(qualification_unavailable());
    }
    let contracts = ui_smoke_contracts(&roots.repo_root)?;
    let index = load_verified_index(roots, &contracts)?;
    let binding = {
        let mut state = store
            .state
            .lock()
            .map_err(|_| qualification_unavailable())?;
        state
            .bindings
            .remove(binding_handle)
            .ok_or_else(qualification_unavailable)?
    };
    verify_binding_files_and_record(&binding, &index, roots, &contracts)?;
    let report = fixed_terminal_report(&binding)?;
    let public_handle = format!("phase6d6-projection-{}", Uuid::new_v4().simple());
    let mut snapshot = execution::project_phase6d6_terminal(&public_handle, report);
    snapshot["launchAction"] = Value::Null;
    let mut state = store
        .state
        .lock()
        .map_err(|_| qualification_unavailable())?;
    let session = state.session;
    insert_projection_locked(
        &mut state,
        public_handle.clone(),
        LoadedProjection {
            session,
            binding,
            snapshot: snapshot.clone(),
        },
    )?;
    Ok(json!({
        "projectionHandle": public_handle,
        "snapshot": snapshot,
    }))
}

/// Capture the canonical sanitized UI state for one loaded projection. The
/// artifact is written create-new only beneath the fixed qualification
/// directory and is bound to the exact backend run/trace and trusted build.
fn capture_inner(
    store: &Phase6d6UiSmokeStore,
    roots: &QualificationRoots,
    projection_handle: &str,
    ui_repetition: u8,
) -> Result<Value, String> {
    if !ui_smoke_enabled() {
        return Err(qualification_unavailable());
    }
    if !(ui_repetition == 1 || ui_repetition == 2) {
        return Err(safe_error(
            "capture_repetition_invalid",
            "UI-smoke repetition must be 1 or 2.",
        ));
    }
    if !projection_handle.starts_with("phase6d6-projection-")
        || !projection_handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(qualification_unavailable());
    }
    let contracts = ui_smoke_contracts(&roots.repo_root)?;
    let index = load_verified_index(roots, &contracts)?;
    let loaded = {
        let state = store
            .state
            .lock()
            .map_err(|_| qualification_unavailable())?;
        let loaded = state
            .projections
            .get(projection_handle)
            .cloned()
            .ok_or_else(qualification_unavailable)?;
        if loaded.session != state.session {
            return Err(qualification_unavailable());
        }
        loaded
    };
    verify_binding_files_and_record(&loaded.binding, &index, roots, &contracts)?;

    let ui_state = derive_ui_state(&loaded.binding, &loaded.snapshot)?;
    verify_ui_state_contract(&loaded.binding.subcase, &ui_state, &contracts)?;
    let artifact_digest = format!(
        "sha256:{}",
        execution::canonical_json_digest(&ui_state).map_err(|_| "capture_state_invalid")?
    );
    let build = development_build()?;
    let sub_run_digest = execution::canonical_json_digest(&json!({
        "kind": "ui_state_capture",
        "subcase": loaded.binding.subcase,
        "uiRepetition": ui_repetition,
        "backendRunId": loaded.binding.run_id,
        "backendTraceDigest": loaded.binding.trace_digest,
        "buildDigest": build.get("digest"),
        "artifactDigest": artifact_digest,
    }))
    .map_err(|_| "capture_state_invalid")?;
    let sub_run_id = format!("ui-subrun-sha256:{sub_run_digest}");

    let run_hex = &loaded.binding.run_id["physical-run-sha256:".len()..];
    let build_hex = build["digest"]
        .as_str()
        .ok_or_else(|| "build_identity_unavailable".to_string())?["sha256:".len()..]
        .to_string();
    let filename = format!(
        "ui_state_{}_rep{}_{}_{}.json",
        loaded.binding.subcase,
        ui_repetition,
        &run_hex[..16],
        &build_hex[..16]
    );
    let destination = secure_ui_capture_path(roots, &filename)?;
    let mut serialized =
        serde_json::to_string_pretty(&ui_state).map_err(|_| "capture_state_invalid".to_string())?;
    serialized.push('\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                safe_error(
                    "artifact_exists",
                    "A UI-state capture for this binding, repetition, and build already exists.",
                )
            } else {
                safe_error(
                    "capture_write_failed",
                    "The UI-state capture could not be written.",
                )
            }
        })?;
    file.write_all(serialized.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            safe_error(
                "capture_write_failed",
                "The UI-state capture could not be written.",
            )
        })?;

    Ok(json!({
        "subcase": loaded.binding.subcase,
        "subRunId": sub_run_id,
        "backendRunId": loaded.binding.run_id,
        "backendTraceDigest": loaded.binding.trace_digest,
        "backendIssueCode": loaded.binding.issue_code,
        "developmentBuild": build,
        "artifact": {
            "kind": "ui_state_capture",
            "path": format!("docs/testing/phase-6d6/evidence/ui/{filename}"),
            "content": ui_state,
            "digest": artifact_digest,
        },
    }))
}

/// Gate-first status path. The complete four-gate decision happens before the
/// root resolver is invoked, so a disabled build performs no qualification
/// filesystem access at the IPC boundary.
fn status_command(
    store: &Phase6d6UiSmokeStore,
    resolve_roots: impl FnOnce() -> Result<QualificationRoots, String>,
) -> Result<Value, String> {
    if !ui_smoke_enabled() {
        return Ok(json!({
            "enabled": false,
            "ready": false,
            "message": null,
            "candidates": [],
        }));
    }
    let roots = match resolve_roots() {
        Ok(roots) => roots,
        Err(code) => {
            return Ok(json!({
                "enabled": ui_smoke_enabled(),
                "ready": false,
                "message": code,
                "candidates": [],
            }));
        }
    };
    status_inner(store, &roots)
}

/// Gate-first load path; resolves roots only after the gates pass.
fn load_command(
    store: &Phase6d6UiSmokeStore,
    resolve_roots: impl FnOnce() -> Result<QualificationRoots, String>,
    binding_handle: &str,
) -> Result<Value, String> {
    if !ui_smoke_enabled() {
        return Err(qualification_unavailable());
    }
    let roots = resolve_roots()?;
    load_inner(store, &roots, binding_handle)
}

/// Gate-first capture path; resolves roots only after the gates pass.
fn capture_command(
    store: &Phase6d6UiSmokeStore,
    resolve_roots: impl FnOnce() -> Result<QualificationRoots, String>,
    projection_handle: String,
    ui_repetition: u8,
) -> Result<Value, String> {
    if !ui_smoke_enabled() {
        return Err(qualification_unavailable());
    }
    let roots = resolve_roots()?;
    capture_inner(store, &roots, &projection_handle, ui_repetition)
}

/// IPC status/candidates command. Reads no qualification files when disabled.
#[tauri::command]
pub fn phase6d6_ui_smoke_status(state: State<'_, Phase6d6UiSmokeStore>) -> Result<Value, String> {
    status_command(&state, QualificationRoots::compile_time)
}

/// IPC load command: one opaque binding handle in, one opaque projection
/// handle plus the trusted DTO out.
#[tauri::command]
pub fn phase6d6_ui_smoke_load_projection(
    binding_handle: String,
    state: State<'_, Phase6d6UiSmokeStore>,
) -> Result<Value, String> {
    load_command(&state, QualificationRoots::compile_time, &binding_handle)
}

/// IPC capture command: one opaque projection handle plus the UI repetition
/// in, the schema-shaped artifact wrapper and trusted bindings out.
#[tauri::command]
pub fn phase6d6_ui_smoke_capture(
    projection_handle: String,
    ui_repetition: u8,
    state: State<'_, Phase6d6UiSmokeStore>,
) -> Result<Value, String> {
    capture_command(
        &state,
        QualificationRoots::compile_time,
        projection_handle,
        ui_repetition,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(feature = "real-execution")]
    use std::path::Path;
    use std::path::PathBuf;
    #[cfg(feature = "real-execution")]
    use tempfile::TempDir;

    static ENV_GUARD: Mutex<()> = Mutex::new(());

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    #[cfg(feature = "real-execution")]
    fn with_gates<T>(body: impl FnOnce() -> T) -> T {
        let _guard = ENV_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("EMUCHEF_RUN_REAL_ADB_TESTS", "1");
        std::env::set_var("EMUCHEF_PHASE_6D6_UI_SMOKE", "1");
        let result = body();
        std::env::remove_var("EMUCHEF_RUN_REAL_ADB_TESTS");
        std::env::remove_var("EMUCHEF_PHASE_6D6_UI_SMOKE");
        result
    }

    fn without_gates<T>(body: impl FnOnce() -> T) -> T {
        let _guard = ENV_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::remove_var("EMUCHEF_RUN_REAL_ADB_TESTS");
        std::env::remove_var("EMUCHEF_PHASE_6D6_UI_SMOKE");
        body()
    }

    #[cfg(feature = "real-execution")]
    fn temp_roots() -> (TempDir, QualificationRoots) {
        let temp = tempfile::tempdir().expect("temporary directory");
        let index_source = repo_root().join(UI_BINDING_INDEX_RELATIVE_PATH);
        let index_dest = temp.path().join(UI_BINDING_INDEX_RELATIVE_PATH);
        fs::create_dir_all(index_dest.parent().expect("index parent")).expect("index directory");
        fs::copy(&index_source, &index_dest).expect("copy index");

        let evidence_root = temp.path().join("docs/testing/phase-6d6/evidence");
        fs::create_dir_all(evidence_root.join("traces")).expect("trace directory");
        let index: Value =
            serde_json::from_str(&fs::read_to_string(&index_dest).expect("index text"))
                .expect("index json");
        for subcase in SUBCASES {
            for binding in index["bindings"][subcase]
                .as_array()
                .expect("binding array for subcase")
            {
                let evidence_path = binding["evidencePath"].as_str().expect("evidence path");
                let trace_path = binding["tracePath"].as_str().expect("trace path");
                let evidence_name = Path::new(evidence_path)
                    .file_name()
                    .expect("evidence file name")
                    .to_string_lossy()
                    .into_owned();
                let trace_name = Path::new(trace_path)
                    .file_name()
                    .expect("trace file name")
                    .to_string_lossy()
                    .into_owned();
                fs::copy(
                    repo_root().join(evidence_path),
                    evidence_root.join(&evidence_name),
                )
                .expect("copy evidence");
                fs::copy(
                    repo_root().join(trace_path),
                    evidence_root.join("traces").join(&trace_name),
                )
                .expect("copy trace");
            }
        }
        let ui_capture_root = evidence_root.join("ui");
        let roots = QualificationRoots {
            repo_root: repo_root(),
            index_path: index_dest,
            evidence_root,
            ui_capture_root,
        };
        (temp, roots)
    }

    #[cfg(feature = "real-execution")]
    fn subcase_candidates(status: &Value, subcase: &str) -> Vec<Value> {
        status["candidates"]
            .as_array()
            .expect("candidates array")
            .iter()
            .filter(|candidate| candidate["subcase"].as_str() == Some(subcase))
            .cloned()
            .collect()
    }

    #[cfg(feature = "real-execution")]
    fn load_first(
        store: &Phase6d6UiSmokeStore,
        roots: &QualificationRoots,
        subcase: &str,
    ) -> Value {
        let status = status_inner(store, roots).expect("status");
        let candidate = subcase_candidates(&status, subcase)
            .into_iter()
            .next()
            .expect("candidate");
        let handle = candidate["handle"].as_str().expect("handle").to_string();
        load_inner(store, roots, &handle).expect("load projection")
    }

    #[test]
    fn ui_smoke_requested_requires_exactly_the_four_exact_gates() {
        let cases = [
            (true, true, Some("1"), Some("1"), true),
            (false, true, Some("1"), Some("1"), false),
            (true, false, Some("1"), Some("1"), false),
            (true, true, None, Some("1"), false),
            (true, true, Some("1"), None, false),
            (true, true, Some("true"), Some("1"), false),
            (true, true, Some("1"), Some("true"), false),
            (true, true, Some("0"), Some("1"), false),
            (true, true, Some("1"), Some("0"), false),
            (true, true, Some("1 "), Some("1"), false),
            (true, true, Some("1"), Some(" 1"), false),
        ];
        for (debug_build, real_execution, global_opt_in, ui_smoke_opt_in, expected) in cases {
            assert_eq!(
                ui_smoke_requested(debug_build, real_execution, global_opt_in, ui_smoke_opt_in),
                expected,
                "gates debug={debug_build} real={real_execution} global={global_opt_in:?} ui={ui_smoke_opt_in:?}",
            );
        }
    }

    #[test]
    fn command_layer_checks_gates_before_resolving_roots() {
        without_gates(|| {
            let store = Phase6d6UiSmokeStore::default();
            let resolves = std::cell::Cell::new(0usize);
            let resolve = || -> Result<QualificationRoots, String> {
                resolves.set(resolves.get() + 1);
                Err("root resolver must not run while disabled".to_string())
            };

            let status = status_command(&store, resolve).expect("disabled status is Ok");
            assert_eq!(status["enabled"], false);
            assert_eq!(status["ready"], false);
            assert_eq!(resolves.get(), 0);

            let load_error = load_command(&store, resolve, "phase6d6-binding-test")
                .expect_err("disabled load must fail closed");
            assert!(load_error.contains("qualification_unavailable"));
            assert_eq!(resolves.get(), 0);

            let capture_error =
                capture_command(&store, resolve, "phase6d6-projection-test".to_string(), 1)
                    .expect_err("disabled capture must fail closed");
            assert!(capture_error.contains("qualification_unavailable"));
            assert_eq!(resolves.get(), 0);
        });
    }

    #[test]
    fn disabled_status_is_inert_and_reads_no_qualification_files() {
        without_gates(|| {
            let roots = QualificationRoots {
                repo_root: PathBuf::from("/nonexistent-repo"),
                index_path: PathBuf::from("/nonexistent-index"),
                evidence_root: PathBuf::from("/nonexistent-evidence"),
                ui_capture_root: PathBuf::from("/nonexistent-ui"),
            };
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status must not read files");
            assert_eq!(status["enabled"], false);
            assert_eq!(status["ready"], false);
            assert_eq!(status["candidates"].as_array().unwrap().len(), 0);
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn status_lists_only_eligible_sanitized_candidates() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            assert_eq!(status["enabled"], true);
            assert_eq!(status["ready"], true);
            assert_eq!(subcase_candidates(&status, "cancellation").len(), 4);
            assert_eq!(subcase_candidates(&status, "transport").len(), 2);
            assert_eq!(subcase_candidates(&status, "root").len(), 2);
            assert_eq!(subcase_candidates(&status, "storage").len(), 2);
            assert_eq!(subcase_candidates(&status, "host_sleep").len(), 4);
            let serialized = status.to_string();
            for forbidden in [
                "physical-run-sha256",
                "docs/testing",
                "device_transport_lost",
                "usb_disconnect_active",
                "be0ba890",
                "18917650",
                "735aba0b",
                "3eabd150",
                "a44a7bcf",
                "sha256:",
            ] {
                assert!(
                    !serialized.contains(forbidden),
                    "candidate leaked {forbidden}"
                );
            }
            for candidate in subcase_candidates(&status, "transport") {
                assert!(candidate["handle"]
                    .as_str()
                    .unwrap()
                    .starts_with("phase6d6-binding-"));
                assert!(matches!(candidate["repetition"].as_u64(), Some(1 | 2)));
            }
            for candidate in subcase_candidates(&status, "host_sleep") {
                assert!(candidate["handle"]
                    .as_str()
                    .unwrap()
                    .starts_with("phase6d6-binding-"));
                assert!(matches!(candidate["repetition"].as_u64(), Some(1 | 2)));
            }
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn status_fails_closed_when_evidence_bytes_are_tampered() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            fs::write(
                roots
                    .evidence_root
                    .join("usb_disconnect_active-rep1-be0ba89089556f91.json"),
                b"{\"tampered\":true}\n",
            )
            .expect("tamper evidence");
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            assert_eq!(status["enabled"], true);
            assert_eq!(status["ready"], false);
            let message = status["message"].as_str().unwrap();
            assert!(
                message.contains("binding")
                    || message.contains("evidence")
                    || message.contains("digest")
            );
            assert_eq!(status["candidates"].as_array().unwrap().len(), 0);
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn status_fails_closed_when_index_is_stale() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let mut index: Value =
                serde_json::from_str(&fs::read_to_string(&roots.index_path).expect("index text"))
                    .expect("index json");
            index["bindings"]["transport"][0]["runId"] =
                json!(format!("physical-run-sha256:{}", "a".repeat(64)));
            fs::write(
                &roots.index_path,
                serde_json::to_vec_pretty(&index).expect("serialize index"),
            )
            .expect("write index");
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            assert_eq!(status["enabled"], true);
            assert_eq!(status["ready"], false);
            assert!(status["message"].as_str().unwrap().contains("binding"));
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn load_projection_projects_through_production_path_without_execution_authority() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();
            let loaded = load_first(&store, &roots, "transport");
            let snapshot = &loaded["snapshot"];
            assert_eq!(snapshot["status"], "failed");
            assert_eq!(snapshot["terminal"], true);
            assert_eq!(snapshot["simulated"], false);
            assert_eq!(snapshot["verificationScope"], "real_device");
            assert_eq!(
                snapshot["errors"][0]["message"],
                "The device connection was lost during execution."
            );
            assert_eq!(
                snapshot["errors"][0]["remediation"]["title"],
                "Reconnect and requalify"
            );
            assert_eq!(snapshot["completion"]["counts"]["completed"], 1);
            assert_eq!(snapshot["completion"]["counts"]["pending"], 1);
            assert_eq!(
                snapshot["terminalPolicy"]["partialChangePresentation"],
                "possible_partial_change"
            );
            assert_eq!(
                snapshot["terminalPolicy"]["recoveryState"],
                "requalification_required"
            );
            assert_eq!(snapshot["terminalPolicy"]["authorityInvalidated"], true);
            assert!(snapshot["launchAction"].is_null());
            assert!(loaded["projectionHandle"]
                .as_str()
                .unwrap()
                .starts_with("phase6d6-projection-"));
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn load_projection_rejects_tampered_evidence_bytes() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            let handle = subcase_candidates(&status, "transport")[0]["handle"]
                .as_str()
                .unwrap()
                .to_string();
            fs::write(
                roots
                    .evidence_root
                    .join("usb_disconnect_active-rep1-be0ba89089556f91.json"),
                b"{\"tampered\":true}\n",
            )
            .expect("tamper evidence");
            let error = load_inner(&store, &roots, &handle).expect_err("load must fail closed");
            assert!(
                error.contains("binding") || error.contains("digest") || error.contains("evidence")
            );
        });
    }

    #[test]
    fn parsed_record_rechecks_reject_wrong_identity_outcome_or_issue() {
        let index: Value = serde_json::from_str(
            &fs::read_to_string(repo_root().join(UI_BINDING_INDEX_RELATIVE_PATH))
                .expect("index text"),
        )
        .expect("index json");
        let binding =
            binding_from_index_entry("cancellation", &index["bindings"]["cancellation"][0])
                .expect("binding");
        let record: Value = serde_json::from_str(
            &fs::read_to_string(repo_root().join(
                "docs/testing/phase-6d6/evidence/cancellation_active-rep1-8f8cbaf7ec6b8d60.json",
            ))
            .expect("record text"),
        )
        .expect("record json");
        let contracts = ui_smoke_contracts(&repo_root()).expect("contracts");

        assert!(verify_parsed_record(&binding, &record, &contracts).is_ok());

        let mut wrong_outcome = record.clone();
        wrong_outcome["outcome"] = json!("failed");
        assert!(verify_parsed_record(&binding, &wrong_outcome, &contracts).is_err());

        let mut wrong_issue = record.clone();
        wrong_issue["observedIssueCode"] = json!("device_transport_lost");
        assert!(verify_parsed_record(&binding, &wrong_issue, &contracts).is_err());

        let mut wrong_run = record.clone();
        wrong_run["runId"] = json!(format!("physical-run-sha256:{}", "b".repeat(64)));
        assert!(verify_parsed_record(&binding, &wrong_run, &contracts).is_err());

        let mut wrong_digest = record.clone();
        wrong_digest["recordDigest"] = json!(format!("sha256:{}", "c".repeat(64)));
        assert!(verify_parsed_record(&binding, &wrong_digest, &contracts).is_err());
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn capture_writes_canonical_artifact_and_returns_trusted_bindings() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();
            let loaded = load_first(&store, &roots, "cancellation");
            let projection_handle = loaded["projectionHandle"].as_str().unwrap().to_string();
            let result = capture_inner(&store, &roots, &projection_handle, 1).expect("capture");
            assert_eq!(result["subcase"], "cancellation");
            let sub_run_id = result["subRunId"].as_str().unwrap();
            assert!(sub_run_id.starts_with("ui-subrun-sha256:"));
            assert_eq!(sub_run_id.len(), "ui-subrun-sha256:".len() + 64);
            assert!(result["backendRunId"]
                .as_str()
                .unwrap()
                .starts_with("physical-run-sha256:"));
            assert!(result["backendTraceDigest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:"));
            assert_eq!(result["backendIssueCode"], Value::Null);
            assert_eq!(
                result["developmentBuild"]["identity"],
                "emuchef-app:development-ui-smoke"
            );
            assert_eq!(
                result["developmentBuild"]["version"],
                env!("CARGO_PKG_VERSION")
            );
            assert!(result["developmentBuild"]["digest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:"));
            assert_eq!(result["artifact"]["kind"], "ui_state_capture");
            let artifact_path = result["artifact"]["path"].as_str().unwrap();
            assert!(artifact_path
                .starts_with("docs/testing/phase-6d6/evidence/ui/ui_state_cancellation_rep1_"));
            let content = &result["artifact"]["content"];
            assert_eq!(content["backendRunId"], result["backendRunId"]);
            assert_eq!(content["authoredTitle"], "Execution cancelled");
            assert_eq!(
                content["authoredIssueText"],
                "This action was cancelled at a safe boundary."
            );
            assert_eq!(content["terminalStepProjection"], "cancelled");
            assert_eq!(content["notAttempted"], 1);
            assert_eq!(
                content["partialChangePresentation"],
                "possible_partial_change"
            );
            assert_eq!(content["authorityInvalidated"], false);
            assert_eq!(content["recoveryState"], "fresh_review_required");
            for forbidden in ["resume", "replay", "checkpoint", "ownership_transfer"] {
                assert!(!content["availableControls"]
                    .as_array()
                    .unwrap()
                    .contains(&Value::String(forbidden.to_string())));
            }
            assert_eq!(
                result["artifact"]["digest"],
                format!(
                    "sha256:{}",
                    execution::canonical_json_digest(content).expect("digest")
                )
            );
            let file_name = Path::new(artifact_path)
                .file_name()
                .unwrap()
                .to_string_lossy();
            let written: Value = serde_json::from_str(
                &fs::read_to_string(roots.ui_capture_root.join(file_name.as_ref()))
                    .expect("artifact file"),
            )
            .expect("artifact json");
            assert_eq!(written, *content);

            let collision = capture_inner(&store, &roots, &projection_handle, 1)
                .expect_err("create-new must reject a duplicate capture");
            assert!(collision.contains("artifact") || collision.contains("exists"));
            let rep_two = capture_inner(&store, &roots, &projection_handle, 2).expect("rep two");
            assert!(rep_two["artifact"]["path"]
                .as_str()
                .unwrap()
                .contains("_rep2_"));
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn capture_fails_closed_when_projection_state_violates_the_ui_contract() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();
            let loaded = load_first(&store, &roots, "transport");
            let projection_handle = loaded["projectionHandle"].as_str().unwrap().to_string();
            let mut snapshot = loaded["snapshot"].clone();
            snapshot["errors"][0]["message"] = json!("A different authored message.");
            {
                let mut state = store.state.lock().unwrap();
                let projections = &mut state.projections;
                projections
                    .get_mut(&projection_handle)
                    .expect("projection")
                    .snapshot = snapshot;
            }
            let error = capture_inner(&store, &roots, &projection_handle, 1)
                .expect_err("contract mismatch must fail closed");
            assert!(error.contains("contract") || error.contains("authored"));
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn capture_ui_state_matches_manifest_contract_for_every_eligible_subcase() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let manifest: Value = serde_json::from_str(
                &fs::read_to_string(
                    roots
                        .repo_root
                        .join("docs/testing/phase-6d6/scenario-manifest.json"),
                )
                .expect("manifest text"),
            )
            .expect("manifest json");
            let contracts = &manifest["uiSmokeContracts"];
            for subcase in ["cancellation", "transport", "root", "storage"] {
                let store = Phase6d6UiSmokeStore::default();
                let loaded = load_first(&store, &roots, subcase);
                let projection_handle = loaded["projectionHandle"].as_str().unwrap().to_string();
                let result = capture_inner(&store, &roots, &projection_handle, 1).expect("capture");
                let content = &result["artifact"]["content"];
                let contract = &contracts[subcase];
                assert_eq!(content["authoredTitle"], contract["authoredTitle"]);
                assert_eq!(content["authoredIssueText"], contract["authoredIssueText"]);
                assert_eq!(
                    content["authoredRemediation"],
                    contract["authoredRemediation"]
                );
                assert_eq!(
                    content["terminalStepProjection"],
                    contract["terminalStepProjection"]
                );
                assert!(content["notAttempted"].as_u64().unwrap() >= 1);
                assert_eq!(
                    content["partialChangePresentation"],
                    contract["partialChangePresentation"]
                );
                assert_eq!(
                    content["authorityInvalidated"],
                    contract["authorityInvalidated"]
                );
                assert_eq!(content["recoveryState"], contract["recoveryState"]);
                for forbidden in contract["forbiddenControls"].as_array().unwrap() {
                    assert!(!content["availableControls"]
                        .as_array()
                        .unwrap()
                        .contains(&Value::String(forbidden.as_str().unwrap().to_string())));
                }
            }
        });
    }

    #[test]
    fn development_build_identity_is_trusted_and_never_exposes_the_executable_path() {
        let build = development_build().expect("build identity");
        assert_eq!(build["identity"], "emuchef-app:development-ui-smoke");
        assert_eq!(build["version"], env!("CARGO_PKG_VERSION"));
        let digest = build["digest"].as_str().unwrap().to_string();
        assert!(digest.starts_with("sha256:"));
        assert_eq!(digest.len(), "sha256:".len() + 64);
        let exe_bytes = fs::read(std::env::current_exe().expect("current exe")).expect("exe bytes");
        assert_eq!(
            digest,
            format!("sha256:{}", hex::encode(Sha256::digest(&exe_bytes)))
        );
    }

    #[cfg(all(unix, feature = "real-execution"))]
    #[test]
    fn capture_rejects_a_symlinked_capture_root() {
        with_gates(|| {
            let outside = tempfile::tempdir().expect("outside temp");
            let target = outside.path().join("outside");
            fs::create_dir(&target).expect("outside dir");
            let (_temp, roots) = temp_roots();
            fs::remove_dir_all(&roots.ui_capture_root).ok();
            std::os::unix::fs::symlink(&target, &roots.ui_capture_root).expect("symlink");
            let store = Phase6d6UiSmokeStore::default();
            let loaded = load_first(&store, &roots, "cancellation");
            let projection_handle = loaded["projectionHandle"].as_str().unwrap().to_string();
            let error = capture_inner(&store, &roots, &projection_handle, 1)
                .expect_err("symlinked capture root must fail closed");
            assert!(error.contains("capture") || error.contains("root"));
        });
    }

    #[test]
    fn parsed_record_rechecks_reject_wrong_scenario_and_repetition() {
        let index: Value = serde_json::from_str(
            &fs::read_to_string(repo_root().join(UI_BINDING_INDEX_RELATIVE_PATH))
                .expect("index text"),
        )
        .expect("index json");
        let binding =
            binding_from_index_entry("cancellation", &index["bindings"]["cancellation"][0])
                .expect("binding");
        let contracts = ui_smoke_contracts(&repo_root()).expect("contracts");
        let mut record: Value = serde_json::from_str(
            &fs::read_to_string(repo_root().join(
                "docs/testing/phase-6d6/evidence/cancellation_active-rep1-8f8cbaf7ec6b8d60.json",
            ))
            .expect("record text"),
        )
        .expect("record json");
        let reseal = |record: &mut Value| {
            if let Some(object) = record.as_object_mut() {
                object.remove("recordDigest");
            }
            let digest = execution::canonical_json_digest(record).expect("canonical digest");
            record.as_object_mut().expect("record object").insert(
                "recordDigest".to_string(),
                json!(format!("sha256:{digest}")),
            );
        };

        record["scenario"] = json!("usb_disconnect_boundary");
        reseal(&mut record);
        assert!(verify_parsed_record(&binding, &record, &contracts).is_err());

        record["scenario"] = json!("cancellation_active");
        record["repetition"] = json!(2);
        reseal(&mut record);
        assert!(verify_parsed_record(&binding, &record, &contracts).is_err());
    }

    #[test]
    fn evidence_relative_path_rejects_escapes() {
        for escaped in [
            "../outside.json",
            "traces/../../outside.json",
            "/absolute.json",
            "..\\outside.json",
        ] {
            assert!(
                evidence_relative_path(&format!("docs/testing/phase-6d6/evidence/{escaped}"))
                    .is_err(),
                "accepted escaped path {escaped}"
            );
        }
        assert_eq!(
            evidence_relative_path(
                "docs/testing/phase-6d6/evidence/cancellation_active-rep1-8f8cbaf7ec6b8d60.json"
            ),
            Ok("cancellation_active-rep1-8f8cbaf7ec6b8d60.json")
        );
    }

    #[cfg(feature = "real-execution")]
    fn self_consistent_index(roots: &QualificationRoots, mutate: impl FnOnce(&mut Value)) -> Value {
        let mut index: Value =
            serde_json::from_str(&fs::read_to_string(&roots.index_path).expect("index text"))
                .expect("index json");
        mutate(&mut index);
        index["digest"] = json!(format!(
            "sha256:{}",
            execution::canonical_json_digest(&index_without_digest(&index)).expect("index digest")
        ));
        index
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn source_digest_mismatch_is_rejected_even_when_self_digest_is_recomputed() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let stale = self_consistent_index(&roots, |index| {
                index["sourceDigests"]["docs/testing/phase-6d6/scenario-manifest.json"] =
                    json!(format!("sha256:{}", "0".repeat(64)));
            });
            fs::write(
                &roots.index_path,
                serde_json::to_vec_pretty(&stale).expect("serialize index"),
            )
            .expect("write index");
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            assert_eq!(status["ready"], false);
            assert!(status["message"].as_str().unwrap().contains("binding"));
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn wrong_scenario_or_wrong_subcase_candidate_is_rejected_even_with_self_consistent_index() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();

            let offline = self_consistent_index(&roots, |index| {
                index["bindings"]["transport"][0]["scenario"] = json!("device_offline");
                index["bindings"]["transport"][0]["issueCode"] = json!("device_offline");
            });
            fs::write(
                &roots.index_path,
                serde_json::to_vec_pretty(&offline).expect("serialize index"),
            )
            .expect("write index");
            let status = status_inner(&store, &roots).expect("status");
            assert_eq!(status["ready"], false);
            assert!(status["message"].as_str().unwrap().contains("binding"));

            let timeout = self_consistent_index(&roots, |index| {
                index["bindings"]["host_sleep"] = json!([{
                    "repetition": 1,
                    "runId": format!("physical-run-sha256:{}", "a".repeat(64)),
                    "scenario": "operation_timeout",
                    "commit": "c".repeat(40),
                    "issueCode": "operation_timed_out",
                    "evidencePath": "docs/testing/phase-6d6/evidence/operation_timeout-rep1-69ce0c5bc594a244.json",
                    "tracePath": "docs/testing/phase-6d6/evidence/traces/operation_timeout-rep1-69ce0c5bc594a244.json",
                    "recordDigest": format!("sha256:{}", "0".repeat(64)),
                    "traceDigest": format!("sha256:{}", "0".repeat(64)),
                    "evidenceSha256": format!("sha256:{}", "0".repeat(64)),
                    "traceSha256": format!("sha256:{}", "0".repeat(64)),
                }]);
            });
            fs::write(
                &roots.index_path,
                serde_json::to_vec_pretty(&timeout).expect("serialize index"),
            )
            .expect("write index");
            let status = status_inner(&store, &roots).expect("status");
            assert_eq!(status["ready"], false);
            assert!(status["message"].as_str().unwrap().contains("binding"));
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn load_projection_rejects_tampered_trace_bytes() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            let handle = subcase_candidates(&status, "transport")[0]["handle"]
                .as_str()
                .unwrap()
                .to_string();
            fs::write(
                roots
                    .evidence_root
                    .join("traces/usb_disconnect_active-rep1-be0ba89089556f91.json"),
                b"{\"tampered\":true}\n",
            )
            .expect("tamper trace");
            let error =
                load_inner(&store, &roots, &handle).expect_err("trace tamper must fail closed");
            assert!(
                error.contains("trace") || error.contains("digest") || error.contains("binding")
            );
        });
    }

    #[cfg(all(unix, feature = "real-execution"))]
    #[test]
    fn evidence_symlink_source_is_rejected() {
        with_gates(|| {
            let outside = tempfile::tempdir().expect("outside temp");
            let outside_file = outside.path().join("outside.json");
            fs::write(&outside_file, b"{\"outside\":true}\n").expect("outside file");
            let (_temp, roots) = temp_roots();
            let evidence_path = roots
                .evidence_root
                .join("cancellation_active-rep1-8f8cbaf7ec6b8d60.json");
            fs::remove_file(&evidence_path).expect("remove evidence");
            std::os::unix::fs::symlink(&outside_file, &evidence_path).expect("symlink evidence");
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            assert_eq!(status["ready"], false);
            assert!(status["message"].as_str().unwrap().contains("evidence"));
        });
    }

    #[cfg(all(unix, feature = "real-execution"))]
    #[test]
    fn trace_symlink_source_is_rejected() {
        with_gates(|| {
            let outside = tempfile::tempdir().expect("outside temp");
            let outside_file = outside.path().join("outside.json");
            fs::write(&outside_file, b"{\"outside\":true}\n").expect("outside file");
            let (_temp, roots) = temp_roots();
            let trace_path = roots
                .evidence_root
                .join("traces/usb_disconnect_active-rep1-be0ba89089556f91.json");
            fs::remove_file(&trace_path).expect("remove trace");
            std::os::unix::fs::symlink(&outside_file, &trace_path).expect("symlink trace");
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            assert_eq!(status["ready"], false);
            assert!(status["message"].as_str().unwrap().contains("evidence"));
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn store_is_bounded_and_sessions_invalidate_stale_handles() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();
            let index: Value =
                serde_json::from_str(&fs::read_to_string(&roots.index_path).expect("index text"))
                    .expect("index json");
            let sample =
                binding_from_index_entry("cancellation", &index["bindings"]["cancellation"][0])
                    .expect("binding");

            {
                let mut state = store.state.lock().unwrap();
                for offset in 0..MAX_CANDIDATES {
                    insert_binding_locked(
                        &mut state,
                        format!("phase6d6-binding-cap-{offset}"),
                        sample.clone(),
                    )
                    .expect("candidate capacity fits");
                }
                assert!(
                    insert_binding_locked(
                        &mut state,
                        "phase6d6-binding-overflow".to_string(),
                        sample.clone(),
                    )
                    .is_err(),
                    "candidate capacity must fail closed"
                );
                state.bindings.clear();
                for offset in 0..MAX_PROJECTIONS {
                    let session = state.session;
                    insert_projection_locked(
                        &mut state,
                        format!("phase6d6-projection-cap-{offset}"),
                        LoadedProjection {
                            session,
                            binding: sample.clone(),
                            snapshot: json!({}),
                        },
                    )
                    .expect("projection capacity fits");
                }
                let session = state.session;
                assert!(
                    insert_projection_locked(
                        &mut state,
                        "phase6d6-projection-overflow".to_string(),
                        LoadedProjection {
                            session,
                            binding: sample.clone(),
                            snapshot: json!({}),
                        },
                    )
                    .is_err(),
                    "projection capacity must fail closed"
                );
            }

            let status_one = status_inner(&store, &roots).expect("status one");
            let handle_one = subcase_candidates(&status_one, "cancellation")[0]["handle"]
                .as_str()
                .unwrap()
                .to_string();
            let loaded_one = load_inner(&store, &roots, &handle_one).expect("load one");
            let projection_one = loaded_one["projectionHandle"].as_str().unwrap().to_string();

            let status_two = status_inner(&store, &roots).expect("status two");
            assert_eq!(status_two["ready"], true);
            assert!(
                load_inner(&store, &roots, &handle_one).is_err(),
                "stale binding handle must be invalidated"
            );
            let capture_error = capture_inner(&store, &roots, &projection_one, 1)
                .expect_err("stale projection must be invalidated");
            assert!(capture_error.contains("unavailable"));

            for _ in 0..4 {
                let status = status_inner(&store, &roots).expect("status cycle");
                let handle = subcase_candidates(&status, "storage")[0]["handle"]
                    .as_str()
                    .unwrap()
                    .to_string();
                let _ = load_inner(&store, &roots, &handle).expect("load cycle");
            }
            let state = store.state.lock().unwrap();
            assert!(state.bindings.len() <= MAX_CANDIDATES);
            assert!(state.projections.len() <= MAX_PROJECTIONS);
            assert!(state.session >= 2);
        });
    }

    #[cfg(feature = "real-execution")]
    #[test]
    fn candidate_labels_are_unique_within_subcase_and_sanitized() {
        with_gates(|| {
            let (_temp, roots) = temp_roots();
            let store = Phase6d6UiSmokeStore::default();
            let status = status_inner(&store, &roots).expect("status");
            for subcase in SUBCASES {
                let candidates = subcase_candidates(&status, subcase);
                let labels = candidates
                    .iter()
                    .filter_map(|candidate| candidate["label"].as_str())
                    .collect::<Vec<_>>();
                let unique: std::collections::HashSet<_> = labels.iter().collect();
                assert_eq!(unique.len(), labels.len(), "duplicate labels in {subcase}");
            }
            let cancellation_candidates = subcase_candidates(&status, "cancellation");
            let cancellation_labels = cancellation_candidates
                .iter()
                .filter_map(|candidate| candidate["label"].as_str())
                .collect::<Vec<_>>();
            assert!(cancellation_labels
                .iter()
                .any(|label| label.contains("active interruption")));
            assert!(cancellation_labels
                .iter()
                .any(|label| label.contains("safe boundary")));
            let serialized = status.to_string();
            for forbidden in [
                "physical-run-sha256",
                "docs/testing",
                "device_transport_lost",
                "usb_disconnect_active",
                "sha256:",
                "serial",
            ] {
                assert!(
                    !serialized.contains(forbidden),
                    "candidate leaked {forbidden}"
                );
            }
        });
    }

    #[test]
    fn candidate_labels_distinguish_multi_scenario_subcases_without_raw_identifiers() {
        assert_ne!(
            candidate_label("cancellation", "cancellation_active", 1),
            candidate_label("cancellation", "cancellation_boundary", 1),
        );
        assert_ne!(
            candidate_label("transport", "usb_disconnect_active", 1),
            candidate_label("transport", "usb_disconnect_boundary", 1),
        );
        assert_ne!(
            candidate_label("host_sleep", "host_sleep_before_deadline", 1),
            candidate_label("host_sleep", "host_sleep_after_deadline", 1),
        );
        assert_ne!(
            candidate_label("host_sleep", "host_sleep_before_deadline", 2),
            candidate_label("host_sleep", "host_sleep_after_deadline", 2),
        );

        let before = candidate_label("host_sleep", "host_sleep_before_deadline", 1);
        let after = candidate_label("host_sleep", "host_sleep_after_deadline", 1);
        assert!(before.contains("Host sleep — before deadline — physical repetition 1"));
        assert!(after.contains("Host sleep — after deadline — physical repetition 1"));
        for label in [&before, &after] {
            for forbidden in [
                "host_sleep_before_deadline",
                "host_sleep_after_deadline",
                "physical-run-sha256",
                "docs/testing",
                "sha256:",
                "serial",
            ] {
                assert!(
                    !label.contains(forbidden),
                    "host-sleep label leaked {forbidden}"
                );
            }
        }
    }
}
