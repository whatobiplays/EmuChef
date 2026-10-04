//! Development-only target-registration orchestration.
//!
//! This module owns the qualification overlay's typed contract and the small
//! orchestration boundary that captures target facts from existing production
//! device observations. Canonical target validation, IDs, and repository
//! mutation remain owned by `tools/device-qualification.mjs`.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::commands::{safe_error, AppState};
use crate::device_observation::{SelectedDeviceCapture, SelectedDeviceObservationSource};
use crate::qualification_build::{
    embedded_build_identity, qualification_gate_inputs, qualification_mode_enabled_at_runtime,
    QualificationBuildIdentity,
};
use crate::qualification_repository::{
    CandidateKind, QualificationCandidateSummary as RepositoryCandidateSummary,
    QualificationOperation, QualificationRepositoryProvider, RepositoryQualificationDescription,
    StoredQualificationCandidate,
};

/// The operator outcomes accepted by a workflow-declared checkpoint.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationCheckpointOutcome {
    Pass,
    Fail,
    UnableToVerify,
}

/// Short name used by the session state machine and its public contract.
pub(crate) type CheckpointOutcome = QualificationCheckpointOutcome;

/// The only connection facts that may currently be attested by an operator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum QualificationConnectionType {
    Usb2,
    Usb3,
}

/// The trusted source of one material target fact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationFactSource {
    ProductionObservation,
    ExplicitRootCheck,
    OperatorAttestation,
}

/// A target fact paired with the authority that established it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationFactPreview<T> {
    pub(crate) value: T,
    pub(crate) source: QualificationFactSource,
}

/// A workflow's operator checkpoint declaration projected from the catalog.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationWorkflowCheckpoint {
    pub(crate) id: String,
    pub(crate) instruction: String,
    pub(crate) fact: String,
    pub(crate) allowed_outcomes: Vec<QualificationCheckpointOutcome>,
    pub(crate) required: bool,
}

/// An automated observation declared by the repository workflow contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QualificationWorkflowObservation {
    pub(crate) id: String,
    pub(crate) required: bool,
}

/// The workflow fields needed by the qualification overlay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationWorkflow {
    pub(crate) id: String,
    pub(crate) version: u64,
    pub(crate) purpose: String,
    pub(crate) production_recipes: Vec<String>,
    pub(crate) required_capabilities: Vec<String>,
    pub(crate) prerequisites: Vec<String>,
    pub(crate) human_checkpoints: Vec<QualificationWorkflowCheckpoint>,
    #[serde(default, skip_serializing)]
    pub(crate) compatibility_dimensions: Vec<String>,
    #[serde(default, skip_serializing)]
    pub(crate) automated_observations: Vec<QualificationWorkflowObservation>,
}

/// A registered target projected without its provenance wrappers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationRootState {
    NonRoot,
    Rooted,
}

/// A registered target projected without its provenance wrappers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationTargetSummary {
    pub(crate) id: String,
    pub(crate) profile_id: String,
    pub(crate) manufacturer: String,
    pub(crate) model: String,
    pub(crate) android_version: String,
    pub(crate) android_api: u64,
    pub(crate) abi_soc_class: String,
    pub(crate) root_state: QualificationRootState,
    pub(crate) connection_type: QualificationConnectionType,
    pub(crate) firmware_build: String,
}

/// The typed target facts captured for a target-registration candidate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationTargetCandidateTarget {
    pub(crate) profile_id: QualificationFactPreview<String>,
    pub(crate) manufacturer: QualificationFactPreview<String>,
    pub(crate) model: QualificationFactPreview<String>,
    pub(crate) android_version: QualificationFactPreview<String>,
    pub(crate) android_api: QualificationFactPreview<u64>,
    pub(crate) abi_soc_class: QualificationFactPreview<String>,
    pub(crate) root_state: QualificationFactPreview<QualificationRootState>,
    pub(crate) connection_type: QualificationFactPreview<QualificationConnectionType>,
    pub(crate) firmware_build: QualificationFactPreview<String>,
    pub(crate) capabilities: Vec<String>,
    pub(crate) deferred_workflows: Vec<String>,
}

/// The reviewable, opaque target-registration candidate projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationTargetCandidatePreview {
    pub(crate) candidate_handle: String,
    pub(crate) kind: CandidateKind,
    pub(crate) captured_at: String,
    pub(crate) target: QualificationTargetCandidateTarget,
    pub(crate) promotable: bool,
    pub(crate) non_promotable_reason: Option<String>,
}

/// A resumable candidate summary safe to expose to React.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationCandidateSummaryDto {
    pub(crate) candidate_handle: String,
    pub(crate) kind: CandidateKind,
    pub(crate) captured_at: String,
    pub(crate) promotable: bool,
    pub(crate) non_promotable_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<QualificationTargetCandidateTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) run_validity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qualification_outcome: Option<String>,
}

/// The complete qualification-mode status displayed by the development overlay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationModeStatus {
    pub(crate) enabled: bool,
    pub(crate) recordable: bool,
    pub(crate) device_selection_locked: bool,
    pub(crate) message: Option<String>,
    pub(crate) build: Option<QualificationBuildIdentity>,
    pub(crate) runtime_contract: Option<String>,
    pub(crate) workflows: Vec<QualificationWorkflow>,
    pub(crate) targets: Vec<QualificationTargetSummary>,
    pub(crate) resumable_candidates: Vec<QualificationCandidateSummaryDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) resumable_session:
        Option<crate::qualification_session::QualificationSessionSnapshot>,
}

/// The only input accepted by target-registration capture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateQualificationTargetCandidateRequest {
    pub(crate) device_handle: String,
    pub(crate) device_plan: String,
    pub(crate) connection_type: QualificationConnectionType,
}

/// Inputs for starting a session against an already registered target and a
/// repository workflow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BeginQualificationSessionRequest {
    pub(crate) device_handle: String,
    pub(crate) device_plan: String,
    pub(crate) target_id: String,
    pub(crate) workflow_id: String,
}

/// Canonical run identity returned after the Node tool records a terminal
/// candidate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QualificationRunRecordingResult {
    pub(crate) run_id: String,
}

/// The canonical registration consequence returned after Node succeeds.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationTargetRegistrationResult {
    pub(crate) target_id: String,
    pub(crate) requires_commit_and_rebuild: bool,
}

/// Runtime inputs used to decide whether qualification commands may be used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QualificationModeState {
    pub(crate) enabled: bool,
    pub(crate) build: Option<QualificationBuildIdentity>,
}

