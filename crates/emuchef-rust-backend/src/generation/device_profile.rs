//! Typed device-profile draft generation from safe standard-probe facts.

use serde::{Deserialize, Serialize};

use crate::authored_models::{
    emit_device_profile_yaml, validate_device_profile, AndroidVersionRange,
    AuthoredModelDiagnostic, DeviceCapabilityDefaults, DeviceMatchCriteria, DeviceProfileV1,
    OrderedValueMap, DEVICE_PROFILE_KIND, SCHEMA_VERSION_V1,
};

use super::identifiers::normalize_identifier_component;

/// Device facts safe to expose to authored-generation clients.
///
/// The exact ADB serial is deliberately absent. Unknown fields are rejected so
/// callers cannot accidentally tunnel trusted transport identifiers through the
/// otherwise safe generation request.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SafeDetectedDeviceFacts {
    pub manufacturer: Option<String>,
    pub brand: Option<String>,
    pub model: Option<String>,
    pub product: Option<String>,
    pub device: Option<String>,
    pub board: Option<String>,
    pub hardware: Option<String>,
    pub abis: Vec<String>,
    pub android_version: Option<i64>,
    pub android_api_level: Option<i64>,
}

/// Input for initial generation or revalidation of an author-edited profile.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DeviceProfileDraftRequest {
    pub facts: SafeDetectedDeviceFacts,
    #[serde(default)]
    pub profile: Option<DeviceProfileV1>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EvidenceState {
    Verified,
    Derived,
    Suggested,
    Missing,
}

/// Provenance for one proposed profile value.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FieldEvidence {
    field: String,
    state: EvidenceState,
    source: String,
    edited_from_proposal: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DiagnosticSeverity {
    Error,
    Warning,
}

/// Stable diagnostic returned while reviewing a draft.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GenerationDiagnostic {
    severity: DiagnosticSeverity,
    code: String,
    message: String,
    field: String,
}

/// Safe proposed destination information derived from a valid profile id.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProposedDestination {
    file_name: Option<String>,
    relative_path: Option<String>,
}

/// Reviewable, side-effect-free device-profile generation result.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceProfileDraft {
    profile: DeviceProfileV1,
    canonical_yaml: Option<String>,
    evidence: Vec<FieldEvidence>,
    diagnostics: Vec<GenerationDiagnostic>,
    destination: ProposedDestination,
}

/// Generate or revalidate a typed device profile without touching the filesystem.
pub(crate) fn generate_device_profile_draft(
    request: DeviceProfileDraftRequest,
) -> DeviceProfileDraft {
    let proposed = proposed_profile(&request.facts);
    let profile = request.profile.unwrap_or_else(|| proposed.clone());
    let evidence = profile_evidence(&request.facts, &proposed, &profile);
    let mut diagnostics = missing_fact_diagnostics(&request.facts);
    let validation = validate_device_profile(&profile);
    diagnostics.extend(validation.iter().map(validation_diagnostic));
    let canonical_yaml = if validation.is_empty() {
        emit_device_profile_yaml(&profile).ok()
    } else {
        None
    };
    let destination = if validation.iter().any(|diagnostic| diagnostic.field == "id") {
        ProposedDestination {
            file_name: None,
            relative_path: None,
        }
    } else {
        let file_name = format!("{}.yaml", profile.id);
        ProposedDestination {
            relative_path: Some(format!("device_profiles/{file_name}")),
            file_name: Some(file_name),
        }
    };

    DeviceProfileDraft {
        profile,
        canonical_yaml,
        evidence,
        diagnostics,
        destination,
    }
}

