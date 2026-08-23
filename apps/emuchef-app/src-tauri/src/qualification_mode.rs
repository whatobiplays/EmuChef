//! Development-only target-registration orchestration.
//!
//! This module owns the qualification overlay's typed contract and the small
//! orchestration boundary that captures target facts from existing production
//! device observations. Canonical target validation, IDs, and repository
//! mutation remain owned by `tools/device-qualification.mjs`.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use tauri::State;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::commands::{match_device_observation, probe_device_facts, safe_error, AppState};
use crate::device_qualification::{
    check_device_root_observation, refresh_current_qualification, CapabilityAvailabilityDto,
    DeviceQualificationState, RootQualificationState,
};
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
    pub(crate) message: Option<String>,
    pub(crate) build: Option<QualificationBuildIdentity>,
    pub(crate) runtime_contract: Option<String>,
    pub(crate) workflows: Vec<QualificationWorkflow>,
    pub(crate) targets: Vec<QualificationTargetSummary>,
    pub(crate) resumable_candidates: Vec<QualificationCandidateSummaryDto>,
}

/// The validity of a run candidate is monotonic: a session may become invalid,
/// but a later device observation can never make it valid again.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunValidity {
    Valid,
    Invalid,
}

/// The product result is kept separate from run validity so an interrupted or
/// otherwise invalid run is never presented as a product failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationOutcome {
    Passed,
    Failed,
    NotObserved,
}

/// Stable reasons for invalidating a qualification session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationInvalidation {
    DeviceUnavailable,
    DeviceIdentityChanged,
    TargetProfileChanged,
    ManufacturerChanged,
    ModelChanged,
    AndroidApiChanged,
    AbiSocClassChanged,
    FirmwareBuildChanged,
    RootStateChanged,
    RefreshFailed,
    MissingRequiredCheckpoint,
    RequiredCheckpointUnableToVerify,
    PrerequisiteFailed,
    ExecutionCancelled,
    ExecutionUnavailable,
}

impl QualificationInvalidation {
    fn as_str(self) -> &'static str {
        match self {
            Self::DeviceUnavailable => "device_unavailable",
            Self::DeviceIdentityChanged => "device_identity_changed",
            Self::TargetProfileChanged => "target_profile_changed",
            Self::ManufacturerChanged => "manufacturer_changed",
            Self::ModelChanged => "model_changed",
            Self::AndroidApiChanged => "android_api_changed",
            Self::AbiSocClassChanged => "abi_soc_class_changed",
            Self::FirmwareBuildChanged => "firmware_build_changed",
            Self::RootStateChanged => "root_state_changed",
            Self::RefreshFailed => "refresh_failed",
            Self::MissingRequiredCheckpoint => "missing_required_checkpoint",
            Self::RequiredCheckpointUnableToVerify => "required_checkpoint_unable_to_verify",
            Self::PrerequisiteFailed => "prerequisite_failed",
            Self::ExecutionCancelled => "execution_cancelled",
            Self::ExecutionUnavailable => "execution_unavailable",
        }
    }
}

/// The material device facts observed by the existing production qualification
/// helpers. These facts are compared against the target bound to a session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QualificationDeviceObservation {
    pub(crate) device_identity: String,
    pub(crate) profile_id: String,
    pub(crate) manufacturer: String,
    pub(crate) model: String,
    pub(crate) android_version: String,
    pub(crate) android_api: u64,
    pub(crate) abi_soc_class: String,
    pub(crate) firmware_build: String,
    pub(crate) root_state: QualificationRootState,
}

impl QualificationDeviceObservation {
    #[cfg(test)]
    fn for_test() -> Self {
        Self {
            device_identity: "device-test".to_string(),
            profile_id: "profile.test".to_string(),
            manufacturer: "Test".to_string(),
            model: "Device".to_string(),
            android_version: "15".to_string(),
            android_api: 35,
            abi_soc_class: "arm64".to_string(),
            firmware_build: "test/build".to_string(),
            root_state: QualificationRootState::NonRoot,
        }
    }
}

/// The immutable target facts captured from the repository target catalog at
/// the point a session starts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QualificationTargetBinding {
    pub(crate) target_id: String,
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

/// One explicitly submitted operator checkpoint. The timestamp is assigned at
/// submission and is never recomputed during restart or finalization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordedQualificationCheckpoint {
    pub(crate) checkpoint_id: String,
    pub(crate) outcome: QualificationCheckpointOutcome,
    pub(crate) observed_at: String,
}

/// Strict on-disk representation of a resumable qualification session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedQualificationSession {
    pub(crate) session_schema_version: u64,
    pub(crate) session_handle: String,
    pub(crate) candidate_handle: String,
    pub(crate) captured_at: String,
    pub(crate) device_handle: String,
    pub(crate) target_id: String,
    pub(crate) target: QualificationTargetBinding,
    pub(crate) workflow_id: String,
    pub(crate) workflow_version: u64,
    pub(crate) device_plan: String,
    pub(crate) required_recipes: Vec<String>,
    pub(crate) prerequisites: Vec<String>,
    pub(crate) human_checkpoints: Vec<QualificationWorkflowCheckpoint>,
    pub(crate) automated_observations: Vec<QualificationWorkflowObservation>,
    pub(crate) recorded_checkpoints: Vec<RecordedQualificationCheckpoint>,
    pub(crate) build: QualificationBuildIdentity,
    pub(crate) runtime_contract: String,
    pub(crate) run_validity: RunValidity,
    pub(crate) invalidation: Option<QualificationInvalidation>,
    pub(crate) bound_review_handle: Option<String>,
    pub(crate) bound_execution_handle: Option<String>,
    pub(crate) terminal_execution_status: Option<String>,
    pub(crate) terminal_outcome: QualificationOutcome,
}

/// Sanitized session state returned to the overlay. Handles are opaque and no
/// candidate paths, process arguments, or raw device observations cross this
/// boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QualificationSessionSnapshot {
    pub(crate) session_handle: String,
    pub(crate) target_id: String,
    pub(crate) workflow_id: String,
    pub(crate) workflow_version: u64,
    pub(crate) device_plan: String,
    pub(crate) required_recipes: Vec<String>,
    pub(crate) human_checkpoints: Vec<QualificationWorkflowCheckpoint>,
    pub(crate) recorded_checkpoints: Vec<RecordedQualificationCheckpoint>,
    pub(crate) run_validity: RunValidity,
    pub(crate) qualification_outcome: QualificationOutcome,
    pub(crate) invalid_reason: Option<String>,
    pub(crate) candidate: Option<QualificationCandidateSummaryDto>,
}

const SESSION_SCHEMA_VERSION: u64 = 1;
const SESSION_HANDLE_PREFIX: &str = "qualification-session-";
const SESSION_HANDLE_HEX_LENGTH: usize = 32;

/// Pure lifecycle state for one qualification run candidate.
pub(crate) struct QualificationSession {
    session_handle: String,
    candidate_handle: String,
    captured_at: String,
    device_handle: String,
    target_id: String,
    target: QualificationTargetBinding,
    workflow_id: String,
    workflow_version: u64,
    device_plan: String,
    required_recipes: Vec<String>,
    prerequisites: Vec<String>,
    human_checkpoints: Vec<QualificationWorkflowCheckpoint>,
    automated_observations: Vec<QualificationWorkflowObservation>,
    recorded_checkpoints: Vec<RecordedQualificationCheckpoint>,
    build: QualificationBuildIdentity,
    runtime_contract: String,
    invalidation: Option<QualificationInvalidation>,
    bound_review_handle: Option<String>,
    bound_execution_handle: Option<String>,
    terminal_execution_status: Option<String>,
    terminal_outcome: QualificationOutcome,
}