impl QualificationModeState {
    fn current(_provider: &QualificationRepositoryProvider) -> Self {
        #[cfg(test)]
        if let Some(build) = _provider.test_mode_build() {
            return Self {
                enabled: true,
                build: Some(build.clone()),
            };
        }
        let inputs = qualification_gate_inputs();
        Self {
            enabled: qualification_mode_enabled_at_runtime(),
            build: inputs.embedded_identity.or_else(embedded_build_identity),
        }
    }
}

/// Return the build identity currently trusted by qualification mode. Persisted
/// sessions use it to avoid resuming evidence under a different binary.
pub(crate) fn current_build_identity(
    provider: &QualificationRepositoryProvider,
) -> Option<QualificationBuildIdentity> {
    QualificationModeState::current(provider).build
}

/// Guard shared by every qualification command that changes trusted state.
fn require_recordable_mode(
    state: &QualificationModeState,
) -> Result<&QualificationBuildIdentity, String> {
    if !state.enabled {
        return Err(safe_qualification_error("qualification_mode_disabled"));
    }
    state
        .build
        .as_ref()
        .ok_or_else(|| safe_qualification_error("qualification_build_unavailable"))
}

fn safe_qualification_error(code: &str) -> String {
    let message = match code {
        "qualification_mode_disabled" => {
            "Device qualification mode is unavailable in this application build."
        }
        "qualification_build_unavailable" => {
            "The application has no recordable qualification build identity."
        }
        "qualification_repository_unavailable" => {
            "Qualification definitions are unavailable. Rebuild the qualification application."
        }
        "qualification_source_changed" => {
            "The qualification source state changed. Rebuild from the unchanged committed source."
        }
        "qualification_candidate_invalid" => {
            "The qualification target, session, or candidate is invalid or no longer available."
        }
        "qualification_target_unverified" => {
            "The connected device target could not be verified from trusted observations."
        }
        "qualification_review_mismatch" => {
            "The reviewed production plan does not match this qualification session."
        }
        "qualification_execution_mismatch" => {
            "The production execution does not match this qualification session."
        }
        "qualification_execution_unavailable" => {
            "The production execution report is unavailable or has not reached a terminal state."
        }
        "qualification_checkpoint_invalid" => {
            "The checkpoint is not allowed by the selected production workflow."
        }
        "qualification_finalization_failed" => {
            "The qualification run candidate could not be finalized."
        }
        _ => "The qualification operation could not be completed.",
    };
    safe_error(code, message)
}

/// Serialize operations that require no active attempt. Session start holds the
/// same gate across observation and candidate creation, before device or root
/// side effects can occur.
pub(crate) fn with_inactive_qualification_session<T>(
    state: &AppState,
    repository: &crate::qualification_repository::QualificationRepository,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _begin_guard = repository
        .lock_begin()
        .map_err(|_| safe_qualification_error("qualification_session_active"))?;
    crate::qualification_session::recover_persisted_sessions(state, repository)?;
    if crate::qualification_session::session_status(state)?.is_some() {
        return Err(safe_qualification_error("qualification_session_active"));
    }
    operation()
}

/// Recover restart-stable qualification attempts while native app state is
/// being initialized. This runs after RecoveryStore loads the process marker
/// and before any UI status projection is available.
pub(crate) fn recover_sessions_at_process_start(state: &AppState) {
    let mode = QualificationModeState::current(&state.qualification_repository);
    if !mode.enabled || mode.build.is_none() {
        return;
    }
    let Some(repository) = state.qualification_repository.get() else {
        return;
    };
    if crate::qualification_session::recover_persisted_sessions(state, repository).is_err() {
        eprintln!(
            "Qualification session startup recovery could not complete; persisted attempts remain fail-closed."
        );
    }
}

#[tauri::command]
pub fn get_device_qualification_mode_status(
    state: State<'_, AppState>,
) -> Result<QualificationModeStatus, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    if mode.enabled && mode.build.is_some() {
        if let Some(repository) = state.qualification_repository.get() {
            crate::qualification_session::recover_persisted_sessions(&state, repository).map_err(
                |_| safe_qualification_error("qualification_session_recovery_unavailable"),
            )?;
        }
        crate::qualification_session::retry_deferred_finalization(&state)
            .map_err(|_| safe_qualification_error("qualification_session_recovery_unavailable"))?;
    }
    let mut status = qualification_mode_status(&mode, &state.qualification_repository)?;
    // Rust may reconcile persisted session state before projecting status, but
    // the command never infers or observes new product transitions. Disabled
    // builds cannot hold an attempt and must not initialize the repository.
    if status.enabled {
        status.resumable_session = crate::qualification_session::session_status(&state)?;
        status.device_selection_locked =
            crate::qualification_session::device_selection_locked(&state)?;
    }
    Ok(status)
}

fn qualification_mode_status(
    mode: &QualificationModeState,
    provider: &QualificationRepositoryProvider,
) -> Result<QualificationModeStatus, String> {
    if !mode.enabled {
        return Ok(disabled_mode_status());
    }
    let Some(build) = mode.build.as_ref() else {
        return Ok(unavailable_mode_status("qualification_build_unavailable"));
    };
    let Some(repository) = provider.get() else {
        return Ok(unavailable_mode_status(
            "qualification_repository_unavailable",
        ));
    };

    let repository_status = repository
        .describe_and_list_candidates()
        .map_err(|_| safe_qualification_error("qualification_repository_unavailable"))?;
    let description = repository_status.description;
    let workflows = workflows_from_description(&description)?;
    let targets = targets_from_description(&description)?;
    let candidates = repository_status
        .candidates
        .into_iter()
        .map(candidate_summary_from_repository)
        .collect::<Result<Vec<_>, _>>()?;
    let recordable = repository_status.recordable && build == &description.build;
    let message = (!recordable).then(|| {
        "The committed repository state no longer matches this qualification build. Rebuild before recording."
            .to_string()
    });

    Ok(QualificationModeStatus {
        enabled: true,
        recordable,
        device_selection_locked: false,
        message,
        build: Some(description.build),
        runtime_contract: Some(description.runtime_contract),
        workflows,
        targets,
        resumable_candidates: candidates,
        resumable_session: None,
    })
}

fn disabled_mode_status() -> QualificationModeStatus {
    QualificationModeStatus {
        enabled: false,
        recordable: false,
        device_selection_locked: false,
        message: Some(
            "Device qualification mode is unavailable in this application build.".to_string(),
        ),
        build: None,
        runtime_contract: None,
        workflows: Vec::new(),
        targets: Vec::new(),
        resumable_candidates: Vec::new(),
        resumable_session: None,
    }
}

fn unavailable_mode_status(code: &str) -> QualificationModeStatus {
    let mut status = disabled_mode_status();
    status.message = Some(match code {
        "qualification_build_unavailable" => {
            "The application has no recordable qualification build identity.".to_string()
        }
        "qualification_repository_unavailable" => {
            "Qualification definitions are unavailable. Rebuild the qualification application."
                .to_string()
        }
        _ => "Device qualification mode is unavailable in this application build.".to_string(),
    });
    status
}