fn proposed_profile(facts: &SafeDetectedDeviceFacts) -> DeviceProfileV1 {
    let manufacturer = present(facts.manufacturer.as_deref());
    let brand = present(facts.brand.as_deref());
    let model = present(facts.model.as_deref());
    let id = match (manufacturer, model) {
        (Some(manufacturer), Some(model)) => {
            let manufacturer = normalize_identifier_component(manufacturer);
            let model = normalize_identifier_component(model);
            if manufacturer.is_empty() || model.is_empty() {
                String::new()
            } else {
                format!("{manufacturer}.{model}")
            }
        }
        _ => String::new(),
    };
    let name = match (manufacturer, model) {
        (Some(manufacturer), Some(model)) => format!("{manufacturer} {model}"),
        _ => String::new(),
    };
    DeviceProfileV1 {
        schema_version: SCHEMA_VERSION_V1,
        kind: DEVICE_PROFILE_KIND.to_string(),
        id,
        name,
        description: None,
        match_criteria: DeviceMatchCriteria {
            manufacturer_contains: manufacturer.map(ToString::to_string).into_iter().collect(),
            brand_contains: brand.map(ToString::to_string).into_iter().collect(),
            model_patterns: model
                .map(|value| format!("^{}$", regex::escape(value)))
                .into_iter()
                .collect(),
            android_version: facts.android_version.map(|minimum| AndroidVersionRange {
                min: Some(minimum),
                max: None,
            }),
        },
        capability_defaults: DeviceCapabilityDefaults {
            adb_available: true,
            apk_install: true,
            shared_storage_write: true,
            app_launch: true,
            shell_command: true,
            package_remove_for_user: false,
            root_shell: false,
            app_data_write: false,
        },
        device_tags: Vec::new(),
        metadata: OrderedValueMap::new(),
    }
}

fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn missing_fact_diagnostics(facts: &SafeDetectedDeviceFacts) -> Vec<GenerationDiagnostic> {
    let mut diagnostics = Vec::new();
    for (field, value, message) in [
        (
            "facts.manufacturer",
            facts.manufacturer.as_deref(),
            "Manufacturer was not reported; enter profile identity and matching values manually.",
        ),
        (
            "facts.model",
            facts.model.as_deref(),
            "Model was not reported; enter profile identity and matching values manually.",
        ),
    ] {
        if present(value).is_none() {
            diagnostics.push(GenerationDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "device_profile_fact_missing".to_string(),
                message: message.to_string(),
                field: field.to_string(),
            });
        }
    }
    diagnostics
}

fn validation_diagnostic(diagnostic: &AuthoredModelDiagnostic) -> GenerationDiagnostic {
    GenerationDiagnostic {
        severity: DiagnosticSeverity::Error,
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        field: diagnostic.field.clone(),
    }
}