impl QualificationSession {
    pub(crate) fn new(
        session_handle: String,
        candidate_handle: String,
        captured_at: String,
        device_handle: String,
        target: QualificationTargetBinding,
        workflow: QualificationWorkflow,
        build: QualificationBuildIdentity,
        runtime_contract: String,
    ) -> Result<Self, String> {
        validate_session_handle(&session_handle)?;
        if candidate_handle.is_empty()
            || captured_at.is_empty()
            || device_handle.is_empty()
            || target.target_id.is_empty()
            || workflow.id.is_empty()
            || runtime_contract.is_empty()
        {
            return Err("qualification session metadata is incomplete".to_string());
        }
        let target_id = target.target_id.clone();
        Ok(Self {
            session_handle,
            candidate_handle,
            captured_at,
            device_handle,
            target,
            target_id,
            workflow_id: workflow.id,
            workflow_version: workflow.version,
            device_plan: String::new(),
            required_recipes: workflow.production_recipes,
            prerequisites: workflow.prerequisites,
            human_checkpoints: workflow.human_checkpoints,
            automated_observations: workflow.automated_observations,
            recorded_checkpoints: Vec::new(),
            build,
            runtime_contract,
            invalidation: None,
            bound_review_handle: None,
            bound_execution_handle: None,
            terminal_execution_status: None,
            terminal_outcome: QualificationOutcome::NotObserved,
        })
    }

    pub(crate) fn with_device_plan(mut self, device_plan: String) -> Self {
        self.device_plan = device_plan;
        self
    }

    #[cfg(test)]
    pub(crate) fn for_test(checkpoint_ids: &[&str]) -> Self {
        let checkpoints = checkpoint_ids
            .iter()
            .map(|id| QualificationWorkflowCheckpoint {
                id: (*id).to_string(),
                instruction: "test checkpoint".to_string(),
                fact: "test_fact".to_string(),
                allowed_outcomes: vec![
                    QualificationCheckpointOutcome::Pass,
                    QualificationCheckpointOutcome::Fail,
                    QualificationCheckpointOutcome::UnableToVerify,
                ],
                required: true,
            })
            .collect::<Vec<_>>();
        let workflow = QualificationWorkflow {
            id: "test-workflow".to_string(),
            version: 1,
            purpose: "test".to_string(),
            production_recipes: vec!["test.recipe".to_string()],
            required_capabilities: Vec::new(),
            prerequisites: checkpoint_ids
                .iter()
                .filter(|id| **id == "clean_or_deliberately_reset_device")
                .map(|id| (*id).to_string())
                .collect(),
            human_checkpoints: checkpoints,
            compatibility_dimensions: Vec::new(),
            automated_observations: vec![QualificationWorkflowObservation {
                id: "execution-report".to_string(),
                required: true,
            }],
        };
        let target = QualificationTargetBinding {
            target_id: "target-test".to_string(),
            profile_id: "profile.test".to_string(),
            manufacturer: "Test".to_string(),
            model: "Device".to_string(),
            android_version: "15".to_string(),
            android_api: 35,
            abi_soc_class: "arm64".to_string(),
            root_state: QualificationRootState::NonRoot,
            connection_type: QualificationConnectionType::Usb3,
            firmware_build: "test/build".to_string(),
        };
        let build = QualificationBuildIdentity {
            app_version: "0.1.0".to_string(),
            git_commit: "1".repeat(40),
            material_build_digest: format!("sha256:{}", "a".repeat(64)),
            real_execution_enabled: true,
            qualification_contract: 1,
        };
        let session = Self::new(
            format!("{SESSION_HANDLE_PREFIX}{}", "b".repeat(32)),
            format!("qualification-candidate-{}", "a".repeat(32)),
            "2026-08-23T12:00:00Z".to_string(),
            "device-test".to_string(),
            target,
            workflow,
            build,
            "real-execution-v1".to_string(),
        )
        .expect("test session should be valid");
        session.with_device_plan("test-plan".to_string())
    }

    pub(crate) fn session_handle(&self) -> &str {
        &self.session_handle
    }

    pub(crate) fn candidate_handle(&self) -> &str {
        &self.candidate_handle
    }

    pub(crate) fn device_handle(&self) -> &str {
        &self.device_handle
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub(crate) fn workflow_version(&self) -> u64 {
        self.workflow_version
    }

    pub(crate) fn required_recipes(&self) -> &[String] {
        &self.required_recipes
    }

    pub(crate) fn device_plan(&self) -> &str {
        &self.device_plan
    }

    pub(crate) fn build_identity(&self) -> &QualificationBuildIdentity {
        &self.build
    }

    pub(crate) fn runtime_contract(&self) -> &str {
        &self.runtime_contract
    }

    pub(crate) fn automated_observations(&self) -> &[QualificationWorkflowObservation] {
        &self.automated_observations
    }

    pub(crate) fn terminal_execution_status(&self) -> Option<&str> {
        self.terminal_execution_status.as_deref()
    }

    pub(crate) fn target(&self) -> &QualificationTargetBinding {
        &self.target
    }

    pub(crate) fn bound_review_handle(&self) -> Option<&str> {
        self.bound_review_handle.as_deref()
    }

    pub(crate) fn bound_execution_handle(&self) -> Option<&str> {
        self.bound_execution_handle.as_deref()
    }

    pub(crate) fn bind_review(&mut self, review_handle: String) -> Result<(), String> {
        if self.run_validity() == RunValidity::Invalid || !self.can_bind_product_workflow() {
            return Err("qualification session is not eligible for review binding".to_string());
        }
        if review_handle.is_empty() {
            return Err("qualification review handle is invalid".to_string());
        }
        self.bound_review_handle = Some(review_handle);
        Ok(())
    }

    pub(crate) fn bind_execution(&mut self, execution_handle: String) -> Result<(), String> {
        if self.run_validity() == RunValidity::Invalid
            || self.bound_review_handle.is_none()
            || !self.can_bind_product_workflow()
        {
            return Err("qualification session is not eligible for execution binding".to_string());
        }
        if execution_handle.is_empty() {
            return Err("qualification execution handle is invalid".to_string());
        }
        self.bound_execution_handle = Some(execution_handle);
        Ok(())
    }

    pub(crate) fn run_validity(&self) -> RunValidity {
        if self.invalidation.is_some() {
            RunValidity::Invalid
        } else {
            RunValidity::Valid
        }
    }

    pub(crate) fn qualification_outcome(&self) -> QualificationOutcome {
        if self.run_validity() == RunValidity::Invalid {
            QualificationOutcome::NotObserved
        } else {
            self.terminal_outcome
        }
    }

    pub(crate) fn invalid_reason(&self) -> Option<String> {
        self.invalidation.map(|reason| reason.as_str().to_string())
    }

    pub(crate) fn invalidate(&mut self, reason: QualificationInvalidation) {
        if self.invalidation.is_none() {
            self.invalidation = Some(reason);
            self.terminal_outcome = QualificationOutcome::NotObserved;
        }
    }

    pub(crate) fn observe_matching_device(&mut self, observation: QualificationDeviceObservation) {
        if self.run_validity() == RunValidity::Invalid {
            return;
        }
        let mismatch = if observation.device_identity != self.device_handle {
            Some(QualificationInvalidation::DeviceIdentityChanged)
        } else if observation.profile_id != self.target.profile_id {
            Some(QualificationInvalidation::TargetProfileChanged)
        } else if observation.manufacturer != self.target.manufacturer {
            Some(QualificationInvalidation::ManufacturerChanged)
        } else if observation.model != self.target.model {
            Some(QualificationInvalidation::ModelChanged)
        } else if observation.android_api != self.target.android_api {
            Some(QualificationInvalidation::AndroidApiChanged)
        } else if observation.abi_soc_class != self.target.abi_soc_class {
            Some(QualificationInvalidation::AbiSocClassChanged)
        } else if observation.firmware_build != self.target.firmware_build {
            Some(QualificationInvalidation::FirmwareBuildChanged)
        } else if observation.root_state != self.target.root_state {
            Some(QualificationInvalidation::RootStateChanged)
        } else {
            None
        };
        if let Some(reason) = mismatch {
            self.invalidate(reason);
        }
    }

    pub(crate) fn record_checkpoint(
        &mut self,
        checkpoint_id: &str,
        outcome: QualificationCheckpointOutcome,
    ) -> Result<(), String> {
        let observed_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| "qualification checkpoint timestamp is unavailable".to_string())?;
        self.record_checkpoint_at(checkpoint_id, outcome, &observed_at)
    }