#[tauri::command]
pub fn create_qualification_target_candidate(
    request: CreateQualificationTargetCandidateRequest,
    state: State<'_, AppState>,
) -> Result<QualificationTargetCandidatePreview, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let build = require_recordable_mode(&mode)?.clone();
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let _begin_guard = repository
        .lock_begin()
        .map_err(|_| safe_qualification_error("qualification_session_active"))?;
    if crate::qualification_session::session_status(&state)?.is_some() {
        return Err(safe_qualification_error("qualification_session_active"));
    }
    repository
        .require_recordable()
        .map_err(|_| safe_qualification_error("qualification_source_changed"))?;
    let payload = {
        let mut source = QualificationObservationSource { state: &state };
        capture_target_registration_payload(&mut source, &request, &build)?
    };
    let handle = repository
        .create_candidate(CandidateKind::TargetRegistration, &payload, None)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    let candidate = repository
        .load_candidate(&handle)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    target_candidate_preview(&candidate)
}

#[tauri::command]
pub fn register_qualification_target(
    candidate_handle: String,
    state: State<'_, AppState>,
) -> Result<QualificationTargetRegistrationResult, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    register_qualification_target_with_mode(&candidate_handle, &mode, &state)
}

pub(crate) fn register_qualification_target_with_mode(
    candidate_handle: &str,
    mode: &QualificationModeState,
    state: &AppState,
) -> Result<QualificationTargetRegistrationResult, String> {
    // Reject disabled/unavailable qualification before recovery can inspect or
    // reconcile a persisted session. A disabled command must have no session
    // or candidate side effects.
    let _ = require_recordable_mode(mode)?;
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    with_inactive_qualification_session(&state, repository, || {
        register_qualification_target_with_repository(
            &candidate_handle,
            mode,
            &state.qualification_repository,
        )
    })
}

fn register_qualification_target_with_repository(
    candidate_handle: &str,
    mode: &QualificationModeState,
    provider: &QualificationRepositoryProvider,
) -> Result<QualificationTargetRegistrationResult, String> {
    let _ = require_recordable_mode(mode)?;
    let repository = provider
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let result = repository
        .register_target(candidate_handle)
        .map_err(|error| qualification_repository_command_error(&error))?;
    if result.operation != QualificationOperation::RegisterTarget
        || result.candidate_kind != CandidateKind::TargetRegistration
        || result.candidate_handle != candidate_handle
    {
        return Err(safe_qualification_error("qualification_candidate_invalid"));
    }
    let target_id = result
        .payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    Ok(QualificationTargetRegistrationResult {
        target_id: target_id.to_string(),
        requires_commit_and_rebuild: true,
    })
}

fn qualification_repository_command_error(error: &str) -> String {
    if error.contains("source state") || error.contains("build identity") {
        safe_qualification_error("qualification_source_changed")
    } else {
        safe_qualification_error("qualification_candidate_invalid")
    }
}

#[tauri::command]
pub fn discard_qualification_candidate(
    candidate_handle: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let _ = require_recordable_mode(&mode)?;
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    repository
        .discard_candidate(&candidate_handle)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    crate::qualification_session::forget_candidate(&state, &candidate_handle);
    Ok(())
}

#[tauri::command]
pub fn begin_qualification_session(
    request: BeginQualificationSessionRequest,
    state: State<'_, AppState>,
) -> Result<crate::qualification_session::QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let build = require_recordable_mode(&mode)?.clone();
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    with_inactive_qualification_session(&state, repository, || {
        repository
            .require_recordable()
            .map_err(|_| safe_qualification_error("qualification_source_changed"))?;
        let description = repository
            .describe()
            .map_err(|_| safe_qualification_error("qualification_repository_unavailable"))?;
        if description.build != build {
            return Err(safe_qualification_error("qualification_source_changed"));
        }
        let workflow = workflows_from_description(&description)?
            .into_iter()
            .find(|workflow| workflow.id == request.workflow_id)
            .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
        let target = targets_from_description(&description)?
            .into_iter()
            .find(|target| target.id == request.target_id)
            .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
        let capture = {
            let mut source = QualificationObservationSource { state: &state };
            source.capture_selected_device(&request.device_handle, &request.device_plan)?
        };
        if workflow.required_capabilities.iter().any(|required| {
            !capture
                .capabilities
                .iter()
                .any(|available| available == required)
        }) {
            return Err(safe_qualification_error("qualification_target_unverified"));
        }
        let captured_at = current_timestamp()?;
        let provisional_payload = serde_json::json!({
            "capturedAt": captured_at,
            "build": build,
            "workflowId": request.workflow_id,
            "workflowVersion": workflow.version,
            "deviceTargetId": request.target_id,
        });
        let candidate_handle = repository
            .create_candidate(CandidateKind::QualificationRun, &provisional_payload, None)
            .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
        let session_handle =
            match crate::qualification_session::session_handle_for_candidate(&candidate_handle) {
                Ok(handle) => handle,
                Err(error) => {
                    let _ = repository.discard_candidate(&candidate_handle);
                    return Err(error);
                }
            };
        let started = crate::qualification_session::begin(
            &state,
            crate::qualification_session::BeginSessionRequest {
                session_handle,
                candidate_handle: candidate_handle.clone(),
                captured_at,
                device_plan: request.device_plan.clone(),
                target: target_binding_from_summary(&target),
                workflow,
                build,
                runtime_contract: description.runtime_contract,
                observation: capture.observation,
            },
        );
        match started {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                // An attempt that never became active must not leave a provisional
                // candidate behind.
                let _ = repository.discard_candidate(&candidate_handle);
                Err(error)
            }
        }
    })
}

#[tauri::command]
pub fn record_qualification_checkpoint(
    session_handle: String,
    checkpoint_id: String,
    outcome: CheckpointOutcome,
    state: State<'_, AppState>,
) -> Result<crate::qualification_session::QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let _ = require_recordable_mode(&mode)?;
    crate::qualification_session::record_checkpoint(
        &state,
        &session_handle,
        &checkpoint_id,
        outcome,
    )
}

/// Close the active attempt as an operator-abandoned invalid candidate. The
/// immutable invalid candidate is materialized so the attempt cannot be
/// resumed, and the product execution state is never changed by abandoning.
#[tauri::command]
pub fn abandon_qualification_session(
    session_handle: String,
    state: State<'_, AppState>,
) -> Result<crate::qualification_session::QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let _ = require_recordable_mode(&mode)?;
    crate::qualification_session::abandon(&state, &session_handle)
}

