//! Analyzer-neutral APK inspection contract.
//!
//! Trusted hosts parse configured external-tool output and send only these safe
//! facts. Native paths, tool paths, command output, and credentials are not part
//! of the protocol shape.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApkAnalyzerKind {
    Apkanalyzer,
    Aapt2,
}

/// Safe authoritative facts produced by a configured APK analyzer adapter.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ApkInspectionFacts {
    pub package_name: Option<String>,
    pub application_label: Option<String>,
    pub version_code: Option<String>,
    pub version_name: Option<String>,
    pub min_sdk: Option<i64>,
    pub target_sdk: Option<i64>,
    pub abis: Vec<String>,
    pub launcher_activities: Vec<String>,
    pub requested_permissions: Vec<String>,
    pub debuggable: Option<bool>,
    pub split: Option<bool>,
    pub base: Option<bool>,
    pub certificate_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ApkInspectionRequest {
    pub analyzer: ApkAnalyzerKind,
    pub facts: ApkInspectionFacts,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum InspectionSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct InspectionDiagnostic {
    severity: InspectionSeverity,
    code: String,
    message: String,
    field: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EvidenceState {
    Verified,
    Missing,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FactEvidence {
    field: String,
    state: EvidenceState,
    source: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApkInspectionResult {
    analyzer: ApkAnalyzerKind,
    facts: ApkInspectionFacts,
    evidence: Vec<FactEvidence>,
    diagnostics: Vec<InspectionDiagnostic>,
    blocking: bool,
}

/// Normalize safe analyzer facts and classify unsupported or missing evidence.
pub(crate) fn inspect_apk_facts(request: ApkInspectionRequest) -> ApkInspectionResult {
    let mut facts = request.facts;
    facts.package_name = present(facts.package_name);
    facts.application_label = present(facts.application_label);
    facts.version_code = present(facts.version_code);
    facts.version_name = present(facts.version_name);
    facts.certificate_sha256 = present(facts.certificate_sha256)
        .map(|value| value.replace(':', "").to_ascii_uppercase())
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    normalize_list(&mut facts.abis);
    normalize_list(&mut facts.launcher_activities);
    normalize_list(&mut facts.requested_permissions);

    let source = match request.analyzer {
        ApkAnalyzerKind::Apkanalyzer => "apkanalyzer",
        ApkAnalyzerKind::Aapt2 => "aapt2",
    };
    let evidence = vec![
        evidence("packageName", facts.package_name.is_some(), source),
        evidence(
            "applicationLabel",
            facts.application_label.is_some(),
            source,
        ),
        evidence("versionCode", facts.version_code.is_some(), source),
        evidence("versionName", facts.version_name.is_some(), source),
        evidence("minSdk", facts.min_sdk.is_some(), source),
        evidence("targetSdk", facts.target_sdk.is_some(), source),
        evidence("abis", !facts.abis.is_empty(), source),
        evidence(
            "launcherActivities",
            !facts.launcher_activities.is_empty(),
            source,
        ),
        evidence(
            "requestedPermissions",
            !facts.requested_permissions.is_empty(),
            source,
        ),
        evidence("debuggable", facts.debuggable.is_some(), source),
        evidence("split", facts.split.is_some(), source),
        evidence("base", facts.base.is_some(), source),
        evidence(
            "certificateSha256",
            facts.certificate_sha256.is_some(),
            source,
        ),
    ];

    let mut diagnostics = Vec::new();
    if facts.package_name.is_none() {
        diagnostics.push(error(
            "apk_package_missing",
            "The APK analyzer did not report a valid package name.",
            "packageName",
        ));
    }
    if facts.split == Some(true) || facts.base == Some(false) {
        diagnostics.push(error(
            "apk_split_unsupported",
            "Split and non-base APKs are not supported by the local APK generator.",
            "split",
        ));
    }
    for (field, available, message) in [
        ("applicationLabel", facts.application_label.is_some(), "The analyzer did not resolve an application label."),
        ("versionCode", facts.version_code.is_some(), "The analyzer did not report a version code."),
        ("versionName", facts.version_name.is_some(), "The analyzer did not report a version name."),
        ("minSdk", facts.min_sdk.is_some(), "The analyzer did not report a minimum SDK."),
        ("targetSdk", facts.target_sdk.is_some(), "The analyzer did not report a target SDK."),
        ("abis", !facts.abis.is_empty(), "The analyzer did not report native ABIs."),
        ("launcherActivities", !facts.launcher_activities.is_empty(), "No verified launcher activity was reported; launch-once generation is unavailable."),
        ("debuggable", facts.debuggable.is_some(), "The analyzer did not report debuggable status."),
        ("certificateSha256", facts.certificate_sha256.is_some(), "The configured analyzer does not expose a signing-certificate SHA-256 fingerprint for this APK."),
    ] {
        if !available {
            diagnostics.push(warning("apk_fact_missing", message, field));
        }
    }
    diagnostics.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.field.cmp(&right.field))
    });
    let blocking = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == InspectionSeverity::Error);

    ApkInspectionResult {
        analyzer: request.analyzer,
        facts,
        evidence,
        diagnostics,
        blocking,
    }
}

fn present(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_list(values: &mut Vec<String>) {
    let ordered = values
        .drain(..)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    values.extend(ordered);
}

fn evidence(field: &str, available: bool, source: &str) -> FactEvidence {
    FactEvidence {
        field: field.to_string(),
        state: if available {
            EvidenceState::Verified
        } else {
            EvidenceState::Missing
        },
        source: source.to_string(),
    }
}

fn error(code: &str, message: &str, field: &str) -> InspectionDiagnostic {
    diagnostic(InspectionSeverity::Error, code, message, field)
}

fn warning(code: &str, message: &str, field: &str) -> InspectionDiagnostic {
    diagnostic(InspectionSeverity::Warning, code, message, field)
}

fn diagnostic(
    severity: InspectionSeverity,
    code: &str,
    message: &str,
    field: &str,
) -> InspectionDiagnostic {
    InspectionDiagnostic {
        severity,
        code: code.to_string(),
        message: message.to_string(),
        field: field.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_normalizes_lists_and_certificate_and_preserves_missing_facts() {
        let result = inspect_apk_facts(ApkInspectionRequest {
            analyzer: ApkAnalyzerKind::Apkanalyzer,
            facts: ApkInspectionFacts {
                package_name: Some(" com.example.app ".to_string()),
                abis: vec!["arm64-v8a".to_string(), "arm64-v8a".to_string()],
                certificate_sha256: Some("AA:BB".to_string()),
                split: Some(false),
                base: Some(true),
                ..ApkInspectionFacts::default()
            },
        });
        assert!(!result.blocking);
        assert_eq!(
            result.facts.package_name.as_deref(),
            Some("com.example.app")
        );
        assert_eq!(result.facts.abis, ["arm64-v8a"]);
        assert_eq!(result.facts.certificate_sha256, None);
    }

    #[test]
    fn split_or_package_missing_is_blocking() {
        let result = inspect_apk_facts(ApkInspectionRequest {
            analyzer: ApkAnalyzerKind::Aapt2,
            facts: ApkInspectionFacts {
                split: Some(true),
                base: Some(false),
                ..ApkInspectionFacts::default()
            },
        });
        assert!(result.blocking);
        assert!(result
            .diagnostics
            .iter()
            .any(|item| item.code == "apk_split_unsupported"));
        assert!(result
            .diagnostics
            .iter()
            .any(|item| item.code == "apk_package_missing"));
    }
}