    pub(crate) fn record_checkpoint_at(
        &mut self,
        checkpoint_id: &str,
        outcome: QualificationCheckpointOutcome,
        observed_at: &str,
    ) -> Result<(), String> {
        let declaration = self
            .human_checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == checkpoint_id)
            .ok_or_else(|| "qualification checkpoint is not declared by the workflow".to_string())?;
        if !declaration.allowed_outcomes.contains(&outcome) {
            return Err("qualification checkpoint outcome is not allowed by the workflow".to_string());
        }
        if observed_at.is_empty() {
            return Err("qualification checkpoint timestamp is invalid".to_string());
        }
        let checkpoint = RecordedQualificationCheckpoint {
            checkpoint_id: checkpoint_id.to_string(),
            outcome,
            observed_at: observed_at.to_string(),
        };
        if let Some(existing) = self
            .recorded_checkpoints
            .iter_mut()
            .find(|recorded| recorded.checkpoint_id == checkpoint_id)
        {
            *existing = checkpoint;
        } else {
            self.recorded_checkpoints.push(checkpoint);
        }
        if declaration.required && outcome == QualificationCheckpointOutcome::UnableToVerify {
            self.invalidate(QualificationInvalidation::RequiredCheckpointUnableToVerify);
        } else if self.prerequisites.iter().any(|id| id == checkpoint_id)
            && outcome != QualificationCheckpointOutcome::Pass
        {
            self.invalidate(QualificationInvalidation::PrerequisiteFailed);
        }
        Ok(())
    }

    pub(crate) fn recorded_checkpoints(&self) -> &[RecordedQualificationCheckpoint] {
        &self.recorded_checkpoints
    }

    pub(crate) fn can_bind_product_workflow(&self) -> bool {
        self.run_validity() == RunValidity::Valid
            && self.prerequisites.iter().all(|prerequisite| {
                self.recorded_checkpoints.iter().any(|recorded| {
                    recorded.checkpoint_id == *prerequisite
                        && recorded.outcome == QualificationCheckpointOutcome::Pass
                })
            })
    }

    pub(crate) fn classify_execution(&mut self, status: &str) -> Result<(), String> {
        self.classify_execution_status(Some(status))
    }

    pub(crate) fn classify_execution_status(
        &mut self,
        status: Option<&str>,
    ) -> Result<(), String> {
        self.terminal_execution_status = status.map(ToString::to_string);
        let Some(status) = status else {
            self.invalidate(QualificationInvalidation::ExecutionUnavailable);
            return Ok(());
        };
        if !matches!(
            status,
            "succeeded" | "succeeded_with_warnings" | "failed" | "cancelled"
        ) {
            self.invalidate(QualificationInvalidation::ExecutionUnavailable);
            return Ok(());
        }
        if self.run_validity() == RunValidity::Invalid {
            return Ok(());
        }
        if self
            .human_checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.required)
            .any(|checkpoint| {
                !self.recorded_checkpoints.iter().any(|recorded| {
                    recorded.checkpoint_id == checkpoint.id
                        && recorded.outcome != QualificationCheckpointOutcome::UnableToVerify
                })
            })
        {
            self.invalidate(QualificationInvalidation::MissingRequiredCheckpoint);
            return Ok(());
        }
        if status == "cancelled" {
            self.invalidate(QualificationInvalidation::ExecutionCancelled);
            return Ok(());
        }
        self.terminal_outcome = if status == "failed"
            || self
                .recorded_checkpoints
                .iter()
                .any(|checkpoint| checkpoint.outcome == QualificationCheckpointOutcome::Fail)
        {
            QualificationOutcome::Failed
        } else {
            QualificationOutcome::Passed
        };
        Ok(())
    }

    pub(crate) fn to_persisted(&self) -> PersistedQualificationSession {
        PersistedQualificationSession {
            session_schema_version: SESSION_SCHEMA_VERSION,
            session_handle: self.session_handle.clone(),
            candidate_handle: self.candidate_handle.clone(),
            captured_at: self.captured_at.clone(),
            device_handle: self.device_handle.clone(),
            target_id: self.target_id.clone(),
            target: self.target.clone(),
            workflow_id: self.workflow_id.clone(),
            workflow_version: self.workflow_version,
            device_plan: self.device_plan.clone(),
            required_recipes: self.required_recipes.clone(),
            prerequisites: self.prerequisites.clone(),
            human_checkpoints: self.human_checkpoints.clone(),
            automated_observations: self.automated_observations.clone(),
            recorded_checkpoints: self.recorded_checkpoints.clone(),
            build: self.build.clone(),
            runtime_contract: self.runtime_contract.clone(),
            run_validity: self.run_validity(),
            invalidation: self.invalidation,
            bound_review_handle: self.bound_review_handle.clone(),
            bound_execution_handle: self.bound_execution_handle.clone(),
            terminal_execution_status: self.terminal_execution_status.clone(),
            terminal_outcome: self.qualification_outcome(),
        }
    }

    pub(crate) fn from_persisted(
        persisted: PersistedQualificationSession,
    ) -> Result<Self, String> {
        if persisted.session_schema_version != SESSION_SCHEMA_VERSION {
            return Err("qualification session schema version is unsupported".to_string());
        }
        validate_session_handle(&persisted.session_handle)?;
        if persisted.candidate_handle.is_empty()
            || persisted.captured_at.is_empty()
            || persisted.device_handle.is_empty()
            || persisted.target_id.is_empty()
            || persisted.workflow_id.is_empty()
            || persisted.runtime_contract.is_empty()
        {
            return Err("qualification session metadata is incomplete".to_string());
        }
        if persisted.target_id != persisted.target.target_id
            || persisted.workflow_id.is_empty()
            || persisted.human_checkpoints.iter().any(|checkpoint| checkpoint.id.is_empty())
        {
            return Err("qualification session binding is inconsistent".to_string());
        }
        let mut ids = std::collections::BTreeSet::new();
        for checkpoint in &persisted.recorded_checkpoints {
            if !ids.insert(checkpoint.checkpoint_id.clone()) {
                return Err("qualification session records a checkpoint more than once".to_string());
            }
            let declaration = persisted
                .human_checkpoints
                .iter()
                .find(|declared| declared.id == checkpoint.checkpoint_id)
                .ok_or_else(|| "qualification session records an unknown checkpoint".to_string())?;
            if !declaration.allowed_outcomes.contains(&checkpoint.outcome)
                || checkpoint.observed_at.is_empty()
            {
                return Err("qualification session checkpoint record is invalid".to_string());
            }
        }
        let invalid = persisted.invalidation.is_some();
        if (invalid && persisted.run_validity != RunValidity::Invalid)
            || (!invalid && persisted.run_validity != RunValidity::Valid)
        {
            return Err("qualification session validity is inconsistent".to_string());
        }
        if invalid && persisted.terminal_outcome != QualificationOutcome::NotObserved {
            return Err("invalid qualification session has a product outcome".to_string());
        }
        Ok(Self {
            session_handle: persisted.session_handle,
            candidate_handle: persisted.candidate_handle,
            captured_at: persisted.captured_at,
            device_handle: persisted.device_handle,
            target_id: persisted.target_id,
            target: persisted.target,
            workflow_id: persisted.workflow_id,
            workflow_version: persisted.workflow_version,
            device_plan: persisted.device_plan,
            required_recipes: persisted.required_recipes,
            prerequisites: persisted.prerequisites,
            human_checkpoints: persisted.human_checkpoints,
            automated_observations: persisted.automated_observations,
            recorded_checkpoints: persisted.recorded_checkpoints,
            build: persisted.build,
            runtime_contract: persisted.runtime_contract,
            invalidation: persisted.invalidation,
            bound_review_handle: persisted.bound_review_handle,
            bound_execution_handle: persisted.bound_execution_handle,
            terminal_execution_status: persisted.terminal_execution_status,
            terminal_outcome: persisted.terminal_outcome,
        })
    }

    pub(crate) fn snapshot(
        &self,
        candidate: Option<QualificationCandidateSummaryDto>,
    ) -> QualificationSessionSnapshot {
        QualificationSessionSnapshot {
            session_handle: self.session_handle.clone(),
            target_id: self.target_id.clone(),
            workflow_id: self.workflow_id.clone(),
            workflow_version: self.workflow_version,
            device_plan: self.device_plan.clone(),
            required_recipes: self.required_recipes.clone(),
            human_checkpoints: self.human_checkpoints.clone(),
            recorded_checkpoints: self.recorded_checkpoints.clone(),
            run_validity: self.run_validity(),
            qualification_outcome: self.qualification_outcome(),
            invalid_reason: self.invalid_reason(),
            candidate,
        }
    }
}

