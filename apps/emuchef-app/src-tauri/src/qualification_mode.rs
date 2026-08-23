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
    QualificationOperation, RepositoryQualificationDescription, StoredQualificationCandidate,
};

/// The operator outcomes accepted by a workflow-declared checkpoint.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationCheckpointOutcome {
    Pass,
    Fail,
    UnableToVerify,
}

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

/// The only input accepted by target-registration capture.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateQualificationTargetCandidateRequest {
    pub(crate) device_handle: String,
    pub(crate) device_plan: String,
    pub(crate) connection_type: QualificationConnectionType,
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
    fn current() -> Self {
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
        _ => "The qualification operation could not be completed.",
    };
    safe_error(code, message)
}

#[tauri::command]
pub fn get_device_qualification_mode_status(
    state: State<'_, AppState>,
) -> Result<QualificationModeStatus, String> {
    let mode = QualificationModeState::current();
    if !mode.enabled {
        return Ok(disabled_mode_status());
    }

    let description = state
        .qualification_repository
        .describe()
        .map_err(|_| safe_qualification_error("qualification_repository_unavailable"))?;
    let workflows = workflows_from_description(&description)?;
    let targets = targets_from_description(&description)?;
    let candidates = state
        .qualification_repository
        .list_candidates()
        .map_err(|_| safe_qualification_error("qualification_repository_unavailable"))?
        .into_iter()
        .map(candidate_summary_from_repository)
        .collect::<Result<Vec<_>, _>>()?;
    let recordable = mode
        .build
        .as_ref()
        .is_some_and(|embedded| embedded == &description.build);
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
        message: None,
        build: None,
        runtime_contract: None,
        workflows: Vec::new(),
        targets: Vec::new(),
        resumable_candidates: Vec::new(),
    }
}

#[tauri::command]
pub fn create_qualification_target_candidate(
    request: CreateQualificationTargetCandidateRequest,
    state: State<'_, AppState>,
) -> Result<QualificationTargetCandidatePreview, String> {
    let mode = QualificationModeState::current();
    let build = require_recordable_mode(&mode)?.clone();
    let payload = capture_target_registration_payload(&state, &request, &build)?;
    let handle = state
        .qualification_repository
        .create_candidate(CandidateKind::TargetRegistration, &payload, None)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    let candidate = state
        .qualification_repository
        .load_candidate(&handle)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
    target_candidate_preview(&candidate)
}

#[tauri::command]
pub fn register_qualification_target(
    candidate_handle: String,
    state: State<'_, AppState>,
) -> Result<QualificationTargetRegistrationResult, String> {
    let mode = QualificationModeState::current();
    let _ = require_recordable_mode(&mode)?;
    let result = state
        .qualification_repository
        .register_target(&candidate_handle)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))?;
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

#[tauri::command]
pub fn discard_qualification_candidate(
    candidate_handle: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mode = QualificationModeState::current();
    let _ = require_recordable_mode(&mode)?;
    state
        .qualification_repository
        .discard_candidate(&candidate_handle)
        .map_err(|_| safe_qualification_error("qualification_candidate_invalid"))
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
    let (facts, _) = probe_device_facts(&request.device_handle, state)?;
    let matched = match_device_observation(&request.device_handle, state)?;
    let profile_id = matched_profile_id(&matched, &request.device_plan).ok_or_else(|| {
        safe_error(
            "device_plan_unmatched",
            "The selected device plan is not a trusted match for this device.",
        )
    })?;
    let current = refresh_current_qualification(state, Some(&request.device_handle))?;
    let snapshot = current.snapshot;
    if snapshot.state != DeviceQualificationState::Supported
        || snapshot.device_identity.is_none()
        || snapshot.android_api_level.is_none()
        || snapshot.abi_class.is_none()
    {
        return Err(safe_qualification_error("qualification_target_unverified"));
    }
    let root = check_device_root_observation(&request.device_handle, state)?;
    if root.device_identity != request.device_handle {
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
    let target = QualificationTargetCandidateTarget {
        profile_id: observed_fact(profile_id),
        manufacturer: observed_fact(manufacturer),
        model: observed_fact(model),
        android_version: observed_fact(android_version),
        android_api: observed_fact(android_api),
        abi_soc_class: observed_fact(abi_soc_class),
        root_state: QualificationFactPreview {
            value: root_state,
            source: QualificationFactSource::ExplicitRootCheck,
        },
        connection_type: QualificationFactPreview {
            value: request.connection_type,
            source: QualificationFactSource::OperatorAttestation,
        },
        firmware_build: observed_fact(firmware_build),
        capabilities: capabilities_from_snapshot(&snapshot),
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
        Value::Number(value) => Ok(value.to_string()),
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
    use crate::device_qualification::CapabilityAvailabilityDto;
    use serde_json::json;

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
        assert!(status.message.is_none());
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
}