fn profile_evidence(
    facts: &SafeDetectedDeviceFacts,
    proposed: &DeviceProfileV1,
    current: &DeviceProfileV1,
) -> Vec<FieldEvidence> {
    let mut evidence = Vec::new();
    let fact_fields = [
        ("facts.manufacturer", facts.manufacturer.as_deref()),
        ("facts.brand", facts.brand.as_deref()),
        ("facts.model", facts.model.as_deref()),
        ("facts.product", facts.product.as_deref()),
        ("facts.device", facts.device.as_deref()),
        ("facts.board", facts.board.as_deref()),
        ("facts.hardware", facts.hardware.as_deref()),
    ];
    for (field, value) in fact_fields {
        evidence.push(evidence_entry(
            field,
            if present(value).is_some() {
                EvidenceState::Verified
            } else {
                EvidenceState::Missing
            },
            "standard_adb_probe",
            false,
        ));
    }
    evidence.push(evidence_entry(
        "facts.abis",
        if facts.abis.is_empty() {
            EvidenceState::Missing
        } else {
            EvidenceState::Verified
        },
        "standard_adb_probe",
        false,
    ));
    for (field, present_value) in [
        ("facts.androidVersion", facts.android_version.is_some()),
        ("facts.androidApiLevel", facts.android_api_level.is_some()),
    ] {
        evidence.push(evidence_entry(
            field,
            if present_value {
                EvidenceState::Verified
            } else {
                EvidenceState::Missing
            },
            "standard_adb_probe",
            false,
        ));
    }

    let identity_available = present(facts.manufacturer.as_deref()).is_some()
        && present(facts.model.as_deref()).is_some()
        && !proposed.id.is_empty();
    let derived = [
        (
            "id",
            current.id != proposed.id,
            evidence_if(identity_available, EvidenceState::Derived),
        ),
        (
            "name",
            current.name != proposed.name,
            evidence_if(identity_available, EvidenceState::Derived),
        ),
        (
            "match.manufacturer_contains",
            current.match_criteria.manufacturer_contains
                != proposed.match_criteria.manufacturer_contains,
            evidence_if(
                present(facts.manufacturer.as_deref()).is_some(),
                EvidenceState::Derived,
            ),
        ),
        (
            "match.brand_contains",
            current.match_criteria.brand_contains != proposed.match_criteria.brand_contains,
            evidence_if(
                present(facts.brand.as_deref()).is_some(),
                EvidenceState::Derived,
            ),
        ),
        (
            "match.model_patterns",
            current.match_criteria.model_patterns != proposed.match_criteria.model_patterns,
            evidence_if(
                present(facts.model.as_deref()).is_some(),
                EvidenceState::Derived,
            ),
        ),
        (
            "match.android_version",
            current.match_criteria.android_version != proposed.match_criteria.android_version,
            evidence_if(facts.android_version.is_some(), EvidenceState::Derived),
        ),
    ];
    for (field, edited, state) in derived {
        evidence.push(evidence_entry(
            field,
            state,
            if state == EvidenceState::Missing {
                "no_standard_probe_source"
            } else {
                "standard_adb_probe"
            },
            edited,
        ));
    }
    evidence.push(evidence_entry(
        "description",
        EvidenceState::Missing,
        "no_standard_probe_source",
        current.description != proposed.description,
    ));
    evidence.push(evidence_entry(
        "device_tags",
        EvidenceState::Missing,
        "no_standard_probe_source",
        current.device_tags != proposed.device_tags,
    ));
    evidence.push(evidence_entry(
        "metadata",
        EvidenceState::Missing,
        "no_standard_probe_source",
        current.metadata != proposed.metadata,
    ));

    for (field, edited, state) in [
        (
            "capability_defaults.adb_available",
            current.capability_defaults.adb_available != proposed.capability_defaults.adb_available,
            EvidenceState::Verified,
        ),
        (
            "capability_defaults.shell_command",
            current.capability_defaults.shell_command != proposed.capability_defaults.shell_command,
            EvidenceState::Verified,
        ),
        (
            "capability_defaults.apk_install",
            current.capability_defaults.apk_install != proposed.capability_defaults.apk_install,
            EvidenceState::Suggested,
        ),
        (
            "capability_defaults.shared_storage_write",
            current.capability_defaults.shared_storage_write
                != proposed.capability_defaults.shared_storage_write,
            EvidenceState::Suggested,
        ),
        (
            "capability_defaults.app_launch",
            current.capability_defaults.app_launch != proposed.capability_defaults.app_launch,
            EvidenceState::Suggested,
        ),
        (
            "capability_defaults.package_remove_for_user",
            current.capability_defaults.package_remove_for_user
                != proposed.capability_defaults.package_remove_for_user,
            EvidenceState::Suggested,
        ),
        (
            "capability_defaults.root_shell",
            current.capability_defaults.root_shell != proposed.capability_defaults.root_shell,
            EvidenceState::Suggested,
        ),
        (
            "capability_defaults.app_data_write",
            current.capability_defaults.app_data_write
                != proposed.capability_defaults.app_data_write,
            EvidenceState::Suggested,
        ),
    ] {
        evidence.push(evidence_entry(
            field,
            state,
            if state == EvidenceState::Verified {
                "successful_standard_adb_probe"
            } else {
                "conservative_default"
            },
            edited,
        ));
    }
    evidence
}

fn evidence_if(available: bool, available_state: EvidenceState) -> EvidenceState {
    if available {
        available_state
    } else {
        EvidenceState::Missing
    }
}