fn validate_session_handle(handle: &str) -> Result<(), String> {
    let suffix = handle
        .strip_prefix(SESSION_HANDLE_PREFIX)
        .ok_or_else(|| "qualification session handle is invalid".to_string())?;
    if suffix.len() != SESSION_HANDLE_HEX_LENGTH
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("qualification session handle is invalid".to_string());
    }
    Ok(())
}

/// The production observation boundary consumed by target registration.
/// Qualification mode may orchestrate these calls, but it does not create a
/// second device-probing, matching, qualification, or root authority.
trait QualificationObservationSource {
    fn probe(&mut self, device_handle: &str) -> Result<Value, String>;
    fn match_device(&mut self, device_handle: &str) -> Result<Value, String>;
    fn qualification(
        &mut self,
        device_handle: &str,
    ) -> Result<crate::device_qualification::DeviceQualificationSnapshotDto, String>;
    fn root(
        &mut self,
        device_handle: &str,
    ) -> Result<crate::device_qualification::RootQualificationCheckDto, String>;
}

struct AppStateQualificationObservationSource<'a> {
    state: &'a AppState,
}

impl QualificationObservationSource for AppStateQualificationObservationSource<'_> {
    fn probe(&mut self, device_handle: &str) -> Result<Value, String> {
        probe_device_facts(device_handle, self.state).map(|(facts, _)| facts)
    }

    fn match_device(&mut self, device_handle: &str) -> Result<Value, String> {
        match_device_observation(device_handle, self.state)
    }

    fn qualification(
        &mut self,
        device_handle: &str,
    ) -> Result<crate::device_qualification::DeviceQualificationSnapshotDto, String> {
        refresh_current_qualification(self.state, Some(device_handle))
            .map(|current| current.snapshot)
    }

    fn root(
        &mut self,
        device_handle: &str,
    ) -> Result<crate::device_qualification::RootQualificationCheckDto, String> {
        check_device_root_observation(device_handle, self.state)
    }
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
struct QualificationModeState {
    enabled: bool,
    build: Option<QualificationBuildIdentity>,
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
            "The qualification target candidate is invalid or no longer available."
        }
        "qualification_target_unverified" => {
            "The connected device target could not be verified from trusted observations."
        }
        "qualification_candidate_invalid" => {
            "The qualification session or candidate is invalid or no longer available."
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

#[tauri::command]
pub fn get_device_qualification_mode_status(
    state: State<'_, AppState>,
) -> Result<QualificationModeStatus, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    qualification_mode_status(&mode, &state.qualification_repository)
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
        message,
        build: Some(description.build),
        runtime_contract: Some(description.runtime_contract),
        workflows,
        targets,
        resumable_candidates: candidates,
    })
}

fn disabled_mode_status() -> QualificationModeStatus {
    QualificationModeStatus {
        enabled: false,
        recordable: false,
        message: Some(
            "Device qualification mode is unavailable in this application build.".to_string(),
        ),
        build: None,
        runtime_contract: None,
        workflows: Vec::new(),
        targets: Vec::new(),
        resumable_candidates: Vec::new(),
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
    repository
        .require_recordable()
        .map_err(|_| safe_qualification_error("qualification_source_changed"))?;
    let payload = capture_target_registration_payload(&state, &request, &build)?;
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
    register_qualification_target_with_repository(
        &candidate_handle,
        &mode,
        &state.qualification_repository,
    )
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
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))
}

#[tauri::command]
pub fn begin_qualification_session(
    request: BeginQualificationSessionRequest,
    state: State<'_, AppState>,
) -> Result<QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let build = require_recordable_mode(&mode)?.clone();
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    repository
        .require_recordable()
        .map_err(|_| safe_qualification_error("qualification_source_changed"))?;
    let description = repository
        .describe()
        .map_err(|_| safe_qualification_error("qualification_repository_unavailable"))?;
    if description.build != build {
        return Err(safe_qualification_error("qualification_source_changed"));
    }
    let workflows = workflows_from_description(&description)?;
    let workflow = workflows
        .into_iter()
        .find(|workflow| workflow.id == request.workflow_id)
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let targets = targets_from_description(&description)?;
    let target = targets
        .into_iter()
        .find(|target| target.id == request.target_id)
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let target_binding = target_binding_from_summary(&target);
    let (observation, capabilities) = {
        let mut source = AppStateQualificationObservationSource { state: &state };
        observe_device_from_source(&mut source, &request.device_handle, &request.device_plan)?
    };
    if workflow
        .required_capabilities
        .iter()
        .any(|required| !capabilities.iter().any(|available| available == required))
    {
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
        .create_candidate(
            CandidateKind::QualificationRun,
            &provisional_payload,
            None,
        )
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    let session_handle = session_handle_for_candidate(&candidate_handle)?;
    let mut session = QualificationSession::new(
        session_handle,
        candidate_handle.clone(),
        provisional_payload["capturedAt"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        request.device_handle,
        target_binding,
        workflow,
        build,
        description.runtime_contract,
    )?
    .with_device_plan(request.device_plan);
    session.observe_matching_device(observation);
    if session.run_validity() == RunValidity::Invalid {
        let _ = repository.discard_candidate(&candidate_handle);
        return Err(safe_qualification_error("qualification_target_unverified"));
    }
    if let Err(error) = repository.save_session(&candidate_handle, &session.to_persisted()) {
        let _ = repository.discard_candidate(&candidate_handle);
        return Err(if error.contains("source state") {
            safe_qualification_error("qualification_source_changed")
        } else {
            safe_qualification_error("qualification_candidate_invalid")
        });
    }
    session_snapshot(repository, &session)
}

#[tauri::command]
pub fn refresh_qualification_session(
    session_handle: String,
    device_handle: String,
    state: State<'_, AppState>,
) -> Result<QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let _ = require_recordable_mode(&mode)?;
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let candidate_handle = session_candidate_handle(&session_handle)?;
    let mut session = load_session(repository, &session_handle, &candidate_handle)?;
    if session.device_handle() != device_handle {
        session.invalidate(QualificationInvalidation::DeviceIdentityChanged);
    } else {
        let observation = {
            let mut source = AppStateQualificationObservationSource { state: &state };
            observe_device_from_source(&mut source, &device_handle, &session.device_plan)
        };
        match observation {
            Ok((observation, _)) => session.observe_matching_device(observation),
            Err(_) => session.invalidate(QualificationInvalidation::RefreshFailed),
        }
    }
    repository
        .save_session(&candidate_handle, &session.to_persisted())
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    session_snapshot(repository, &session)
}

