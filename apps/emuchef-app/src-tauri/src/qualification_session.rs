//! Lifecycle ownership for one active device-qualification attempt.
//!
//! This module owns: active-session cardinality, session start validation,
//! persisted-session versioning, device association and clean-restart
//! reassociation, declared intent locking, ordered lifecycle observations,
//! required-checkpoint state, review and execution association, terminal
//! classification, automatic immutable candidate materialization, monotonic
//! invalidation, clean-shutdown resumability, crash invalidation, and operator
//! abandonment.
//!
//! Trusted Tauri orchestration feeds this module one closed observation type
//! after an ordinary product transition commits and before the product result
//! is published. Qualification failures are contained here and can never change
//! the committed product result. Canonical target and evidence recording rules
//! remain owned by the qualification repository module and the repository tool;
//! this module never authors canonical evidence.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::commands::{safe_error, AppState};
use crate::device_observation::SelectedDeviceObservation;
use crate::device_qualification::RootQualificationState;
use crate::handles::ReviewedPlanSnapshot;
use crate::qualification_build::QualificationBuildIdentity;
use crate::qualification_mode::{
    CheckpointOutcome, QualificationCandidateSummaryDto, QualificationCheckpointOutcome,
    QualificationConnectionType, QualificationRootState, QualificationWorkflow,
    QualificationWorkflowCheckpoint, QualificationWorkflowObservation,
};
use crate::qualification_repository::{
    AuthoredRecipeDigest, CandidateKind, QualificationCandidateSummary, QualificationRepository,
    StoredQualificationCandidate, CANDIDATE_HANDLE_PREFIX,
};

/// Persisted-session contract version. Version 1 sessions were written before
/// the observation-owned lifecycle existed, and version 2 sessions recorded
/// process-local handles as durable authority. Neither can prove that every
/// authoritative transition was captured, so both fail closed.
pub(crate) const SESSION_SCHEMA_VERSION: u64 = 4;
const SESSION_HANDLE_PREFIX: &str = "qualification-session-";
const SESSION_HANDLE_HEX_LENGTH: usize = 32;

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

/// The sanitized lifecycle phase projected to the development overlay. React
/// branches on this closed presentation phase and never interprets the
/// internal invalidation model.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum QualificationSessionPhase {
    ExecutionPending,
    ExecutionActive,
    TerminalAwaitingEvidence,
    Closed,
}

/// Closed typed invalidation model. These reasons are internal only: the
/// overlay receives the backend-authored explanation string, never a token.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationInvalidation {
    DeviceUnavailable,
    DeviceIdentityChanged,
    TargetProfileChanged,
    ManufacturerChanged,
    ModelChanged,
    AndroidVersionChanged,
    AndroidApiChanged,
    AbiSocClassChanged,
    FirmwareBuildChanged,
    RootStateChanged,
    ProductIntentChanged,
    MissingRequiredPrerequisite,
    RequiredCheckpointUnableToVerify,
    PrerequisiteFailed,
    ExecutionCancelled,
    ExecutionUnavailable,
    ExecutionEvidenceInvalidated,
    LifecycleTransitionMissed,
    ObservationFailed,
    IncompatibleSessionVersion,
    UnprovenShutdown,
    OperatorAbandoned,
}

impl QualificationInvalidation {
    /// Backend-authored operator explanation. No internal token, path, serial,
    /// or arbitrary backend text escapes through this projection.
    pub(crate) fn explanation(self) -> &'static str {
        match self {
            Self::DeviceUnavailable => "The selected device is no longer available.",
            Self::DeviceIdentityChanged => {
                "The connected device no longer matches this qualification attempt."
            }
            Self::TargetProfileChanged => {
                "The device no longer matches the registered target profile."
            }
            Self::ManufacturerChanged => {
                "The device manufacturer no longer matches the registered target."
            }
            Self::ModelChanged => "The device model no longer matches the registered target.",
            Self::AndroidVersionChanged => {
                "The Android version no longer matches the registered target."
            }
            Self::AndroidApiChanged => {
                "The Android API level no longer matches the registered target."
            }
            Self::AbiSocClassChanged => {
                "The device ABI and SoC class no longer match the registered target."
            }
            Self::FirmwareBuildChanged => {
                "The device firmware build no longer matches the registered target."
            }
            Self::RootStateChanged => {
                "The device root state no longer matches the registered target."
            }
            Self::ProductIntentChanged => {
                "The reviewed setup no longer matches this qualification attempt."
            }
            Self::MissingRequiredPrerequisite => {
                "A required checkpoint was not passed before the run started."
            }
            Self::RequiredCheckpointUnableToVerify => {
                "A required checkpoint could not be verified."
            }
            Self::PrerequisiteFailed => {
                "A required prerequisite checkpoint failed before the run started."
            }
            Self::ExecutionCancelled => "The production run was cancelled.",
            Self::ExecutionUnavailable => {
                "The production run result could not be tied to this qualification attempt."
            }
            Self::ExecutionEvidenceInvalidated => {
                "The completed run could not be tied to trustworthy device evidence."
            }
            Self::LifecycleTransitionMissed => {
                "A required qualification transition was missed or arrived out of order."
            }
            Self::ObservationFailed => "Qualification evidence could not be retained.",
            Self::IncompatibleSessionVersion => {
                "The saved qualification attempt was created by an incompatible application version."
            }
            Self::UnprovenShutdown => {
                "The previous application session did not end cleanly, so this attempt cannot be trusted."
            }
            Self::OperatorAbandoned => "The operator abandoned this qualification attempt.",
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
/// submission and is never recomputed during restart or materialization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordedQualificationCheckpoint {
    pub(crate) checkpoint_id: String,
    pub(crate) outcome: QualificationCheckpointOutcome,
    pub(crate) observed_at: String,
}

/// Strict on-disk representation of a resumable qualification session.
///
/// Process-local handles (the selected device, the bound review, and the
/// bound execution) are deliberately absent: they are valid only inside one
/// application process and must never become durable authority. A resumed
/// session re-establishes its device and review associations from new
/// authoritative product observations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedQualificationSession {
    pub(crate) session_schema_version: u64,
    pub(crate) session_handle: String,
    pub(crate) candidate_handle: String,
    pub(crate) captured_at: String,
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
    /// Durable lifecycle fact: the previous process admitted a real execution
    /// for this attempt. The process-local execution handle itself is never
    /// durable. A resumed session that admitted execution without retaining an
    /// authoritative terminal transition can never produce valid evidence.
    pub(crate) execution_admitted: bool,
    pub(crate) terminal_execution_status: Option<String>,
    pub(crate) terminal_observed_at: Option<String>,
    pub(crate) terminal_outcome: QualificationOutcome,
    pub(crate) authored_recipe_digests: Option<Vec<AuthoredRecipeDigest>>,
    pub(crate) awaiting_evidence: bool,
    pub(crate) closed: bool,
}

/// Sanitized session state returned to the overlay. Handles are opaque and no
/// candidate paths, process arguments, or raw device observations cross this
/// boundary. The invalid reason is a backend-authored operator explanation and
/// never an internal invalidation token.
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
    pub(crate) phase: QualificationSessionPhase,
    pub(crate) run_validity: RunValidity,
    pub(crate) qualification_outcome: QualificationOutcome,
    pub(crate) recordable: bool,
    pub(crate) invalid_reason: Option<String>,
    pub(crate) candidate: Option<QualificationCandidateSummaryDto>,
}

/// Pure lifecycle state for one qualification run candidate.
pub(crate) struct QualificationSession {
    session_handle: String,
    candidate_handle: String,
    captured_at: String,
    target: QualificationTargetBinding,
    target_id: String,
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
    /// Process-local: the review this process bound, never restored from disk.
    bound_review_handle: Option<String>,
    /// Process-local: the execution this process admitted, never restored from
    /// disk.
    bound_execution_handle: Option<String>,
    /// Durable lifecycle fact: a real execution was admitted for this attempt.
    execution_admitted: bool,
    terminal_execution_status: Option<String>,
    terminal_observed_at: Option<String>,
    terminal_outcome: QualificationOutcome,
    authored_recipe_digests: Option<Vec<AuthoredRecipeDigest>>,
    /// Exact sanitized terminal execution report bytes retained with the
    /// attempt. They are persisted beside the strict session file and become
    /// the candidate artifact when the attempt materializes.
    terminal_report: Option<Vec<u8>>,
    awaiting_evidence: bool,
    closed: bool,
}