fn evidence_entry(
    field: &str,
    state: EvidenceState,
    source: &str,
    edited_from_proposal: bool,
) -> FieldEvidence {
    FieldEvidence {
        field: field.to_string(),
        state,
        source: source.to_string(),
        edited_from_proposal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn complete_facts() -> SafeDetectedDeviceFacts {
        SafeDetectedDeviceFacts {
            manufacturer: Some("AYANEO".to_string()),
            brand: Some("AYANEO".to_string()),
            model: Some("Pocket S Mini (2026)+".to_string()),
            product: Some("pocket_s_mini".to_string()),
            device: Some("pocket_s_mini".to_string()),
            board: Some("kalama".to_string()),
            hardware: Some("qcom".to_string()),
            abis: vec!["arm64-v8a".to_string(), "armeabi-v7a".to_string()],
            android_version: Some(13),
            android_api_level: Some(33),
        }
    }

    #[test]
    fn complete_facts_generate_conservative_canonical_profile() {
        let draft = generate_device_profile_draft(DeviceProfileDraftRequest {
            facts: complete_facts(),
            profile: None,
        });
        assert_eq!(draft.profile.schema_version, 1);
        assert_eq!(draft.profile.kind, "device_profile");
        assert_eq!(draft.profile.id, "ayaneo.pocket_s_mini_2026");
        assert_eq!(
            draft.profile.match_criteria.model_patterns,
            vec![r"^Pocket S Mini \(2026\)\+$"]
        );
        assert_eq!(
            draft.profile.match_criteria.android_version,
            Some(AndroidVersionRange {
                min: Some(13),
                max: None
            })
        );
        assert!(draft.profile.capability_defaults.adb_available);
        assert!(draft.profile.capability_defaults.apk_install);
        assert!(draft.profile.capability_defaults.shared_storage_write);
        assert!(draft.profile.capability_defaults.app_launch);
        assert!(draft.profile.capability_defaults.shell_command);
        assert!(!draft.profile.capability_defaults.package_remove_for_user);
        assert!(!draft.profile.capability_defaults.root_shell);
        assert!(!draft.profile.capability_defaults.app_data_write);
        assert!(draft.canonical_yaml.is_some());
        assert_eq!(
            draft.destination.relative_path.as_deref(),
            Some("device_profiles/ayaneo.pocket_s_mini_2026.yaml")
        );
        assert!(!serde_json::to_string(&draft).unwrap().contains("serial"));
    }

    #[test]
    fn missing_identity_facts_remain_editable_and_invalid_without_panicking() {
        let draft = generate_device_profile_draft(DeviceProfileDraftRequest {
            facts: SafeDetectedDeviceFacts::default(),
            profile: None,
        });
        assert!(draft.profile.id.is_empty());
        assert!(draft.profile.name.is_empty());
        assert!(draft.canonical_yaml.is_none());
        assert!(draft
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "device_profile_fact_missing"));
        assert!(draft.evidence.iter().any(|evidence| {
            evidence.field == "id"
                && evidence.state == EvidenceState::Missing
                && !evidence.edited_from_proposal
        }));
        assert!(draft.destination.file_name.is_none());
    }

    #[test]
    fn edited_fields_retain_original_evidence_and_mark_override() {
        let facts = complete_facts();
        let mut profile = proposed_profile(&facts);
        profile.capability_defaults.root_shell = true;
        profile.name = "Author name".to_string();
        let draft = generate_device_profile_draft(DeviceProfileDraftRequest {
            facts,
            profile: Some(profile),
        });
        let evidence = serde_json::to_value(&draft.evidence).unwrap();
        assert!(evidence.as_array().unwrap().iter().any(|entry| {
            entry["field"] == "name"
                && entry["state"] == "derived"
                && entry["editedFromProposal"] == true
        }));
        assert!(evidence.as_array().unwrap().iter().any(|entry| {
            entry["field"] == "capability_defaults.root_shell"
                && entry["state"] == "suggested"
                && entry["editedFromProposal"] == true
        }));
    }

    #[test]
    fn safe_fact_shape_rejects_serials_and_unknown_fields() {
        let result = serde_json::from_value::<DeviceProfileDraftRequest>(json!({
            "facts": {"manufacturer": "AYANEO", "serial": "SECRET"}
        }));
        assert!(result.is_err());
    }

    #[test]
    fn identifier_normalization_collapses_and_trims_separators() {
        assert_eq!(
            normalize_identifier_component("  Pocket---S / Mini  "),
            "pocket_s_mini"
        );
        assert_eq!(normalize_identifier_component("Élite"), "lite");
    }
}