#[tauri::command]
pub fn bind_qualification_review(
    session_handle: String,
    review_handle: String,
    state: State<'_, AppState>,
) -> Result<QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let _ = require_recordable_mode(&mode)?;
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let candidate_handle = session_candidate_handle(&session_handle)?;
    let mut session = load_session(repository, &session_handle, &candidate_handle)?;
    let review = state
        .handles
        .lock()
        .map_err(|_| safe_qualification_error("qualification_review_mismatch"))?
        .review(&review_handle)
        .map_err(|_| safe_qualification_error("qualification_review_mismatch"))?
        .clone();
    if !review_matches_session(&session, &review) {
        return Err(safe_qualification_error("qualification_review_mismatch"));
    }
    session
        .bind_review(review_handle)
        .map_err(|_| safe_qualification_error("qualification_checkpoint_invalid"))?;
    repository
        .save_session(&candidate_handle, &session.to_persisted())
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    session_snapshot(repository, &session)
}

#[tauri::command]
pub fn bind_qualification_execution(
    session_handle: String,
    execution_handle: String,
    state: State<'_, AppState>,
) -> Result<QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let _ = require_recordable_mode(&mode)?;
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let candidate_handle = session_candidate_handle(&session_handle)?;
    let mut session = load_session(repository, &session_handle, &candidate_handle)?;
    let binding = state
        .executions
        .lock()
        .map_err(|_| safe_qualification_error("qualification_execution_mismatch"))?
        .qualification_binding(&execution_handle)
        .map_err(|_| safe_qualification_error("qualification_execution_mismatch"))?;
    if !binding.real
        || binding.device_handle != session.device_handle()
        || binding.review_handle != session.bound_review_handle().unwrap_or_default()
        || !review_matches_session(&session, &binding.review)
    {
        return Err(safe_qualification_error("qualification_execution_mismatch"));
    }
    session
        .bind_execution(execution_handle)
        .map_err(|_| safe_qualification_error("qualification_execution_mismatch"))?;
    repository
        .save_session(&candidate_handle, &session.to_persisted())
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    session_snapshot(repository, &session)
}

#[tauri::command]
pub fn record_qualification_checkpoint(
    session_handle: String,
    checkpoint_id: String,
    outcome: CheckpointOutcome,
    state: State<'_, AppState>,
) -> Result<QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let _ = require_recordable_mode(&mode)?;
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    let candidate_handle = session_candidate_handle(&session_handle)?;
    let mut session = load_session(repository, &session_handle, &candidate_handle)?;
    session
        .record_checkpoint(&checkpoint_id, outcome)
        .map_err(|_| safe_qualification_error("qualification_checkpoint_invalid"))?;
    repository
        .save_session(&candidate_handle, &session.to_persisted())
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    session_snapshot(repository, &session)
}