#[tauri::command]
pub fn record_qualification_run(
    candidate_handle: String,
    state: State<'_, AppState>,
) -> Result<QualificationRunRecordingResult, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let _ = require_recordable_mode(&mode)?;
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    with_inactive_qualification_session(&state, repository, || {
        let result = repository
            .record_run(&candidate_handle)
            .map_err(|error| qualification_repository_command_error(&error))?;
        if result.operation != QualificationOperation::RecordRun
            || result.candidate_kind != CandidateKind::QualificationRun
            || result.candidate_handle != candidate_handle
        {
            return Err(safe_qualification_error("qualification_candidate_invalid"));
        }
        let run_id = result
            .payload
            .get("runId")
            .and_then(Value::as_str)
            .filter(|run_id| !run_id.is_empty())
            .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
        crate::qualification_session::forget_candidate(&state, &candidate_handle);
        Ok(QualificationRunRecordingResult {
            run_id: run_id.to_string(),
        })
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetRegistrationCandidatePayload {
    captured_at: String,
    build: QualificationBuildIdentity,
    target: QualificationTargetCandidateTarget,
}

/// Qualification-specific device observation source.
///
/// Qualification target and session setup explicitly composes the two
/// existing trusted authorities: the passive typed device observation owned by
/// `device_observation`, and the explicit root-check authority owned by
/// `device_qualification`. Neither authority probes on the other's behalf, and
/// the composed observation is committed through the ordinary device
/// observation seam so an active attempt observes exactly the capture that
/// target registration and session start consumed.
struct QualificationObservationSource<'a> {
    state: &'a AppState,
}

impl SelectedDeviceObservationSource for QualificationObservationSource<'_> {
    fn capture_selected_device(
        &mut self,
        device_handle: &str,
        device_plan: &str,
    ) -> Result<SelectedDeviceCapture, String> {
        let mut passive =
            crate::device_observation::AppStateObservationSource { state: self.state };
        let capture = passive.capture_selected_device(device_handle, device_plan)?;
        let root =
            crate::device_qualification::check_device_root_observation(device_handle, self.state)?;
        if root.device_identity != device_handle
            || root.session_epoch != capture.observation.session_epoch.unwrap_or_default()
        {
            return Err(crate::device_observation::unverified_device_error());
        }
        let observation = capture.observation.with_root_state(root.qualification);
        crate::device_observation::commit_selected_observation(self.state, observation.clone())?;
        Ok(SelectedDeviceCapture {
            observation,
            capabilities: capture.capabilities,
        })
    }
}

fn capture_target_registration_payload<S: SelectedDeviceObservationSource>(
    source: &mut S,
    request: &CreateQualificationTargetCandidateRequest,
    build: &QualificationBuildIdentity,
) -> Result<Value, String> {
    let capture = source.capture_selected_device(&request.device_handle, &request.device_plan)?;
    target_registration_payload(&capture, request.connection_type, build)
}

/// Build the canonical target-registration candidate payload from one trusted
/// typed device capture. Every material fact must have been established by the
/// capture: an unobserved fact fails the candidate closed instead of being
/// assumed.
fn target_registration_payload(
    capture: &crate::device_observation::SelectedDeviceCapture,
    connection_type: QualificationConnectionType,
    build: &QualificationBuildIdentity,
) -> Result<Value, String> {
    let observation = &capture.observation;
    let root_state = crate::qualification_session::project_root_state(&required_observation_fact(
        observation.root_state.clone(),
    )?)
    .ok_or_else(|| safe_qualification_error("qualification_target_unverified"))?;
    let target = QualificationTargetCandidateTarget {
        profile_id: observed_fact(required_observation_fact(observation.profile_id.clone())?),
        manufacturer: observed_fact(required_observation_fact(observation.manufacturer.clone())?),
        model: observed_fact(required_observation_fact(observation.model.clone())?),
        android_version: observed_fact(required_observation_fact(
            observation.android_version.clone(),
        )?),
        android_api: observed_fact(required_observation_fact(observation.android_api)?),
        abi_soc_class: observed_fact(required_observation_fact(
            observation.abi_soc_class.clone(),
        )?),
        root_state: QualificationFactPreview {
            value: root_state,
            source: QualificationFactSource::ExplicitRootCheck,
        },
        connection_type: QualificationFactPreview {
            value: connection_type,
            source: QualificationFactSource::OperatorAttestation,
        },
        firmware_build: observed_fact(required_observation_fact(
            observation.firmware_build.clone(),
        )?),
        capabilities: capture.capabilities.clone(),
        deferred_workflows: Vec::new(),
    };
    let payload = TargetRegistrationCandidatePayload {
        captured_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?,
        build: build.clone(),
        target,
    };
    serde_json::to_value(payload)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))
}

/// Require one material observation fact for canonical registration.
fn required_observation_fact<T>(value: Option<T>) -> Result<T, String> {
    value.ok_or_else(|| safe_qualification_error("qualification_target_unverified"))
}

fn observed_fact<T>(value: T) -> QualificationFactPreview<T> {
    QualificationFactPreview {
        value,
        source: QualificationFactSource::ProductionObservation,
    }
}

fn workflows_from_description(
    description: &RepositoryQualificationDescription,
) -> Result<Vec<QualificationWorkflow>, String> {
    let workflows = description
        .workflow_catalog
        .get("workflows")
        .and_then(Value::as_array)
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    workflows
        .iter()
        .cloned()
        .map(|workflow| {
            serde_json::from_value(workflow)
                .map_err(|_| safe_qualification_error("qualification_repository_unavailable"))
        })
        .collect()
}

fn targets_from_description(
    description: &RepositoryQualificationDescription,
) -> Result<Vec<QualificationTargetSummary>, String> {
    let targets = description
        .device_targets
        .get("targets")
        .and_then(Value::as_array)
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    targets.iter().map(target_summary_from_value).collect()
}

fn target_summary_from_value(target: &Value) -> Result<QualificationTargetSummary, String> {
    let id = required_json_string(target, "id")?;
    let profile_id = target_fact_value::<String>(target, "profileId")?;
    let manufacturer = target_fact_value::<String>(target, "manufacturer")?;
    let model = target_fact_value::<String>(target, "model")?;
    let android_version = target_fact_value::<String>(target, "androidVersion")?;
    let android_api = target_fact_value::<u64>(target, "androidApi")?;
    let abi_soc_class = target_fact_value::<String>(target, "abiSocClass")?;
    let root_state = target_fact_value::<QualificationRootState>(target, "rootState")?;
    let connection_type =
        target_fact_value::<QualificationConnectionType>(target, "connectionType")?;
    let firmware_build = target_fact_value::<String>(target, "firmwareBuild")?;
    Ok(QualificationTargetSummary {
        id,
        profile_id,
        manufacturer,
        model,
        android_version,
        android_api,
        abi_soc_class,
        root_state,
        connection_type,
        firmware_build,
    })
}