impl QualificationSession {
    pub(crate) fn new(
        session_handle: String,
        candidate_handle: String,
        captured_at: String,
        target: QualificationTargetBinding,
        workflow: QualificationWorkflow,
        build: QualificationBuildIdentity,
        runtime_contract: String,
    ) -> Result<Self, String> {
        validate_session_handle(&session_handle)?;
        if candidate_handle.is_empty()
            || captured_at.is_empty()
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
            execution_admitted: false,
            terminal_execution_status: None,
            terminal_observed_at: None,
            terminal_outcome: QualificationOutcome::NotObserved,
            authored_recipe_digests: None,
            terminal_report: None,
            awaiting_evidence: false,
            closed: false,
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
            format!("{CANDIDATE_HANDLE_PREFIX}{}", "a".repeat(32)),
            "2026-08-23T12:00:00Z".to_string(),
            target,
            workflow,
            build,
            "real-execution-v1".to_string(),
        )
        .expect("test session should be valid");
        let mut session = session.with_device_plan("test-plan".to_string());
        session.set_authored_recipe_digests(vec![AuthoredRecipeDigest {
            id: "test.recipe".to_string(),
            sha256: "a".repeat(64),
        }]);
        session
    }

    pub(crate) fn candidate_handle(&self) -> &str {
        &self.candidate_handle
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

    pub(crate) fn terminal_report_bytes(&self) -> Option<&[u8]> {
        self.terminal_report.as_deref()
    }

    pub(crate) fn set_terminal_report(&mut self, report: Option<Vec<u8>>) {
        self.terminal_report = report;
    }

    pub(crate) fn set_authored_recipe_digests(&mut self, digests: Vec<AuthoredRecipeDigest>) {
        self.authored_recipe_digests = Some(digests);
    }

    pub(crate) fn authored_recipe_digests(&self) -> Option<&[AuthoredRecipeDigest]> {
        self.authored_recipe_digests.as_deref()
    }

    pub(crate) fn target(&self) -> &QualificationTargetBinding {
        &self.target
    }

    pub(crate) fn bound_execution_handle(&self) -> Option<&str> {
        self.bound_execution_handle.as_deref()
    }

    /// Whether a real execution was admitted for this attempt. This is a
    /// durable lifecycle fact, unlike the process-local execution handle.
    pub(crate) fn execution_admitted(&self) -> bool {
        self.execution_admitted
    }

    /// Re-attach the process-local bindings this process established for the
    /// active attempt. A session loaded from disk never carries them, so it
    /// only learns its bindings from the process-local store.
    fn attach_process_local_bindings(&mut self, review: Option<String>, execution: Option<String>) {
        self.bound_review_handle = review;
        self.bound_execution_handle = execution;
    }

    pub(crate) fn recorded_checkpoints(&self) -> &[RecordedQualificationCheckpoint] {
        &self.recorded_checkpoints
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    pub(crate) fn phase(&self) -> QualificationSessionPhase {
        if self.closed {
            QualificationSessionPhase::Closed
        } else if self.terminal_execution_status.is_some() || self.awaiting_evidence {
            QualificationSessionPhase::TerminalAwaitingEvidence
        } else if self.bound_execution_handle.is_some() {
            QualificationSessionPhase::ExecutionActive
        } else {
            QualificationSessionPhase::ExecutionPending
        }
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

    /// Whether this attempt can still produce recordable evidence.
    pub(crate) fn recordable(&self) -> bool {
        self.run_validity() == RunValidity::Valid && !self.closed
    }

    /// Backend-authored sanitized explanation, never the internal token.
    pub(crate) fn invalid_reason(&self) -> Option<String> {
        self.invalidation
            .map(|reason| reason.explanation().to_string())
    }

    pub(crate) fn invalidate(&mut self, reason: QualificationInvalidation) {
        if self.invalidation.is_none() {
            self.invalidation = Some(reason);
            self.terminal_outcome = QualificationOutcome::NotObserved;
        }
    }

    pub(crate) fn close(&mut self) {
        self.closed = true;
        self.awaiting_evidence = false;
    }

    /// Whether prerequisite checkpoints have all passed.
    pub(crate) fn prerequisites_passed(&self) -> bool {
        self.prerequisites.iter().all(|prerequisite| {
            self.recorded_checkpoints.iter().any(|recorded| {
                recorded.checkpoint_id == *prerequisite
                    && recorded.outcome == QualificationCheckpointOutcome::Pass
            })
        })
    }

    /// Required checkpoints that have not yet produced a usable outcome.
    fn missing_required_checkpoints(&self) -> Vec<&QualificationWorkflowCheckpoint> {
        self.human_checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.required)
            .filter(|checkpoint| {
                !self.recorded_checkpoints.iter().any(|recorded| {
                    recorded.checkpoint_id == checkpoint.id
                        && recorded.outcome != QualificationCheckpointOutcome::UnableToVerify
                })
            })
            .collect()
    }

    fn required_checkpoints_unable_to_verify(&self) -> bool {
        self.human_checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.required)
            .any(|checkpoint| {
                self.recorded_checkpoints.iter().any(|recorded| {
                    recorded.checkpoint_id == checkpoint.id
                        && recorded.outcome == QualificationCheckpointOutcome::UnableToVerify
                })
            })
    }

    /// Compare one typed committed device observation with the session target.
    /// Absent material facts are not compared because not every trusted seam
    /// observes every fact; present facts always are.
    pub(crate) fn observe_matching_target(&mut self, observation: &SelectedDeviceObservation) {
        if self.run_validity() == RunValidity::Invalid || self.closed {
            return;
        }
        let mismatch = if observation
            .profile_id
            .as_deref()
            .is_some_and(|profile_id| profile_id != self.target.profile_id)
        {
            Some(QualificationInvalidation::TargetProfileChanged)
        } else if observation
            .manufacturer
            .as_deref()
            .is_some_and(|manufacturer| manufacturer != self.target.manufacturer)
        {
            Some(QualificationInvalidation::ManufacturerChanged)
        } else if observation
            .model
            .as_deref()
            .is_some_and(|model| model != self.target.model)
        {
            Some(QualificationInvalidation::ModelChanged)
        } else if observation
            .android_version
            .as_deref()
            .is_some_and(|version| version != self.target.android_version)
        {
            Some(QualificationInvalidation::AndroidVersionChanged)
        } else if observation
            .android_api
            .is_some_and(|android_api| android_api != self.target.android_api)
        {
            Some(QualificationInvalidation::AndroidApiChanged)
        } else if observation
            .abi_soc_class
            .as_deref()
            .is_some_and(|abi| abi != self.target.abi_soc_class)
        {
            Some(QualificationInvalidation::AbiSocClassChanged)
        } else if observation
            .firmware_build
            .as_deref()
            .is_some_and(|firmware| firmware != self.target.firmware_build)
        {
            Some(QualificationInvalidation::FirmwareBuildChanged)
        } else {
            self.matching_root_mismatch(observation.root_state.clone())
        };
        if let Some(reason) = mismatch {
            self.invalidate(reason);
        }
    }

    fn matching_root_mismatch(
        &self,
        root_state: Option<RootQualificationState>,
    ) -> Option<QualificationInvalidation> {
        // An unavailable or failed root check does not establish a root fact,
        // so it can never prove that the target contract was violated.
        let observed = project_root_state(&root_state?)?;
        if observed == self.target.root_state {
            None
        } else {
            Some(QualificationInvalidation::RootStateChanged)
        }
    }

    /// Compare one committed explicit root-check result with the target.
    pub(crate) fn observe_root_authority(&mut self, root_state: RootQualificationState) {
        if self.run_validity() == RunValidity::Invalid || self.closed {
            return;
        }
        if let Some(reason) = self.matching_root_mismatch(Some(root_state)) {
            self.invalidate(reason);
        }
    }

    /// Bind one authoritative review when it matches the locked intent. A
    /// non-matching review is ignored; execution admission is the authoritative
    /// point where intent drift invalidates the attempt.
    pub(crate) fn observe_review(&mut self, review: &ReviewObservation) {
        if self.run_validity() == RunValidity::Invalid
            || self.closed
            || self.bound_review_handle.is_some()
            || self.terminal_execution_status.is_some()
        {
            return;
        }
        if self.review_matches(review) {
            self.bound_review_handle = Some(review.review_handle.clone());
        }
    }

    fn review_matches(&self, review: &ReviewObservation) -> bool {
        if review.device_plan != self.device_plan {
            return false;
        }
        if review.selected_recipes != self.required_recipes {
            return false;
        }
        if review
            .target_id
            .as_deref()
            .is_some_and(|target_id| target_id != self.target_id)
        {
            return false;
        }
        if review
            .manufacturer
            .as_deref()
            .is_some_and(|manufacturer| manufacturer != self.target.manufacturer)
        {
            return false;
        }
        if review
            .model
            .as_deref()
            .is_some_and(|model| model != self.target.model)
        {
            return false;
        }
        if review
            .android_api
            .is_some_and(|android_api| android_api != self.target.android_api)
        {
            return false;
        }
        true
    }

    /// Bind one authoritative real-execution admission. Product execution has
    /// already committed; any qualification problem invalidates only evidence.
    pub(crate) fn admit_execution(
        &mut self,
        observation: &ExecutionAdmissionObservation,
        associated_device_handle: Option<&str>,
    ) {
        if self.run_validity() == RunValidity::Invalid || self.closed {
            return;
        }
        if self.terminal_execution_status.is_some() {
            self.invalidate(QualificationInvalidation::LifecycleTransitionMissed);
            return;
        }
        if associated_device_handle != Some(observation.device_handle.as_str()) {
            self.invalidate(QualificationInvalidation::LifecycleTransitionMissed);
            return;
        }
        if self
            .bound_review_handle
            .as_deref()
            .is_some_and(|handle| handle != observation.review.review_handle)
            || !self.review_matches(&observation.review)
            || self.authored_recipe_digests.is_none()
        {
            self.invalidate(QualificationInvalidation::ProductIntentChanged);
            return;
        }
        if !self.prerequisites_passed() {
            self.invalidate(QualificationInvalidation::MissingRequiredPrerequisite);
            return;
        }
        self.execution_admitted = true;
        self.bound_review_handle = Some(observation.review.review_handle.clone());
        self.bound_execution_handle = Some(observation.execution_handle.clone());
    }

    /// Classify one authoritative terminal execution retention.
    pub(crate) fn classify_terminal(&mut self, observation: &TerminalExecutionObservation) {
        if self.run_validity() == RunValidity::Invalid || self.closed {
            return;
        }
        if self.bound_execution_handle.as_deref() != Some(observation.execution_handle.as_str()) {
            self.invalidate(QualificationInvalidation::LifecycleTransitionMissed);
            return;
        }
        if observation.authority_invalidated {
            self.invalidate(QualificationInvalidation::ExecutionEvidenceInvalidated);
            return;
        }
        let Some(status) = observation.status.as_deref() else {
            self.invalidate(QualificationInvalidation::ExecutionUnavailable);
            return;
        };
        if !is_terminal_execution_status(status) {
            self.invalidate(QualificationInvalidation::LifecycleTransitionMissed);
            return;
        }
        if validate_checkpoint_timestamp(&observation.observed_at).is_err() {
            self.invalidate(QualificationInvalidation::ExecutionUnavailable);
            return;
        }
        if !observation.report_available {
            self.invalidate(QualificationInvalidation::ExecutionUnavailable);
            return;
        }
        self.terminal_execution_status = Some(status.to_string());
        self.terminal_observed_at = Some(observation.observed_at.clone());
        self.terminal_report = observation.report_bytes.clone();
        if status == "cancelled" {
            self.invalidate(QualificationInvalidation::ExecutionCancelled);
            return;
        }
        if self.required_checkpoints_unable_to_verify() {
            self.invalidate(QualificationInvalidation::RequiredCheckpointUnableToVerify);
            return;
        }
        if !self.missing_required_checkpoints().is_empty() {
            self.awaiting_evidence = true;
            return;
        }
        self.resolve_terminal_outcome();
    }

    /// Resolve the product outcome from the retained terminal execution and the
    /// recorded checkpoints. An attempt that first awaits missing evidence and
    /// later receives it resolves here, so the materialized candidate always
    /// carries the outcome its evidence supports. The resolution is monotonic:
    /// an already resolved outcome is never recomputed.
    pub(crate) fn resolve_terminal_outcome(&mut self) {
        if self.terminal_outcome != QualificationOutcome::NotObserved {
            return;
        }
        let Some(status) = self.terminal_execution_status.as_deref() else {
            return;
        };
        if !is_terminal_execution_status(status) || status == "cancelled" {
            return;
        }
        if self.required_checkpoints_unable_to_verify()
            || !self.missing_required_checkpoints().is_empty()
        {
            return;
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
    }

    /// Whether the remaining required evidence has arrived after the terminal
    /// execution was retained.
    pub(crate) fn terminal_evidence_ready(&self) -> bool {
        if self.terminal_execution_status.is_none() || self.closed {
            return false;
        }
        if self.required_checkpoints_unable_to_verify() {
            return false;
        }
        if !self.missing_required_checkpoints().is_empty() {
            return false;
        }
        if self.terminal_outcome == QualificationOutcome::NotObserved {
            let status = self
                .terminal_execution_status
                .as_deref()
                .unwrap_or_default();
            return is_terminal_execution_status(status) && status != "cancelled";
        }
        true
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
        if self.run_validity() == RunValidity::Invalid || self.closed {
            return Err("qualification session is no longer active".to_string());
        }
        let declaration = self
            .human_checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == checkpoint_id)
            .ok_or_else(|| {
                "qualification checkpoint is not declared by the workflow".to_string()
            })?;
        if !declaration.allowed_outcomes.contains(&outcome) {
            return Err(
                "qualification checkpoint outcome is not allowed by the workflow".to_string(),
            );
        }
        // Recorded checkpoint evidence is immutable: a second submission for
        // the same checkpoint never replaces the retained outcome or
        // timestamp.
        if self
            .recorded_checkpoints
            .iter()
            .any(|recorded| recorded.checkpoint_id == checkpoint_id)
        {
            return Err("qualification checkpoint was already recorded".to_string());
        }
        // While the attempt is awaiting the evidence its terminal run is
        // missing, only declared required checkpoints can complete it.
        if self.awaiting_evidence && !declaration.required {
            return Err(
                "qualification is awaiting required evidence and accepts no other checkpoint"
                    .to_string(),
            );
        }
        validate_checkpoint_timestamp(observed_at)?;
        let checkpoint = RecordedQualificationCheckpoint {
            checkpoint_id: checkpoint_id.to_string(),
            outcome,
            observed_at: observed_at.to_string(),
        };
        self.recorded_checkpoints.push(checkpoint);
        if declaration.required && outcome == QualificationCheckpointOutcome::UnableToVerify {
            self.invalidate(QualificationInvalidation::RequiredCheckpointUnableToVerify);
        } else if self.prerequisites.iter().any(|id| id == checkpoint_id)
            && outcome != QualificationCheckpointOutcome::Pass
        {
            self.invalidate(QualificationInvalidation::PrerequisiteFailed);
        }
        Ok(())
    }

    pub(crate) fn to_persisted(&self) -> PersistedQualificationSession {
        PersistedQualificationSession {
            session_schema_version: SESSION_SCHEMA_VERSION,
            session_handle: self.session_handle.clone(),
            candidate_handle: self.candidate_handle.clone(),
            captured_at: self.captured_at.clone(),
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
            execution_admitted: self.execution_admitted,
            terminal_execution_status: self.terminal_execution_status.clone(),
            terminal_observed_at: self.terminal_observed_at.clone(),
            terminal_outcome: self.qualification_outcome(),
            authored_recipe_digests: self.authored_recipe_digests.clone(),
            awaiting_evidence: self.awaiting_evidence,
            closed: self.closed,
        }
    }

    pub(crate) fn from_persisted(persisted: PersistedQualificationSession) -> Result<Self, String> {
        if persisted.session_schema_version != SESSION_SCHEMA_VERSION {
            return Err("qualification session schema version is unsupported".to_string());
        }
        validate_session_handle(&persisted.session_handle)?;
        if persisted.candidate_handle.is_empty()
            || persisted.captured_at.is_empty()
            || persisted.target_id.is_empty()
            || persisted.workflow_id.is_empty()
            || persisted.runtime_contract.is_empty()
            || persisted.device_plan.is_empty()
        {
            return Err("qualification session metadata is incomplete".to_string());
        }
        if persisted.target_id != persisted.target.target_id
            || persisted
                .human_checkpoints
                .iter()
                .any(|checkpoint| checkpoint.id.is_empty())
        {
            return Err("qualification session binding is inconsistent".to_string());
        }
        let required_recipe_ids = persisted
            .required_recipes
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let Some(authored_recipe_digests) = persisted.authored_recipe_digests.as_ref() else {
            return Err("qualification session recipe digests are missing".to_string());
        };
        let digest_recipe_ids = authored_recipe_digests
            .iter()
            .map(|digest| digest.id.as_str())
            .collect::<BTreeSet<_>>();
        if required_recipe_ids.len() != persisted.required_recipes.len()
            || digest_recipe_ids.len() != authored_recipe_digests.len()
            || digest_recipe_ids != required_recipe_ids
            || persisted
                .required_recipes
                .iter()
                .any(|recipe| !valid_qualification_recipe_id(recipe))
            || authored_recipe_digests.iter().any(|digest| {
                !valid_qualification_recipe_id(&digest.id)
                    || digest.sha256.len() != 64
                    || !digest
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        {
            return Err("qualification session recipe digests are inconsistent".to_string());
        }
        let mut ids = BTreeSet::new();
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
                || validate_checkpoint_timestamp(&checkpoint.observed_at).is_err()
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
        if persisted.awaiting_evidence && persisted.terminal_execution_status.is_none() {
            return Err("qualification session awaits evidence without a terminal run".to_string());
        }
        if persisted.terminal_execution_status.is_some() && !persisted.execution_admitted {
            return Err(
                "qualification session retained a terminal run it never admitted".to_string(),
            );
        }
        if persisted.terminal_execution_status.is_some()
            && persisted
                .terminal_observed_at
                .as_deref()
                .is_none_or(|timestamp| validate_checkpoint_timestamp(timestamp).is_err())
        {
            return Err("qualification terminal observation timestamp is invalid".to_string());
        }
        Ok(Self {
            session_handle: persisted.session_handle,
            candidate_handle: persisted.candidate_handle,
            captured_at: persisted.captured_at,
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
            // Process-local associations are never restored: a resumed attempt
            // re-establishes them from new authoritative product observations.
            bound_review_handle: None,
            bound_execution_handle: None,
            execution_admitted: persisted.execution_admitted,
            terminal_execution_status: persisted.terminal_execution_status,
            terminal_observed_at: persisted.terminal_observed_at,
            terminal_outcome: persisted.terminal_outcome,
            authored_recipe_digests: persisted.authored_recipe_digests,
            awaiting_evidence: persisted.awaiting_evidence,
            closed: persisted.closed,
            terminal_report: None,
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
            phase: self.phase(),
            run_validity: self.run_validity(),
            qualification_outcome: self.qualification_outcome(),
            recordable: self.recordable(),
            invalid_reason: self.invalid_reason(),
            candidate,
        }
    }
}

/// Project one committed explicit root-check result into the target contract.
/// Unavailable or failed checks cannot establish a root fact.
pub(crate) fn project_root_state(root: &RootQualificationState) -> Option<QualificationRootState> {
    match root {
        RootQualificationState::Granted => Some(QualificationRootState::Rooted),
        RootQualificationState::Denied => Some(QualificationRootState::NonRoot),
        RootQualificationState::Unavailable | RootQualificationState::CheckFailed { .. } => None,
    }
}

fn is_terminal_execution_status(status: &str) -> bool {
    matches!(
        status,
        "succeeded" | "succeeded_with_warnings" | "failed" | "cancelled"
    )
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

fn valid_qualification_recipe_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_checkpoint_timestamp(timestamp: &str) -> Result<(), String> {
    let bytes = timestamp.as_bytes();
    let fixed_shape = bytes.len() >= 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[11..13].iter().all(u8::is_ascii_digit)
        && bytes[14..16].iter().all(u8::is_ascii_digit)
        && bytes[17..19].iter().all(u8::is_ascii_digit);
    if !fixed_shape {
        return Err("qualification checkpoint timestamp is invalid".to_string());
    }
    let mut timezone_start = 19;
    if bytes.get(timezone_start) == Some(&b'.') {
        timezone_start += 1;
        let fraction_start = timezone_start;
        while bytes.get(timezone_start).is_some_and(u8::is_ascii_digit) {
            timezone_start += 1;
        }
        if timezone_start == fraction_start {
            return Err("qualification checkpoint timestamp is invalid".to_string());
        }
    }
    let valid_timezone = match bytes.get(timezone_start) {
        Some(b'Z') => timezone_start + 1 == bytes.len(),
        Some(b'+' | b'-') => {
            timezone_start + 6 == bytes.len()
                && bytes[timezone_start + 3] == b':'
                && bytes[timezone_start + 1..timezone_start + 3]
                    .iter()
                    .all(u8::is_ascii_digit)
                && bytes[timezone_start + 4..timezone_start + 6]
                    .iter()
                    .all(u8::is_ascii_digit)
        }
        _ => false,
    };
    if !valid_timezone || OffsetDateTime::parse(timestamp, &Rfc3339).is_err() {
        return Err("qualification checkpoint timestamp is invalid".to_string());
    }
    Ok(())
}

pub(crate) fn session_handle_for_candidate(candidate_handle: &str) -> Result<String, String> {
    let suffix = candidate_handle
        .strip_prefix(CANDIDATE_HANDLE_PREFIX)
        .ok_or_else(|| "qualification candidate handle is invalid".to_string())?;
    let handle = format!("{SESSION_HANDLE_PREFIX}{suffix}");
    validate_session_handle(&handle)?;
    Ok(handle)
}

pub(crate) fn candidate_handle_for_session(session_handle: &str) -> Result<String, String> {
    validate_session_handle(session_handle)?;
    let suffix = session_handle
        .strip_prefix(SESSION_HANDLE_PREFIX)
        .expect("validated session handle has the expected prefix");
    Ok(format!("{CANDIDATE_HANDLE_PREFIX}{suffix}"))
}

/// Committed review facts consumed by qualification association.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewObservation {
    pub(crate) review_handle: String,
    pub(crate) device_handle: String,
    pub(crate) device_plan: String,
    pub(crate) selected_recipes: Vec<String>,
    pub(crate) target_id: Option<String>,
    pub(crate) manufacturer: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) android_api: Option<u64>,
}

/// Project one committed reviewed plan into the closed observation type. Only
/// fields the product already published for that review are read.
pub(crate) fn review_observation(
    review_handle: &str,
    review: &ReviewedPlanSnapshot,
) -> ReviewObservation {
    let selected_recipes = review
        .response
        .get("selectedRecipes")
        .and_then(Value::as_array)
        .map(|recipes| {
            recipes
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let target_id = review
        .target
        .get("id")
        .or_else(|| review.response.get("deviceTargetId"))
        .or_else(|| review.response.get("targetId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    ReviewObservation {
        review_handle: review_handle.to_string(),
        device_handle: review.device_handle.clone(),
        device_plan: review
            .response
            .get("devicePlan")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        selected_recipes,
        target_id,
        manufacturer: review
            .target
            .get("manufacturer")
            .and_then(Value::as_str)
            .map(str::to_string),
        model: review
            .target
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        android_api: review.target.get("androidApiLevel").and_then(Value::as_u64),
    }
}

/// One committed real-execution admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionAdmissionObservation {
    pub(crate) execution_handle: String,
    pub(crate) review: ReviewObservation,
    pub(crate) device_handle: String,
}

/// One committed terminal real-execution retention.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalExecutionObservation {
    pub(crate) execution_handle: String,
    pub(crate) status: Option<String>,
    pub(crate) observed_at: String,
    pub(crate) report_available: bool,
    /// Exact sanitized report bytes retained by the product authority. They
    /// become the immutable candidate artifact.
    pub(crate) report_bytes: Option<Vec<u8>>,
    /// True when the committed terminal report invalidated device identity or
    /// root authority, so the run cannot back trustworthy qualification
    /// evidence.
    pub(crate) authority_invalidated: bool,
}

/// Closed lifecycle observation interface fed by trusted Tauri orchestration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum QualificationLifecycleObservation {
    DeviceObserved(Box<SelectedDeviceObservation>),
    DeviceObservationFailed {
        device_handle: String,
    },
    RootChecked {
        device_handle: String,
        root_state: RootQualificationState,
    },
    ReviewCreated(Box<ReviewObservation>),
    RealExecutionAdmitted(Box<ExecutionAdmissionObservation>),
    RealExecutionTerminal(Box<TerminalExecutionObservation>),
    RealExecutionLost {
        execution_handle: String,
    },
}

/// Process-local, non-durable qualification session authority.
///
/// Device handles are valid only inside one application process, so the
/// association between the active attempt and a live device handle is held
/// here, never trusted across restart, and re-established automatically on the
/// first trusted device observation after a clean restart.
#[derive(Default)]
pub(crate) struct QualificationSessionStore {
    active_candidate: Option<String>,
    associated_device_handle: Option<String>,
    associated_device_session_epoch: Option<u64>,
    /// Trusted observation state accumulated for the currently selected
    /// device in this process. The trusted seams observe different subsets of
    /// the target facts, so reassociation waits until this state proves
    /// compatibility with the immutable registered target.
    observed_device: Option<SelectedDeviceObservation>,
    /// Process-local review this attempt bound. Never durable and never
    /// trusted across restart.
    bound_review_handle: Option<String>,
    /// Process-local execution this attempt admitted. Never durable and never
    /// trusted across restart.
    bound_execution_handle: Option<String>,
    poisoned_candidate: Option<String>,
    /// Run candidates this process created for an attempt that has not started
    /// yet. Launch recovery skips them so a provisional candidate can never be
    /// mistaken for an interrupted attempt from a previous process.
    pending_candidates: BTreeSet<String>,
}

impl QualificationSessionStore {
    pub(crate) fn active_candidate(&self) -> Option<&str> {
        self.active_candidate.as_deref()
    }

    pub(crate) fn associated_device_handle(&self) -> Option<&str> {
        self.associated_device_handle.as_deref()
    }

    fn associated_device_session_epoch(&self) -> Option<u64> {
        self.associated_device_session_epoch
    }

    pub(crate) fn observed_device(&self) -> Option<&SelectedDeviceObservation> {
        self.observed_device.as_ref()
    }

    fn bound_review_handle(&self) -> Option<&str> {
        self.bound_review_handle.as_deref()
    }

    pub(crate) fn bound_execution_handle(&self) -> Option<&str> {
        self.bound_execution_handle.as_deref()
    }

    /// Retain the process-local review and execution bindings the active
    /// attempt established.
    fn retain_bindings(&mut self, review: Option<String>, execution: Option<String>) {
        if review.is_some() {
            self.bound_review_handle = review;
        }
        if execution.is_some() {
            self.bound_execution_handle = execution;
        }
    }

    /// Retain one committed observation of the currently selected device.
    fn observe_device(&mut self, observation: SelectedDeviceObservation) {
        self.observed_device = Some(observation);
    }

    #[cfg(test)]
    pub(crate) fn poisoned_candidate(&self) -> Option<&str> {
        self.poisoned_candidate.as_deref()
    }

    pub(crate) fn is_poisoned(&self, candidate_handle: &str) -> bool {
        self.poisoned_candidate.as_deref() == Some(candidate_handle)
    }

    /// Record one freshly created run candidate as pending in this process.
    pub(crate) fn mark_pending(&mut self, candidate_handle: &str) {
        self.pending_candidates.insert(candidate_handle.to_string());
    }

    /// Whether this process created the candidate and has not started its
    /// attempt yet.
    pub(crate) fn is_pending(&self, candidate_handle: &str) -> bool {
        self.pending_candidates.contains(candidate_handle)
    }

    fn set_active(&mut self, candidate_handle: String) {
        self.pending_candidates.remove(&candidate_handle);
        self.active_candidate = Some(candidate_handle);
        self.associated_device_handle = None;
        self.associated_device_session_epoch = None;
        self.observed_device = None;
        self.bound_review_handle = None;
        self.bound_execution_handle = None;
    }

    fn associate(&mut self, device_handle: String, session_epoch: Option<u64>) {
        self.associated_device_handle = Some(device_handle);
        self.associated_device_session_epoch = session_epoch;
    }

    fn clear_observed_device(&mut self) {
        self.observed_device = None;
    }

    fn poison(&mut self, candidate_handle: String) {
        self.pending_candidates.remove(&candidate_handle);
        self.poisoned_candidate = Some(candidate_handle.clone());
        if self.active_candidate.as_deref() == Some(candidate_handle.as_str()) {
            self.active_candidate = None;
        }
        self.associated_device_handle = None;
        self.associated_device_session_epoch = None;
        self.observed_device = None;
        self.bound_review_handle = None;
        self.bound_execution_handle = None;
    }

    /// Drop all process-local authority for one candidate. Called when a
    /// candidate is discarded or recorded, regardless of session outcome.
    pub(crate) fn forget(&mut self, candidate_handle: &str) {
        self.pending_candidates.remove(candidate_handle);
        if self.active_candidate.as_deref() == Some(candidate_handle) {
            self.active_candidate = None;
            self.associated_device_handle = None;
            self.associated_device_session_epoch = None;
            self.observed_device = None;
            self.bound_review_handle = None;
            self.bound_execution_handle = None;
        }
        if self.poisoned_candidate.as_deref() == Some(candidate_handle) {
            self.poisoned_candidate = None;
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

fn session_error(code: &str, message: &str) -> String {
    safe_error(code, message)
}

fn inactive_error() -> String {
    session_error(
        "qualification_session_inactive",
        "No qualification attempt is active.",
    )
}

fn unavailable_error() -> String {
    session_error(
        "qualification_repository_unavailable",
        "Qualification definitions are unavailable. Rebuild the qualification application.",
    )
}

fn invalid_error() -> String {
    session_error(
        "qualification_candidate_invalid",
        "The qualification target, session, or candidate is invalid or no longer available.",
    )
}

fn persistence_error() -> String {
    session_error(
        "qualification_session_unavailable",
        "Qualification evidence could not be retained.",
    )
}

/// Poison one attempt in memory and persist a candidate-local fail-closed
/// marker. If the candidate directory cannot retain that marker, preserve the
/// native process marker so the next launch cannot prove a clean handoff.
fn poison_attempt(
    state: &AppState,
    provider: &QualificationRepository,
    store: &mut QualificationSessionStore,
    candidate_handle: &str,
) {
    let durable = provider.mark_session_poisoned(candidate_handle).is_ok();
    store.poison(candidate_handle.to_string());
    if durable {
        return;
    }
    match state.recovery.lock() {
        Ok(mut recovery) => recovery.preserve_unproven_handoff(),
        Err(poisoned) => {
            state.recovery.clear_poison();
            poisoned.into_inner().preserve_unproven_handoff();
        }
    }
}

fn load_session(
    provider: &QualificationRepository,
    candidate_handle: &str,
) -> Result<QualificationSession, String> {
    let persisted = provider.load_session(candidate_handle)?;
    if persisted.candidate_handle != candidate_handle {
        return Err(invalid_error());
    }
    let mut session =
        QualificationSession::from_persisted(persisted).map_err(|_| invalid_error())?;
    if session.terminal_execution_status().is_some() {
        session.set_terminal_report(provider.load_session_report(candidate_handle)?);
    }
    Ok(session)
}

/// Load the active attempt together with the bindings it established in this
/// process. Persisted sessions never carry process-local authority, so the
/// process-local store is the only source for those bindings.
fn load_active_session(
    provider: &QualificationRepository,
    store: &QualificationSessionStore,
    candidate_handle: &str,
) -> Result<QualificationSession, String> {
    let mut session = load_session(provider, candidate_handle)?;
    session.attach_process_local_bindings(
        store.bound_review_handle().map(str::to_string),
        store.bound_execution_handle().map(str::to_string),
    );
    Ok(session)
}

/// Persist one session transition. A save failure must fail evidence closed:
/// the strict session file is removed so a later launch cannot resurrect a
/// stale valid attempt, and the caller poisons the in-memory session.
fn persist(
    provider: &QualificationRepository,
    session: &QualificationSession,
) -> Result<(), String> {
    match provider.save_session(session.candidate_handle(), &session.to_persisted()) {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = provider.remove_session(session.candidate_handle());
            Err(persistence_error())
        }
    }
}

fn persist_terminal_report(
    provider: &QualificationRepository,
    session: &QualificationSession,
) -> Result<(), String> {
    match session.terminal_report_bytes() {
        Some(bytes) => provider.save_session_report(session.candidate_handle(), bytes),
        None => {
            let _ = provider.remove_session_report(session.candidate_handle());
            Ok(())
        }
    }
}

fn finalize_candidate(
    provider: &QualificationRepository,
    session: &QualificationSession,
) -> Result<(), String> {
    let report_bytes = session.terminal_report_bytes();
    let payload = run_candidate_payload(session, report_bytes)?;
    provider
        .finalize_candidate(
            session.candidate_handle(),
            CandidateKind::QualificationRun,
            &payload,
            report_bytes,
        )
        .map(|_| ())
        .map_err(|_| persistence_error())
}

pub(crate) fn current_timestamp() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| invalid_error())
}

/// Build the immutable terminal or invalid candidate payload for one session.
fn run_candidate_payload(
    session: &QualificationSession,
    report_bytes: Option<&[u8]>,
) -> Result<Value, String> {
    let authored_content = session
        .authored_recipe_digests()
        .ok_or_else(persistence_error)?;
    let target = session.target();
    let target_root_state =
        serde_json::to_value(&target.root_state).map_err(|_| persistence_error())?;
    let target_connection_type =
        serde_json::to_value(target.connection_type).map_err(|_| persistence_error())?;
    let fingerprint = json!({
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
    let automated_observations = if valid {
        let observed_at = session
            .terminal_observed_at
            .as_deref()
            .ok_or_else(persistence_error)?;
        let outcome = match session.terminal_execution_status() {
            Some("succeeded") | Some("succeeded_with_warnings") => "passed",
            Some("failed") => "failed",
            _ => return Err(persistence_error()),
        };
        session
            .automated_observations()
            .iter()
            .filter(|observation| observation.id == "execution-report")
            .map(|observation| {
                json!({
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
            json!({
                "checkpointId": checkpoint.checkpoint_id,
                "outcome": checkpoint.outcome,
                "observedAt": checkpoint.observed_at,
            })
        })
        .collect::<Vec<_>>();
    let artifacts = report_bytes
        .map(|bytes| {
            vec![json!({
                "id": "execution-report",
                "kind": "production_execution_report",
                "path": "execution-report.json",
                "sha256": hex::encode(Sha256::digest(bytes)),
            })]
        })
        .unwrap_or_default();
    Ok(json!({
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

/// Qualification-run candidates whose immutable payload is not signed yet.
fn resumable_candidates(
    provider: &QualificationRepository,
) -> Result<Vec<QualificationCandidateSummary>, String> {
    let mut resumable = Vec::new();
    for candidate in provider.list_candidates()? {
        if candidate.kind != CandidateKind::QualificationRun
            || candidate.run_validity.is_some()
            || candidate.qualification_outcome.is_some()
        {
            continue;
        }
        // A poison marker is a durable terminal recovery decision. Keep the
        // candidate visible to the operator for discard, but never deserialize
        // or retry recovery of the same untrusted session on later status reads.
        if provider.session_is_poisoned(&candidate.candidate_handle)? {
            continue;
        }
        resumable.push(candidate);
    }
    Ok(resumable)
}

/// Perform launch recovery for persisted sessions. A session created in this
/// process is already authoritative; only sessions restored from disk are
/// recovered, and only after the existing native clean-handoff contract proves
/// the previous process ended normally.
fn ensure_recovered(
    state: &AppState,
    provider: &QualificationRepository,
    store: &mut QualificationSessionStore,
) -> Result<(), String> {
    if store.active_candidate.is_some() {
        return Ok(());
    }
    let handoff_proven = state
        .recovery
        .lock()
        .map(|recovery| recovery.session_handoff_proven())
        .unwrap_or(false);
    for candidate in resumable_candidates(provider)? {
        if store.is_pending(&candidate.candidate_handle) {
            continue;
        }
        match recover_candidate(state, provider, store, &candidate, handoff_proven) {
            Ok(true) => return Ok(()),
            Ok(false) | Err(_) => continue,
        }
    }
    Ok(())
}

/// Recover restart-stable attempts from the native process startup path or an
/// authoritative product transition. Presentation reads never call this
/// function, so status queries cannot advance qualification lifecycle state.
pub(crate) fn recover_persisted_sessions(
    state: &AppState,
    provider: &QualificationRepository,
) -> Result<(), String> {
    let mut store = match state.qualification_sessions.lock() {
        Ok(store) => store,
        Err(poisoned) => {
            state.qualification_sessions.clear_poison();
            poisoned.into_inner()
        }
    };
    ensure_recovered(state, provider, &mut store)
}

/// Recover one persisted candidate. Returns true when it resumed as the active
/// session in this process.
fn recover_candidate(
    state: &AppState,
    provider: &QualificationRepository,
    store: &mut QualificationSessionStore,
    candidate: &QualificationCandidateSummary,
    handoff_proven: bool,
) -> Result<bool, String> {
    let candidate_handle = &candidate.candidate_handle;
    if provider
        .session_is_poisoned(candidate_handle)
        .unwrap_or(true)
    {
        if let Ok(value) = provider.load_session_json(candidate_handle) {
            if let Ok(persisted) = serde_json::from_value::<PersistedQualificationSession>(value) {
                if persisted.candidate_handle == *candidate_handle {
                    if let Ok(mut session) = QualificationSession::from_persisted(persisted) {
                        session.invalidate(QualificationInvalidation::ObservationFailed);
                        if session.authored_recipe_digests().is_none() {
                            if let Ok(digests) =
                                provider.capture_authored_recipe_digests(session.required_recipes())
                            {
                                session.set_authored_recipe_digests(digests);
                            }
                        }
                        let _ = finalize_candidate(provider, &session);
                    }
                }
            }
        }
        return Ok(false);
    }
    let value = match provider.load_session_json(candidate_handle) {
        Ok(value) => value,
        Err(_) => {
            poison_attempt(state, provider, store, candidate_handle);
            return Ok(false);
        }
    };
    let Some(version) = value.get("sessionSchemaVersion").and_then(Value::as_u64) else {
        poison_attempt(state, provider, store, candidate_handle);
        return Ok(false);
    };
    if version != SESSION_SCHEMA_VERSION {
        let mut session = match legacy_session_for_recovery(&value) {
            Ok(session) => session,
            Err(_) => {
                poison_attempt(state, provider, store, candidate_handle);
                return Ok(false);
            }
        };
        match provider.capture_authored_recipe_digests(session.required_recipes()) {
            Ok(digests) => session.set_authored_recipe_digests(digests),
            Err(_) => {
                poison_attempt(state, provider, store, candidate_handle);
                return Ok(false);
            }
        }
        session.invalidate(QualificationInvalidation::IncompatibleSessionVersion);
        match finalize_candidate(provider, &session) {
            Ok(()) => {
                let _ = provider.remove_session_report(candidate_handle);
                store.forget(candidate_handle);
                Ok(false)
            }
            Err(error) => {
                poison_attempt(state, provider, store, candidate_handle);
                Err(error)
            }
        }
    } else {
        recover_current_candidate(state, provider, store, candidate, value, handoff_proven)
    }
}

fn recover_current_candidate(
    state: &AppState,
    provider: &QualificationRepository,
    store: &mut QualificationSessionStore,
    candidate: &QualificationCandidateSummary,
    value: Value,
    handoff_proven: bool,
) -> Result<bool, String> {
    let candidate_handle = &candidate.candidate_handle;
    let persisted: PersistedQualificationSession = match serde_json::from_value(value) {
        Ok(persisted) => persisted,
        Err(_) => {
            poison_attempt(state, provider, store, candidate_handle);
            return Ok(false);
        }
    };
    if persisted.candidate_handle != *candidate_handle {
        poison_attempt(state, provider, store, candidate_handle);
        return Ok(false);
    }
    let mut session = match QualificationSession::from_persisted(persisted) {
        Ok(session) => session,
        Err(_) => {
            poison_attempt(state, provider, store, candidate_handle);
            return Ok(false);
        }
    };
    session.set_terminal_report(provider.load_session_report(candidate_handle)?);
    if session.run_validity() == RunValidity::Invalid {
        return finalize_recovered_invalid(state, provider, store, session);
    }
    if !handoff_proven {
        session.invalidate(QualificationInvalidation::UnprovenShutdown);
        if persist(provider, &session).is_err() {
            poison_attempt(state, provider, store, candidate_handle);
            return Err(persistence_error());
        }
        return finalize_recovered_invalid(state, provider, store, session);
    }
    if session.execution_admitted() && session.terminal_execution_status().is_none() {
        // The previous process admitted a real execution but never retained
        // its authoritative terminal transition. The attempt can never prove
        // what that run produced, so it fails closed instead of resuming as
        // execution-active with a binding this process never established.
        session.invalidate(QualificationInvalidation::LifecycleTransitionMissed);
        if persist(provider, &session).is_err() {
            poison_attempt(state, provider, store, candidate_handle);
            return Err(persistence_error());
        }
        return finalize_recovered_invalid(state, provider, store, session);
    }
    store.set_active(candidate_handle.clone());
    Ok(true)
}

/// Materialize one recovered attempt that can never produce valid evidence and
/// forget its process-local authority. A persistence failure poisons the
/// attempt in memory and never claims the invalid candidate exists.
fn finalize_recovered_invalid(
    state: &AppState,
    provider: &QualificationRepository,
    store: &mut QualificationSessionStore,
    session: QualificationSession,
) -> Result<bool, String> {
    let candidate_handle = session.candidate_handle().to_string();
    match finalize_candidate(provider, &session) {
        Ok(()) => {
            let _ = provider.remove_session_report(&candidate_handle);
            store.forget(&candidate_handle);
            Ok(false)
        }
        Err(error) => {
            poison_attempt(state, provider, store, &candidate_handle);
            Err(error)
        }
    }
}

/// Best-effort read of a version-1 session so an incompatible attempt can be
/// converted into an invalid audit candidate without trusting its contents.
fn legacy_session_for_recovery(value: &Value) -> Result<QualificationSession, String> {
    let session_handle = value
        .get("sessionHandle")
        .and_then(Value::as_str)
        .ok_or_else(invalid_error)?;
    let candidate_handle = value
        .get("candidateHandle")
        .and_then(Value::as_str)
        .ok_or_else(invalid_error)?;
    let captured_at = value
        .get("capturedAt")
        .and_then(Value::as_str)
        .ok_or_else(invalid_error)?;
    // The legacy shape always carried a process-local device handle; its
    // presence is part of the version-1 contract being rejected here.
    value
        .get("deviceHandle")
        .and_then(Value::as_str)
        .ok_or_else(invalid_error)?;
    let target: QualificationTargetBinding =
        serde_json::from_value(value.get("target").cloned().ok_or_else(invalid_error)?)
            .map_err(|_| invalid_error())?;
    let workflow_id = value
        .get("workflowId")
        .and_then(Value::as_str)
        .ok_or_else(invalid_error)?;
    let workflow_version = value
        .get("workflowVersion")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let device_plan = value
        .get("devicePlan")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let required_recipes: Vec<String> = serde_json::from_value(
        value
            .get("requiredRecipes")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .map_err(|_| invalid_error())?;
    let prerequisites: Vec<String> = serde_json::from_value(
        value
            .get("prerequisites")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .map_err(|_| invalid_error())?;
    let human_checkpoints: Vec<QualificationWorkflowCheckpoint> = serde_json::from_value(
        value
            .get("humanCheckpoints")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .map_err(|_| invalid_error())?;
    let automated_observations: Vec<QualificationWorkflowObservation> = serde_json::from_value(
        value
            .get("automatedObservations")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .map_err(|_| invalid_error())?;
    let build: QualificationBuildIdentity =
        serde_json::from_value(value.get("build").cloned().ok_or_else(invalid_error)?)
            .map_err(|_| invalid_error())?;
    let runtime_contract = value
        .get("runtimeContract")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let session = QualificationSession::new(
        session_handle.to_string(),
        candidate_handle.to_string(),
        captured_at.to_string(),
        target,
        QualificationWorkflow {
            id: workflow_id.to_string(),
            version: workflow_version,
            purpose: String::new(),
            production_recipes: required_recipes,
            required_capabilities: Vec::new(),
            prerequisites,
            human_checkpoints,
            compatibility_dimensions: Vec::new(),
            automated_observations,
        },
        build,
        runtime_contract,
    )?;
    Ok(session.with_device_plan(device_plan.to_string()))
}

/// Apply one closed lifecycle observation to the active session.
///
/// Qualification failures never propagate to the caller: the session either
/// records the transition, becomes invalid, or becomes poisoned/non-recordable.
/// When no attempt is active this is a no-persistence no-op.
pub(crate) fn observe(state: &AppState, observation: QualificationLifecycleObservation) {
    let Some(provider) = state.qualification_repository.get() else {
        return;
    };
    let mut store = match state.qualification_sessions.lock() {
        Ok(store) => store,
        Err(poisoned) => {
            let store = poisoned.into_inner();
            state.qualification_sessions.clear_poison();
            store
        }
    };
    if ensure_recovered(state, provider, &mut store).is_err() {
        return;
    }
    let Some(candidate_handle) = store.active_candidate.clone() else {
        return;
    };
    if store.is_poisoned(&candidate_handle) {
        return;
    }
    let mut session = match load_active_session(provider, &store, &candidate_handle) {
        Ok(session) => session,
        Err(_) => {
            poison_attempt(state, provider, &mut store, &candidate_handle);
            return;
        }
    };
    if session.is_closed() {
        store.forget(&candidate_handle);
        return;
    }
    match &observation {
        QualificationLifecycleObservation::DeviceObserved(observation) => {
            apply_device_observation(&mut session, &mut store, observation);
        }
        QualificationLifecycleObservation::DeviceObservationFailed { device_handle } => {
            if store
                .associated_device_handle()
                .is_some_and(|associated| associated == device_handle)
            {
                session.invalidate(QualificationInvalidation::ObservationFailed);
            }
        }
        QualificationLifecycleObservation::RootChecked {
            device_handle,
            root_state,
        } => {
            if store
                .associated_device_handle()
                .is_some_and(|associated| associated == device_handle)
            {
                session.observe_root_authority(root_state.clone());
            }
        }
        QualificationLifecycleObservation::ReviewCreated(review) => {
            if store
                .associated_device_handle()
                .is_some_and(|associated| associated == review.device_handle)
            {
                // Authored source digests are captured at trusted session start
                // and remain immutable. A later review must not replace them
                // with bytes from a changed working tree.
                session.observe_review(review);
            }
        }
        QualificationLifecycleObservation::RealExecutionAdmitted(admission) => {
            session.admit_execution(admission, store.associated_device_handle());
        }
        QualificationLifecycleObservation::RealExecutionTerminal(terminal) => {
            session.classify_terminal(terminal);
        }
        QualificationLifecycleObservation::RealExecutionLost { execution_handle } => {
            if session
                .bound_execution_handle()
                .is_some_and(|bound| bound == execution_handle)
                && session.terminal_execution_status().is_none()
            {
                session.invalidate(QualificationInvalidation::ExecutionUnavailable);
            }
        }
    }
    let _ = finish_transition(state, provider, &mut store, session);
}

/// Invalidate an associated attempt when an authoritative product probe cannot
/// be decoded into the typed facts required to preserve device identity.
pub(crate) fn observe_device_observation_failure(state: &AppState, device_handle: &str) {
    observe(
        state,
        QualificationLifecycleObservation::DeviceObservationFailed {
            device_handle: device_handle.to_string(),
        },
    );
}

fn apply_device_observation(
    session: &mut QualificationSession,
    store: &mut QualificationSessionStore,
    observation: &SelectedDeviceObservation,
) {
    if session.run_validity() == RunValidity::Invalid || session.is_closed() {
        return;
    }
    if let Some(associated) = store.associated_device_handle().map(str::to_string) {
        if associated != observation.device_handle {
            session.invalidate(QualificationInvalidation::DeviceIdentityChanged);
        } else if store
            .associated_device_session_epoch()
            .is_some_and(|epoch| observation.session_epoch != Some(epoch))
        {
            session.invalidate(QualificationInvalidation::DeviceUnavailable);
        } else {
            session.observe_matching_target(observation);
        }
        return;
    }
    // Without an association, first accumulate the trusted observation state
    // of the currently selected device. Any observed conflict with the
    // immutable registered target invalidates the attempt; a state that merely
    // lacks conflicting facts is not yet evidence of compatibility, because
    // the trusted seams do not all observe the same subset of target facts.
    let accumulated = match store.observed_device() {
        Some(existing) if existing.device_handle == observation.device_handle => {
            existing.merged_with(observation)
        }
        _ => observation.clone(),
    };
    session.observe_matching_target(&accumulated);
    if session.run_validity() == RunValidity::Valid && !session.is_closed() {
        if accumulated.proves_target_compatibility() && accumulated.session_epoch.is_some() {
            store.associate(observation.device_handle.clone(), accumulated.session_epoch);
        } else {
            store.observe_device(accumulated);
        }
    }
}

/// Persist the new session state and materialize immutable evidence when the
/// attempt closed (terminal evidence complete, or invalid). Every failure here
/// stays inside qualification evidence.
fn finish_transition(
    state: &AppState,
    provider: &QualificationRepository,
    store: &mut QualificationSessionStore,
    mut session: QualificationSession,
) -> Option<QualificationSessionSnapshot> {
    session.resolve_terminal_outcome();
    // Retain the process-local review and execution bindings this transition
    // established. They are never persisted, so the store is their only
    // durable-for-this-process home.
    store.retain_bindings(
        session.bound_review_handle.clone(),
        session.bound_execution_handle.clone(),
    );
    let candidate_handle = session.candidate_handle().to_string();
    if persist(provider, &session).is_err() {
        poison_attempt(state, provider, store, &candidate_handle);
        return None;
    }
    if session.terminal_execution_status().is_some()
        && persist_terminal_report(provider, &session).is_err()
    {
        poison_attempt(state, provider, store, &candidate_handle);
        return None;
    }
    let invalid = session.run_validity() == RunValidity::Invalid;
    if invalid || session.terminal_evidence_ready() {
        if !invalid
            && !session
                .authored_recipe_digests()
                .is_some_and(|expected| provider.authored_recipe_digests_match(expected))
        {
            // Source authority can change temporarily while an execution is
            // active. Retain the terminal observation and keep the attempt
            // pending until canonical source again matches its session-start
            // fingerprint; never close it with evidence that cannot be recorded.
            return candidate_summary(provider, &candidate_handle)
                .ok()
                .map(|candidate| session.snapshot(Some(candidate)));
        }
        if finalize_candidate(provider, &session).is_err() {
            poison_attempt(state, provider, store, &candidate_handle);
            return None;
        }
        let mut closed = session;
        closed.close();
        if persist(provider, &closed).is_err() {
            poison_attempt(state, provider, store, &candidate_handle);
            return None;
        }
        if provider.remove_session_report(&candidate_handle).is_err() {
            poison_attempt(state, provider, store, &candidate_handle);
            return None;
        }
        if provider.remove_session(&candidate_handle).is_err() {
            poison_attempt(state, provider, store, &candidate_handle);
            return None;
        }
        store.forget(&candidate_handle);
        session = closed;
    }
    candidate_summary(provider, &candidate_handle)
        .ok()
        .map(|candidate| session.snapshot(Some(candidate)))
}

/// Caller-resolved inputs for starting one qualification attempt. The caller
/// must have resolved the canonical workflow and target from repository state,
/// validated the selected device through device observation, and captured the
/// trusted build identity.
pub(crate) struct BeginSessionRequest {
    pub(crate) session_handle: String,
    pub(crate) candidate_handle: String,
    pub(crate) captured_at: String,
    pub(crate) device_plan: String,
    pub(crate) target: QualificationTargetBinding,
    pub(crate) workflow: QualificationWorkflow,
    pub(crate) build: QualificationBuildIdentity,
    pub(crate) runtime_contract: String,
    pub(crate) observation: SelectedDeviceObservation,
}

pub(crate) fn begin(
    state: &AppState,
    request: BeginSessionRequest,
) -> Result<QualificationSessionSnapshot, String> {
    let provider = state
        .qualification_repository
        .get()
        .ok_or_else(unavailable_error)?;
    let mut store = state
        .qualification_sessions
        .lock()
        .map_err(|_| persistence_error())?;
    if store.active_candidate.is_some() {
        return Err(session_error(
            "qualification_session_active",
            "A qualification attempt is already active. Finish or abandon it before starting another.",
        ));
    }
    // The candidate was created for this attempt moments ago, so it must never
    // be recovered as an interrupted attempt from a previous process.
    store.mark_pending(&request.candidate_handle);
    ensure_recovered(state, provider, &mut store)?;
    if store.active_candidate.is_some() {
        store.forget(&request.candidate_handle);
        return Err(session_error(
            "qualification_session_active",
            "A qualification attempt is already active. Finish or abandon it before starting another.",
        ));
    }
    let mut session = QualificationSession::new(
        request.session_handle,
        request.candidate_handle.clone(),
        request.captured_at,
        request.target,
        request.workflow,
        request.build,
        request.runtime_contract,
    )?
    .with_device_plan(request.device_plan);
    let authored_recipe_digests = provider
        .capture_authored_recipe_digests(session.required_recipes())
        .map_err(|_| {
            session_error(
                "qualification_source_changed",
                "Qualification source changed before the attempt could be retained.",
            )
        })?;
    session.set_authored_recipe_digests(authored_recipe_digests);
    if request
        .observation
        .root_state
        .as_ref()
        .and_then(project_root_state)
        != Some(session.target.root_state.clone())
    {
        return Err(session_error(
            "qualification_target_unverified",
            "A fresh root check could not verify the registered target state.",
        ));
    }
    session.observe_matching_target(&request.observation);
    if session.run_validity() == RunValidity::Invalid {
        return Err(session_error(
            "qualification_target_unverified",
            "The selected device does not match the registered qualification target.",
        ));
    }
    persist(provider, &session)?;
    store.set_active(request.candidate_handle.clone());
    store.associate(
        request.observation.device_handle.clone(),
        request.observation.session_epoch,
    );
    let candidate = candidate_summary(provider, session.candidate_handle())?;
    Ok(session.snapshot(Some(candidate)))
}

/// Record one explicit operator checkpoint. When the terminal execution is
/// already retained, completing the remaining required evidence materializes
/// the immutable candidate automatically.
pub(crate) fn record_checkpoint(
    state: &AppState,
    session_handle: &str,
    checkpoint_id: &str,
    outcome: CheckpointOutcome,
) -> Result<QualificationSessionSnapshot, String> {
    let provider = state
        .qualification_repository
        .get()
        .ok_or_else(unavailable_error)?;
    let candidate_handle = candidate_handle_for_session(session_handle)?;
    let mut store = state
        .qualification_sessions
        .lock()
        .map_err(|_| persistence_error())?;
    ensure_recovered(state, provider, &mut store)?;
    if store.is_poisoned(&candidate_handle) {
        return Err(persistence_error());
    }
    if store.active_candidate() != Some(candidate_handle.as_str()) {
        return Err(inactive_error());
    }
    let mut session = load_active_session(provider, &store, &candidate_handle)?;
    session
        .record_checkpoint(checkpoint_id, outcome)
        .map_err(|_| {
            session_error(
                "qualification_checkpoint_invalid",
                "The checkpoint is not allowed by the selected production workflow.",
            )
        })?;
    finish_transition(state, provider, &mut store, session).ok_or_else(persistence_error)
}

/// Abandon the active attempt. The attempt closes as invalid/not-observed and
/// an immutable invalid candidate is materialized when durable persistence
/// succeeds; otherwise the attempt is poisoned and never claimed as recorded.
pub(crate) fn abandon(
    state: &AppState,
    session_handle: &str,
) -> Result<QualificationSessionSnapshot, String> {
    let provider = state
        .qualification_repository
        .get()
        .ok_or_else(unavailable_error)?;
    let candidate_handle = candidate_handle_for_session(session_handle)?;
    let mut store = state
        .qualification_sessions
        .lock()
        .map_err(|_| persistence_error())?;
    ensure_recovered(state, provider, &mut store)?;
    if store.is_poisoned(&candidate_handle) {
        store.forget(&candidate_handle);
        return Err(persistence_error());
    }
    if store.active_candidate() != Some(candidate_handle.as_str()) {
        return Err(inactive_error());
    }
    let mut session = load_active_session(provider, &store, &candidate_handle)?;
    session.invalidate(QualificationInvalidation::OperatorAbandoned);
    finish_transition(state, provider, &mut store, session).ok_or_else(persistence_error)
}

/// Drop process-local authority for one candidate. Called after the operator
/// records or discards a candidate so a later launch cannot reassociate it.
pub(crate) fn forget_candidate(state: &AppState, candidate_handle: &str) {
    if let Ok(mut store) = state.qualification_sessions.lock() {
        store.forget(candidate_handle);
    }
}

/// Reset all process-local qualification session authority for a new frontend
/// session. Persisted candidates are unaffected.
pub(crate) fn reset(state: &AppState) {
    if let Ok(mut store) = state.qualification_sessions.lock() {
        store.reset();
    }
}

/// Whether one candidate handle is currently the active in-process attempt.
/// Report one authoritative inventory reconciliation to the active attempt.
///
/// When the device an attempt is associated with is no longer present, the
/// product has dropped that device's authority, so the attempt can no longer
/// prove it ran against the same device and fails closed.
pub(crate) fn observe_device_inventory(state: &AppState, available_devices: &[(String, u64)]) {
    let Some(provider) = state.qualification_repository.get() else {
        return;
    };
    let mut store = match state.qualification_sessions.lock() {
        Ok(store) => store,
        Err(poisoned) => {
            let store = poisoned.into_inner();
            state.qualification_sessions.clear_poison();
            store
        }
    };
    if ensure_recovered(state, provider, &mut store).is_err() {
        return;
    }
    let Some(candidate_handle) = store.active_candidate.clone() else {
        return;
    };
    if store.is_poisoned(&candidate_handle) {
        return;
    }
    let Some(associated) = store.associated_device_handle().map(str::to_string) else {
        if let Some(observed) = store.observed_device() {
            if !available_devices.iter().any(|(handle, epoch)| {
                handle == &observed.device_handle && observed.session_epoch == Some(*epoch)
            }) {
                store.clear_observed_device();
            }
        }
        return;
    };
    if available_devices.iter().any(|(handle, epoch)| {
        handle == &associated
            && store
                .associated_device_session_epoch()
                .is_none_or(|expected| expected == *epoch)
    }) {
        return;
    }
    let mut session = match load_active_session(provider, &store, &candidate_handle) {
        Ok(session) => session,
        Err(_) => {
            poison_attempt(state, provider, &mut store, &candidate_handle);
            return;
        }
    };
    if session.is_closed() {
        store.forget(&candidate_handle);
        return;
    }
    session.invalidate(QualificationInvalidation::DeviceUnavailable);
    let _ = finish_transition(state, provider, &mut store, session);
}

/// Sanitized snapshot of the active session, if one exists.
pub(crate) fn session_status(
    state: &AppState,
) -> Result<Option<QualificationSessionSnapshot>, String> {
    let Some(provider) = state.qualification_repository.get() else {
        return Ok(None);
    };
    let store = match state.qualification_sessions.lock() {
        Ok(store) => store,
        Err(poisoned) => {
            state.qualification_sessions.clear_poison();
            poisoned.into_inner()
        }
    };
    let Some(candidate_handle) = store.active_candidate.clone() else {
        return Ok(None);
    };
    if store.is_poisoned(&candidate_handle) {
        return Ok(None);
    }
    session_snapshot(provider, &store, &candidate_handle).map(Some)
}

/// Resolve the active session's device plan for product match observations.
/// The plan is durable session intent; no device association is inferred here.
pub(crate) fn active_device_plan(state: &AppState) -> Option<String> {
    let provider = state.qualification_repository.get()?;
    let mut store = match state.qualification_sessions.lock() {
        Ok(store) => store,
        Err(poisoned) => {
            state.qualification_sessions.clear_poison();
            poisoned.into_inner()
        }
    };
    if ensure_recovered(state, provider, &mut store).is_err() {
        return None;
    }
    let candidate_handle = store.active_candidate.clone()?;
    if store.is_poisoned(&candidate_handle) {
        return None;
    }
    load_active_session(provider, &store, &candidate_handle)
        .ok()
        .map(|session| session.device_plan.clone())
}

/// Project whether the active attempt currently owns the selected product
/// device. A restored session remains selectable until a trusted observation
/// reestablishes its process-local association.
pub(crate) fn device_selection_locked(state: &AppState) -> Result<bool, String> {
    if state.qualification_repository.get().is_none() {
        return Ok(false);
    }
    let store = match state.qualification_sessions.lock() {
        Ok(store) => store,
        Err(poisoned) => {
            state.qualification_sessions.clear_poison();
            poisoned.into_inner()
        }
    };
    let Some(candidate_handle) = store.active_candidate.clone() else {
        return Ok(false);
    };
    if store.is_poisoned(&candidate_handle) {
        return Ok(false);
    }
    Ok(store.associated_device_handle().is_some())
}

/// Snapshot of one candidate's session state as sanitized presentation data.
fn session_snapshot(
    provider: &QualificationRepository,
    store: &QualificationSessionStore,
    candidate_handle: &str,
) -> Result<QualificationSessionSnapshot, String> {
    let session = load_active_session(provider, store, candidate_handle)?;
    let candidate = candidate_summary(provider, candidate_handle)?;
    Ok(session.snapshot(Some(candidate)))
}

fn candidate_summary(
    provider: &QualificationRepository,
    candidate_handle: &str,
) -> Result<QualificationCandidateSummaryDto, String> {
    let candidate = provider.load_candidate(candidate_handle)?;
    candidate_summary_from_stored(candidate)
}

fn candidate_summary_from_stored(
    candidate: StoredQualificationCandidate,
) -> Result<QualificationCandidateSummaryDto, String> {
    let summary = candidate.summary();
    let target = summary
        .target
        .map(|target| match serde_json::from_value(target) {
            Ok(target) => Ok(Some(target)),
            Err(_) if summary.kind == CandidateKind::QualificationRun => Ok(None),
            Err(_) => Err(invalid_error()),
        })
        .transpose()?
        .flatten();
    Ok(QualificationCandidateSummaryDto {
        candidate_handle: summary.candidate_handle,
        kind: summary.kind,
        captured_at: summary.captured_at,
        promotable: summary.promotable,
        non_promotable_reason: summary.non_promotable_reason,
        target,
        run_validity: summary.run_validity,
        qualification_outcome: summary.qualification_outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::AdbManager;
    use crate::commands::{AppState, InputContractSnapshot, PlatformToolsSelectionStore};
    use crate::device_qualification::RootQualificationStore;
    use crate::execution::ExecutionHandleStore;
    use crate::handles::SessionHandles;
    use crate::qualification_repository::{
        QualificationRepositoryProvider, QualificationSourceState, QualificationToolRunner,
    };
    use crate::recovery::RecoveryStore;
    use crate::saved_configurations::SavedConfigurationStore;
    use crate::sidecar::SidecarState;
    use crate::support::SupportStore;
    use crate::updates::{ActivityGate, UpdateService};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tauri::Manager;
    use tempfile::TempDir;

    /// Runner used only where a repository is constructed for session tests.
    /// Session persistence and candidate materialization never invoke Node.
    struct UnusedToolRunner;

    impl QualificationToolRunner for UnusedToolRunner {
        fn run(&self, _repo_root: &Path, _args: &[String]) -> Result<Vec<u8>, String> {
            Err("session tests must not invoke the canonical qualification tool".to_string())
        }
    }

    fn build_json() -> Value {
        json!({
            "appVersion": "0.1.0",
            "gitCommit": "1".repeat(40),
            "materialBuildDigest": format!("sha256:{}", "a".repeat(64)),
            "realExecutionEnabled": true,
            "qualificationContract": 1,
        })
    }

    fn test_build() -> QualificationBuildIdentity {
        serde_json::from_value(build_json()).expect("test build should decode")
    }

    fn target() -> QualificationTargetBinding {
        QualificationTargetBinding {
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
        }
    }

    fn workflow(prerequisite: bool) -> QualificationWorkflow {
        let mut checkpoints = vec![QualificationWorkflowCheckpoint {
            id: "device_state_verified".to_string(),
            instruction: "Verify the device state".to_string(),
            fact: "device_state".to_string(),
            allowed_outcomes: vec![
                QualificationCheckpointOutcome::Pass,
                QualificationCheckpointOutcome::Fail,
                QualificationCheckpointOutcome::UnableToVerify,
            ],
            required: true,
        }];
        if prerequisite {
            checkpoints.push(QualificationWorkflowCheckpoint {
                id: "clean_or_deliberately_reset_device".to_string(),
                instruction: "Reset the device".to_string(),
                fact: "device_reset".to_string(),
                allowed_outcomes: vec![
                    QualificationCheckpointOutcome::Pass,
                    QualificationCheckpointOutcome::Fail,
                    QualificationCheckpointOutcome::UnableToVerify,
                ],
                required: true,
            });
        }
        QualificationWorkflow {
            id: "test-workflow".to_string(),
            version: 1,
            purpose: "test".to_string(),
            production_recipes: vec!["test.recipe".to_string()],
            required_capabilities: Vec::new(),
            prerequisites: if prerequisite {
                vec!["clean_or_deliberately_reset_device".to_string()]
            } else {
                Vec::new()
            },
            human_checkpoints: checkpoints,
            compatibility_dimensions: Vec::new(),
            automated_observations: vec![QualificationWorkflowObservation {
                id: "execution-report".to_string(),
                required: true,
            }],
        }
    }

    fn observation(device_handle: &str) -> SelectedDeviceObservation {
        SelectedDeviceObservation {
            device_handle: device_handle.to_string(),
            session_epoch: Some(1),
            profile_id: Some("profile.test".to_string()),
            manufacturer: Some("Test".to_string()),
            model: Some("Device".to_string()),
            android_version: Some("15".to_string()),
            android_api: Some(35),
            abi_soc_class: Some("arm64".to_string()),
            firmware_build: Some("test/build".to_string()),
            root_state: Some(RootQualificationState::Denied),
        }
    }

    /// A device observation that established only identity facts. It matches
    /// the registered target for the facts it carries, but cannot prove
    /// compatibility on its own.
    fn partial_observation(device_handle: &str) -> SelectedDeviceObservation {
        SelectedDeviceObservation {
            device_handle: device_handle.to_string(),
            session_epoch: Some(1),
            profile_id: None,
            manufacturer: Some("Test".to_string()),
            model: Some("Device".to_string()),
            android_version: Some("15".to_string()),
            android_api: None,
            abi_soc_class: None,
            firmware_build: Some("test/build".to_string()),
            root_state: None,
        }
    }

    fn review(handle: &str, device_handle: &str) -> ReviewObservation {
        ReviewObservation {
            review_handle: handle.to_string(),
            device_handle: device_handle.to_string(),
            device_plan: "test-plan".to_string(),
            selected_recipes: vec!["test.recipe".to_string()],
            target_id: Some("target-test".to_string()),
            manufacturer: Some("Test".to_string()),
            model: Some("Device".to_string()),
            android_api: Some(35),
        }
    }

    fn test_repository(temp: &TempDir) -> QualificationRepository {
        std::fs::create_dir_all(temp.path().join("authored/recipes"))
            .expect("recipe directory should be created");
        std::fs::write(
            temp.path().join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\n",
        )
        .expect("recipe fixture should be written");
        QualificationRepository::new_for_test_with_source_state(
            temp.path().to_path_buf(),
            Box::new(UnusedToolRunner),
            test_build(),
            QualificationSourceState {
                head: "1".repeat(40),
                tracked_worktree_clean: true,
            },
        )
    }

    fn test_app(
        provider: QualificationRepositoryProvider,
        proven_clean_handoff: bool,
    ) -> (TempDir, tauri::App<tauri::test::MockRuntime>) {
        test_app_with_frontend_session(provider, proven_clean_handoff, true)
    }

    fn test_app_before_frontend_session(
        provider: QualificationRepositoryProvider,
        proven_clean_handoff: bool,
    ) -> (TempDir, tauri::App<tauri::test::MockRuntime>) {
        test_app_with_frontend_session(provider, proven_clean_handoff, false)
    }

    fn test_app_with_frontend_session(
        provider: QualificationRepositoryProvider,
        proven_clean_handoff: bool,
        begin_frontend_session: bool,
    ) -> (TempDir, tauri::App<tauri::test::MockRuntime>) {
        let temp = tempfile::tempdir().expect("test app directory should be created");
        let app_root = temp.path();
        let marker = app_root.join("session-active.marker");
        if !proven_clean_handoff {
            std::fs::write(&marker, b"1").expect("interrupted marker should be written");
        }
        let recovery = RecoveryStore::load(app_root.join("recovery-draft.json"), marker);
        let mut recovery = recovery;
        if begin_frontend_session {
            recovery
                .begin_session()
                .expect("recovery session should begin");
        }
        let app_state = AppState {
            sidecar: SidecarState::new(app_root.join("sidecar-cache")),
            catalog: Err("test catalog is not needed by qualification commands".to_string()),
            qualification_repository: provider,
            adb: Mutex::new(AdbManager::new(app_root.join("platform-tools"))),
            platform_tools_selections: Mutex::new(PlatformToolsSelectionStore::default()),
            input_contracts: Mutex::new(InputContractSnapshot::default()),
            handles: Mutex::new(SessionHandles::default()),
            root_qualification: Mutex::new(RootQualificationStore::default()),
            executions: Mutex::new(ExecutionHandleStore::default()),
            qualification_sessions: Mutex::new(QualificationSessionStore::default()),
            saved_configurations: Mutex::new(SavedConfigurationStore::load(
                app_root.join("recent-configurations.json"),
            )),
            recovery: Mutex::new(recovery),
            support: Mutex::new(SupportStore::new(app_root.join("support-cache"))),
            updates: UpdateService::from_production_document()
                .expect("test update trust should be available"),
            update_activity: ActivityGate::default(),
        };
        let app = tauri::test::mock_app();
        assert!(app.manage(app_state));
        (temp, app)
    }

    /// Launch the application again over the same repository directory with a
    /// proven clean handoff, exactly like the ordinary launch recovery path.
    fn clean_restart(temp: &TempDir) -> (TempDir, tauri::App<tauri::test::MockRuntime>) {
        let repository = test_repository(temp);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (app_temp, app) = test_app(provider, true);
        crate::qualification_mode::recover_sessions_at_process_start(&app.state::<AppState>());
        (app_temp, app)
    }

    fn persisted_session_for_recipe_digests(
        required_recipes: &[&str],
        digests: &[(&str, &str)],
    ) -> PersistedQualificationSession {
        let mut persisted = QualificationSession::for_test(&[]).to_persisted();
        persisted.required_recipes = required_recipes
            .iter()
            .map(|recipe| (*recipe).to_string())
            .collect();
        persisted.authored_recipe_digests = Some(
            digests
                .iter()
                .map(|(id, sha256)| AuthoredRecipeDigest {
                    id: (*id).to_string(),
                    sha256: (*sha256).to_string(),
                })
                .collect(),
        );
        persisted
    }

    #[test]
    fn restored_session_rejects_empty_or_missing_authored_recipe_digests() {
        let digest = "a".repeat(64);
        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["test.recipe"],
                &[],
            ))
            .is_err()
        );

        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["recipe.one", "recipe.two"],
                &[("recipe.one", &digest)],
            ))
            .is_err()
        );

        let mut missing_set =
            persisted_session_for_recipe_digests(&["test.recipe"], &[("test.recipe", &digest)]);
        missing_set.authored_recipe_digests = None;
        assert!(QualificationSession::from_persisted(missing_set).is_err());
    }

    #[test]
    fn restored_session_rejects_duplicate_recipe_ids_in_required_and_digest_sets() {
        let digest = "a".repeat(64);
        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["recipe.one", "recipe.one"],
                &[("recipe.one", &digest), ("recipe.one", &digest)],
            ))
            .is_err()
        );
        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["recipe.one"],
                &[("recipe.one", &digest), ("recipe.one", &digest)],
            ))
            .is_err()
        );
    }

    #[test]
    fn restored_session_rejects_extra_or_malformed_authored_recipe_digests() {
        let digest = "a".repeat(64);
        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["recipe.one"],
                &[("recipe.one", &digest), ("recipe.extra", &digest)],
            ))
            .is_err()
        );
        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["bad recipe id"],
                &[("bad recipe id", &digest)],
            ))
            .is_err()
        );
        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["recipe.one"],
                &[("recipe.one", "abc")],
            ))
            .is_err()
        );
        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["recipe.one"],
                &[("recipe.one", &"g".repeat(64))],
            ))
            .is_err()
        );
        assert!(
            QualificationSession::from_persisted(persisted_session_for_recipe_digests(
                &["recipe.one"],
                &[("recipe.one", &"A".repeat(64))],
            ))
            .is_err()
        );
    }

    #[test]
    fn restored_session_accepts_complete_recipe_digests_in_different_order() {
        let first = "a".repeat(64);
        let second = "b".repeat(64);
        let persisted = persisted_session_for_recipe_digests(
            &["recipe.one", "recipe.two"],
            &[("recipe.two", &second), ("recipe.one", &first)],
        );

        assert!(QualificationSession::from_persisted(persisted).is_ok());
    }

    #[test]
    fn native_startup_recovers_session_before_begin_app_session() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(
                &app.state::<AppState>(),
                begin_request(&candidate, CAPTURED_AT, observation("device-one")),
            )
            .unwrap();
        }

        let provider = QualificationRepositoryProvider::for_test(test_repository(&temp));
        let (_app_temp, app) = test_app_before_frontend_session(provider, true);
        assert!(app
            .state::<AppState>()
            .recovery
            .lock()
            .unwrap()
            .session_handoff_proven());

        crate::qualification_mode::recover_sessions_at_process_start(&app.state::<AppState>());
        let recovered = session_status(&app.state::<AppState>())
            .unwrap()
            .expect("native process startup should recover the cleanly handed-off session");
        assert_eq!(recovered.phase, QualificationSessionPhase::ExecutionPending);
        assert!(recovered.recordable);
        assert!(!device_selection_locked(&app.state::<AppState>()).unwrap());
    }

    #[test]
    fn qualification_status_recovers_persisted_sessions_before_projection() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(
                &app.state::<AppState>(),
                begin_request(&candidate, CAPTURED_AT, observation("device-one")),
            )
            .unwrap();
        }

        let provider = QualificationRepositoryProvider::for_test(test_repository(&temp));
        let (_app_temp, app) = test_app_before_frontend_session(provider, true);
        let status = crate::qualification_mode::get_device_qualification_mode_status(app.state());
        assert!(
            status.is_err(),
            "this minimal fixture intentionally omits the workflow catalog needed for projection"
        );
        assert_eq!(
            app.state::<AppState>()
                .qualification_sessions
                .lock()
                .unwrap()
                .active_candidate(),
            Some(candidate.as_str())
        );
    }

    #[test]
    fn active_device_plan_recovers_a_poisoned_session_store_lock() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        let state = app.state::<AppState>();
        begin(
            &state,
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _store = state.qualification_sessions.lock().unwrap();
            panic!("inject a poisoned session-store mutex");
        }));

        assert_eq!(active_device_plan(&state).as_deref(), Some("test-plan"));
        assert!(!state.qualification_sessions.is_poisoned());
    }

    #[test]
    fn empty_current_recipe_digest_set_is_recovered_as_discardable() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let session_path = repository
            .candidate_root()
            .join(&candidate)
            .join("session.json");
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (app_temp, app) = test_app(provider, true);
            begin(
                &app.state::<AppState>(),
                begin_request(&candidate, CAPTURED_AT, observation("device-one")),
            )
            .unwrap();
            let mut persisted = app
                .state::<AppState>()
                .qualification_repository
                .get()
                .expect("qualification repository should be available")
                .load_session_json(&candidate)
                .expect("the admitted session should have been persisted");
            persisted["authoredRecipeDigests"] = json!([]);
            std::fs::write(
                &session_path,
                serde_json::to_vec_pretty(&persisted).unwrap(),
            )
            .unwrap();
            drop(app);
            drop(app_temp);
        }

        let provider = QualificationRepositoryProvider::for_test(test_repository(&temp));
        let (_app_temp, app) = test_app_before_frontend_session(provider, true);
        let app_state = app.state::<AppState>();
        let status = crate::qualification_mode::get_device_qualification_mode_status(app.state());
        assert!(status.is_err());
        assert!(session_status(&app_state).unwrap().is_none());

        let repository = app_state.qualification_repository.get().unwrap();
        let summary = repository
            .list_candidates()
            .unwrap()
            .into_iter()
            .find(|summary| summary.candidate_handle == candidate)
            .expect("the malformed candidate should remain available for discard");
        assert!(!summary.promotable);
        assert!(summary.non_promotable_reason.is_some());
        assert!(resumable_candidates(repository).unwrap().is_empty());
    }

    fn create_run_candidate(repository: &QualificationRepository, captured_at: &str) -> String {
        repository
            .create_candidate(
                CandidateKind::QualificationRun,
                &json!({
                    "capturedAt": captured_at,
                    "build": build_json(),
                }),
                None,
            )
            .expect("run candidate should be created")
    }

    fn begin_request(
        candidate_handle: &str,
        captured_at: &str,
        observation: SelectedDeviceObservation,
    ) -> BeginSessionRequest {
        BeginSessionRequest {
            session_handle: session_handle_for_candidate(candidate_handle)
                .expect("candidate handle should convert"),
            candidate_handle: candidate_handle.to_string(),
            captured_at: captured_at.to_string(),
            device_plan: "test-plan".to_string(),
            target: target(),
            workflow: workflow(true),
            build: test_build(),
            runtime_contract: "real-execution-v1".to_string(),
            observation,
        }
    }

    const CAPTURED_AT: &str = "2026-08-23T12:00:00Z";

    #[test]
    fn inactive_observation_is_a_no_persistence_no_op() {
        let (_temp, app) = test_app(
            QualificationRepositoryProvider::unavailable_for_test(),
            true,
        );
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(observation("device-one"))),
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        assert!(&app
            .state::<AppState>()
            .qualification_sessions
            .lock()
            .unwrap()
            .active_candidate()
            .is_none());
    }

    #[test]
    fn begin_persists_one_active_session_and_rejects_a_second_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        let snapshot = begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .expect("first attempt should begin");
        assert_eq!(snapshot.phase, QualificationSessionPhase::ExecutionPending);
        assert!(snapshot.recordable);
        assert!(session_status(&app.state::<AppState>()).unwrap().is_some());
        assert!(device_selection_locked(&app.state::<AppState>()).unwrap());

        let app_state = app.state::<AppState>();
        let repository = app_state.qualification_repository.get().unwrap();
        let session_before = repository.load_session_json(&candidate).unwrap();
        let candidates_before = repository.list_candidates().unwrap().len();
        let side_effects = std::cell::Cell::new(0);
        let rejected = crate::qualification_mode::with_inactive_qualification_session(
            &app_state,
            repository,
            || {
                side_effects.set(side_effects.get() + 1);
                Ok(())
            },
        )
        .expect_err("an active attempt must be rejected before the begin operation runs");
        assert!(rejected.contains("qualification_session_active"));
        assert_eq!(side_effects.get(), 0);
        assert_eq!(
            repository.load_session_json(&candidate).unwrap(),
            session_before
        );
        assert_eq!(
            repository.list_candidates().unwrap().len(),
            candidates_before
        );
        assert_eq!(
            session_status(&app.state::<AppState>())
                .unwrap()
                .unwrap()
                .session_handle,
            snapshot.session_handle,
        );

        let second = create_run_candidate(
            &app.state::<AppState>()
                .qualification_repository
                .get()
                .unwrap(),
            CAPTURED_AT,
        );
        let error = begin(
            &app.state::<AppState>(),
            begin_request(&second, CAPTURED_AT, observation("device-one")),
        )
        .expect_err("a second attempt must be rejected");
        assert!(
            error.contains("already active"),
            "unexpected error: {error}"
        );
        assert!(device_selection_locked(&app.state::<AppState>()).unwrap());
        assert_eq!(
            session_status(&app.state::<AppState>())
                .unwrap()
                .unwrap()
                .session_handle,
            snapshot.session_handle,
        );
    }

    #[test]
    fn begin_rejects_a_device_that_does_not_match_the_registered_target() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        let mut drifted = observation("device-one");
        drifted.model = Some("Other".to_string());
        assert!(begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, drifted)
        )
        .is_err());
        let state = app.state::<AppState>();
        let store = state.qualification_sessions.lock().unwrap();
        assert!(store.active_candidate().is_none());
    }

    fn begin_with_prerequisite_and_admission(
        app: &tauri::App<tauri::test::MockRuntime>,
        candidate: &str,
    ) {
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(observation("device-one"))),
        );
        let session_handle = session_handle_for_candidate(candidate).unwrap();
        let snapshot = record_checkpoint(
            &app.state::<AppState>(),
            &session_handle,
            "clean_or_deliberately_reset_device",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        assert_eq!(snapshot.session_handle, session_handle);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::ReviewCreated(Box::new(review(
                "review-one",
                "device-one",
            ))),
        );
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionAdmitted(Box::new(
                ExecutionAdmissionObservation {
                    execution_handle: "execution-one".to_string(),
                    review: review("review-one", "device-one"),
                    device_handle: "device-one".to_string(),
                },
            )),
        );
    }

    #[test]
    fn terminal_execution_materializes_a_valid_candidate_automatically() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        begin_with_prerequisite_and_admission(&app, &candidate);
        record_checkpoint(
            &app.state::<AppState>(),
            &session_handle_for_candidate(&candidate).unwrap(),
            "device_state_verified",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: "2026-10-02T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"{\"status\":\"succeeded\"}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("valid")
        );
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("passed")
        );
        assert!(stored.promotable);
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
    }

    #[test]
    fn execution_report_observation_uses_the_authoritative_terminal_timestamp() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        begin_with_prerequisite_and_admission(&app, &candidate);
        const TERMINAL_AT: &str = "2026-09-30T19:27:12Z";
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: TERMINAL_AT.to_string(),
                    report_available: true,
                    report_bytes: Some(b"{\"status\":\"succeeded\"}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        record_checkpoint(
            &app.state::<AppState>(),
            &session_handle_for_candidate(&candidate).unwrap(),
            "device_state_verified",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();

        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload["automatedObservations"][0]["observedAt"],
            TERMINAL_AT
        );
    }

    #[test]
    fn source_change_after_admission_keeps_run_pending_until_source_is_restored() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        begin_with_prerequisite_and_admission(&app, &candidate);
        std::fs::write(
            temp.path().join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\nsteps:\n  - changed after admission\n",
        )
        .unwrap();
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: "2026-09-30T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"{\"status\":\"succeeded\"}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        let pending = record_checkpoint(
            &app.state::<AppState>(),
            &session_handle_for_candidate(&candidate).unwrap(),
            "device_state_verified",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        assert_eq!(
            pending.phase,
            QualificationSessionPhase::TerminalAwaitingEvidence
        );

        let app_state = app.state::<AppState>();
        let repository = app_state.qualification_repository.get().unwrap();
        let provisional = repository.load_candidate(&candidate).unwrap();
        assert!(provisional.payload.get("runValidity").is_none());

        std::fs::write(
            temp.path().join("authored/recipes/test.recipe.yaml"),
            b"id: test.recipe\n",
        )
        .unwrap();
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(observation("device-one"))),
        );
        let stored = repository.load_candidate(&candidate).unwrap();
        assert_eq!(stored.payload["runValidity"], "valid");
        assert_eq!(
            stored.payload["fingerprint"]["authoredContent"][0]["sha256"],
            hex::encode(Sha256::digest(b"id: test.recipe\n"))
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
    }

    #[test]
    fn source_mutation_before_review_cannot_replace_the_session_start_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();

        let recipe_path = temp.path().join("authored/recipes/test.recipe.yaml");
        std::fs::write(
            &recipe_path,
            b"id: test.recipe\nsteps:\n  - temporary change\n",
        )
        .unwrap();
        app.state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .set_source_state_for_test(QualificationSourceState {
                head: "1".repeat(40),
                tracked_worktree_clean: false,
            });
        begin_with_prerequisite_and_admission(&app, &candidate);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: "2026-09-30T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"{\"status\":\"succeeded\"}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        record_checkpoint(
            &app.state::<AppState>(),
            &session_handle_for_candidate(&candidate).unwrap(),
            "device_state_verified",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();

        let app_state = app.state::<AppState>();
        let repository = app_state.qualification_repository.get().unwrap();
        assert!(session_status(&app.state::<AppState>()).unwrap().is_some());
        assert!(repository
            .load_candidate(&candidate)
            .unwrap()
            .payload
            .get("runValidity")
            .is_none());

        std::fs::write(&recipe_path, b"id: test.recipe\n").unwrap();
        repository.set_source_state_for_test(QualificationSourceState {
            head: "1".repeat(40),
            tracked_worktree_clean: true,
        });
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(observation("device-one"))),
        );
        let stored = repository.load_candidate(&candidate).unwrap();
        assert_eq!(stored.payload["runValidity"], "valid");
        assert_eq!(
            stored.payload["fingerprint"]["authoredContent"][0]["sha256"],
            hex::encode(Sha256::digest(b"id: test.recipe\n"))
        );
    }

    #[test]
    fn terminal_with_a_missing_required_checkpoint_awaits_evidence_then_materializes() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        begin_with_prerequisite_and_admission(&app, &candidate);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: "2026-10-02T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"{}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        let awaiting = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(
            awaiting.phase,
            QualificationSessionPhase::TerminalAwaitingEvidence
        );
        assert!(awaiting.recordable);
        record_checkpoint(
            &app.state::<AppState>(),
            &awaiting.session_handle,
            "device_state_verified",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("passed")
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
    }

    #[test]
    fn conflicting_device_observation_invalidates_and_materializes_an_invalid_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        let mut drifted = observation("device-one");
        drifted.android_api = Some(36);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(drifted)),
        );
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid")
        );
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("not_observed")
        );
        let limitations = stored
            .payload
            .get("limitations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            limitations.first().and_then(Value::as_str),
            Some("The Android API level no longer matches the registered target.")
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
    }

    #[test]
    fn failing_to_materialize_an_invalid_candidate_poisons_the_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        app.state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .fail_next_finalize_for_test();
        let mut drifted = observation("device-one");
        drifted.android_api = Some(36);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(drifted)),
        );
        let state = app.state::<AppState>();
        let store = state.qualification_sessions.lock().unwrap();
        assert_eq!(store.poisoned_candidate(), Some(candidate.as_str()));
        assert!(store.active_candidate().is_none());
        drop(store);
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
    }

    #[test]
    fn session_persistence_failure_cannot_resume_valid_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let session_path = repository
            .candidate_root()
            .join(&candidate)
            .join("session.json");
        let provider = QualificationRepositoryProvider::for_test(repository);
        {
            let (_app_temp, app) = test_app(provider, true);
            begin(
                &app.state::<AppState>(),
                begin_request(&candidate, CAPTURED_AT, observation("device-one")),
            )
            .unwrap();
            std::fs::remove_file(&session_path).unwrap();
            std::fs::create_dir(&session_path).unwrap();
            let mut drifted = observation("device-one");
            drifted.android_api = Some(36);
            observe(
                &app.state::<AppState>(),
                QualificationLifecycleObservation::DeviceObserved(Box::new(drifted)),
            );
        }

        let (_app_temp, app) = clean_restart(&temp);
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert!(!stored.promotable);
    }

    #[test]
    fn terminal_report_persistence_failure_cannot_resume_valid_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let report_path = repository
            .candidate_root()
            .join(&candidate)
            .join("session-terminal-report.json");
        let provider = QualificationRepositoryProvider::for_test(repository);
        {
            let (_app_temp, app) = test_app(provider, true);
            begin(
                &app.state::<AppState>(),
                begin_request(&candidate, CAPTURED_AT, observation("device-one")),
            )
            .unwrap();
            begin_with_prerequisite_and_admission(&app, &candidate);
            std::fs::create_dir(&report_path).unwrap();
            observe(
                &app.state::<AppState>(),
                QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                    TerminalExecutionObservation {
                        execution_handle: "execution-one".to_string(),
                        status: Some("succeeded".to_string()),
                        observed_at: "2026-09-30T19:27:12Z".to_string(),
                        report_available: true,
                        report_bytes: Some(b"{\"status\":\"succeeded\"}".to_vec()),
                        authority_invalidated: false,
                    },
                )),
            );
        }

        let (_app_temp, app) = clean_restart(&temp);
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert!(!stored.promotable);
    }

    #[test]
    fn candidate_finalization_failure_cannot_materialize_valid_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        {
            let (_app_temp, app) = test_app(provider, true);
            begin(
                &app.state::<AppState>(),
                begin_request(&candidate, CAPTURED_AT, observation("device-one")),
            )
            .unwrap();
            begin_with_prerequisite_and_admission(&app, &candidate);
            observe(
                &app.state::<AppState>(),
                QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                    TerminalExecutionObservation {
                        execution_handle: "execution-one".to_string(),
                        status: Some("succeeded".to_string()),
                        observed_at: "2026-09-30T19:27:12Z".to_string(),
                        report_available: true,
                        report_bytes: Some(b"{\"status\":\"succeeded\"}".to_vec()),
                        authority_invalidated: false,
                    },
                )),
            );
            app.state::<AppState>()
                .qualification_repository
                .get()
                .unwrap()
                .fail_next_finalize_for_test();
            assert!(record_checkpoint(
                &app.state::<AppState>(),
                &session_handle_for_candidate(&candidate).unwrap(),
                "device_state_verified",
                QualificationCheckpointOutcome::Pass,
            )
            .is_err());
        }

        let (_app_temp, app) = clean_restart(&temp);
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert!(!stored.promotable);
        assert_ne!(stored.payload["runValidity"], "valid");
    }

    #[test]
    fn failed_durable_poison_preserves_unproven_process_handoff() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let poison_path = repository
            .candidate_root()
            .join(&candidate)
            .join("session-poison.json");
        let session_path = repository
            .candidate_root()
            .join(&candidate)
            .join("session.json");
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        std::fs::remove_file(&session_path).unwrap();
        std::fs::create_dir(&session_path).unwrap();
        std::fs::create_dir(&poison_path).unwrap();
        let mut drifted = observation("device-one");
        drifted.android_api = Some(36);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(drifted)),
        );

        let marker = app_temp.path().join("session-active.marker");
        app.state::<AppState>()
            .recovery
            .lock()
            .unwrap()
            .finish_process_termination()
            .unwrap();
        assert!(
            marker.is_file(),
            "failed poison must retain the active marker"
        );
        let loaded = RecoveryStore::load(app_temp.path().join("draft.json"), marker);
        assert!(!loaded.session_handoff_proven());
    }

    #[test]
    fn operator_abandonment_closes_the_attempt_as_an_invalid_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        let snapshot = begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        let abandoned = abandon(&app.state::<AppState>(), &snapshot.session_handle).unwrap();
        assert_eq!(abandoned.run_validity, RunValidity::Invalid);
        assert_eq!(abandoned.phase, QualificationSessionPhase::Closed);
        assert_eq!(
            abandoned.invalid_reason.as_deref(),
            Some("The operator abandoned this qualification attempt.")
        );
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid")
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
    }

    #[test]
    fn clean_restart_resumes_a_compatible_session_and_reassociates_on_the_first_observation() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
        }
        let (_app_temp, app) = clean_restart(&temp);
        let resumed = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(
            resumed.session_handle,
            session_handle_for_candidate(&candidate).unwrap()
        );
        assert!(!device_selection_locked(&app.state::<AppState>()).unwrap());
        assert!(app
            .state::<AppState>()
            .qualification_sessions
            .lock()
            .unwrap()
            .associated_device_handle()
            .is_none());
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(observation("device-two"))),
        );
        let associated = app
            .state::<AppState>()
            .qualification_sessions
            .lock()
            .unwrap()
            .associated_device_handle()
            .map(str::to_string);
        assert_eq!(associated.as_deref(), Some("device-two"));
        let persisted = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_session(&candidate)
            .unwrap();
        assert_eq!(persisted.run_validity, RunValidity::Valid);
        // Process-local handles never become durable authority: the stored
        // session document cannot carry the device this process associated.
        let document = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_session_json(&candidate)
            .unwrap();
        assert!(document.get("deviceHandle").is_none());
        assert!(document.get("boundReviewHandle").is_none());
        assert!(document.get("boundExecutionHandle").is_none());
    }

    #[test]
    fn clean_restart_after_review_reestablishes_the_review_from_this_process() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        let session_handle = session_handle_for_candidate(&candidate).unwrap();
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
            record_checkpoint(
                &app.state::<AppState>(),
                &session_handle,
                "clean_or_deliberately_reset_device",
                QualificationCheckpointOutcome::Pass,
            )
            .unwrap();
            observe(
                &app.state::<AppState>(),
                QualificationLifecycleObservation::ReviewCreated(Box::new(review(
                    "review-one",
                    "device-one",
                ))),
            );
        }
        let (_app_temp, app) = clean_restart(&temp);
        let resumed = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(resumed.phase, QualificationSessionPhase::ExecutionPending);
        // The review this process binds must come from a new authoritative
        // review, never from the previous process's handle.
        let document = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_session_json(&candidate)
            .unwrap();
        assert!(document.get("boundReviewHandle").is_none());
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(observation("device-two"))),
        );
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::ReviewCreated(Box::new(review(
                "review-two",
                "device-two",
            ))),
        );
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionAdmitted(Box::new(
                ExecutionAdmissionObservation {
                    execution_handle: "execution-two".to_string(),
                    review: review("review-two", "device-two"),
                    device_handle: "device-two".to_string(),
                },
            )),
        );
        let active = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(active.phase, QualificationSessionPhase::ExecutionActive);
        assert_eq!(active.run_validity, RunValidity::Valid);
    }

    #[test]
    fn clean_restart_with_an_admitted_non_terminal_execution_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
            begin_with_prerequisite_and_admission(&app, &candidate);
        }
        let (_app_temp, app) = clean_restart(&temp);
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid")
        );
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("not_observed")
        );
        let limitations = stored
            .payload
            .get("limitations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            limitations.first().and_then(Value::as_str),
            Some("A required qualification transition was missed or arrived out of order.")
        );
        // The durable session records that execution was admitted without
        // ever trusting the previous process's execution handle.
        let document = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_session_json(&candidate)
            .unwrap();
        assert_eq!(document.get("executionAdmitted"), Some(&json!(true)));
        assert!(document.get("boundExecutionHandle").is_none());
    }

    #[test]
    fn clean_restart_recovers_terminal_awaiting_evidence_from_retained_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
            begin_with_prerequisite_and_admission(&app, &candidate);
            observe(
                &app.state::<AppState>(),
                QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                    TerminalExecutionObservation {
                        execution_handle: "execution-one".to_string(),
                        status: Some("succeeded".to_string()),
                        observed_at: "2026-10-02T19:27:12Z".to_string(),
                        report_available: true,
                        report_bytes: Some(b"{\"status\":\"succeeded\"}".to_vec()),
                        authority_invalidated: false,
                    },
                )),
            );
            let awaiting = session_status(&app.state::<AppState>()).unwrap().unwrap();
            assert_eq!(
                awaiting.phase,
                QualificationSessionPhase::TerminalAwaitingEvidence
            );
        }
        let (_app_temp, app) = clean_restart(&temp);
        let resumed = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(
            resumed.phase,
            QualificationSessionPhase::TerminalAwaitingEvidence
        );
        assert!(resumed.recordable);
        record_checkpoint(
            &app.state::<AppState>(),
            &resumed.session_handle,
            "device_state_verified",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("passed")
        );
        assert_eq!(
            stored
                .payload
                .get("artifacts")
                .and_then(Value::as_array)
                .and_then(|artifacts| artifacts.first())
                .and_then(|artifact| artifact.get("id"))
                .and_then(Value::as_str),
            Some("execution-report")
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
    }

    #[test]
    fn root_change_after_terminal_retention_invalidates_an_open_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        begin_with_prerequisite_and_admission(&app, &candidate);

        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: "2026-10-02T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"{\"status\":\"succeeded\"}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        assert_eq!(
            session_status(&app.state::<AppState>())
                .unwrap()
                .unwrap()
                .phase,
            QualificationSessionPhase::TerminalAwaitingEvidence
        );

        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RootChecked {
                device_handle: "device-one".to_string(),
                root_state: RootQualificationState::Granted,
            },
        );

        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(stored.payload["runValidity"], "invalid");
    }

    #[test]
    fn offline_inventory_invalidates_the_associated_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();

        observe_device_inventory(&app.state::<AppState>(), &[]);

        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(stored.payload["runValidity"], "invalid");
    }

    #[test]
    fn changed_inventory_epoch_invalidates_even_when_the_handle_is_reused() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();

        observe_device_inventory(&app.state::<AppState>(), &[("device-one".to_string(), 2)]);

        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(stored.payload["runValidity"], "invalid");
    }

    #[test]
    fn retained_terminal_phase_overrides_a_process_local_execution_binding() {
        let mut session = QualificationSession::for_test(&[]);
        session.bound_execution_handle = Some("execution-one".to_string());
        session.terminal_execution_status = Some("succeeded".to_string());

        assert_eq!(
            session.phase(),
            QualificationSessionPhase::TerminalAwaitingEvidence
        );
    }

    #[test]
    fn partial_device_observations_do_not_reassociate_a_resumed_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
        }
        let (_app_temp, app) = clean_restart(&temp);
        let mut probe = partial_observation("device-two");
        probe.session_epoch = Some(9);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(probe)),
        );
        {
            let state = app.state::<AppState>();
            let store = state.qualification_sessions.lock().unwrap();
            assert!(store.associated_device_handle().is_none());
            assert_eq!(
                store
                    .observed_device()
                    .map(|observation| observation.device_handle.as_str()),
                Some("device-two")
            );
        }
        let resumed = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(resumed.phase, QualificationSessionPhase::ExecutionPending);
        assert_eq!(resumed.run_validity, RunValidity::Valid);
    }

    #[test]
    fn complete_hardware_observation_without_profile_does_not_reassociate() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
        }
        let (_app_temp, app) = clean_restart(&temp);
        let mut hardware_only = observation("device-two");
        hardware_only.profile_id = None;
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(hardware_only)),
        );
        assert!(app
            .state::<AppState>()
            .qualification_sessions
            .lock()
            .unwrap()
            .associated_device_handle()
            .is_none());
        assert!(session_status(&app.state::<AppState>()).unwrap().is_some());
    }

    #[test]
    fn wrong_profile_invalidates_a_restored_attempt_even_when_hardware_matches() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
        }
        let (_app_temp, app) = clean_restart(&temp);
        let mut wrong_profile = observation("device-two");
        wrong_profile.profile_id = Some("profile.changed".to_string());
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(wrong_profile)),
        );
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(stored.payload["runValidity"], "invalid");
    }

    #[test]
    fn accumulated_partial_observations_reassociate_once_compatibility_is_proven() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
        }
        let (_app_temp, app) = clean_restart(&temp);
        let mut probe = partial_observation("device-two");
        probe.session_epoch = Some(9);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(probe)),
        );
        // The passive support projection observes the facts the probe does
        // not: ABI class, Android API level, and the committed root state.
        let mut support = SelectedDeviceObservation::new("device-two");
        support.session_epoch = Some(9);
        support.profile_id = Some("profile.test".to_string());
        support.android_api = Some(35);
        support.abi_soc_class = Some("arm64".to_string());
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(support)),
        );
        let associated = app
            .state::<AppState>()
            .qualification_sessions
            .lock()
            .unwrap()
            .associated_device_handle()
            .map(str::to_string);
        assert_eq!(associated.as_deref(), Some("device-two"));
        let resumed = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(resumed.run_validity, RunValidity::Valid);
    }

    #[test]
    fn partial_conflicting_device_observation_invalidates_a_resumed_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
        }
        let (_app_temp, app) = clean_restart(&temp);
        let mut conflicting = partial_observation("device-two");
        conflicting.model = Some("Other".to_string());
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(conflicting)),
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        let limitations = stored
            .payload
            .get("limitations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            limitations.first().and_then(Value::as_str),
            Some("The device model no longer matches the registered target.")
        );
    }

    #[test]
    fn complete_conflicting_device_observation_invalidates_a_resumed_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
        }
        let (_app_temp, app) = clean_restart(&temp);
        let mut conflicting = observation("device-two");
        conflicting.android_api = Some(36);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::DeviceObserved(Box::new(conflicting)),
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        assert!(app
            .state::<AppState>()
            .qualification_sessions
            .lock()
            .unwrap()
            .associated_device_handle()
            .is_none());
    }

    #[test]
    fn unproven_shutdown_invalidates_a_persisted_attempt_on_launch() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let captured = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        {
            let provider = QualificationRepositoryProvider::for_test(repository);
            let (_app_temp, app) = test_app(provider, true);
            begin(&app.state::<AppState>(), captured).unwrap();
        }
        let restarted = test_repository(&temp);
        let provider = QualificationRepositoryProvider::for_test(restarted);
        let (_app_temp, app) = test_app(provider, false);
        crate::qualification_mode::recover_sessions_at_process_start(&app.state::<AppState>());
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid")
        );
        let limitations = stored
            .payload
            .get("limitations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            limitations.first().and_then(Value::as_str),
            Some(
                "The previous application session did not end cleanly, so this attempt cannot be trusted."
            )
        );
    }

    #[test]
    fn version_one_persisted_sessions_fail_closed_into_an_invalid_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let mut legacy = json!({
            "sessionSchemaVersion": 1,
            "sessionHandle": session_handle_for_candidate(&candidate).unwrap(),
            "candidateHandle": candidate,
            "capturedAt": CAPTURED_AT,
            "deviceHandle": "device-one",
            "targetId": "target-test",
            "target": serde_json::to_value(target()).unwrap(),
            "workflowId": "test-workflow",
            "workflowVersion": 1,
            "devicePlan": "test-plan",
            "requiredRecipes": ["test.recipe"],
            "prerequisites": [],
            "humanCheckpoints": [],
            "automatedObservations": [],
            "recordedCheckpoints": [],
            "build": build_json(),
            "runtimeContract": "real-execution-v1",
            "runValidity": "valid",
            "invalidation": null,
            "boundReviewHandle": null,
            "boundExecutionHandle": null,
            "terminalExecutionStatus": null,
            "terminalOutcome": "not_observed",
        });
        let path = repository
            .candidate_root()
            .join(&candidate)
            .join("session.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        legacy["sessionSchemaVersion"] = json!(1);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        crate::qualification_mode::recover_sessions_at_process_start(&app.state::<AppState>());
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid")
        );
        let limitations = stored
            .payload
            .get("limitations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            limitations.first().and_then(Value::as_str),
            Some(
                "The saved qualification attempt was created by an incompatible application version."
            )
        );
    }

    #[test]
    fn version_two_persisted_sessions_fail_closed_without_trusting_handles() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        // Version 2 recorded process-local handles as durable authority. Those
        // handles are meaningless in a new process, so the attempt must fail
        // closed instead of resuming with a stale device, review, or execution.
        let legacy = json!({
            "sessionSchemaVersion": 2,
            "sessionHandle": session_handle_for_candidate(&candidate).unwrap(),
            "candidateHandle": candidate,
            "capturedAt": CAPTURED_AT,
            "deviceHandle": "device-one",
            "targetId": "target-test",
            "target": serde_json::to_value(target()).unwrap(),
            "workflowId": "test-workflow",
            "workflowVersion": 1,
            "devicePlan": "test-plan",
            "requiredRecipes": ["test.recipe"],
            "prerequisites": [],
            "humanCheckpoints": [],
            "automatedObservations": [],
            "recordedCheckpoints": [],
            "build": build_json(),
            "runtimeContract": "real-execution-v1",
            "runValidity": "valid",
            "invalidation": null,
            "boundReviewHandle": "review-one",
            "boundExecutionHandle": "execution-one",
            "terminalExecutionStatus": null,
            "terminalOutcome": "not_observed",
            "awaitingEvidence": false,
            "closed": false,
        });
        let path = repository
            .candidate_root()
            .join(&candidate)
            .join("session.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        crate::qualification_mode::recover_sessions_at_process_start(&app.state::<AppState>());
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
        {
            let state = app.state::<AppState>();
            let store = state.qualification_sessions.lock().unwrap();
            assert!(store.active_candidate().is_none());
            assert!(store.associated_device_handle().is_none());
        }
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid")
        );
        let limitations = stored
            .payload
            .get("limitations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            limitations.first().and_then(Value::as_str),
            Some(
                "The saved qualification attempt was created by an incompatible application version."
            )
        );
    }

    #[test]
    fn terminal_observation_for_an_unbound_execution_invalidates_the_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: "2026-10-02T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"{}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid")
        );
    }

    #[test]
    fn admission_without_a_passed_prerequisite_invalidates_instead_of_binding() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        let state = app.state::<AppState>();
        begin(
            &state,
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        observe(
            &state,
            QualificationLifecycleObservation::DeviceObserved(Box::new(observation("device-one"))),
        );
        observe(
            &state,
            QualificationLifecycleObservation::ReviewCreated(Box::new(review(
                "review-one",
                "device-one",
            ))),
        );
        observe(
            &state,
            QualificationLifecycleObservation::RealExecutionAdmitted(Box::new(
                ExecutionAdmissionObservation {
                    execution_handle: "execution-one".to_string(),
                    review: review("review-one", "device-one"),
                    device_handle: "device-one".to_string(),
                },
            )),
        );

        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid"),
            "an attempt that skipped a required prerequisite cannot produce valid evidence"
        );
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("not_observed")
        );
        assert!(session_status(&state).unwrap().is_none());

        // The attempt never bound the execution, so a later terminal transition
        // cannot repair the invalidated evidence.
        observe(
            &state,
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: "2026-10-02T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"report".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        let stored_after = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored_after
                .payload
                .get("runValidity")
                .and_then(Value::as_str),
            Some("invalid")
        );
    }

    #[test]
    fn a_failed_prerequisite_invalidates_the_attempt_before_admission() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        let state = app.state::<AppState>();
        begin(
            &state,
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        observe(
            &state,
            QualificationLifecycleObservation::DeviceObserved(Box::new(observation("device-one"))),
        );
        record_checkpoint(
            &state,
            &session_handle_for_candidate(&candidate).unwrap(),
            "clean_or_deliberately_reset_device",
            QualificationCheckpointOutcome::Fail,
        )
        .unwrap();

        let stored = state
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored.payload.get("runValidity").and_then(Value::as_str),
            Some("invalid")
        );
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("not_observed")
        );
        assert!(session_status(&state).unwrap().is_none());
    }

    #[test]
    fn phase_progresses_from_pending_to_active_to_closed() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        begin_with_prerequisite_and_admission(&app, &candidate);
        record_checkpoint(
            &app.state::<AppState>(),
            &session_handle_for_candidate(&candidate).unwrap(),
            "device_state_verified",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        let active = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(active.phase, QualificationSessionPhase::ExecutionActive);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("failed".to_string()),
                    observed_at: "2026-10-02T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"{}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("failed")
        );
        assert!(session_status(&app.state::<AppState>()).unwrap().is_none());
    }

    #[test]
    fn simulation_paths_never_touch_the_active_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        // Simulated execution is ordinary product behavior with no observation
        // seam, so the attempt remains untouched and still recordable.
        let current = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(current.phase, QualificationSessionPhase::ExecutionPending);
        assert!(current.recordable);
    }

    #[test]
    fn repository_paths_are_resolved_from_the_trusted_root() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        assert!(repository
            .repo_root()
            .ends_with(temp.path().file_name().unwrap()));
    }

    #[test]
    fn unused_runner_reports_a_clear_error() {
        let runner = UnusedToolRunner;
        assert!(runner.run(Path::new("/tmp"), &[]).is_err());
    }

    #[test]
    fn session_store_forgets_poison_and_association_state() {
        let mut store = QualificationSessionStore::default();
        assert_eq!(store.poisoned_candidate(), None);
        store.poison("qualification-candidate-one".to_string());
        assert!(store.is_poisoned("qualification-candidate-one"));
        store.forget("qualification-candidate-one");
        assert_eq!(store.poisoned_candidate(), None);
        store.set_active("qualification-candidate-two".to_string());
        store.associate("device-one".to_string(), Some(3));
        assert_eq!(store.associated_device_handle(), Some("device-one"));
        assert_eq!(store.associated_device_session_epoch(), Some(3));
        store.forget("qualification-candidate-two");
        assert_eq!(store.associated_device_handle(), None);
        assert_eq!(store.associated_device_session_epoch(), None);
        store.reset();
        assert!(store.active_candidate().is_none());
    }

    #[test]
    fn recorded_checkpoint_evidence_is_immutable() {
        let mut session = QualificationSession::for_test(&["device_state_verified"]);
        session
            .record_checkpoint_at(
                "device_state_verified",
                QualificationCheckpointOutcome::Pass,
                "2026-08-23T12:00:00Z",
            )
            .unwrap();
        let error = session
            .record_checkpoint_at(
                "device_state_verified",
                QualificationCheckpointOutcome::Fail,
                "2026-08-23T12:30:00Z",
            )
            .expect_err("a recorded checkpoint must reject a second submission");
        assert!(error.contains("already recorded"));
        let recorded = session.recorded_checkpoints();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].outcome, QualificationCheckpointOutcome::Pass);
        assert_eq!(recorded[0].observed_at, "2026-08-23T12:00:00Z");
    }

    #[test]
    fn duplicate_checkpoint_submission_is_rejected_through_the_session_entry_point() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        begin(
            &app.state::<AppState>(),
            begin_request(&candidate, CAPTURED_AT, observation("device-one")),
        )
        .unwrap();
        let session_handle = session_handle_for_candidate(&candidate).unwrap();
        record_checkpoint(
            &app.state::<AppState>(),
            &session_handle,
            "clean_or_deliberately_reset_device",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        let error = record_checkpoint(
            &app.state::<AppState>(),
            &session_handle,
            "clean_or_deliberately_reset_device",
            QualificationCheckpointOutcome::Fail,
        )
        .expect_err("the session entry point must reject a duplicate checkpoint");
        let value: Value = serde_json::from_str(&error).expect("the error should be JSON");
        assert_eq!(value["code"], "qualification_checkpoint_invalid");
        let snapshot = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(snapshot.recorded_checkpoints.len(), 1);
        assert_eq!(
            snapshot.recorded_checkpoints[0].outcome,
            QualificationCheckpointOutcome::Pass
        );
    }

    #[test]
    fn terminal_awaiting_evidence_accepts_only_unrecorded_required_checkpoints() {
        let temp = tempfile::tempdir().unwrap();
        let repository = test_repository(&temp);
        let candidate = create_run_candidate(&repository, CAPTURED_AT);
        let provider = QualificationRepositoryProvider::for_test(repository);
        let (_app_temp, app) = test_app(provider, true);
        let mut request = begin_request(&candidate, CAPTURED_AT, observation("device-one"));
        request
            .workflow
            .human_checkpoints
            .push(QualificationWorkflowCheckpoint {
                id: "optional_review".to_string(),
                instruction: "Optional review".to_string(),
                fact: "optional".to_string(),
                allowed_outcomes: vec![QualificationCheckpointOutcome::Pass],
                required: false,
            });
        begin(&app.state::<AppState>(), request).unwrap();
        begin_with_prerequisite_and_admission(&app, &candidate);
        observe(
            &app.state::<AppState>(),
            QualificationLifecycleObservation::RealExecutionTerminal(Box::new(
                TerminalExecutionObservation {
                    execution_handle: "execution-one".to_string(),
                    status: Some("succeeded".to_string()),
                    observed_at: "2026-10-02T19:27:12Z".to_string(),
                    report_available: true,
                    report_bytes: Some(b"{}".to_vec()),
                    authority_invalidated: false,
                },
            )),
        );
        let awaiting = session_status(&app.state::<AppState>()).unwrap().unwrap();
        assert_eq!(
            awaiting.phase,
            QualificationSessionPhase::TerminalAwaitingEvidence
        );
        for (checkpoint, outcome) in [
            (
                "clean_or_deliberately_reset_device",
                QualificationCheckpointOutcome::Pass,
            ),
            ("optional_review", QualificationCheckpointOutcome::Pass),
        ] {
            let error = record_checkpoint(
                &app.state::<AppState>(),
                &awaiting.session_handle,
                checkpoint,
                outcome,
            )
            .expect_err("awaiting evidence must only accept unrecorded required checkpoints");
            let value: Value = serde_json::from_str(&error).expect("the error should be JSON");
            assert_eq!(value["code"], "qualification_checkpoint_invalid");
        }
        record_checkpoint(
            &app.state::<AppState>(),
            &awaiting.session_handle,
            "device_state_verified",
            QualificationCheckpointOutcome::Pass,
        )
        .unwrap();
        let stored = app
            .state::<AppState>()
            .qualification_repository
            .get()
            .unwrap()
            .load_candidate(&candidate)
            .unwrap();
        assert_eq!(
            stored
                .payload
                .get("qualificationOutcome")
                .and_then(Value::as_str),
            Some("passed")
        );
    }

    #[test]
    fn arc_dyn_tool_runner_marker_keeps_imports_used() {
        let runner: Arc<dyn QualificationToolRunner + Send + Sync> = Arc::new(UnusedToolRunner);
        assert!(runner.run(PathBuf::from("/tmp").as_path(), &[]).is_err());
    }
}