#[tauri::command]
pub fn finalize_qualification_candidate(
    session_handle: String,
    state: State<'_, AppState>,
) -> Result<QualificationSessionSnapshot, String> {
    let mode = QualificationModeState::current(&state.qualification_repository);
    let build = require_recordable_mode(&mode)?.clone();
    let repository = state
        .qualification_repository
        .get()
        .ok_or_else(|| safe_qualification_error("qualification_repository_unavailable"))?;
    repository
        .require_recordable()
        .map_err(|_| safe_qualification_error("qualification_source_changed"))?;
    let candidate_handle = session_candidate_handle(&session_handle)?;
    let mut session = load_session(repository, &session_handle, &candidate_handle)?;
    if session.build_identity() != &build {
        session.invalidate(QualificationInvalidation::ExecutionUnavailable);
    }
    let mut report_bytes = None;
    if let Some(execution_handle) = session.bound_execution_handle().map(str::to_string) {
        let binding = state
            .executions
            .lock()
            .ok()
            .and_then(|executions| executions.qualification_binding(&execution_handle).ok());
        let Some(binding) = binding else {
            session.invalidate(QualificationInvalidation::ExecutionUnavailable);
            session.classify_execution_status(None)?;
            repository
                .save_session(&candidate_handle, &session.to_persisted())
                .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
            let payload = run_candidate_payload(repository, &session, None)?;
            repository
                .finalize_candidate(
                    &candidate_handle,
                    CandidateKind::QualificationRun,
                    &payload,
                    None,
                )
                .map_err(|_| safe_qualification_error("qualification_finalization_failed"))?;
            return session_snapshot(repository, &session);
        };
        if !binding.real
            || binding.device_handle != session.device_handle()
            || binding.review_handle != session.bound_review_handle().unwrap_or_default()
            || !review_matches_session(&session, &binding.review)
        {
            session.invalidate(QualificationInvalidation::ExecutionUnavailable);
        } else {
            session.classify_execution_status(binding.status.as_deref())?;
            if binding.terminal
                && binding.report_available
                && binding.status.as_deref().is_some_and(is_terminal_execution_status)
            {
                let report = state
                    .executions
                    .lock()
                    .map_err(|_| safe_qualification_error("qualification_execution_unavailable"))
                    .and_then(|executions| {
                        crate::execution::production_execution_report_bytes(
                            &executions,
                            &execution_handle,
                        )
                    });
                match report {
                    Ok(bytes) => report_bytes = Some(bytes),
                    Err(_) => session.invalidate(QualificationInvalidation::ExecutionUnavailable),
                }
            } else if session.run_validity() == RunValidity::Valid {
                session.invalidate(QualificationInvalidation::ExecutionUnavailable);
            }
        }
    } else {
        session.classify_execution_status(None)?;
    }
    repository
        .save_session(&candidate_handle, &session.to_persisted())
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    let payload = run_candidate_payload(repository, &session, report_bytes.as_deref())?;
    repository
        .finalize_candidate(
            &candidate_handle,
            CandidateKind::QualificationRun,
            &payload,
            report_bytes.as_deref(),
        )
        .map_err(|_| safe_qualification_error("qualification_finalization_failed"))?;
    session_snapshot(repository, &session)
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
    Ok(QualificationRunRecordingResult {
        run_id: run_id.to_string(),
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetRegistrationCandidatePayload {
    captured_at: String,
    build: QualificationBuildIdentity,
    target: QualificationTargetCandidateTarget,
}

fn capture_target_registration_payload(
    state: &AppState,
    request: &CreateQualificationTargetCandidateRequest,
    build: &QualificationBuildIdentity,
) -> Result<Value, String> {
    let mut source = AppStateQualificationObservationSource { state };
    capture_target_registration_payload_from(&mut source, request, build)
}

fn capture_target_registration_payload_from(
    source: &mut impl QualificationObservationSource,
    request: &CreateQualificationTargetCandidateRequest,
    build: &QualificationBuildIdentity,
) -> Result<Value, String> {
    let (observation, capabilities) = observe_device_from_source(
        source,
        &request.device_handle,
        &request.device_plan,
    )?;
    let target = QualificationTargetCandidateTarget {
        profile_id: observed_fact(observation.profile_id),
        manufacturer: observed_fact(observation.manufacturer),
        model: observed_fact(observation.model),
        android_version: observed_fact(observation.android_version),
        android_api: observed_fact(observation.android_api),
        abi_soc_class: observed_fact(observation.abi_soc_class),
        root_state: QualificationFactPreview {
            value: observation.root_state,
            source: QualificationFactSource::ExplicitRootCheck,
        },
        connection_type: QualificationFactPreview {
            value: request.connection_type,
            source: QualificationFactSource::OperatorAttestation,
        },
        firmware_build: observed_fact(observation.firmware_build),
        capabilities,
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

fn observe_device_from_source(
    source: &mut impl QualificationObservationSource,
    device_handle: &str,
    device_plan: &str,
) -> Result<(QualificationDeviceObservation, Vec<String>), String> {
    let facts = source.probe(device_handle)?;
    let matched = source.match_device(device_handle)?;
    let profile_id = matched_profile_id(&matched, device_plan).ok_or_else(|| {
        safe_error(
            "device_plan_unmatched",
            "The selected device plan is not a trusted match for this device.",
        )
    })?;
    let snapshot = source.qualification(device_handle)?;
    if snapshot.state != DeviceQualificationState::Supported
        || snapshot.device_identity.as_deref() != Some(device_handle)
        || snapshot.android_api_level.is_none()
        || snapshot.abi_class.is_none()
    {
        return Err(safe_qualification_error("qualification_target_unverified"));
    }
    let root = source.root(device_handle)?;
    if root.device_identity != device_handle {
        return Err(safe_qualification_error("qualification_target_unverified"));
    }
    let root_state = match root.qualification {
        RootQualificationState::Granted => QualificationRootState::Rooted,
        RootQualificationState::Denied => QualificationRootState::NonRoot,
        RootQualificationState::Unavailable | RootQualificationState::CheckFailed { .. } => {
            return Err(safe_qualification_error("qualification_target_unverified"));
        }
    };
    let android_version = required_fact_string(&facts, "android_version")?;
    let android_api = required_fact_u64(&facts, "android_api_level")?;
    let manufacturer = required_fact_string(&facts, "manufacturer")?;
    let model = required_fact_string(&facts, "model")?;
    let firmware_build = required_fact_string(&facts, "firmware_build")?;
    if snapshot.android_api_level.map(u64::from) != Some(android_api) {
        return Err(safe_qualification_error("qualification_target_unverified"));
    }
    let abi_soc_class = snapshot
        .abi_class
        .map(str::to_string)
        .ok_or_else(|| safe_qualification_error("qualification_target_unverified"))?;
    Ok((
        QualificationDeviceObservation {
            device_identity: device_handle.to_string(),
            profile_id,
            manufacturer,
            model,
            android_version,
            android_api,
            abi_soc_class,
            firmware_build,
            root_state,
        },
        capabilities_from_snapshot(&snapshot),
    ))
}

fn observed_fact<T>(value: T) -> QualificationFactPreview<T> {
    QualificationFactPreview {
        value,
        source: QualificationFactSource::ProductionObservation,
    }
}

fn capabilities_from_snapshot(
    snapshot: &crate::device_qualification::DeviceQualificationSnapshotDto,
) -> Vec<String> {
    let mut capabilities = Vec::new();
    if snapshot.package_manager == CapabilityAvailabilityDto::Available {
        capabilities.push("apk_install".to_string());
    }
    if snapshot.storage == CapabilityAvailabilityDto::Available {
        capabilities.push("shared_storage_write".to_string());
    }
    capabilities
}

fn required_fact_string(facts: &Value, field: &str) -> Result<String, String> {
    let value = facts
        .get(field)
        .filter(|value| !value.is_null())
        .ok_or_else(|| safe_qualification_error("qualification_target_unverified"))?;
    match value {
        Value::String(value) if !value.is_empty() => Ok(value.clone()),
        Value::Number(value) if field == "android_version" => Ok(value.to_string()),
        _ => Err(safe_qualification_error("qualification_target_unverified")),
    }
}

fn required_fact_u64(facts: &Value, field: &str) -> Result<u64, String> {
    facts
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| safe_qualification_error("qualification_target_unverified"))
}

fn matched_profile_id(result: &Value, device_plan: &str) -> Option<String> {
    ["candidates", "safeGenericPlans", "blankSetupPlans"]
        .into_iter()
        .filter_map(|field| result.get(field).and_then(Value::as_array))
        .flatten()
        .find(|candidate| candidate.get("planId").and_then(Value::as_str) == Some(device_plan))
        .and_then(|candidate| candidate.get("profileId").and_then(Value::as_str))
        .filter(|profile_id| !profile_id.is_empty())
        .map(str::to_string)
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

fn target_binding_from_summary(target: &QualificationTargetSummary) -> QualificationTargetBinding {
    QualificationTargetBinding {
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

fn session_handle_for_candidate(candidate_handle: &str) -> Result<String, String> {
    let suffix = candidate_handle
        .strip_prefix(crate::qualification_repository::CANDIDATE_HANDLE_PREFIX)
        .ok_or_else(|| safe_qualification_error("qualification_candidate_invalid"))?;
    let handle = format!("{SESSION_HANDLE_PREFIX}{suffix}");
    validate_session_handle(&handle)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    Ok(handle)
}

fn session_candidate_handle(session_handle: &str) -> Result<String, String> {
    validate_session_handle(session_handle)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    let suffix = session_handle
        .strip_prefix(SESSION_HANDLE_PREFIX)
        .expect("validated session handle has the expected prefix");
    Ok(format!(
        "{}{}",
        crate::qualification_repository::CANDIDATE_HANDLE_PREFIX,
        suffix
    ))
}

fn load_session(
    repository: &crate::qualification_repository::QualificationRepository,
    session_handle: &str,
    candidate_handle: &str,
) -> Result<QualificationSession, String> {
    let persisted = repository
        .load_session(candidate_handle)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    if persisted.session_handle != session_handle {
        return Err(safe_qualification_error("qualification_candidate_invalid"));
    }
    QualificationSession::from_persisted(persisted)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))
}

fn session_snapshot(
    repository: &crate::qualification_repository::QualificationRepository,
    session: &QualificationSession,
) -> Result<QualificationSessionSnapshot, String> {
    let candidate = repository
        .load_candidate(session.candidate_handle())
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    let summary = candidate_summary_from_repository(candidate.summary())?;
    Ok(session.snapshot(Some(summary)))
}

fn review_matches_session(
    session: &QualificationSession,
    review: &crate::handles::ReviewedPlanSnapshot,
) -> bool {
    if review.device_handle != session.device_handle()
        || review
            .response
            .get("devicePlan")
            .and_then(Value::as_str)
            != Some(session.device_plan())
    {
        return false;
    }
    let Some(recipes) = review.response.get("selectedRecipes").and_then(Value::as_array) else {
        return false;
    };
    let recipes = recipes
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>();
    if recipes.as_deref() != Some(session.required_recipes().iter().map(String::as_str).collect::<Vec<_>>().as_slice()) {
        return false;
    }
    let target = &review.target;
    if let Some(target_id) = target
        .get("id")
        .or_else(|| review.response.get("deviceTargetId"))
        .or_else(|| review.response.get("targetId"))
        .and_then(Value::as_str)
    {
        if target_id != session.target_id() {
            return false;
        }
    }
    target.get("manufacturer").and_then(Value::as_str) == Some(&session.target().manufacturer)
        && target.get("model").and_then(Value::as_str) == Some(&session.target().model)
        && target
            .get("androidApiLevel")
            .and_then(Value::as_u64)
            == Some(session.target().android_api)
}

fn is_terminal_execution_status(status: &str) -> bool {
    matches!(
        status,
        "succeeded" | "succeeded_with_warnings" | "failed" | "cancelled"
    )
}

fn run_candidate_payload(
    repository: &crate::qualification_repository::QualificationRepository,
    session: &QualificationSession,
    report_bytes: Option<&[u8]>,
) -> Result<Value, String> {
    let mut authored_content = Vec::new();
    for recipe_id in session.required_recipes() {
        if recipe_id.is_empty() || recipe_id.contains('/') || recipe_id.contains('\\') || recipe_id.contains("..") {
            return Err(safe_qualification_error("qualification_finalization_failed"));
        }
        let path = repository
            .repo_root()
            .join("authored/recipes")
            .join(format!("{recipe_id}.yaml"));
        let bytes = std::fs::read(path)
            .map_err(|_| safe_qualification_error("qualification_finalization_failed"))?;
        authored_content.push(serde_json::json!({
            "id": recipe_id,
            "sha256": hex::encode(Sha256::digest(bytes)),
        }));
    }
    let target = session.target();
    let target_root_state = serde_json::to_value(&target.root_state)
        .map_err(|_| safe_qualification_error("qualification_finalization_failed"))?;
    let target_connection_type = serde_json::to_value(target.connection_type)
        .map_err(|_| safe_qualification_error("qualification_finalization_failed"))?;
    let fingerprint = serde_json::json!({
        "schemaVersion": 2,
        "emuchefBuild": session.build_identity(),
        "workflowVersion": session.workflow_version(),
        "authoredContent": authored_content,
        "runtimeContract": session.runtime_contract(),
        "deviceProfile": target.profile_id,
        "androidApi": target.android_api,
        "firmwareBuild": target.firmware_build,
        "abiSocClass": target.abi_soc_class,
        "rootState": target_root_state,
        "connectionType": target_connection_type,
    });
    let valid = session.run_validity() == RunValidity::Valid;
    let observed_at = current_timestamp()?;
    let automated_observations = if valid {
        let outcome = match session.terminal_execution_status() {
            Some("succeeded") | Some("succeeded_with_warnings") => "passed",
            Some("failed") => "failed",
            _ => return Err(safe_qualification_error("qualification_finalization_failed")),
        };
        session
            .automated_observations()
            .iter()
            .filter(|observation| observation.id == "execution-report")
            .map(|observation| {
                serde_json::json!({
                    "id": observation.id,
                    "outcome": outcome,
                    "observedAt": observed_at,
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let human_checkpoints = session
        .recorded_checkpoints()
        .iter()
        .map(|checkpoint| {
            serde_json::json!({
                "checkpointId": checkpoint.checkpoint_id,
                "outcome": checkpoint.outcome,
                "observedAt": checkpoint.observed_at,
            })
        })
        .collect::<Vec<_>>();
    let artifacts = report_bytes
        .map(|bytes| {
            vec![serde_json::json!({
                "id": "execution-report",
                "kind": "production_execution_report",
                "path": "execution-report.json",
                "sha256": hex::encode(Sha256::digest(bytes)),
            })]
        })
        .unwrap_or_default();
    Ok(serde_json::json!({
        "candidateSchemaVersion": 1,
        "candidateId": session.candidate_handle(),
        "kind": "qualification_run",
        "capturedAt": session.to_persisted().captured_at,
        "build": session.build_identity(),
        "workflowId": session.workflow_id(),
        "workflowVersion": session.workflow_version(),
        "deviceTargetId": target.target_id,
        "fingerprint": fingerprint,
        "runValidity": session.run_validity(),
        "qualificationOutcome": session.qualification_outcome(),
        "automatedObservations": automated_observations,
        "humanCheckpoints": human_checkpoints,
        "targetWideFailure": Value::Null,
        "limitations": session
            .invalid_reason()
            .into_iter()
            .collect::<Vec<_>>(),
        "artifacts": artifacts,
    }))
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
    use crate::device_qualification::{
        CapabilityAvailabilityDto, DeviceQualificationSnapshotDto, RootQualificationCheckDto,
        RootQualificationFailureReason,
    };
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

    #[derive(Clone)]
    struct FakeObservationSource {
        calls: Vec<&'static str>,
        facts: Value,
        matched: Value,
        qualification: DeviceQualificationSnapshotDto,
        root: RootQualificationCheckDto,
    }

    impl QualificationObservationSource for FakeObservationSource {
        fn probe(&mut self, _device_handle: &str) -> Result<Value, String> {
            self.calls.push("probe");
            Ok(self.facts.clone())
        }

        fn match_device(&mut self, _device_handle: &str) -> Result<Value, String> {
            self.calls.push("match");
            Ok(self.matched.clone())
        }

        fn qualification(
            &mut self,
            _device_handle: &str,
        ) -> Result<DeviceQualificationSnapshotDto, String> {
            self.calls.push("qualification");
            Ok(self.qualification.clone())
        }

        fn root(&mut self, device_handle: &str) -> Result<RootQualificationCheckDto, String> {
            self.calls.push("root");
            let mut root = self.root.clone();
            root.device_identity = device_handle.to_string();
            Ok(root)
        }
    }

    fn observation_source() -> FakeObservationSource {
        FakeObservationSource {
            calls: Vec::new(),
            facts: json!({
                "manufacturer": "AYANEO",
                "model": "Pocket S2",
                "android_version": 15,
                "android_api_level": 35,
                "firmware_build": "vendor/build"
            }),
            matched: json!({
                "candidates": [{ "planId": "selected-plan", "profileId": "ayaneo.pocket_s2" }]
            }),
            qualification: DeviceQualificationSnapshotDto {
                state: DeviceQualificationState::Supported,
                summary: "supported",
                limitations: Vec::new(),
                android_major: Some(15),
                android_api_level: Some(35),
                abi_class: Some("arm64"),
                storage: CapabilityAvailabilityDto::Available,
                package_manager: CapabilityAvailabilityDto::Available,
                activity_manager: CapabilityAvailabilityDto::Unavailable,
                root: None,
                runtime_generation: 1,
                qualification_revision: 1,
                device_identity: Some("device-opaque".to_string()),
            },
            root: RootQualificationCheckDto {
                qualification: RootQualificationState::Denied,
                runtime_generation: 1,
                qualification_revision: 1,
                device_identity: "device-opaque".to_string(),
            },
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
            adb: Mutex::new(AdbManager::new(app_root.join("platform-tools"))),
            platform_tools_selections: Mutex::new(PlatformToolsSelectionStore::default()),
            input_contracts: Mutex::new(InputContractSnapshot::default()),
            handles: Mutex::new(SessionHandles::default()),
            root_qualification: Mutex::new(
                crate::device_qualification::RootQualificationStore::default(),
            ),
            executions: Mutex::new(ExecutionHandleStore::default()),
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
    fn target_capture_uses_the_production_observation_order() {
        let mut source = observation_source();
        let payload = capture_target_registration_payload_from(
            &mut source,
            &capture_request(),
            &test_build(),
        )
        .expect("trusted observations should produce a candidate");

        assert_eq!(
            source.calls,
            vec!["probe", "match", "qualification", "root"]
        );
        assert_eq!(payload["target"]["profileId"]["value"], "ayaneo.pocket_s2");
        assert_eq!(payload["target"]["androidVersion"]["value"], "15");
        assert_eq!(payload["target"]["rootState"]["value"], "non_root");
        assert_eq!(
            payload["target"]["connectionType"]["source"],
            "operator_attestation"
        );
    }

    #[test]
    fn target_capture_rejects_non_string_identity_observations() {
        for field in ["manufacturer", "model", "firmware_build"] {
            let mut source = observation_source();
            source.facts[field] = json!(35);
            assert!(
                capture_target_registration_payload_from(
                    &mut source,
                    &capture_request(),
                    &test_build(),
                )
                .is_err(),
                "{field} must remain a string"
            );
        }
    }

    #[test]
    fn target_capture_accepts_numeric_android_version_but_rejects_unverified_root() {
        let mut numeric_version = observation_source();
        let payload = capture_target_registration_payload_from(
            &mut numeric_version,
            &capture_request(),
            &test_build(),
        )
        .expect("numeric Android version should normalize");
        assert_eq!(payload["target"]["androidVersion"]["value"], "15");

        let mut granted = observation_source();
        granted.root.qualification = RootQualificationState::Granted;
        let granted_payload = capture_target_registration_payload_from(
            &mut granted,
            &capture_request(),
            &test_build(),
        )
        .expect("granted root should be recorded as rooted");
        assert_eq!(granted_payload["target"]["rootState"]["value"], "rooted");

        for root in [
            RootQualificationState::Unavailable,
            RootQualificationState::CheckFailed {
                reason: RootQualificationFailureReason::TimedOut,
                message: "timed out".to_string(),
            },
        ] {
            let mut source = observation_source();
            source.root.qualification = root;
            assert!(capture_target_registration_payload_from(
                &mut source,
                &capture_request(),
                &test_build(),
            )
            .is_err());
        }
    }

    #[test]
    fn qualification_dtos_round_trip_the_camel_case_frontend_contract() {
        let input = json!({
            "enabled": true,
            "recordable": true,
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
    fn capability_projection_uses_only_production_availability() {
        let snapshot = crate::device_qualification::DeviceQualificationSnapshotDto {
            state: DeviceQualificationState::Supported,
            summary: "supported",
            limitations: Vec::new(),
            android_major: Some(15),
            android_api_level: Some(35),
            abi_class: Some("arm64"),
            storage: CapabilityAvailabilityDto::Available,
            package_manager: CapabilityAvailabilityDto::Available,
            activity_manager: CapabilityAvailabilityDto::Unavailable,
            root: None,
            runtime_generation: 1,
            qualification_revision: 1,
            device_identity: Some("device_opaque".to_string()),
        };
        assert_eq!(
            capabilities_from_snapshot(&snapshot),
            vec!["apk_install", "shared_storage_write"]
        );
    }

    #[test]
    fn selected_plan_resolves_only_from_production_match_projection() {
        let result = json!({
            "candidates": [{ "planId": "wrong", "profileId": "wrong.profile" }],
            "safeGenericPlans": [{ "planId": "selected", "profileId": "selected.profile" }]
        });
        assert_eq!(
            matched_profile_id(&result, "selected"),
            Some("selected.profile".to_string())
        );
        assert_eq!(matched_profile_id(&result, "missing"), None);
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
            report_bytes: None,
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

    #[test]
    fn required_facts_fail_closed_but_normalize_numeric_android_release() {
        let facts = json!({ "android_version": 15, "android_api_level": 35 });
        assert_eq!(
            required_fact_string(&facts, "android_version").unwrap(),
            "15"
        );
        assert_eq!(required_fact_u64(&facts, "android_api_level").unwrap(), 35);
        assert!(required_fact_string(&json!({}), "manufacturer").is_err());
        assert!(required_fact_string(&json!({ "model": null }), "model").is_err());
        assert!(
            required_fact_u64(&json!({ "android_api_level": "35" }), "android_api_level").is_err()
        );
    }

    fn test_session() -> QualificationSession {
        QualificationSession::for_test(&["device_behavior_verified"])
    }

    fn checkpoint_session() -> QualificationSession {
        QualificationSession::for_test(&["device_behavior_verified"])
    }

    fn test_observation() -> QualificationDeviceObservation {
        QualificationDeviceObservation::for_test()
    }

    #[test]
    fn invalidated_session_never_returns_to_valid() {
        let mut session = test_session();
        session.invalidate(QualificationInvalidation::DeviceIdentityChanged);
        assert_eq!(session.run_validity(), RunValidity::Invalid);
        session.observe_matching_device(test_observation());
        assert_eq!(session.run_validity(), RunValidity::Invalid);
    }

    #[test]
    fn checkpoint_ids_and_outcomes_must_come_from_the_workflow_contract() {
        let mut session = checkpoint_session();
        assert!(session
            .record_checkpoint(
                "device_behavior_verified",
                CheckpointOutcome::Pass
            )
            .is_ok());
        assert!(session
            .record_checkpoint("invented", CheckpointOutcome::Pass)
            .is_err());
        assert!(session
            .record_checkpoint(
                "device_behavior_verified",
                CheckpointOutcome::UnableToVerify
            )
            .is_ok());
        assert_eq!(session.run_validity(), RunValidity::Invalid);
        assert_eq!(session.qualification_outcome(), QualificationOutcome::NotObserved);
    }

    #[test]
    fn recorded_checkpoint_timestamp_survives_persistence_and_reload() {
        let mut session = checkpoint_session();
        session
            .record_checkpoint_at(
                "device_behavior_verified",
                CheckpointOutcome::Pass,
                "2026-08-23T12:34:56Z",
            )
            .unwrap();
        let restored = QualificationSession::from_persisted(session.to_persisted()).unwrap();
        assert_eq!(
            restored.recorded_checkpoints()[0].observed_at,
            "2026-08-23T12:34:56Z"
        );
    }

    #[test]
    fn prerequisite_checkpoint_failure_invalidates_before_review_binding() {
        let mut session = QualificationSession::for_test(&["clean_or_deliberately_reset_device"]);
        session
            .record_checkpoint(
                "clean_or_deliberately_reset_device",
                CheckpointOutcome::Fail,
            )
            .unwrap();
        assert_eq!(session.run_validity(), RunValidity::Invalid);
        assert!(!session.can_bind_product_workflow());
    }

    #[test]
    fn execution_status_classification_preserves_reportable_product_failure() {
        let mut session = checkpoint_session();
        session
            .record_checkpoint(
                "device_behavior_verified",
                CheckpointOutcome::Pass,
            )
            .unwrap();
        session.classify_execution("failed").unwrap();
        assert_eq!(session.run_validity(), RunValidity::Valid);
        assert_eq!(session.qualification_outcome(), QualificationOutcome::Failed);
    }

    #[test]
    fn cancelled_execution_invalidates_without_becoming_a_product_failure() {
        let mut session = checkpoint_session();
        session
            .record_checkpoint(
                "device_behavior_verified",
                CheckpointOutcome::Pass,
            )
            .unwrap();
        session.classify_execution("cancelled").unwrap();
        assert_eq!(session.run_validity(), RunValidity::Invalid);
        assert_eq!(session.qualification_outcome(), QualificationOutcome::NotObserved);
    }
}