fn target_fact_value<T: DeserializeOwned>(target: &Value, field: &str) -> Result<T, String> {
    let fact = target
        .get(field)
        .cloned()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let fact: QualificationFactPreview<T> = serde_json::from_value(fact)
        .map_err(|_| safe_qualification_error("qualification_repository_unavailable"))?;
    Ok(fact.value)
}

fn required_json_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))
}

fn candidate_summary_from_repository(
    candidate: RepositoryCandidateSummary,
) -> Result<QualificationCandidateSummaryDto, String> {
    let target = candidate
        .target
        .map(|target| match serde_json::from_value(target) {
            Ok(target) => Ok(Some(target)),
            Err(_) if candidate.kind == CandidateKind::QualificationRun => Ok(None),
            Err(_) => Err(safe_qualification_error("qualification_candidate_invalid")),
        })
        .transpose()?
        .flatten();
    Ok(QualificationCandidateSummaryDto {
        candidate_handle: candidate.candidate_handle,
        kind: candidate.kind,
        captured_at: candidate.captured_at,
        promotable: candidate.promotable,
        non_promotable_reason: candidate.non_promotable_reason,
        target,
        run_validity: candidate.run_validity,
        qualification_outcome: candidate.qualification_outcome,
    })
}

fn current_timestamp() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))
}

fn target_binding_from_summary(
    target: &QualificationTargetSummary,
) -> crate::qualification_session::QualificationTargetBinding {
    crate::qualification_session::QualificationTargetBinding {
        target_id: target.id.clone(),
        profile_id: target.profile_id.clone(),
        manufacturer: target.manufacturer.clone(),
        model: target.model.clone(),
        android_version: target.android_version.clone(),
        android_api: target.android_api,
        abi_soc_class: target.abi_soc_class.clone(),
        root_state: target.root_state.clone(),
        connection_type: target.connection_type,
        firmware_build: target.firmware_build.clone(),
    }
}

fn target_candidate_preview(
    candidate: &StoredQualificationCandidate,
) -> Result<QualificationTargetCandidatePreview, String> {
    if candidate.kind != CandidateKind::TargetRegistration {
        return Err(safe_qualification_error("qualification_candidate_invalid"));
    }
    let captured_at = candidate
        .captured_at
        .clone()
        .ok_or_else(|| safe_qualification_error("qualification_candidate_invalid"))?;
    let target = candidate
        .payload
        .get("target")
        .cloned()
        .ok_or_else(|| safe_qualification_error("qualification_candidate_invalid"))?;
    let target = serde_json::from_value(target)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    Ok(QualificationTargetCandidatePreview {
        candidate_handle: candidate.candidate_handle.clone(),
        kind: candidate.kind,
        captured_at,
        target,
        promotable: candidate.promotable,
        non_promotable_reason: candidate.non_promotable_reason.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::AdbManager;
    use crate::commands::{AppState, InputContractSnapshot, PlatformToolsSelectionStore};
    use crate::device_observation::{SelectedDeviceCapture, SelectedDeviceObservation};
    use crate::device_qualification::{RootQualificationFailureReason, RootQualificationState};
    use crate::execution::ExecutionHandleStore;
    use crate::handles::SessionHandles;
    use crate::qualification_repository::QualificationRepositoryProvider;
    use crate::recovery::RecoveryStore;
    use crate::saved_configurations::SavedConfigurationStore;
    use crate::sidecar::SidecarState;
    use crate::support::SupportStore;
    use crate::updates::{ActivityGate, UpdateService};
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tauri::Manager;

    /// Deterministic stand-in for the production device-observation seam.
    ///
    /// The production implementation composes the ADB probe, the catalog
    /// match, the passive support projection, and the explicit root check into
    /// one typed capture. Tests supply that exact typed result instead, so the
    /// candidate payload projection is proven without a device or a sidecar
    /// process.
    struct FakeDeviceSource {
        capture: Option<SelectedDeviceCapture>,
        error: Option<String>,
        calls: usize,
    }

    impl FakeDeviceSource {
        fn returning(capture: SelectedDeviceCapture) -> Self {
            Self {
                capture: Some(capture),
                error: None,
                calls: 0,
            }
        }

        fn failing(message: &str) -> Self {
            Self {
                capture: None,
                error: Some(message.to_string()),
                calls: 0,
            }
        }
    }

    impl SelectedDeviceObservationSource for FakeDeviceSource {
        fn capture_selected_device(
            &mut self,
            _device_handle: &str,
            _device_plan: &str,
        ) -> Result<SelectedDeviceCapture, String> {
            self.calls += 1;
            if let Some(error) = self.error.take() {
                return Err(error);
            }
            Ok(self
                .capture
                .take()
                .expect("the fake observation source must be consulted once"))
        }
    }

    fn trusted_capture() -> SelectedDeviceCapture {
        let facts = crate::device_observation::DeviceProbeFacts::decode(&json!({
            "manufacturer": "AYANEO",
            "model": "Pocket S2",
            "android_version": 15,
            "android_api_level": 35,
            "firmware_build": "vendor/build"
        }))
        .expect("the test probe payload should decode into typed facts");
        SelectedDeviceCapture {
            observation: SelectedDeviceObservation::new("device-opaque")
                .with_probe_facts(&facts)
                .with_profile_id("ayaneo.pocket_s2")
                .with_snapshot(&crate::device_observation::DeviceQualificationSnapshotDto {
                    state: crate::device_observation::DeviceQualificationState::Supported,
                    summary: "supported",
                    limitations: Vec::new(),
                    android_major: Some(15),
                    android_api_level: Some(35),
                    abi_class: Some("arm64"),
                    storage: crate::device_observation::CapabilityAvailabilityDto::Available,
                    package_manager:
                        crate::device_observation::CapabilityAvailabilityDto::Available,
                    activity_manager:
                        crate::device_observation::CapabilityAvailabilityDto::Unavailable,
                    root: None,
                    runtime_generation: 1,
                    qualification_revision: 1,
                    device_identity: Some("device-opaque".to_string()),
                })
                .with_root_state(RootQualificationState::Denied),
            capabilities: vec!["apk_install".to_string()],
        }
    }

    fn capture_request() -> CreateQualificationTargetCandidateRequest {
        CreateQualificationTargetCandidateRequest {
            device_handle: "device-opaque".to_string(),
            device_plan: "selected-plan".to_string(),
            connection_type: QualificationConnectionType::Usb3,
        }
    }

    fn test_build() -> QualificationBuildIdentity {
        serde_json::from_value(json!({
            "appVersion": "0.1.0",
            "gitCommit": "1".repeat(40),
            "materialBuildDigest": format!("sha256:{}", "a".repeat(64)),
            "realExecutionEnabled": true,
            "qualificationContract": 1
        }))
        .expect("test build should decode")
    }

    fn test_app(
        provider: QualificationRepositoryProvider,
    ) -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
        let temp = tempfile::tempdir().expect("test app directory should be created");
        let app_root = temp.path();
        let app_state = AppState {
            sidecar: SidecarState::new(app_root.join("sidecar-cache")),
            catalog: Err("test catalog is not needed by qualification commands".to_string()),
            qualification_repository: provider,
            qualification_transition_gate: Mutex::new(()),
            adb: Mutex::new(AdbManager::new(app_root.join("platform-tools"))),
            platform_tools_selections: Mutex::new(PlatformToolsSelectionStore::default()),
            input_contracts: Mutex::new(InputContractSnapshot::default()),
            handles: Mutex::new(SessionHandles::default()),
            root_qualification: Mutex::new(
                crate::device_qualification::RootQualificationStore::default(),
            ),
            executions: Mutex::new(ExecutionHandleStore::default()),
            qualification_sessions: Mutex::new(
                crate::qualification_session::QualificationSessionStore::default(),
            ),
            saved_configurations: Mutex::new(SavedConfigurationStore::load(
                app_root.join("recent-configurations.json"),
            )),
            recovery: Mutex::new(RecoveryStore::load(
                app_root.join("recovery-draft.json"),
                app_root.join("session-active.marker"),
            )),
            support: Mutex::new(SupportStore::new(app_root.join("support-cache"))),
            updates: UpdateService::from_production_document()
                .expect("test update trust should be available"),
            update_activity: ActivityGate::default(),
        };
        let app = tauri::test::mock_app();
        assert!(app.manage(app_state));
        (temp, app)
    }

    #[test]
    fn exported_status_keeps_disabled_mode_from_initializing_repository() {
        let (_temp, app) = test_app(QualificationRepositoryProvider::default());
        let status = get_device_qualification_mode_status(app.state())
            .expect("disabled status should be returned through the command");

        assert!(!status.enabled);
        assert!(!status.recordable);
        assert!(status.workflows.is_empty());
        assert!(status.resumable_session.is_none());
        assert!(!app
            .state::<AppState>()
            .qualification_repository
            .is_initialized_for_test());
    }

    #[test]
    fn exported_create_maps_disabled_mode_without_touching_node() {
        let (_temp, app) = test_app(QualificationRepositoryProvider::default());
        let error = create_qualification_target_candidate(capture_request(), app.state())
            .expect_err("disabled mode must reject candidate creation");
        let error: Value = serde_json::from_str(&error).expect("command error should be JSON");

        assert_eq!(error["code"], "qualification_mode_disabled");
        assert!(!app
            .state::<AppState>()
            .qualification_repository
            .is_initialized_for_test());
    }

    #[test]
    fn exported_registration_blocks_after_successful_registration() {
        let build = test_build();
        let runner = RegistrationRunner::default();
        let calls = runner.clone();
        let temp = tempfile::tempdir().expect("test repository directory should be created");
        let repository = crate::qualification_repository::QualificationRepository::new_for_test_with_source_state(
            temp.path().to_path_buf(),
            Box::new(runner),
            build.clone(),
            crate::qualification_repository::QualificationSourceState {
                head: build.git_commit.clone(),
                tracked_worktree_clean: true,
            },
        );
        let candidate = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": serde_json::to_value(&build).expect("build should serialize") }),
                None,
            )
            .expect("candidate should be stored");
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider);

        let result = register_qualification_target(candidate.clone(), app.state())
            .expect("exported registration should succeed once");
        assert!(result.requires_commit_and_rebuild);

        let error = register_qualification_target(candidate, app.state())
            .expect_err("exported registration must block until rebuild");
        let error: Value = serde_json::from_str(&error).expect("command error should be JSON");
        assert_eq!(error["code"], "qualification_source_changed");
        assert_eq!(
            *calls.calls.lock().expect("calls should not be poisoned"),
            1
        );
    }

    #[test]
    fn exported_discard_removes_candidate() {
        let build = test_build();
        let temp = tempfile::tempdir().expect("test repository directory should be created");
        let repository = crate::qualification_repository::QualificationRepository::new_for_test_with_source_state(
            temp.path().to_path_buf(),
            Box::new(RegistrationRunner::default()),
            build.clone(),
            crate::qualification_repository::QualificationSourceState {
                head: build.git_commit.clone(),
                tracked_worktree_clean: true,
            },
        );
        let candidate = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": serde_json::to_value(&build).expect("build should serialize") }),
                None,
            )
            .expect("candidate should be stored");
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider);

        discard_qualification_candidate(candidate.clone(), app.state())
            .expect("exported discard should remove the candidate");
        assert!(app
            .state::<AppState>()
            .qualification_repository
            .get()
            .expect("test repository should be available")
            .load_candidate(&candidate)
            .is_err());
    }

    #[test]
    fn disabled_status_does_not_initialize_or_touch_the_repository() {
        let provider = QualificationRepositoryProvider::default();
        let status = qualification_mode_status(
            &QualificationModeState {
                enabled: false,
                build: None,
            },
            &provider,
        )
        .expect("disabled status should be safe");

        assert!(!status.enabled);
        assert!(!status.recordable);
        assert!(status.workflows.is_empty());
        assert!(status.targets.is_empty());
        assert!(status.resumable_candidates.is_empty());
        assert!(status.resumable_session.is_none());
        assert!(!provider.is_initialized_for_test());
    }

    #[test]
    fn enabled_status_without_a_trusted_checkout_is_sanitized_and_empty() {
        let provider = QualificationRepositoryProvider::unavailable_for_test();
        let status = qualification_mode_status(
            &QualificationModeState {
                enabled: true,
                build: Some(test_build()),
            },
            &provider,
        )
        .expect("unavailable status should be safe");

        assert!(!status.enabled);
        assert!(!status.recordable);
        assert!(status.message.is_some());
        assert!(status.workflows.is_empty());
        assert!(status.targets.is_empty());
        assert!(status.resumable_candidates.is_empty());
    }

    #[test]
    fn registration_command_blocks_the_next_lifecycle_operation_until_rebuild() {
        let temp = tempfile::tempdir().expect("temporary repository should be created");
        let runner = RegistrationRunner::default();
        let calls = runner.clone();
        let build = test_build();
        let repository = crate::qualification_repository::QualificationRepository::new_for_test_with_source_state(
            temp.path().to_path_buf(),
            Box::new(runner),
            build.clone(),
            crate::qualification_repository::QualificationSourceState {
                head: build.git_commit.clone(),
                tracked_worktree_clean: true,
            },
        );
        let candidate = repository
            .create_candidate(
                CandidateKind::TargetRegistration,
                &json!({ "build": serde_json::to_value(&build).expect("build should serialize") }),
                None,
            )
            .expect("candidate should be stored");
        let provider = QualificationRepositoryProvider::for_test(repository);
        let mode = QualificationModeState {
            enabled: true,
            build: Some(build),
        };

        register_qualification_target_with_repository(&candidate, &mode, &provider)
            .expect("first registration should succeed");
        let error = register_qualification_target_with_repository(&candidate, &mode, &provider)
            .expect_err("second registration must wait for a clean rebuild");
        let error: Value = serde_json::from_str(&error).expect("error should be JSON");
        assert_eq!(error["code"], "qualification_source_changed");
        assert_eq!(
            *calls.calls.lock().expect("calls should not be poisoned"),
            1
        );
    }

    #[test]
    fn target_capture_projects_the_exact_trusted_observation_into_the_candidate() {
        let mut source = FakeDeviceSource::returning(trusted_capture());
        let payload =
            capture_target_registration_payload(&mut source, &capture_request(), &test_build())
                .expect("a trusted capture should produce a candidate");

        assert_eq!(source.calls, 1);
        assert_eq!(payload["target"]["profileId"]["value"], "ayaneo.pocket_s2");
        assert_eq!(
            payload["target"]["profileId"]["source"],
            "production_observation"
        );
        assert_eq!(payload["target"]["androidVersion"]["value"], "15");
        assert_eq!(payload["target"]["androidApi"]["value"], 35);
        assert_eq!(payload["target"]["rootState"]["value"], "non_root");
        assert_eq!(
            payload["target"]["rootState"]["source"],
            "explicit_root_check"
        );
        assert_eq!(payload["target"]["connectionType"]["value"], "usb3");
        assert_eq!(
            payload["target"]["connectionType"]["source"],
            "operator_attestation"
        );
        assert_eq!(payload["target"]["capabilities"], json!(["apk_install"]));
        assert_eq!(payload["build"]["qualificationContract"], 1);
        assert!(payload["capturedAt"].as_str().is_some());
        let encoded = payload.to_string();
        assert!(!encoded.contains("device-opaque"));
        assert!(!encoded.contains("serial"));
    }

    #[test]
    fn target_capture_fails_closed_when_the_device_is_not_trusted() {
        let mut source =
            FakeDeviceSource::failing(&safe_qualification_error("qualification_target_unverified"));
        let error =
            capture_target_registration_payload(&mut source, &capture_request(), &test_build())
                .expect_err("an untrusted device must not produce a candidate");

        assert_eq!(source.calls, 1);
        assert!(error.contains("qualification_target_unverified"));
    }

    #[test]
    fn target_capture_rejects_root_checks_that_never_established_a_fact() {
        for root_state in [
            None,
            Some(RootQualificationState::CheckFailed {
                reason: RootQualificationFailureReason::TimedOut,
                message: "timed out".to_string(),
            }),
            Some(RootQualificationState::CheckFailed {
                reason: RootQualificationFailureReason::Transport,
                message: "transport failed".to_string(),
            }),
            Some(RootQualificationState::CheckFailed {
                reason: RootQualificationFailureReason::UnexpectedResponse,
                message: "unexpected response".to_string(),
            }),
        ] {
            let mut capture = trusted_capture();
            capture.observation.root_state = root_state;
            let mut source = FakeDeviceSource::returning(capture);
            let error =
                capture_target_registration_payload(&mut source, &capture_request(), &test_build())
                    .expect_err("an unproven root check must not produce a target candidate");
            assert!(
                error.contains("qualification_target_unverified"),
                "unexpected target-capture error: {error}"
            );
        }
    }

    #[test]
    fn target_capture_projects_completed_root_checks_both_ways() {
        for (root, expected) in [
            (RootQualificationState::Granted, "rooted"),
            (RootQualificationState::Denied, "non_root"),
            (RootQualificationState::Unavailable, "non_root"),
        ] {
            let mut capture = trusted_capture();
            capture.observation.root_state = Some(root);
            let mut source = FakeDeviceSource::returning(capture);
            let payload =
                capture_target_registration_payload(&mut source, &capture_request(), &test_build())
                    .expect("completed root checks should project to the target contract");
            assert_eq!(payload["target"]["rootState"]["value"], expected);
            assert_eq!(
                payload["target"]["rootState"]["source"],
                "explicit_root_check"
            );
        }
    }

    #[derive(Clone, Default)]
    struct RegistrationRunner {
        calls: Arc<Mutex<usize>>,
    }

    impl crate::qualification_repository::QualificationToolRunner for RegistrationRunner {
        fn run(&self, _repo_root: &Path, args: &[String]) -> Result<Vec<u8>, String> {
            if args.first().map(String::as_str) != Some("--register-target") {
                return Err("unexpected test operation".to_string());
            }
            *self.calls.lock().expect("calls should not be poisoned") += 1;
            serde_json::to_vec(&json!({
                "operation": "register_target",
                "candidateHandle": args[1],
                "candidateKind": "target_registration",
                "payload": { "id": "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }
            }))
            .map_err(|_| "test response should serialize".to_string())
        }
    }

    #[test]
    fn qualification_dtos_round_trip_the_camel_case_frontend_contract() {
        let input = json!({
            "enabled": true,
            "recordable": true,
            "deviceSelectionLocked": false,
            "message": null,
            "build": {
                "appVersion": "0.1.0",
                "gitCommit": "1".repeat(40),
                "materialBuildDigest": format!("sha256:{}", "a".repeat(64)),
                "realExecutionEnabled": true,
                "qualificationContract": 1
            },
            "runtimeContract": "real-execution-v1",
            "workflows": [{
                "id": "retroarch-plus-bios",
                "version": 2,
                "purpose": "Provision RetroArch.",
                "productionRecipes": ["app.retroarch.provision"],
                "requiredCapabilities": ["apk_install"],
                "prerequisites": [],
                "humanCheckpoints": [{
                    "id": "clean_or_deliberately_reset_device",
                    "instruction": "Verify the baseline.",
                    "fact": "The baseline was verified.",
                    "allowedOutcomes": ["pass", "fail", "unable_to_verify"],
                    "required": true
                }]
            }],
            "targets": [{
                "id": "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "profileId": "ayaneo.pocket_s2",
                "manufacturer": "AYANEO",
                "model": "Pocket S2",
                "androidVersion": "15",
                "androidApi": 35,
                "abiSocClass": "arm64",
                "rootState": "non_root",
                "connectionType": "usb3",
                "firmwareBuild": "vendor/build"
            }],
            "resumableCandidates": [{
                "candidateHandle": "qualification-candidate-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "kind": "target_registration",
                "capturedAt": "2026-08-23T12:00:00Z",
                "promotable": true,
                "nonPromotableReason": null,
                "target": {
                    "profileId": { "value": "ayaneo.pocket_s2", "source": "production_observation" },
                    "manufacturer": { "value": "AYANEO", "source": "production_observation" },
                    "model": { "value": "Pocket S2", "source": "production_observation" },
                    "androidVersion": { "value": "15", "source": "production_observation" },
                    "androidApi": { "value": 35, "source": "production_observation" },
                    "abiSocClass": { "value": "arm64", "source": "production_observation" },
                    "rootState": { "value": "non_root", "source": "explicit_root_check" },
                    "connectionType": { "value": "usb3", "source": "operator_attestation" },
                    "firmwareBuild": { "value": "vendor/build", "source": "production_observation" },
                    "capabilities": ["apk_install"],
                    "deferredWorkflows": []
                }
            }]
        });

        let mut status: QualificationModeStatus =
            serde_json::from_value(input).expect("camelCase qualification DTO should deserialize");
        let encoded = serde_json::to_value(&status).expect("qualification DTO should serialize");
        assert_eq!(encoded["runtimeContract"], "real-execution-v1");
        assert!(encoded["build"]["appVersion"].is_string());
        assert!(encoded["build"]["qualificationContract"].is_number());
        assert!(encoded["workflows"][0]["productionRecipes"].is_array());
        assert!(encoded["workflows"][0]["humanCheckpoints"][0]["allowedOutcomes"].is_array());
        assert!(encoded["targets"][0]["profileId"].is_string());
        assert!(encoded["targets"][0]["androidApi"].is_number());
        assert!(encoded["resumableCandidates"][0]["candidateHandle"].is_string());
        assert!(encoded["resumableCandidates"][0]["capturedAt"].is_string());
        assert!(encoded["resumableCandidates"][0]["nonPromotableReason"].is_null());
        assert!(
            encoded["resumableCandidates"][0]["target"]["connectionType"]["source"].is_string()
        );
        assert!(encoded.get("resumableSession").is_none());
        let consequence = serde_json::to_value(QualificationTargetRegistrationResult {
            target_id: "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            requires_commit_and_rebuild: true,
        })
        .expect("registration consequence should serialize");
        assert_eq!(
            consequence["targetId"],
            "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(consequence["requiresCommitAndRebuild"], true);
        assert!(consequence.get("target_id").is_none());
        status.message = Some("status remains typed".to_string());
        assert_eq!(status.message.as_deref(), Some("status remains typed"));
    }

    #[test]
    fn disabled_mode_status_is_empty_and_does_not_claim_recordability() {
        let status = disabled_mode_status();
        assert!(!status.enabled);
        assert!(!status.recordable);
        assert!(status
            .message
            .as_deref()
            .is_some_and(|message| message.contains("unavailable")));
        assert!(status.build.is_none());
        assert!(status.runtime_contract.is_none());
        assert!(status.workflows.is_empty());
        assert!(status.targets.is_empty());
        assert!(status.resumable_candidates.is_empty());
    }

    #[test]
    fn recordable_guard_returns_sanitized_stable_errors() {
        let disabled = QualificationModeState {
            enabled: false,
            build: None,
        };
        let unavailable_build = QualificationModeState {
            enabled: true,
            build: None,
        };
        for (state, code) in [
            (&disabled, "qualification_mode_disabled"),
            (&unavailable_build, "qualification_build_unavailable"),
        ] {
            let error = require_recordable_mode(state).expect_err("guard should reject state");
            let value: Value = serde_json::from_str(&error).expect("error should be JSON");
            assert_eq!(value["code"], code);
            assert!(!error.contains("/"));
            assert!(!error.contains("candidatePath"));
        }
    }

    #[test]
    fn target_summary_projects_v2_fact_values_without_reauthoring_them() {
        let target = json!({
            "id": "device-target-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "profileId": { "value": "profile", "source": "production_observation" },
            "manufacturer": { "value": "AYANEO", "source": "production_observation" },
            "model": { "value": "Pocket", "source": "production_observation" },
            "androidVersion": { "value": "15", "source": "production_observation" },
            "androidApi": { "value": 35, "source": "production_observation" },
            "abiSocClass": { "value": "arm64", "source": "production_observation" },
            "rootState": { "value": "non_root", "source": "explicit_root_check" },
            "connectionType": { "value": "usb3", "source": "operator_attestation" },
            "firmwareBuild": { "value": "vendor/build", "source": "production_observation" }
        });
        let summary = target_summary_from_value(&target).expect("target should project");
        assert_eq!(summary.profile_id, "profile");
        assert_eq!(summary.root_state, QualificationRootState::NonRoot);
        assert_eq!(summary.connection_type, QualificationConnectionType::Usb3);
        assert_eq!(summary.android_api, 35);
    }

    #[test]
    fn target_candidate_preview_preserves_the_stored_fact_sources() {
        let target = json!({
            "profileId": { "value": "profile", "source": "production_observation" },
            "manufacturer": { "value": "AYANEO", "source": "production_observation" },
            "model": { "value": "Pocket", "source": "production_observation" },
            "androidVersion": { "value": "15", "source": "production_observation" },
            "androidApi": { "value": 35, "source": "production_observation" },
            "abiSocClass": { "value": "arm64", "source": "production_observation" },
            "rootState": { "value": "non_root", "source": "explicit_root_check" },
            "connectionType": { "value": "usb2", "source": "operator_attestation" },
            "firmwareBuild": { "value": "vendor/build", "source": "production_observation" },
            "capabilities": ["apk_install"],
            "deferredWorkflows": []
        });
        let mut candidate = StoredQualificationCandidate {
            candidate_handle: "qualification-candidate-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            kind: CandidateKind::TargetRegistration,
            captured_at: Some("2026-08-23T12:00:00Z".to_string()),
            build: None,
            payload: json!({ "target": target }),
            report: None,
            promotable: false,
            non_promotable_reason: Some("source changed".to_string()),
        };
        let preview = target_candidate_preview(&candidate).expect("candidate should project");
        assert_eq!(preview.kind, CandidateKind::TargetRegistration);
        assert!(!preview.promotable);
        assert_eq!(
            preview.target.connection_type.source,
            QualificationFactSource::OperatorAttestation
        );
        assert_eq!(
            preview.target.root_state.source,
            QualificationFactSource::ExplicitRootCheck
        );
        candidate.payload["target"]["model"]["value"] = json!("changed only in test");
        assert_eq!(preview.target.model.value, "Pocket");
    }
}
