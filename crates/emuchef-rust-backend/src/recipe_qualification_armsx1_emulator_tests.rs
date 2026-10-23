//! Opt-in live qualification for ARMSX1 on one operator-attested Android emulator.
//!
//! The ignored test independently validates the selected ADB target and device
//! identity before it invokes EmuChef's production planner, HTTPS resolver, and
//! real ADB executor. The test never provisions an AVD or installs SDK software.

use std::env;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::apk_manifest::inspect_apk_manifest;
use crate::executor::adb::{
    AdbCommandError, AdbCommandExecutor, AdbCommandResult, FakeAdbCommandExecutor,
    ProcessAdbCommandExecutor, RealAdbDevice,
};
use crate::executor::{DeviceOperationError, ExecutorDevice, PermissionOperation};
use crate::owned_process::ProcessOperation;
use crate::planner::{
    ExecutionArtifactSource, ExecutionParamValue, ExecutionPlan, TargetDeviceBinding,
};
use crate::review_projection::ReviewProjection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

const OPT_IN: &str = "EMUCHEF_RUN_ARMSX1_EMULATOR";
const SERIAL: &str = "EMUCHEF_ARMSX1_AVD_SERIAL";
const AVD_NAME: &str = "EMUCHEF_ARMSX1_AVD_NAME";
const ATTESTATION: &str = "EMUCHEF_ARMSX1_DEDICATED_AVD";
const PACKAGE_ALLOWLIST: &str = "EMUCHEF_ARMSX1_PACKAGE_ALLOWLIST";
const EXPECTED_AVD: &str = "EmuChef-ARMSX1-Android16-ARM64";
const REQUIRED_SERIAL: &str = "emulator-5554";
const PACKAGE: &str = "com.nanodata.armsx";
const EVIDENCE_PATH_ENV: &str = "EMUCHEF_ARMSX1_EVIDENCE_PATH";
const DIAGNOSTIC_EVIDENCE_PATH_ENV: &str = "EMUCHEF_ARMSX1_DIAGNOSTIC_EVIDENCE_PATH";
const ADB_PREFLIGHT_EXECUTABLE: &str = "adb";
const EXPECTED_INSTALL_STEP_SUFFIX: &str = "/install_armsx1";
const EXPECTED_RESOLVE_STEP_SUFFIX: &str = "/resolve_armsx1_apk";
const EXPECTED_RECIPE: &str = "app.armsx1.install";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceStage {
    Preflight,
    PlanValidation,
    RunnerSetup,
    NetworkResolution,
    ApkIdentity,
    Install,
    Version,
    Cleanup,
}

impl EvidenceStage {
    const ORDERED: [Self; 8] = [
        Self::Preflight,
        Self::PlanValidation,
        Self::RunnerSetup,
        Self::NetworkResolution,
        Self::ApkIdentity,
        Self::Install,
        Self::Version,
        Self::Cleanup,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Preflight => 0,
            Self::PlanValidation => 1,
            Self::RunnerSetup => 2,
            Self::NetworkResolution => 3,
            Self::ApkIdentity => 4,
            Self::Install => 5,
            Self::Version => 6,
            Self::Cleanup => 7,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutcomeCategory {
    Preflight,
    ContractDrift,
    UnsafePlan,
    RunnerSetup,
    MutationTarget,
    ReplaceExistingDisabled,
    NetworkResolution,
    NoEligibleRelease,
    ApkIdentity,
    InstallSigning,
    Install,
    Version,
    IncompleteCleanup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum OutcomeStatus {
    Passed,
    Failed,
    NotRun,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckOutcome {
    stage: EvidenceStage,
    status: OutcomeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<OutcomeCategory>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct QualificationEvidence {
    schema_version: u8,
    status: String,
    live_status: String,
    execution: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<OutcomeCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_code: Option<String>,
    outcomes: Vec<CheckOutcome>,
    install_runs: Vec<InstallRunOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_definition_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recipe_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contract_app_definition_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contract_recipe_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_abi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review_digest: Option<String>,
    resolved_releases: Vec<ResolvedReleaseEvidence>,
    package_observations: Vec<PackageObservation>,
    cleanup: CleanupEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallRunOutcome {
    run: u8,
    status: OutcomeStatus,
    install_step: InstallStepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replace_existing: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InstallStepStatus {
    NotRun,
    Executed,
    Skipped,
    Failed,
    Blocked,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CleanupStatus {
    NotRun,
    Complete,
    Incomplete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CleanupEvidence {
    status: CleanupStatus,
    package_disposition: String,
    process_observations: Vec<ProcessCleanupEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessCleanupEvidence {
    run: u8,
    step_id: String,
    status: ProcessCleanupStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProcessCleanupStatus {
    NotRequired,
    Confirmed,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PackageObservationPhase {
    Before,
    AfterFirstInstall,
    AfterSecondInstall,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageObservation {
    phase: PackageObservationPhase,
    package: String,
    installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedReleaseEvidence {
    run: u8,
    tag: String,
    asset: String,
    apk_sha256: String,
    package_identity_verified: bool,
    arm64_v8a_verified: bool,
    signing_verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceHashContract {
    app_definition_sha256: String,
    recipe_sha256: String,
    expected_apk_sha256: String,
    source_strategy: String,
    provider: String,
    service_origin: String,
    repository: String,
    include_prereleases: bool,
    asset_pattern: String,
    invert_asset_pattern: bool,
    artifact_cache: String,
}

#[derive(Clone, Copy)]
struct IsolatedCacheRoots<'a> {
    runtime: &'a Path,
    cache: &'a Path,
}

struct LivePlanPreparation {
    plan: Option<ExecutionPlan>,
    review: Option<ReviewProjection>,
    plan_digest: Option<String>,
    app_definition_sha256: String,
    recipe_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeviceProperties {
    state: String,
    boot_completed: String,
    api_level: String,
    abi: String,
    brand: String,
    release: String,
    manufacturer: String,
    model: String,
    emulator: String,
    uid: String,
    avd_name: Option<String>,
}

impl QualificationEvidence {
    fn failed(category: OutcomeCategory) -> Self {
        let mut evidence = Self::initial();
        evidence.fail(category);
        evidence
    }

    fn initial() -> Self {
        Self {
            schema_version: 1,
            status: "blocked".to_owned(),
            live_status: "unverified".to_owned(),
            execution: "NOT_RUN".to_owned(),
            reason: None,
            failure_code: None,
            outcomes: EvidenceStage::ORDERED
                .into_iter()
                .map(|stage| CheckOutcome {
                    stage,
                    status: OutcomeStatus::NotRun,
                    category: None,
                })
                .collect(),
            install_runs: vec![
                InstallRunOutcome {
                    run: 1,
                    status: OutcomeStatus::NotRun,
                    install_step: InstallStepStatus::NotRun,
                    step_id: None,
                    replace_existing: None,
                },
                InstallRunOutcome {
                    run: 2,
                    status: OutcomeStatus::NotRun,
                    install_step: InstallStepStatus::NotRun,
                    step_id: None,
                    replace_existing: None,
                },
            ],
            version_outcome: None,
            app_definition_sha256: None,
            recipe_sha256: None,
            contract_app_definition_sha256: None,
            contract_recipe_sha256: None,
            api_abi: None,
            plan_digest: None,
            review_digest: None,
            resolved_releases: Vec::new(),
            package_observations: Vec::new(),
            cleanup: CleanupEvidence {
                status: CleanupStatus::NotRun,
                package_disposition: "retained_for_operator_review".to_owned(),
                process_observations: Vec::new(),
            },
        }
    }

    fn complete(&mut self) {
        self.status = "passed".to_owned();
        self.live_status = "verified".to_owned();
        self.execution = "COMPLETED".to_owned();
    }

    fn mark_passed(&mut self, stage: EvidenceStage) {
        if stage != EvidenceStage::Cleanup
            && self.outcomes.iter().any(|outcome| {
                outcome.stage.index() < stage.index() && outcome.status != OutcomeStatus::Passed
            })
        {
            return;
        }
        if let Some(outcome) = self
            .outcomes
            .iter_mut()
            .find(|outcome| outcome.stage == stage)
        {
            if outcome.status != OutcomeStatus::Failed {
                outcome.status = OutcomeStatus::Passed;
                outcome.category = None;
            }
        }
    }

    fn mark_install_run(
        &mut self,
        run: u8,
        step_id: Option<String>,
        install_step: InstallStepStatus,
        replace_existing: Option<bool>,
    ) {
        if let Some(outcome) = self
            .install_runs
            .iter_mut()
            .find(|outcome| outcome.run == run)
        {
            outcome.install_step = install_step;
            outcome.step_id = step_id;
            outcome.replace_existing = replace_existing;
            outcome.status = match install_step {
                InstallStepStatus::Executed => OutcomeStatus::Passed,
                InstallStepStatus::Skipped
                | InstallStepStatus::Failed
                | InstallStepStatus::Blocked
                | InstallStepStatus::Cancelled => OutcomeStatus::Failed,
                InstallStepStatus::NotRun => OutcomeStatus::NotRun,
            };
        }
    }

    fn mark_cleanup_complete(&mut self) {
        self.cleanup.status = CleanupStatus::Complete;
        self.mark_passed(EvidenceStage::Cleanup);
    }

    fn mark_cleanup_incomplete(&mut self) {
        self.cleanup.status = CleanupStatus::Incomplete;
    }

    fn fail(&mut self, category: OutcomeCategory) {
        let failed_stage = match category {
            OutcomeCategory::Preflight => EvidenceStage::Preflight,
            OutcomeCategory::ContractDrift
            | OutcomeCategory::UnsafePlan
            | OutcomeCategory::MutationTarget
            | OutcomeCategory::ReplaceExistingDisabled => EvidenceStage::PlanValidation,
            OutcomeCategory::RunnerSetup => EvidenceStage::RunnerSetup,
            OutcomeCategory::NetworkResolution | OutcomeCategory::NoEligibleRelease => {
                EvidenceStage::NetworkResolution
            }
            OutcomeCategory::ApkIdentity => EvidenceStage::ApkIdentity,
            OutcomeCategory::InstallSigning | OutcomeCategory::Install => EvidenceStage::Install,
            OutcomeCategory::Version => EvidenceStage::Version,
            OutcomeCategory::IncompleteCleanup => EvidenceStage::Cleanup,
        };
        if let Some(outcome) = self
            .outcomes
            .iter_mut()
            .find(|outcome| outcome.stage == failed_stage)
        {
            outcome.status = OutcomeStatus::Failed;
            outcome.category = Some(category);
        }
        if category == OutcomeCategory::IncompleteCleanup {
            self.mark_cleanup_incomplete();
        }
        self.status = "blocked".to_owned();
        self.live_status = "unverified".to_owned();
        if self.reason.is_none()
            || (category != OutcomeCategory::IncompleteCleanup
                && self.reason == Some(OutcomeCategory::IncompleteCleanup))
        {
            self.reason = Some(category);
        }
        if category != OutcomeCategory::IncompleteCleanup {
            for outcome in &mut self.outcomes {
                if outcome.stage.index() > failed_stage.index()
                    && outcome.stage != EvidenceStage::Cleanup
                {
                    outcome.status = OutcomeStatus::NotRun;
                    outcome.category = None;
                }
            }
            self.version_outcome = if failed_stage == EvidenceStage::Version {
                Some("failed".to_owned())
            } else {
                None
            };
        }
    }
}

fn classify_failure(stage: EvidenceStage, diagnostic: &str) -> OutcomeCategory {
    match stage {
        EvidenceStage::Preflight => OutcomeCategory::Preflight,
        EvidenceStage::PlanValidation => OutcomeCategory::UnsafePlan,
        EvidenceStage::RunnerSetup => OutcomeCategory::RunnerSetup,
        EvidenceStage::NetworkResolution => {
            if diagnostic.starts_with("remote_release_not_found:")
                || diagnostic.starts_with("remote_asset_no_match:")
            {
                OutcomeCategory::NoEligibleRelease
            } else {
                OutcomeCategory::NetworkResolution
            }
        }
        EvidenceStage::ApkIdentity => OutcomeCategory::ApkIdentity,
        EvidenceStage::Install if apk_preinstall_failure_code(diagnostic).is_some() => {
            OutcomeCategory::ApkIdentity
        }
        EvidenceStage::Install if diagnostic.starts_with("android_signing_failed:") => {
            OutcomeCategory::InstallSigning
        }
        EvidenceStage::Install if diagnostic.starts_with("android_version_failed:") => {
            OutcomeCategory::Version
        }
        EvidenceStage::Install => OutcomeCategory::Install,
        EvidenceStage::Version => OutcomeCategory::Version,
        EvidenceStage::Cleanup => OutcomeCategory::IncompleteCleanup,
    }
}

fn write_evidence_atomic<T: Serialize>(path: &Path, evidence: &T) -> Result<(), &'static str> {
    let path = resolve_safe_evidence_destination(path)?;
    let bytes = serde_json::to_vec_pretty(evidence).map_err(|_| "evidence_serialization_failed")?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| "evidence_directory_create_failed")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "evidence_temporary_file_create_failed")?;
    file.write_all(&bytes)
        .map_err(|_| "evidence_file_write_failed")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "evidence_file_sync_failed")?;
    file.persist(&path)
        .map_err(|_| "evidence_atomic_replace_failed")?;
    sync_evidence_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_evidence_directory(path: &Path) -> Result<(), &'static str> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "evidence_directory_sync_failed")
}

#[cfg(not(unix))]
fn sync_evidence_directory(_path: &Path) -> Result<(), &'static str> {
    Ok(())
}

fn default_evidence_path() -> PathBuf {
    repository_root().join("docs/testing/recipe-qualification/armsx1/live-evidence.json")
}

fn contract_path() -> PathBuf {
    repository_root().join("tests/fixtures/recipe-qualification/armsx1/qualification-contract.json")
}

fn evidence_path() -> Result<PathBuf, &'static str> {
    let path = env::var_os(EVIDENCE_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_evidence_path);
    resolve_safe_evidence_destination(&path)
}

fn diagnostic_evidence_path() -> Result<PathBuf, &'static str> {
    let path = env::var_os(DIAGNOSTIC_EVIDENCE_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repository_root()
                .join("docs/testing/recipe-qualification/armsx1/preflight-diagnostic-evidence.json")
        });
    resolve_safe_evidence_destination(&path)
}

fn has_chatgpt_path_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().eq_ignore_ascii_case(".chatgpt"))
    })
}

fn canonicalize_path_with_missing_tail(path: &Path) -> Result<PathBuf, &'static str> {
    let mut current = path;
    let mut missing_tail = Vec::new();
    loop {
        match fs::canonicalize(current) {
            Ok(mut resolved) => {
                for component in missing_tail.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = current
                    .file_name()
                    .ok_or("evidence_path_unsafe")?
                    .to_os_string();
                missing_tail.push(component);
                current = current.parent().ok_or("evidence_path_unsafe")?;
            }
            Err(_) => return Err("evidence_path_unsafe"),
        }
    }
}

fn resolve_safe_evidence_destination(path: &Path) -> Result<PathBuf, &'static str> {
    if !path.is_absolute() || has_chatgpt_path_component(path) {
        return Err("evidence_path_unsafe");
    }
    let file_name = path.file_name().ok_or("evidence_path_unsafe")?;
    let parent = path.parent().ok_or("evidence_path_unsafe")?;
    let resolved_parent = canonicalize_path_with_missing_tail(parent)?;
    let resolved_path = resolved_parent.join(file_name);
    if has_chatgpt_path_component(&resolved_path) {
        return Err("evidence_path_unsafe");
    }
    match fs::symlink_metadata(&resolved_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_dir() => {
            return Err("evidence_path_unsafe");
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("evidence_path_unsafe"),
    }

    let repository = fs::canonicalize(repository_root()).map_err(|_| "evidence_path_unsafe")?;
    let approved_directory = canonicalize_path_with_missing_tail(
        &repository.join("docs/testing/recipe-qualification/armsx1"),
    )?;
    if resolved_path.starts_with(&repository) && !resolved_path.starts_with(&approved_directory) {
        return Err("evidence_path_unsafe");
    }
    Ok(resolved_path)
}

fn persist_evidence(path: &Path, evidence: &QualificationEvidence) -> Result<(), &'static str> {
    write_evidence_atomic(path, evidence)
}

fn persist_or_abort(path: &Path, evidence: &QualificationEvidence) {
    if persist_evidence(path, evidence).is_err() {
        panic!("qualification_evidence_write_failed");
    }
}

fn record_live_preflight_failure(
    evidence: &mut QualificationEvidence,
    code: &str,
) -> OutcomeCategory {
    evidence.fail(OutcomeCategory::Preflight);
    evidence.failure_code = Some(code.to_owned());
    OutcomeCategory::Preflight
}

fn fail_with_evidence(
    path: &Path,
    evidence: &mut QualificationEvidence,
    category: OutcomeCategory,
) -> ! {
    evidence.fail(category);
    persist_or_abort(path, evidence);
    panic!("qualification_blocked:{category:?}");
}

fn fail_with_preflight_code(path: &Path, evidence: &mut QualificationEvidence, code: &str) -> ! {
    let category = record_live_preflight_failure(evidence, code);
    persist_or_abort(path, evidence);
    panic!("qualification_blocked:{category:?}");
}

fn fail_with_runtime_evidence<D: ExecutorDevice>(
    path: &Path,
    evidence: &mut QualificationEvidence,
    category: OutcomeCategory,
    runner: crate::executor::ExecutorRunner<D>,
    temporary: tempfile::TempDir,
) -> ! {
    evidence.fail(category);
    close_live_runtime(evidence, runner, temporary);
    persist_or_abort(path, evidence);
    panic!("qualification_blocked:{category:?}");
}

fn after_admission<T, E>(
    admission: Result<(), OutcomeCategory>,
    setup: impl FnOnce() -> Result<T, E>,
) -> Result<Result<T, E>, OutcomeCategory> {
    admission.map(|()| setup())
}

fn close_live_runtime<D: ExecutorDevice>(
    evidence: &mut QualificationEvidence,
    runner: crate::executor::ExecutorRunner<D>,
    temporary: tempfile::TempDir,
) {
    drop(runner);
    if temporary.close().is_err() {
        evidence.mark_cleanup_incomplete();
        evidence.fail(OutcomeCategory::IncompleteCleanup);
    } else if evidence.cleanup.status != CleanupStatus::Incomplete {
        evidence.mark_cleanup_complete();
    }
}

fn complete_live_qualification(
    evidence: &mut QualificationEvidence,
) -> Result<(), OutcomeCategory> {
    validate_install_run_records(evidence)?;
    if evidence.cleanup.status != CleanupStatus::Complete {
        return Err(OutcomeCategory::IncompleteCleanup);
    }
    if evidence
        .outcomes
        .iter()
        .any(|outcome| outcome.status != OutcomeStatus::Passed)
    {
        return Err(evidence.reason.unwrap_or(OutcomeCategory::Version));
    }
    evidence.complete();
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExecutionFailure {
    pub(crate) primary: Option<OutcomeCategory>,
    pub(crate) cleanup_incomplete: bool,
}

const APK_PREINSTALL_FAILURE_CODES: [(&str, &str); 8] = [
    (
        "apk_preinstall_sha256_failed:",
        "apk_preinstall_sha256_failed",
    ),
    (
        "apk_preinstall_manifest_failed:",
        "apk_preinstall_manifest_failed",
    ),
    (
        "apk_preinstall_package_mismatch:",
        "apk_preinstall_package_mismatch",
    ),
    (
        "apk_preinstall_arm64_missing:",
        "apk_preinstall_arm64_missing",
    ),
    (
        "apk_preinstall_arm64_observation_failed:",
        "apk_preinstall_arm64_observation_failed",
    ),
    (
        "apk_preinstall_signature_verifier_unavailable:",
        "apk_preinstall_signature_verifier_unavailable",
    ),
    (
        "apk_preinstall_signature_verifier_failed:",
        "apk_preinstall_signature_verifier_failed",
    ),
    (
        "apk_preinstall_signature_rejected:",
        "apk_preinstall_signature_rejected",
    ),
];

fn apk_preinstall_failure_code(diagnostic: &str) -> Option<&'static str> {
    APK_PREINSTALL_FAILURE_CODES
        .iter()
        .find_map(|(prefix, code)| diagnostic.starts_with(prefix).then_some(*code))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ApkIdentityCheckResults {
    package_identity_verified: bool,
    arm64_v8a_verified: bool,
    signing_verified: bool,
}

impl ApkIdentityCheckResults {
    const VERIFIED: Self = Self {
        package_identity_verified: true,
        arm64_v8a_verified: true,
        signing_verified: true,
    };

    const fn all_verified(self) -> bool {
        self.package_identity_verified && self.arm64_v8a_verified && self.signing_verified
    }
}

fn apk_identity_check_results(
    install_step: InstallStepStatus,
    record: Option<&crate::executor::StepRunRecord>,
) -> ApkIdentityCheckResults {
    use crate::executor::StepFailureKind;

    if install_step == InstallStepStatus::Executed
        || record.is_some_and(|record| {
            matches!(
                record.failure_kind,
                Some(
                    StepFailureKind::InstallSigningRejected
                        | StepFailureKind::InstallVersionRejected
                        | StepFailureKind::InstallRejected
                )
            )
        })
    {
        return ApkIdentityCheckResults::VERIFIED;
    }

    let Some(code) = record
        .and_then(|record| record.message.as_deref())
        .and_then(apk_preinstall_failure_code)
    else {
        return ApkIdentityCheckResults::default();
    };

    match code {
        "apk_preinstall_arm64_missing" | "apk_preinstall_arm64_observation_failed" => {
            ApkIdentityCheckResults {
                package_identity_verified: true,
                ..ApkIdentityCheckResults::default()
            }
        }
        "apk_preinstall_signature_verifier_unavailable"
        | "apk_preinstall_signature_verifier_failed"
        | "apk_preinstall_signature_rejected" => ApkIdentityCheckResults {
            package_identity_verified: true,
            arm64_v8a_verified: true,
            signing_verified: false,
        },
        _ => ApkIdentityCheckResults::default(),
    }
}

pub(crate) fn classify_execution_failure(
    result: &crate::executor::ExecutionRunResult,
) -> ExecutionFailure {
    use crate::executor::StepRunStatus;
    use crate::owned_process::ProcessCleanup;

    let cleanup_incomplete = result
        .steps
        .iter()
        .any(|record| record.cleanup == Some(ProcessCleanup::Uncertain));
    let primary = result.steps.iter().find_map(|record| {
        if !matches!(
            record.status,
            StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
        ) {
            return None;
        }
        let diagnostic = record.message.as_deref().unwrap_or_default();
        if apk_preinstall_failure_code(diagnostic).is_some() {
            return Some(OutcomeCategory::ApkIdentity);
        }
        let structured_install_category = match record.failure_kind {
            Some(crate::executor::StepFailureKind::InstallSigningRejected) => {
                Some(OutcomeCategory::InstallSigning)
            }
            Some(crate::executor::StepFailureKind::InstallVersionRejected) => {
                Some(OutcomeCategory::Version)
            }
            Some(crate::executor::StepFailureKind::InstallRejected) => {
                Some(OutcomeCategory::Install)
            }
            _ => None,
        };
        if structured_install_category.is_some() {
            return structured_install_category;
        }

        let stage = if diagnostic.starts_with("apk_package_mismatch:") {
            EvidenceStage::ApkIdentity
        } else if diagnostic.starts_with("android_signing_failed:")
            || diagnostic.starts_with("android_install_failed:")
            || diagnostic.starts_with("android_version_failed:")
            || record.step_id.ends_with(EXPECTED_INSTALL_STEP_SUFFIX)
        {
            EvidenceStage::Install
        } else {
            EvidenceStage::NetworkResolution
        };
        Some(classify_failure(stage, diagnostic))
    });
    ExecutionFailure {
        primary: primary.or_else(|| {
            (!result.success && !cleanup_incomplete).then_some(OutcomeCategory::NetworkResolution)
        }),
        cleanup_incomplete,
    }
}

#[test]
fn structured_install_failure_kinds_map_to_sanitized_live_evidence_categories() {
    for (failure_kind, expected_category) in [
        (
            crate::executor::StepFailureKind::InstallSigningRejected,
            OutcomeCategory::InstallSigning,
        ),
        (
            crate::executor::StepFailureKind::InstallVersionRejected,
            OutcomeCategory::Version,
        ),
        (
            crate::executor::StepFailureKind::InstallRejected,
            OutcomeCategory::Install,
        ),
    ] {
        let result = crate::executor::ExecutionRunResult {
            success: false,
            cancelled: false,
            total_steps: 1,
            steps: vec![crate::executor::StepRunRecord {
                step_id: "recipe/install".to_string(),
                status: crate::executor::StepRunStatus::Failed,
                message: Some(
                    "The device operation could not be started or completed safely.".to_string(),
                ),
                outputs: crate::model::OrderedMap::default(),
                failure_kind: Some(failure_kind),
                cleanup: None,
            }],
            resolved_releases: Vec::new(),
            resolved_artifacts: Vec::new(),
        };

        assert_eq!(
            classify_execution_failure(&result).primary,
            Some(expected_category)
        );
    }
}

fn install_step_observation(
    result: &crate::executor::ExecutionRunResult,
    expected_step_id: &str,
) -> (Option<String>, InstallStepStatus) {
    use crate::executor::StepRunStatus;

    let record = result
        .steps
        .iter()
        .find(|record| record.step_id == expected_step_id);
    let status = match record.map(|record| &record.status) {
        Some(StepRunStatus::Executed) => InstallStepStatus::Executed,
        Some(StepRunStatus::Skipped) => InstallStepStatus::Skipped,
        Some(StepRunStatus::Failed) => InstallStepStatus::Failed,
        Some(StepRunStatus::Blocked) => InstallStepStatus::Blocked,
        Some(StepRunStatus::Cancelled) => InstallStepStatus::Cancelled,
        None => InstallStepStatus::NotRun,
    };
    (record.map(|record| record.step_id.clone()), status)
}

fn validate_install_run_records(evidence: &QualificationEvidence) -> Result<(), OutcomeCategory> {
    let expected_step_id = format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}");
    if evidence.install_runs.len() != 2 {
        return Err(OutcomeCategory::Install);
    }
    for run in [1, 2] {
        let Some(record) = evidence
            .install_runs
            .iter()
            .find(|record| record.run == run)
        else {
            return Err(OutcomeCategory::Install);
        };
        if record.replace_existing != Some(true) {
            return Err(OutcomeCategory::ReplaceExistingDisabled);
        }
        if record.status != OutcomeStatus::Passed
            || record.install_step != InstallStepStatus::Executed
            || record.step_id.as_deref() != Some(expected_step_id.as_str())
        {
            return Err(OutcomeCategory::Install);
        }
    }
    Ok(())
}

fn is_safe_release_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 128
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

fn is_expected_release_asset(asset: &str) -> bool {
    if let Some(date) = asset
        .strip_prefix("ARMSX1-release-")
        .and_then(|name| name.strip_suffix("-arm64-v8a.apk"))
    {
        return date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit());
    }
    let Some(version) = asset
        .strip_prefix("ARMSX1-")
        .and_then(|name| name.strip_suffix(".apk"))
    else {
        return false;
    };
    let components = version.split('.').collect::<Vec<_>>();
    components.len() == 3
        && components.iter().all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_file(path: &Path) -> Result<String, &'static str> {
    let mut file = File::open(path).map_err(|_| "apk_preinstall_sha256_failed")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "apk_preinstall_sha256_failed")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn apk_has_arm64_native_library(path: &Path) -> Result<bool, &'static str> {
    let file = File::open(path).map_err(|_| "apk_preinstall_arm64_observation_failed")?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| "apk_preinstall_arm64_observation_failed")?;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| "apk_preinstall_arm64_observation_failed")?;
        let name = entry.name();
        if entry.size() > 0 && name.starts_with("lib/arm64-v8a/") && name.ends_with(".so") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn system_apk_signature_verifier(path: &Path) -> Result<(), &'static str> {
    let mut candidates = vec![PathBuf::from("apksigner")];
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        let Ok(root) = env::var(variable) else {
            continue;
        };
        let build_tools = PathBuf::from(root).join("build-tools");
        let Ok(entries) = fs::read_dir(build_tools) else {
            continue;
        };
        let mut versions = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("apksigner").is_file())
            .collect::<Vec<_>>();
        versions.sort();
        versions.reverse();
        candidates.extend(versions.into_iter().map(|path| path.join("apksigner")));
    }

    for executable in candidates {
        let output =
            match Command::new(&executable)
                .arg("verify")
                .arg("--print-certs")
                .arg(path)
                .output()
            {
                Ok(output) => output,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(
                    "apk_preinstall_signature_verifier_failed: verifier process could not start",
                ),
            };
        return if output.status.success() {
            Ok(())
        } else {
            Err("apk_preinstall_signature_rejected: APK signature verification failed")
        };
    }
    Err("apk_preinstall_signature_verifier_unavailable: APK signature verifier is unavailable")
}

fn validate_apk_before_install(
    path: &Path,
    expected_sha256: &str,
    expected_package: &str,
    signature_verifier: fn(&Path) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let actual_sha256 = sha256_file(path)?;
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Err("apk_preinstall_sha256_failed: bytes did not match trusted release digest");
    }
    let manifest = inspect_apk_manifest(path)
        .map_err(|_| "apk_preinstall_manifest_failed: package metadata could not be inspected")?;
    if manifest.package_name != expected_package {
        return Err("apk_preinstall_package_mismatch: App-owned package identity differs");
    }
    if !apk_has_arm64_native_library(path)? {
        return Err("apk_preinstall_arm64_missing: APK has no arm64-v8a native library");
    }
    signature_verifier(path)
}

#[derive(Debug)]
struct VerifiedInstallDevice<D> {
    inner: D,
    expected_apk_sha256: String,
    expected_package: String,
    signature_verifier: fn(&Path) -> Result<(), &'static str>,
}

impl<D> VerifiedInstallDevice<D> {
    fn new(
        inner: D,
        expected_apk_sha256: String,
        signature_verifier: fn(&Path) -> Result<(), &'static str>,
    ) -> Self {
        Self {
            inner,
            expected_apk_sha256,
            expected_package: PACKAGE.to_owned(),
            signature_verifier,
        }
    }
}

impl<D: ExecutorDevice> ExecutorDevice for VerifiedInstallDevice<D> {
    fn configure_identity_guard(&mut self, target: Option<&TargetDeviceBinding>) {
        self.inner.configure_identity_guard(target);
    }

    fn configure_root_authority(&mut self, reviewed_root_authorized: bool) {
        self.inner
            .configure_root_authority(reviewed_root_authorized);
    }

    fn owns_per_command_root_authority(&self) -> bool {
        self.inner.owns_per_command_root_authority()
    }

    fn revalidate_root(&mut self) -> Result<(), DeviceOperationError> {
        self.inner.revalidate_root()
    }

    fn uses_fake_device_filesystem(&self) -> bool {
        self.inner.uses_fake_device_filesystem()
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        validate_apk_before_install(
            apk_path,
            &self.expected_apk_sha256,
            &self.expected_package,
            self.signature_verifier,
        )
        .map_err(|code| DeviceOperationError::other(code.to_owned()))?;
        self.inner.install_apk(apk_path, replace_existing)
    }

    fn record_installed_package(&mut self, package_name: &str) {
        self.inner.record_installed_package(package_name);
    }

    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError> {
        self.inner.push(source, dest, sync)
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.inner.mkdir_p(path)
    }

    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.inner.remove_file(path)
    }

    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        self.inner.remove_tree(path)
    }

    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError> {
        self.inner
            .copy_on_device(source, dest, recursive, privileged)
    }

    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError> {
        self.inner.package_installed(package_name)
    }

    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        self.inner.path_exists(path)
    }

    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        self.inner.path_is_dir(path)
    }

    fn apply_permission_operation(
        &mut self,
        operation: PermissionOperation,
    ) -> Result<(), DeviceOperationError> {
        self.inner.apply_permission_operation(operation)
    }

    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError> {
        self.inner.launch_app(package_name, activity)
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError> {
        self.inner.force_stop_app(package_name)
    }
}

#[test]
fn release_asset_names_accept_dated_and_exact_semver_forms_only() {
    for accepted in [
        "ARMSX1-release-20260925-arm64-v8a.apk",
        "ARMSX1-0.3.6.apk",
        "ARMSX1-1.2.3.apk",
        "ARMSX1-10.20.30.apk",
    ] {
        assert!(
            is_expected_release_asset(accepted),
            "{accepted} must be accepted"
        );
    }
    for rejected in [
        "ARMSX1-v1.2.3.apk",
        "ARMSX1-1.2.apk",
        "ARMSX1-1.2.3.4.apk",
        "ARMSX1-1.2.3-beta.apk",
        "ARMSX1-1.2.3-debug.apk",
        "ARMSX1-release-debug.apk",
        "ARMSX1-release-20260923-arm64-v8a-sources.apk",
        "ARMSX1-nightly.apk",
        "ARMSX2-1.2.3.apk",
    ] {
        assert!(
            !is_expected_release_asset(rejected),
            "{rejected} must be rejected"
        );
    }
}

fn write_test_apk_with_native_library(
    workspace: &tempfile::TempDir,
    package_name: &'static str,
    add_arm64_library: bool,
) -> PathBuf {
    let source =
        crate::apk_manifest::tests::write_valid_test_apk_with_package(workspace, package_name);
    let destination = workspace.path().join(format!(
        "{package_name}-{}.apk",
        if add_arm64_library {
            "native-arm64"
        } else {
            "manifest-only"
        }
    ));
    let mut archive =
        zip::ZipArchive::new(File::open(&source).expect("source APK fixture should open"))
            .expect("source APK fixture should be a ZIP archive");
    let mut output = zip::ZipWriter::new(
        File::create(&destination).expect("destination APK fixture should be writable"),
    );
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .expect("source APK entry should be readable");
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .expect("source APK entry should copy");
        output
            .start_file(entry.name(), options)
            .expect("destination APK entry should start");
        output
            .write_all(&contents)
            .expect("destination APK entry should write");
    }
    if add_arm64_library {
        output
            .start_file("lib/arm64-v8a/libfixture.so", options)
            .expect("ARM64 fixture entry should start");
        output
            .write_all(b"\x7fELFfixture")
            .expect("ARM64 fixture entry should write");
    }
    output
        .finish()
        .expect("destination APK fixture should finish");
    destination
}

fn accept_test_signature(_path: &Path) -> Result<(), &'static str> {
    Ok(())
}

fn reject_test_signature(_path: &Path) -> Result<(), &'static str> {
    Err("apk_preinstall_signature_rejected: synthetic signature rejection")
}

fn unavailable_test_signature(_path: &Path) -> Result<(), &'static str> {
    Err("apk_preinstall_signature_verifier_unavailable: synthetic unavailable verifier")
}

fn failed_test_signature_verifier(_path: &Path) -> Result<(), &'static str> {
    Err("apk_preinstall_signature_verifier_failed: synthetic verifier execution failure")
}

#[test]
fn apk_preinstall_validation_fails_closed_before_install_mutation() {
    let workspace = tempfile::tempdir().expect("APK validation workspace should be created");
    let native = write_test_apk_with_native_library(&workspace, PACKAGE, true);
    let manifest_only = write_test_apk_with_native_library(&workspace, PACKAGE, false);
    let wrong_package = write_test_apk_with_native_library(&workspace, "dev.example.wrong", true);
    let native_sha256 = sha256_file(&native).expect("native APK hash should be readable");
    let manifest_only_sha256 =
        sha256_file(&manifest_only).expect("manifest-only APK hash should be readable");
    let wrong_package_sha256 =
        sha256_file(&wrong_package).expect("wrong-package APK hash should be readable");
    assert_eq!(
        inspect_apk_manifest(&native)
            .expect("native APK fixture manifest should inspect")
            .package_name,
        PACKAGE
    );

    let mut accepted = VerifiedInstallDevice::new(
        crate::executor::FakeDryRunDevice::default(),
        native_sha256.clone(),
        accept_test_signature,
    );
    accepted
        .install_apk(&native, true)
        .expect("fully validated APK should reach the device adapter");
    assert!(!accepted.inner.commands().is_empty());

    let cases = [
        (
            manifest_only,
            manifest_only_sha256,
            "apk_preinstall_arm64_missing:",
        ),
        (
            wrong_package,
            wrong_package_sha256,
            "apk_preinstall_package_mismatch:",
        ),
        (
            native.clone(),
            "00".repeat(32),
            "apk_preinstall_sha256_failed:",
        ),
    ];
    for (path, expected_sha256, expected_code) in cases {
        let mut rejected = VerifiedInstallDevice::new(
            crate::executor::FakeDryRunDevice::default(),
            expected_sha256,
            accept_test_signature,
        );
        let error = rejected
            .install_apk(&path, true)
            .expect_err("invalid APK must fail before install dispatch");
        assert!(
            error.message.starts_with(expected_code),
            "expected {expected_code}, got {}",
            error.message
        );
        assert!(rejected.inner.commands().is_empty());
    }

    let mut unsigned = VerifiedInstallDevice::new(
        crate::executor::FakeDryRunDevice::default(),
        native_sha256,
        reject_test_signature,
    );
    let error = unsigned
        .install_apk(&native, true)
        .expect_err("unverified signing evidence must fail closed");
    assert!(error
        .message
        .starts_with("apk_preinstall_signature_rejected:"));
    assert!(unsigned.inner.commands().is_empty());
}

fn record_test_install_failure(
    message: &str,
    failure_kind: Option<crate::executor::StepFailureKind>,
) -> (QualificationEvidence, ExecutionFailure) {
    let mut evidence = QualificationEvidence::initial();
    for stage in [
        EvidenceStage::Preflight,
        EvidenceStage::PlanValidation,
        EvidenceStage::RunnerSetup,
        EvidenceStage::NetworkResolution,
    ] {
        evidence.mark_passed(stage);
    }
    evidence.resolved_releases.push(ResolvedReleaseEvidence {
        run: 1,
        tag: "0.3.6".to_owned(),
        asset: "ARMSX1-0.3.6.apk".to_owned(),
        apk_sha256: "a".repeat(64),
        package_identity_verified: false,
        arm64_v8a_verified: false,
        signing_verified: false,
    });
    let result = crate::executor::ExecutionRunResult {
        success: false,
        cancelled: false,
        total_steps: 1,
        steps: vec![crate::executor::StepRunRecord {
            step_id: format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}"),
            status: crate::executor::StepRunStatus::Failed,
            message: Some(message.to_owned()),
            outputs: crate::model::OrderedMap::default(),
            failure_kind,
            cleanup: None,
        }],
        resolved_releases: Vec::new(),
        resolved_artifacts: Vec::new(),
    };
    let install_step_id = format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}");
    let failure = record_execution_result(&mut evidence, 1, &install_step_id, true, true, &result);
    (evidence, failure)
}

#[test]
fn local_signature_verifier_failures_are_apk_identity_and_never_call_adb() {
    let workspace = tempfile::tempdir().expect("APK validation workspace should be created");
    let native = write_test_apk_with_native_library(&workspace, PACKAGE, true);
    let native_sha256 = sha256_file(&native).expect("native APK hash should be readable");
    type TestSignatureVerifier = fn(&Path) -> Result<(), &'static str>;
    let cases: [(TestSignatureVerifier, &str); 3] = [
        (
            unavailable_test_signature,
            "apk_preinstall_signature_verifier_unavailable",
        ),
        (
            failed_test_signature_verifier,
            "apk_preinstall_signature_verifier_failed",
        ),
        (reject_test_signature, "apk_preinstall_signature_rejected"),
    ];

    for (signature_verifier, failure_code) in cases {
        let real_device =
            RealAdbDevice::with_executor("adb", None, FakeAdbCommandExecutor::default());
        let mut device =
            VerifiedInstallDevice::new(real_device, native_sha256.clone(), signature_verifier);
        let error = device
            .install_apk(&native, true)
            .expect_err("local signature verification failure must stop before ADB");
        assert!(error.message.starts_with(&format!("{failure_code}:")));
        assert_eq!(
            classify_failure(EvidenceStage::Install, &error.message),
            OutcomeCategory::ApkIdentity
        );
        assert!(device.inner.command_executor().calls().is_empty());

        let (evidence, failure) = record_test_install_failure(
            &error.message,
            Some(crate::executor::StepFailureKind::OperationFailed),
        );
        assert_eq!(failure.primary, Some(OutcomeCategory::ApkIdentity));
        assert_eq!(evidence.failure_code.as_deref(), Some(failure_code));
        let release = evidence
            .resolved_releases
            .first()
            .expect("release evidence should be retained");
        assert!(release.package_identity_verified);
        assert!(release.arm64_v8a_verified);
        assert!(!release.signing_verified);
        assert_eq!(evidence.outcomes[4].stage, EvidenceStage::ApkIdentity);
        assert_eq!(evidence.outcomes[4].status, OutcomeStatus::Failed);
        assert_eq!(
            evidence.outcomes[4].category,
            Some(OutcomeCategory::ApkIdentity)
        );
        assert_eq!(evidence.outcomes[5].status, OutcomeStatus::NotRun);
        assert_eq!(evidence.outcomes[6].status, OutcomeStatus::NotRun);
        assert_eq!(evidence.install_runs[0].status, OutcomeStatus::NotRun);
        assert_eq!(
            evidence.install_runs[0].install_step,
            InstallStepStatus::NotRun
        );
        assert_eq!(evidence.install_runs[0].replace_existing, Some(true));
    }
}

#[test]
fn android_update_incompatible_response_remains_an_install_signing_failure() {
    let mut fake_adb = FakeAdbCommandExecutor::default();
    fake_adb.push_completed(
        1,
        "",
        "Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: package=private cert=private]",
    );
    let mut device = RealAdbDevice::with_executor("adb", None, fake_adb);
    let message = device
        .install_apk(Path::new("fixture.apk"), true)
        .expect_err("fake package-manager signing conflict should fail");
    assert_eq!(
        classify_failure(EvidenceStage::Install, &message),
        OutcomeCategory::InstallSigning
    );
    assert_eq!(device.command_executor().calls().len(), 1);

    let (evidence, failure) = record_test_install_failure(
        &message,
        Some(crate::executor::StepFailureKind::InstallSigningRejected),
    );
    assert_eq!(failure.primary, Some(OutcomeCategory::InstallSigning));
    let identity = &evidence.outcomes[4];
    assert_eq!(identity.status, OutcomeStatus::Passed);
    assert_eq!(identity.category, None);
    let install = &evidence.outcomes[5];
    assert_eq!(install.status, OutcomeStatus::Failed);
    assert_eq!(install.category, Some(OutcomeCategory::InstallSigning));
    assert_eq!(evidence.outcomes[6].status, OutcomeStatus::NotRun);
    let release = evidence
        .resolved_releases
        .first()
        .expect("release evidence should be retained");
    assert!(release.package_identity_verified);
    assert!(release.arm64_v8a_verified);
    assert!(release.signing_verified);
    assert_eq!(evidence.install_runs[0].status, OutcomeStatus::Failed);
    assert_eq!(
        evidence.install_runs[0].install_step,
        InstallStepStatus::Failed
    );
}

fn record_execution_result(
    evidence: &mut QualificationEvidence,
    run: u8,
    expected_install_step_id: &str,
    replace_existing: bool,
    release_required: bool,
    result: &crate::executor::ExecutionRunResult,
) -> ExecutionFailure {
    let mut valid_release_count = 0;
    let mut invalid_release = false;
    for record in &result.steps {
        if let Some(cleanup) = record.cleanup {
            let status = match cleanup {
                crate::owned_process::ProcessCleanup::NotRequired => {
                    ProcessCleanupStatus::NotRequired
                }
                crate::owned_process::ProcessCleanup::Confirmed => ProcessCleanupStatus::Confirmed,
                crate::owned_process::ProcessCleanup::Uncertain => ProcessCleanupStatus::Uncertain,
            };
            evidence
                .cleanup
                .process_observations
                .push(ProcessCleanupEvidence {
                    run,
                    step_id: record.step_id.clone(),
                    status,
                });
        }
    }
    for release in &result.resolved_releases {
        if is_safe_release_tag(&release.release_tag)
            && is_expected_release_asset(&release.asset_name)
            && is_sha256(&release.calculated_sha256)
        {
            valid_release_count += 1;
            evidence.resolved_releases.push(ResolvedReleaseEvidence {
                run,
                tag: release.release_tag.clone(),
                asset: release.asset_name.clone(),
                apk_sha256: release.calculated_sha256.to_ascii_lowercase(),
                package_identity_verified: false,
                arm64_v8a_verified: false,
                signing_verified: false,
            });
        } else {
            invalid_release = true;
        }
    }
    if valid_release_count == 1 && !invalid_release && result.resolved_releases.len() == 1 {
        evidence.mark_passed(EvidenceStage::NetworkResolution);
    }

    let (step_id, observed_install_step) =
        install_step_observation(result, expected_install_step_id);
    let install_record = result
        .steps
        .iter()
        .find(|record| record.step_id == expected_install_step_id);
    let preinstall_failure_code = install_record
        .and_then(|record| record.message.as_deref())
        .and_then(apk_preinstall_failure_code);
    let install_step = if preinstall_failure_code.is_some() {
        InstallStepStatus::NotRun
    } else {
        observed_install_step
    };
    evidence.mark_install_run(run, step_id, install_step, Some(replace_existing));
    if let Some(code) = preinstall_failure_code {
        evidence.failure_code = Some(code.to_owned());
    }
    let identity_checks = apk_identity_check_results(observed_install_step, install_record);
    for release in evidence
        .resolved_releases
        .iter_mut()
        .filter(|release| release.run == run)
    {
        release.package_identity_verified = identity_checks.package_identity_verified;
        release.arm64_v8a_verified = identity_checks.arm64_v8a_verified;
        release.signing_verified = identity_checks.signing_verified;
    }
    if identity_checks.all_verified() {
        evidence.mark_passed(EvidenceStage::ApkIdentity);
    }
    if observed_install_step == InstallStepStatus::Executed {
        evidence.mark_passed(EvidenceStage::Install);
    }

    let mut failure = classify_execution_failure(result);
    if invalid_release || result.resolved_releases.len() > 1 {
        failure.primary = Some(OutcomeCategory::ApkIdentity);
    }
    let release_observed =
        valid_release_count == 1 && result.resolved_releases.len() == 1 && !invalid_release;
    let release_is_optional_cache_hit = !release_required && result.resolved_releases.is_empty();
    if failure.primary.is_none() && result.success {
        if !release_observed && !release_is_optional_cache_hit {
            failure.primary = Some(OutcomeCategory::NetworkResolution);
        } else if install_step != InstallStepStatus::Executed {
            failure.primary = Some(OutcomeCategory::Install);
        }
    }
    if let Some(category) = failure.primary {
        match category {
            OutcomeCategory::ApkIdentity
            | OutcomeCategory::InstallSigning
            | OutcomeCategory::Install => {}
            OutcomeCategory::Version if observed_install_step == InstallStepStatus::Executed => {
                evidence.mark_passed(EvidenceStage::Install);
            }
            OutcomeCategory::Version => {}
            _ => {}
        }
        evidence.fail(category);
    }

    if failure.cleanup_incomplete {
        evidence.mark_cleanup_incomplete();
        evidence.fail(OutcomeCategory::IncompleteCleanup);
    }
    failure
}

fn compare_repeat_versions(
    first: &PackageObservation,
    second: &PackageObservation,
) -> Result<&'static str, OutcomeCategory> {
    if first.phase != PackageObservationPhase::AfterFirstInstall
        || second.phase != PackageObservationPhase::AfterSecondInstall
        || first.package != PACKAGE
        || second.package != PACKAGE
        || !first.installed
        || !second.installed
    {
        return Err(OutcomeCategory::Version);
    }
    let first_code = first
        .version_code
        .as_deref()
        .and_then(|code| code.parse::<u64>().ok())
        .ok_or(OutcomeCategory::Version)?;
    let second_code = second
        .version_code
        .as_deref()
        .and_then(|code| code.parse::<u64>().ok())
        .ok_or(OutcomeCategory::Version)?;
    match second_code.cmp(&first_code) {
        std::cmp::Ordering::Equal => Ok("same_version_replacement"),
        std::cmp::Ordering::Greater => Ok("version_upgrade"),
        std::cmp::Ordering::Less => Err(OutcomeCategory::Version),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LiveIdentity {
    serial: String,
    avd_name: String,
}

fn validate_invocation(
    opt_in: Option<&str>,
    serial: Option<&str>,
    avd_name: Option<&str>,
    attestation: Option<&str>,
    allowlist: Option<&str>,
) -> Result<LiveIdentity, &'static str> {
    if opt_in != Some("1") {
        return Err("explicit_opt_in_missing");
    }
    let serial = serial
        .filter(|value| !value.is_empty())
        .ok_or("exact_serial_missing")?;
    if !serial.starts_with("emulator-")
        || !serial[9..].bytes().all(|byte| byte.is_ascii_digit())
        || serial != REQUIRED_SERIAL
    {
        return Err("exact_emulator_serial_invalid");
    }
    let avd_name = avd_name
        .filter(|value| !value.is_empty())
        .ok_or("designated_avd_name_missing")?;
    if avd_name != EXPECTED_AVD {
        return Err("designated_avd_name_mismatch");
    }
    if attestation != Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED") {
        return Err("dedicated_avd_attestation_missing");
    }
    if allowlist != Some(PACKAGE) {
        return Err("package_allowlist_mismatch");
    }
    Ok(LiveIdentity {
        serial: serial.to_owned(),
        avd_name: avd_name.to_owned(),
    })
}

const DIAGNOSTIC_SERIAL: &str = REQUIRED_SERIAL;

fn validate_diagnostic_invocation(
    opt_in: Option<&str>,
    serial: Option<&str>,
    avd_name: Option<&str>,
    attestation: Option<&str>,
    allowlist: Option<&str>,
) -> Result<LiveIdentity, &'static str> {
    let identity = validate_invocation(opt_in, serial, avd_name, attestation, allowlist)?;
    if identity.serial != DIAGNOSTIC_SERIAL {
        return Err("exact_emulator_serial_invalid");
    }
    Ok(identity)
}

fn adb(serial: &str, args: &[&str]) -> Result<AdbCommandResult, String> {
    adb_with_executable(ADB_PREFLIGHT_EXECUTABLE, serial, args)
}

fn adb_with_executable(
    executable: &str,
    serial: &str,
    args: &[&str],
) -> Result<AdbCommandResult, String> {
    let command = std::iter::once(executable.to_owned())
        .chain(["-s".to_owned(), serial.to_owned()])
        .chain(args.iter().map(|argument| (*argument).to_owned()))
        .collect::<Vec<_>>();
    let mut executor = ProcessAdbCommandExecutor::default();
    executor
        .run_for(&command, ProcessOperation::Probe)
        .map_err(|error| match error {
            AdbCommandError::TimedOut { .. } => "adb_preflight_timed_out".to_owned(),
            _ => "adb_preflight_command_failed".to_owned(),
        })
}

fn adb_text(serial: &str, args: &[&str]) -> Result<String, String> {
    let output = adb(serial, args)?;
    if output.returncode != 0 {
        return Err("adb_preflight_command_failed".to_owned());
    }
    Ok(output.stdout.trim().to_owned())
}

/// Sanitized package-observation failure categories shared by diagnostic and live preflight.
///
/// Command and parsing failures stay separate so callers never infer absence from an
/// unavailable or malformed package-manager response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PackageObservationError {
    PathTimedOut,
    PathCommandFailed,
    PathAmbiguous,
    MetadataTimedOut,
    MetadataCommandFailed,
    MetadataAmbiguous,
    VersionUnavailable,
}

impl PackageObservationError {
    fn diagnostic_code(self) -> &'static str {
        match self {
            Self::PathTimedOut => "initial_package_observation_path_timed_out",
            Self::PathCommandFailed => "initial_package_observation_path_failed",
            Self::PathAmbiguous => "initial_package_observation_path_ambiguous",
            Self::MetadataTimedOut => "initial_package_observation_metadata_timed_out",
            Self::MetadataCommandFailed => "initial_package_observation_metadata_failed",
            Self::MetadataAmbiguous => "initial_package_observation_metadata_ambiguous",
            Self::VersionUnavailable => "initial_package_observation_version_unavailable",
        }
    }

    fn live_code(self) -> &'static str {
        match self {
            Self::PathTimedOut | Self::MetadataTimedOut => "adb_preflight_timed_out",
            Self::PathCommandFailed => "adb_preflight_command_failed",
            Self::PathAmbiguous => "package_path_observation_ambiguous",
            Self::MetadataCommandFailed => "package_metadata_observation_failed",
            Self::MetadataAmbiguous => "package_metadata_observation_ambiguous",
            Self::VersionUnavailable => "package_version_code_unavailable",
        }
    }
}

/// Result of validating the single `pm path` response for the exact ARMSX1 package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PackagePathObservation {
    Absent,
    Installed,
}

fn package_version_observation(
    serial: &str,
    phase: PackageObservationPhase,
) -> Result<PackageObservation, String> {
    package_version_observation_with_probe(phase, |args| adb(serial, args))
}

fn package_version_observation_with_probe(
    phase: PackageObservationPhase,
    mut probe: impl FnMut(&[&str]) -> Result<AdbCommandResult, String>,
) -> Result<PackageObservation, String> {
    observe_initial_package_with_probe(phase, &mut probe)
        .map_err(|error| error.live_code().to_owned())
}

/// Reads the initial package state through an injected read-only probe.
///
/// A package is considered absent only when `pm path <package>` exits with status 1
/// and emits no output. Installed packages require one exact package line followed
/// by unambiguous metadata for that exact package identity.
fn observe_initial_package_with_probe(
    phase: PackageObservationPhase,
    probe: &mut impl FnMut(&[&str]) -> Result<AdbCommandResult, String>,
) -> Result<PackageObservation, PackageObservationError> {
    let path = probe(&["shell", "pm", "path", PACKAGE]).map_err(|error| match error.as_str() {
        "adb_preflight_timed_out" => PackageObservationError::PathTimedOut,
        _ => PackageObservationError::PathCommandFailed,
    })?;
    if classify_package_path(&path)? == PackagePathObservation::Absent {
        return Ok(PackageObservation {
            phase,
            package: PACKAGE.to_owned(),
            installed: false,
            version_code: None,
            version_name: None,
        });
    }

    let dump =
        probe(&["shell", "dumpsys", "package", PACKAGE]).map_err(|error| match error.as_str() {
            "adb_preflight_timed_out" => PackageObservationError::MetadataTimedOut,
            _ => PackageObservationError::MetadataCommandFailed,
        })?;
    let (version_code, version_name) = classify_package_metadata(&dump)?;
    Ok(PackageObservation {
        phase,
        package: PACKAGE.to_owned(),
        installed: true,
        version_code: Some(version_code),
        version_name,
    })
}

/// Classifies one `pm path` command result without inspecting raw error prose.
fn classify_package_path(
    output: &AdbCommandResult,
) -> Result<PackagePathObservation, PackageObservationError> {
    if output.returncode == 1 && output.stdout.is_empty() && output.stderr.is_empty() {
        return Ok(PackagePathObservation::Absent);
    }
    if output.returncode != 0 || !output.stderr.is_empty() {
        return Err(PackageObservationError::PathAmbiguous);
    }

    let line = output.stdout.strip_suffix('\n').unwrap_or(&output.stdout);
    if line.is_empty() || line.contains('\n') || line.contains('\r') || line.trim() != line {
        return Err(PackageObservationError::PathAmbiguous);
    }
    let Some(path) = line.strip_prefix("package:/") else {
        return Err(PackageObservationError::PathAmbiguous);
    };
    if path.is_empty()
        || path
            .chars()
            .any(|character| character.is_ascii_whitespace() || character.is_ascii_control())
    {
        return Err(PackageObservationError::PathAmbiguous);
    }
    Ok(PackagePathObservation::Installed)
}

/// Parses package metadata only when it contains one exact package record.
///
/// Prefix-colliding package records, duplicate fields, and malformed output are
/// rejected as ambiguous. The returned values are sanitized metadata, never raw
/// ADB diagnostics.
fn classify_package_metadata(
    output: &AdbCommandResult,
) -> Result<(String, Option<String>), PackageObservationError> {
    let stdout = output.stdout.trim();
    if output.returncode != 0 {
        return Err(if stdout.is_empty() && output.stderr.is_empty() {
            PackageObservationError::MetadataCommandFailed
        } else {
            PackageObservationError::MetadataAmbiguous
        });
    }
    if stdout.is_empty() || !output.stderr.is_empty() {
        return Err(PackageObservationError::MetadataAmbiguous);
    }

    let lines = stdout.lines().collect::<Vec<_>>();
    let headers = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| package_header_name(line).map(|name| (index, name)))
        .collect::<Vec<_>>();
    if headers.len() != 1 || headers[0].1 != PACKAGE {
        return Err(PackageObservationError::MetadataAmbiguous);
    }

    let block_start = headers[0].0 + 1;
    let block_end = lines[block_start..]
        .iter()
        .position(|line| package_header_name(line).is_some())
        .map_or(lines.len(), |offset| block_start + offset);
    let block = &lines[block_start..block_end];
    let version_code_count = block
        .iter()
        .map(|line| line.matches("versionCode=").count())
        .sum::<usize>();
    if version_code_count == 0 {
        return Err(PackageObservationError::VersionUnavailable);
    }
    if version_code_count != 1 {
        return Err(PackageObservationError::MetadataAmbiguous);
    }
    let Some(version_code) = block
        .iter()
        .flat_map(|line| line.split_whitespace())
        .find_map(|token| token.strip_prefix("versionCode="))
    else {
        return Err(PackageObservationError::MetadataAmbiguous);
    };
    if version_code.is_empty()
        || !version_code
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(PackageObservationError::MetadataAmbiguous);
    }
    let version_name_count = block
        .iter()
        .map(|line| line.matches("versionName=").count())
        .sum::<usize>();
    if version_name_count > 1 {
        return Err(PackageObservationError::MetadataAmbiguous);
    }
    let version_name = match version_name_count {
        0 => None,
        _ => Some(
            block
                .iter()
                .flat_map(|line| line.split_whitespace())
                .find_map(|token| token.strip_prefix("versionName="))
                .filter(|name| !name.is_empty())
                .ok_or(PackageObservationError::MetadataAmbiguous)?
                .to_owned(),
        ),
    };
    Ok((version_code.to_owned(), version_name))
}

/// Extracts the package name from a `dumpsys package` package header.
fn package_header_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("Package [")?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}

/// Refuse to replace an installation unless this run observed the package absent.
/// The repeated install is attempted only after this run's first install succeeds;
/// Android's package manager enforces signer compatibility on replacement.
fn admit_initial_install(package_installed_before_run: bool) -> Result<(), &'static str> {
    if package_installed_before_run {
        return Err("preexisting_package_not_owned_by_qualification");
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_avd_name_response(response: &str) -> Result<Option<&str>, &'static str> {
    if response.is_empty() {
        return Ok(None);
    }
    let lines = response.lines().collect::<Vec<_>>();
    match lines.as_slice() {
        [name, "OK"] if valid_avd_console_name(name) => Ok(Some(name)),
        _ => Err("emulator_avd_name_protocol_invalid"),
    }
}

fn valid_avd_console_name(name: &str) -> bool {
    if name.trim() != name
        || name
            .chars()
            .any(|character| character <= '\u{1f}' || character == '\u{7f}')
    {
        return false;
    }
    ["KO", "ERROR"].iter().all(|prefix| {
        let Some(remainder) = name
            .get(..prefix.len())
            .filter(|part| part.eq_ignore_ascii_case(prefix))
        else {
            return true;
        };
        let remainder = &name[remainder.len()..];
        !(remainder.is_empty()
            || remainder.starts_with(':')
            || remainder.chars().next().is_some_and(char::is_whitespace))
    })
}

fn parse_avd_name_console_result(
    result: Result<AdbCommandResult, String>,
) -> Result<Option<String>, &'static str> {
    let Ok(result) = result else {
        return Ok(None);
    };
    if result.stdout.is_empty() && result.stderr.is_empty() {
        return Ok(None);
    }
    if result.returncode != 0 || !result.stderr.is_empty() {
        return Err("emulator_avd_name_protocol_invalid");
    }
    parse_avd_name_response(&result.stdout).map(|name| name.map(str::to_owned))
}

fn parse_avd_name_property_response(response: &str) -> Result<&str, &'static str> {
    let response = response.strip_suffix('\n').unwrap_or(response);
    let response = response.strip_suffix('\r').unwrap_or(response);
    if response.is_empty()
        || response
            .chars()
            .any(|character| character == '\r' || character == '\n')
    {
        return Err("emulator_avd_name_property_invalid");
    }
    Ok(response)
}

fn parse_avd_name_property_result(
    result: Result<AdbCommandResult, String>,
) -> Result<String, &'static str> {
    let result = result.map_err(|_| "emulator_avd_name_property_unavailable")?;
    if result.returncode != 0 || !result.stderr.is_empty() {
        return Err("emulator_avd_name_property_unavailable");
    }
    parse_avd_name_property_response(&result.stdout).map(str::to_owned)
}

fn validate_device_profile(properties: &DeviceProperties) -> Result<(), &'static str> {
    if properties.state != "device" {
        return Err("identity_state_mismatch");
    }
    if properties.boot_completed != "1" {
        return Err("identity_boot_completed_mismatch");
    }
    if properties.api_level != "36" {
        return Err("identity_api_level_mismatch");
    }
    if properties.abi != "arm64-v8a" {
        return Err("identity_abi_mismatch");
    }
    if !properties.brand.eq_ignore_ascii_case("Google") {
        return Err("identity_brand_mismatch");
    }
    if properties.release != "16" {
        return Err("identity_android_release_mismatch");
    }
    if !properties
        .manufacturer
        .to_ascii_lowercase()
        .contains("google")
    {
        return Err("identity_manufacturer_mismatch");
    }
    if properties.model != "sdk_gphone64_arm64" {
        return Err("identity_model_mismatch");
    }
    if properties.emulator != "1" {
        return Err("identity_emulator_marker_mismatch");
    }
    if properties.uid != "2000" {
        return Err("identity_shell_uid_mismatch");
    }
    Ok(())
}

fn validate_device_properties(
    properties: &DeviceProperties,
    expected_avd_name: &str,
) -> Result<(), &'static str> {
    validate_device_profile(properties)?;
    match properties.avd_name.as_deref() {
        Some(actual) if actual == expected_avd_name => Ok(()),
        Some(_) => Err("designated_avd_name_mismatch"),
        None => Err("designated_avd_name_missing"),
    }
}

fn resolve_device_avd_name(
    properties: &DeviceProperties,
    console_result: Result<AdbCommandResult, String>,
    property_lookup: impl FnOnce() -> Result<AdbCommandResult, String>,
    expected_avd_name: &str,
) -> Result<String, &'static str> {
    validate_device_profile(properties)?;
    let name = match parse_avd_name_console_result(console_result)? {
        Some(name) => name,
        None => parse_avd_name_property_result(property_lookup())?,
    };
    if name != expected_avd_name {
        return Err("designated_avd_name_mismatch");
    }
    Ok(name)
}

/// Preserves the package observation boundary used by live device verification.
fn verify_package_observation(
    serial: &str,
    observe: impl FnOnce(&str, PackageObservationPhase) -> Result<PackageObservation, String>,
) -> Result<PackageObservation, String> {
    observe(serial, PackageObservationPhase::Before)
}

fn verify_device(
    identity: &LiveIdentity,
) -> Result<(String, PackageObservation, TargetDeviceBinding), String> {
    let mut properties = DeviceProperties {
        state: adb_text(&identity.serial, &["get-state"])?,
        boot_completed: adb_text(
            &identity.serial,
            &["shell", "getprop", "sys.boot_completed"],
        )?,
        api_level: adb_text(
            &identity.serial,
            &["shell", "getprop", "ro.build.version.sdk"],
        )?,
        abi: adb_text(
            &identity.serial,
            &["shell", "getprop", "ro.product.cpu.abi"],
        )?,
        brand: adb_text(&identity.serial, &["shell", "getprop", "ro.product.brand"])?,
        release: adb_text(
            &identity.serial,
            &["shell", "getprop", "ro.build.version.release"],
        )?,
        manufacturer: adb_text(
            &identity.serial,
            &["shell", "getprop", "ro.product.manufacturer"],
        )?,
        model: adb_text(&identity.serial, &["shell", "getprop", "ro.product.model"])?,
        emulator: adb_text(&identity.serial, &["shell", "getprop", "ro.kernel.qemu"])?,
        uid: adb_text(&identity.serial, &["shell", "id", "-u"])?,
        avd_name: None,
    };
    let avd_name = resolve_device_avd_name(
        &properties,
        adb(&identity.serial, &["emu", "avd", "name"]),
        || {
            adb(
                &identity.serial,
                &["shell", "getprop", "ro.boot.qemu.avd_name"],
            )
        },
        &identity.avd_name,
    )
    .map_err(str::to_owned)?;
    properties.avd_name = Some(avd_name);
    validate_device_properties(&properties, &identity.avd_name).map_err(str::to_owned)?;
    let android_api_level = properties
        .api_level
        .parse::<i64>()
        .map_err(|_| "emulator_identity_capability_or_non_root_check_failed")?;
    let target_device = TargetDeviceBinding {
        serial: identity.serial.clone(),
        manufacturer: Some(properties.manufacturer.clone()),
        model: Some(properties.model.clone()),
        android_api_level: Some(android_api_level),
    };
    let api_abi = format!("api={};abi={}", properties.api_level, properties.abi);
    let before = verify_package_observation(&identity.serial, package_version_observation)?;
    Ok((api_abi, before, target_device))
}

/// Exposes only read-only ADB probes to the preflight diagnostic.
trait PreflightAdb {
    fn probe(&mut self, serial: &str, args: &[&str]) -> Result<AdbCommandResult, String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadOnlyPreflightOperation {
    DeviceState,
    BootCompleted,
    ApiLevel,
    Abi,
    Brand,
    AndroidRelease,
    Manufacturer,
    Model,
    EmulatorMarker,
    ShellUid,
    ConsoleAvdName,
    PropertyAvdName,
    PackagePath,
    PackageMetadata,
}

fn read_only_preflight_operation(args: &[&str]) -> Option<ReadOnlyPreflightOperation> {
    match args {
        ["get-state"] => Some(ReadOnlyPreflightOperation::DeviceState),
        ["shell", "getprop", "sys.boot_completed"] => {
            Some(ReadOnlyPreflightOperation::BootCompleted)
        }
        ["shell", "getprop", "ro.build.version.sdk"] => Some(ReadOnlyPreflightOperation::ApiLevel),
        ["shell", "getprop", "ro.product.cpu.abi"] => Some(ReadOnlyPreflightOperation::Abi),
        ["shell", "getprop", "ro.product.brand"] => Some(ReadOnlyPreflightOperation::Brand),
        ["shell", "getprop", "ro.build.version.release"] => {
            Some(ReadOnlyPreflightOperation::AndroidRelease)
        }
        ["shell", "getprop", "ro.product.manufacturer"] => {
            Some(ReadOnlyPreflightOperation::Manufacturer)
        }
        ["shell", "getprop", "ro.product.model"] => Some(ReadOnlyPreflightOperation::Model),
        ["shell", "getprop", "ro.kernel.qemu"] => Some(ReadOnlyPreflightOperation::EmulatorMarker),
        ["shell", "id", "-u"] => Some(ReadOnlyPreflightOperation::ShellUid),
        ["emu", "avd", "name"] => Some(ReadOnlyPreflightOperation::ConsoleAvdName),
        ["shell", "getprop", "ro.boot.qemu.avd_name"] => {
            Some(ReadOnlyPreflightOperation::PropertyAvdName)
        }
        ["shell", "pm", "path", PACKAGE] => Some(ReadOnlyPreflightOperation::PackagePath),
        ["shell", "dumpsys", "package", PACKAGE] => {
            Some(ReadOnlyPreflightOperation::PackageMetadata)
        }
        _ => None,
    }
}

fn dispatch_read_only_preflight_probe(
    serial: &str,
    args: &[&str],
    dispatch: impl FnOnce(&str, &[&str]) -> Result<AdbCommandResult, String>,
) -> Result<AdbCommandResult, String> {
    if read_only_preflight_operation(args).is_none() {
        return Err("preflight_probe_not_allowlisted".to_owned());
    }
    dispatch(serial, args)
}

struct SystemPreflightAdb;

impl PreflightAdb for SystemPreflightAdb {
    fn probe(&mut self, serial: &str, args: &[&str]) -> Result<AdbCommandResult, String> {
        dispatch_read_only_preflight_probe(serial, args, adb)
    }
}

const DIAGNOSTIC_CHECK_NAMES: [&str; 14] = [
    "invocation",
    "identity_state",
    "identity_boot_completed",
    "identity_api_level",
    "identity_abi",
    "identity_brand",
    "identity_android_release",
    "identity_manufacturer",
    "identity_model",
    "identity_emulator_marker",
    "identity_shell_uid",
    "avd_name",
    "initial_package_observation",
    "package_admission",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DiagnosticCheckOutcome {
    check: &'static str,
    outcome: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PreflightDiagnosticEvidence {
    schema_version: u8,
    recorded_at_unix_seconds: u64,
    status: &'static str,
    preflight_status: &'static str,
    failure_category: Option<&'static str>,
    failure_code: Option<&'static str>,
    checks: Vec<DiagnosticCheckOutcome>,
    live_qualification: &'static str,
    planning: &'static str,
    release_resolution: &'static str,
    network: &'static str,
    apk_download: &'static str,
    installation: &'static str,
    package_mutation: &'static str,
}

impl PreflightDiagnosticEvidence {
    fn initial() -> Self {
        Self {
            schema_version: 1,
            recorded_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            status: "completed",
            preflight_status: "blocked",
            failure_category: None,
            failure_code: None,
            checks: DIAGNOSTIC_CHECK_NAMES
                .into_iter()
                .map(|check| DiagnosticCheckOutcome {
                    check,
                    outcome: "NOT_RUN",
                })
                .collect(),
            live_qualification: "unverified",
            planning: "NOT_RUN",
            release_resolution: "NOT_RUN",
            network: "NOT_RUN",
            apk_download: "NOT_RUN",
            installation: "NOT_RUN",
            package_mutation: "NOT_RUN",
        }
    }

    fn mark_check(&mut self, check: &'static str, outcome: &'static str) {
        if let Some(check_outcome) = self.checks.iter_mut().find(|item| item.check == check) {
            check_outcome.outcome = outcome;
        }
    }

    fn fail(&mut self, check: &'static str, category: &'static str, failure_code: &'static str) {
        self.mark_check(check, "failed");
        self.failure_category = Some(category);
        self.failure_code = Some(failure_code);
        self.preflight_status = "blocked";
    }
}

fn diagnostic_invocation_failure_code(error: &str) -> &'static str {
    match error {
        "explicit_opt_in_missing" => "invocation_opt_in_missing",
        "exact_serial_missing" => "invocation_serial_missing",
        "exact_emulator_serial_invalid" => "invocation_serial_invalid",
        "designated_avd_name_missing" => "invocation_avd_name_missing",
        "designated_avd_name_mismatch" => "invocation_avd_name_mismatch",
        "dedicated_avd_attestation_missing" => "invocation_attestation_missing",
        "package_allowlist_mismatch" => "invocation_package_allowlist_mismatch",
        _ => "invocation_invalid",
    }
}

type IdentityPropertySpec = (
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    fn(&str) -> bool,
);

fn diagnostic_identity_property(
    evidence: &mut PreflightDiagnosticEvidence,
    adb: &mut impl PreflightAdb,
    serial: &str,
    spec: IdentityPropertySpec,
) -> Result<String, &'static str> {
    let (args, check, read_failure_code, mismatch_code, _expected, valid) = spec;
    let result = adb.probe(serial, args);
    let value = match result {
        Ok(result) if result.returncode == 0 => result.stdout.trim().to_owned(),
        _ => {
            evidence.fail(check, "device_identity", read_failure_code);
            return Err(read_failure_code);
        }
    };
    if !valid(&value) {
        evidence.fail(check, "device_identity", mismatch_code);
        return Err(mismatch_code);
    }
    evidence.mark_check(check, "passed");
    Ok(value)
}

fn diagnostic_avd_name_failure(error: &'static str) -> (&'static str, &'static str) {
    match error {
        "emulator_avd_name_protocol_invalid" => {
            ("avd_name_verification", "avd_name_console_invalid")
        }
        "emulator_avd_name_property_unavailable" => {
            ("avd_name_verification", "avd_name_property_unavailable")
        }
        "emulator_avd_name_property_invalid" => {
            ("avd_name_verification", "avd_name_property_invalid")
        }
        "designated_avd_name_missing" => ("avd_name_verification", "avd_name_missing"),
        "designated_avd_name_mismatch" => ("avd_name_verification", "avd_name_mismatch"),
        _ => ("avd_name_verification", "avd_name_verification_failed"),
    }
}

fn diagnostic_initial_package_observation(
    adb: &mut impl PreflightAdb,
    serial: &str,
) -> Result<PackageObservation, &'static str> {
    observe_initial_package_with_probe(PackageObservationPhase::Before, &mut |args| {
        adb.probe(serial, args)
    })
    .map_err(PackageObservationError::diagnostic_code)
}

/// Gather sanitized outcomes using only the read-only `PreflightAdb` capability.
fn run_preflight_diagnostic_with(
    opt_in: Option<&str>,
    serial: Option<&str>,
    avd_name: Option<&str>,
    attestation: Option<&str>,
    allowlist: Option<&str>,
    adb: &mut impl PreflightAdb,
) -> PreflightDiagnosticEvidence {
    let mut evidence = PreflightDiagnosticEvidence::initial();
    let identity =
        match validate_diagnostic_invocation(opt_in, serial, avd_name, attestation, allowlist) {
            Ok(identity) => identity,
            Err(error) => {
                evidence.fail(
                    "invocation",
                    "invocation",
                    diagnostic_invocation_failure_code(error),
                );
                return evidence;
            }
        };
    evidence.mark_check("invocation", "passed");

    let property_specs: [IdentityPropertySpec; 10] = [
        (
            &["get-state"],
            "identity_state",
            "identity_state_read_failed",
            "identity_state_mismatch",
            "device",
            |value| value == "device",
        ),
        (
            &["shell", "getprop", "sys.boot_completed"],
            "identity_boot_completed",
            "identity_boot_completed_read_failed",
            "identity_boot_completed_mismatch",
            "1",
            |value| value == "1",
        ),
        (
            &["shell", "getprop", "ro.build.version.sdk"],
            "identity_api_level",
            "identity_api_level_read_failed",
            "identity_api_level_mismatch",
            "36",
            |value| value == "36",
        ),
        (
            &["shell", "getprop", "ro.product.cpu.abi"],
            "identity_abi",
            "identity_abi_read_failed",
            "identity_abi_mismatch",
            "arm64-v8a",
            |value| value == "arm64-v8a",
        ),
        (
            &["shell", "getprop", "ro.product.brand"],
            "identity_brand",
            "identity_brand_read_failed",
            "identity_brand_mismatch",
            "Google",
            |value| value.eq_ignore_ascii_case("Google"),
        ),
        (
            &["shell", "getprop", "ro.build.version.release"],
            "identity_android_release",
            "identity_android_release_read_failed",
            "identity_android_release_mismatch",
            "16",
            |value| value == "16",
        ),
        (
            &["shell", "getprop", "ro.product.manufacturer"],
            "identity_manufacturer",
            "identity_manufacturer_read_failed",
            "identity_manufacturer_mismatch",
            "google",
            |value| value.to_ascii_lowercase().contains("google"),
        ),
        (
            &["shell", "getprop", "ro.product.model"],
            "identity_model",
            "identity_model_read_failed",
            "identity_model_mismatch",
            "sdk_gphone64_arm64",
            |value| value == "sdk_gphone64_arm64",
        ),
        (
            &["shell", "getprop", "ro.kernel.qemu"],
            "identity_emulator_marker",
            "identity_emulator_marker_read_failed",
            "identity_emulator_marker_mismatch",
            "1",
            |value| value == "1",
        ),
        (
            &["shell", "id", "-u"],
            "identity_shell_uid",
            "identity_shell_uid_read_failed",
            "identity_shell_uid_mismatch",
            "2000",
            |value| value == "2000",
        ),
    ];
    let mut observed_values = Vec::with_capacity(property_specs.len());
    for spec in property_specs {
        match diagnostic_identity_property(&mut evidence, adb, &identity.serial, spec) {
            Ok(value) => observed_values.push(value),
            Err(_) => return evidence,
        }
    }
    let mut properties = DeviceProperties {
        state: observed_values[0].clone(),
        boot_completed: observed_values[1].clone(),
        api_level: observed_values[2].clone(),
        abi: observed_values[3].clone(),
        brand: observed_values[4].clone(),
        release: observed_values[5].clone(),
        manufacturer: observed_values[6].clone(),
        model: observed_values[7].clone(),
        emulator: observed_values[8].clone(),
        uid: observed_values[9].clone(),
        avd_name: None,
    };

    let console_result = adb.probe(&identity.serial, &["emu", "avd", "name"]);
    let avd_name = match resolve_device_avd_name(
        &properties,
        console_result,
        || {
            adb.probe(
                &identity.serial,
                &["shell", "getprop", "ro.boot.qemu.avd_name"],
            )
        },
        &identity.avd_name,
    ) {
        Ok(avd_name) => avd_name,
        Err(error) => {
            let (category, code) = diagnostic_avd_name_failure(error);
            evidence.fail("avd_name", category, code);
            return evidence;
        }
    };
    properties.avd_name = Some(avd_name);
    evidence.mark_check("avd_name", "passed");

    match diagnostic_initial_package_observation(adb, &identity.serial) {
        Ok(observation) => {
            evidence.mark_check("initial_package_observation", "passed");
            if admit_initial_install(observation.installed).is_err() {
                evidence.fail(
                    "package_admission",
                    "package_admission",
                    "initial_package_preexisting",
                );
                return evidence;
            }
            evidence.mark_check("package_admission", "passed");
        }
        Err(code) => {
            evidence.fail(
                "initial_package_observation",
                "initial_package_observation",
                code,
            );
            return evidence;
        }
    }

    evidence.preflight_status = "passed";
    evidence
}

fn persist_diagnostic_evidence(
    path: &Path,
    evidence: &PreflightDiagnosticEvidence,
) -> Result<(), &'static str> {
    write_evidence_atomic(path, evidence)
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate is nested in the repository")
        .to_path_buf()
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target)?;
        } else {
            fs::copy(path, target)?;
        }
    }
    Ok(())
}

fn live_plan(target_device: TargetDeviceBinding) -> Result<LivePlanPreparation, OutcomeCategory> {
    use crate::catalog_source::CatalogSnapshot;
    use crate::model::OrderedMap;
    use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

    let temporary = tempfile::tempdir().map_err(|_| OutcomeCategory::UnsafePlan)?;
    let authored = temporary.path().join("authored");
    copy_tree(&repository_root().join("authored"), &authored)
        .map_err(|_| OutcomeCategory::UnsafePlan)?;
    let (app_definition_sha256, recipe_sha256) = read_authored_source_hashes(&authored)?;

    let plans = authored.join("device_plans");
    fs::create_dir_all(&plans).map_err(|_| OutcomeCategory::UnsafePlan)?;
    fs::write(
        plans.join("qualification.armsx1.google-api36.yaml"),
        r#"schema_version: 1
kind: device_plan
id: qualification.armsx1.google-api36
name: ARMSX1 Google API 36 qualification
description: Test-only plan for one dedicated Android ARM64 emulator.
device_profile_ref: google.sdk_gphone64_arm64
recipes: []
defaults:
  show_advanced_steps: false
"#,
    )
    .map_err(|_| OutcomeCategory::UnsafePlan)?;
    let catalog =
        CatalogSnapshot::legacy_local(authored).map_err(|_| OutcomeCategory::UnsafePlan)?;
    let prepared = plan_configuration(ConfigurationContextRequest {
        catalog,
        configuration_root: None,
        user_configuration: None,
        device_plan: Some("qualification.armsx1.google-api36".to_owned()),
        selected_recipes: Some(vec!["app.armsx1.install".to_owned()]),
        explicit_bindings: OrderedMap::new(),
        device_context: None,
        target_device: Some(target_device),
        runtime_capability_availability: None,
    })
    .map_err(|_| OutcomeCategory::UnsafePlan)?;
    Ok(LivePlanPreparation {
        plan: prepared.plan,
        review: prepared.review,
        plan_digest: prepared.plan_digest,
        app_definition_sha256,
        recipe_sha256,
    })
}

fn load_source_hash_contract() -> Result<SourceHashContract, OutcomeCategory> {
    let contract = fs::read(contract_path()).map_err(|_| OutcomeCategory::ContractDrift)?;
    let contract: serde_json::Value =
        serde_json::from_slice(&contract).map_err(|_| OutcomeCategory::ContractDrift)?;
    let app_definition = contract
        .get("appDefinition")
        .ok_or(OutcomeCategory::ContractDrift)?;
    let app_definition_sha256 = app_definition
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or(OutcomeCategory::ContractDrift)?
        .to_owned();
    let recipe_sha256 = contract
        .get("authoredSource")
        .and_then(|recipe| recipe.get("sha256"))
        .and_then(serde_json::Value::as_str)
        .ok_or(OutcomeCategory::ContractDrift)?
        .to_owned();
    let expected_apk_sha256 = contract
        .get("release")
        .and_then(|release| release.get("expectedApkSha256"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| is_sha256(value))
        .ok_or(OutcomeCategory::ContractDrift)?
        .to_ascii_lowercase();
    let app_string = |field: &str| {
        app_definition
            .get(field)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or(OutcomeCategory::ContractDrift)
    };
    let include_prereleases = app_definition
        .get("prerelease")
        .and_then(serde_json::Value::as_bool)
        .ok_or(OutcomeCategory::ContractDrift)?;
    // The authored source omits this optional field, which means inversion is disabled.
    let invert_asset_pattern = match app_definition.get("invertAssetPattern") {
        None => false,
        Some(value) => value.as_bool().ok_or(OutcomeCategory::ContractDrift)?,
    };
    let artifact_cache = contract
        .get("artifact")
        .and_then(|artifact| artifact.get("cache"))
        .and_then(serde_json::Value::as_str)
        .ok_or(OutcomeCategory::ContractDrift)?
        .to_owned();
    Ok(SourceHashContract {
        app_definition_sha256,
        recipe_sha256,
        expected_apk_sha256,
        source_strategy: app_string("sourceStrategy")?,
        provider: app_string("provider")?,
        service_origin: app_string("baseUrl")?,
        repository: app_string("repository")?,
        include_prereleases,
        asset_pattern: app_string("assetPattern")?,
        invert_asset_pattern,
        artifact_cache,
    })
}

fn read_authored_source_hashes(authored: &Path) -> Result<(String, String), OutcomeCategory> {
    let app =
        fs::read(authored.join("apps/armsx1.yaml")).map_err(|_| OutcomeCategory::ContractDrift)?;
    let recipe = fs::read(authored.join("recipes/app.armsx1.install.yaml"))
        .map_err(|_| OutcomeCategory::ContractDrift)?;
    Ok((sha256_hex(&app), sha256_hex(&recipe)))
}

fn admit_live_mutation(
    contract: &SourceHashContract,
    app_definition_sha256: &str,
    recipe_sha256: &str,
    package_allowlist: Option<&str>,
    plan: &ExecutionPlan,
    review: &ReviewProjection,
) -> Result<(), OutcomeCategory> {
    if app_definition_sha256 != contract.app_definition_sha256
        || recipe_sha256 != contract.recipe_sha256
    {
        return Err(OutcomeCategory::ContractDrift);
    }

    if package_allowlist != Some(PACKAGE)
        || plan.apps.len() != 1
        || plan.apps[0].id != "armsx1"
        || plan.apps[0].package_id != PACKAGE
    {
        return Err(OutcomeCategory::MutationTarget);
    }

    let replace_existing = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with(EXPECTED_INSTALL_STEP_SUFFIX))
        .and_then(|step| step.params.get("replace_existing"));
    if replace_existing
        == Some(&ExecutionParamValue::Literal {
            value: serde_json::Value::Bool(false),
        })
    {
        return Err(OutcomeCategory::ReplaceExistingDisabled);
    }

    if plan.steps.len() != 2 || plan.artifacts.len() != 1 {
        return Err(OutcomeCategory::UnsafePlan);
    }
    let resolve = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with(EXPECTED_RESOLVE_STEP_SUFFIX));
    let install = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with(EXPECTED_INSTALL_STEP_SUFFIX));
    let (Some(resolve), Some(install)) = (resolve, install) else {
        return Err(OutcomeCategory::UnsafePlan);
    };
    if resolve.id != format!("{EXPECTED_RECIPE}{EXPECTED_RESOLVE_STEP_SUFFIX}")
        || install.id != format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}")
        || resolve.recipe_ref != EXPECTED_RECIPE
        || resolve.type_name != "resolve_artifacts"
        || !resolve.dependencies.is_empty()
        || install.recipe_ref != EXPECTED_RECIPE
        || install.type_name != "install_apk"
        || install.app_id.as_deref() != Some("armsx1")
        || install.dependencies.len() != 1
        || install.dependencies[0] != resolve.id
        || !install.skip_if.is_empty()
        || install.params.get("app")
            != Some(&ExecutionParamValue::Ref {
                ref_value: format!("artifacts.{EXPECTED_RECIPE}/installer.local_path"),
            })
        || replace_existing
            != Some(&ExecutionParamValue::Literal {
                value: serde_json::Value::Bool(true),
            })
    {
        return Err(OutcomeCategory::UnsafePlan);
    }

    let artifact = &plan.artifacts[0];
    if artifact.id != format!("{EXPECTED_RECIPE}/installer")
        || artifact.type_name != "remote_file"
        || artifact.app_provenance.as_ref().is_none_or(|provenance| {
            provenance.app_id != "armsx1" || provenance.artifact_id != "apk"
        })
    {
        return Err(OutcomeCategory::UnsafePlan);
    }
    validate_release_source_contract(contract, artifact)?;

    let Some(feature) = review.features.first() else {
        return Err(OutcomeCategory::UnsafePlan);
    };
    let sections = &feature.sections;
    let has_exact_download = sections
        .iter()
        .filter(|section| section.kind == "downloads" && section.actions.len() == 1)
        .count()
        == 1;
    let has_exact_install = sections
        .iter()
        .filter(|section| section.kind == "installs" && section.actions.len() == 1)
        .count()
        == 1;
    let exact_review = review.can_execute
        && review.inputs.is_empty()
        && review.work.action_count == 2
        && review.features.len() == 1
        && feature.name == "Install or update ARMSX1"
        && !feature.automatically_added
        && has_exact_download
        && has_exact_install
        && sections.iter().all(|section| match section.kind {
            "downloads" | "installs" => section.actions.len() == 1,
            _ => section.actions.is_empty(),
        });
    if !exact_review {
        return Err(OutcomeCategory::UnsafePlan);
    }
    Ok(())
}

fn validate_release_source_contract(
    contract: &SourceHashContract,
    artifact: &crate::planner::ExecutionArtifact,
) -> Result<(), OutcomeCategory> {
    if contract.source_strategy != "latest_release"
        || contract.provider != "github"
        || contract.service_origin != "https://github.com"
        || contract.repository != "ARMSX2/ARMSX1"
        || contract.include_prereleases
        || contract.asset_pattern
            != r"^ARMSX1-(?:release-[0-9]{8}-arm64-v8a|[0-9]+\.[0-9]+\.[0-9]+)\.apk$"
        || contract.invert_asset_pattern
        || contract.artifact_cache != "default"
        || artifact.cache != contract.artifact_cache
    {
        return Err(OutcomeCategory::ContractDrift);
    }

    let source_matches_contract = match &artifact.source {
        ExecutionArtifactSource::RemoteRelease {
            provider,
            service_origin,
            repository,
            include_prereleases,
            asset_pattern,
            invert_asset_pattern,
        } => {
            provider == &contract.provider
                && service_origin == &contract.service_origin
                && repository == &contract.repository
                && include_prereleases == &contract.include_prereleases
                && asset_pattern.as_deref() == Some(contract.asset_pattern.as_str())
                && invert_asset_pattern == &contract.invert_asset_pattern
        }
        _ => false,
    };

    if source_matches_contract {
        Ok(())
    } else {
        Err(OutcomeCategory::ContractDrift)
    }
}

fn validate_fresh_isolated_cache(
    contract: &SourceHashContract,
    artifact: &crate::planner::ExecutionArtifact,
    isolated_root: &Path,
    cache_root: &Path,
) -> Result<(), OutcomeCategory> {
    let root_is_directory =
        fs::symlink_metadata(isolated_root).is_ok_and(|metadata| metadata.file_type().is_dir());
    let cache_is_absent = matches!(
        fs::symlink_metadata(cache_root),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    );
    if !root_is_directory
        || !isolated_root.is_absolute()
        || cache_root != isolated_root.join("cache")
        || !cache_is_absent
        || artifact.cache != contract.artifact_cache
        || contract.artifact_cache != "default"
    {
        return Err(OutcomeCategory::ContractDrift);
    }
    Ok(())
}

fn admit_live_runner(
    contract: &SourceHashContract,
    app_definition_sha256: &str,
    recipe_sha256: &str,
    package_allowlist: Option<&str>,
    plan: &ExecutionPlan,
    review: &ReviewProjection,
    cache_roots: IsolatedCacheRoots<'_>,
) -> Result<(), OutcomeCategory> {
    admit_live_mutation(
        contract,
        app_definition_sha256,
        recipe_sha256,
        package_allowlist,
        plan,
        review,
    )?;
    validate_fresh_isolated_cache(
        contract,
        &plan.artifacts[0],
        cache_roots.runtime,
        cache_roots.cache,
    )
}

#[test]
fn live_preflight_rejects_missing_or_mismatched_operator_inputs() {
    assert_eq!(
        validate_invocation(None, None, None, None, None),
        Err("explicit_opt_in_missing")
    );
    assert_eq!(
        validate_invocation(
            Some("1"),
            Some("emulator-5554"),
            Some(EXPECTED_AVD),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE)
        ),
        Ok(LiveIdentity {
            serial: "emulator-5554".into(),
            avd_name: EXPECTED_AVD.into()
        })
    );
    assert_eq!(
        validate_invocation(
            Some("1"),
            Some("emulator-5554"),
            Some(""),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE)
        ),
        Err("designated_avd_name_missing")
    );
    assert_eq!(
        validate_invocation(
            Some("1"),
            Some("emulator-5554"),
            Some("other"),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE)
        ),
        Err("designated_avd_name_mismatch")
    );
    assert_eq!(
        validate_invocation(
            Some("1"),
            Some("emulator-5554"),
            Some(EXPECTED_AVD),
            None,
            Some(PACKAGE)
        ),
        Err("dedicated_avd_attestation_missing")
    );
    assert_eq!(
        validate_invocation(
            Some("1"),
            Some("emulator-5554"),
            Some(EXPECTED_AVD),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some("com.other.app")
        ),
        Err("package_allowlist_mismatch")
    );
}

fn admission_fixture() -> (
    SourceHashContract,
    String,
    String,
    ExecutionPlan,
    ReviewProjection,
) {
    let contract = load_source_hash_contract().expect("fixture contract hashes should load");
    let target = TargetDeviceBinding {
        serial: "emulator-5554".to_owned(),
        manufacturer: Some("Google".to_owned()),
        model: Some("sdk_gphone64_arm64".to_owned()),
        android_api_level: Some(36),
    };
    let prepared =
        live_plan(target.clone()).expect("offline production planner should prepare the plan");
    let app_hash = prepared.app_definition_sha256.clone();
    let recipe_hash = prepared.recipe_sha256.clone();
    let plan = prepared
        .plan
        .expect("offline qualification plan should exist");
    let review = prepared
        .review
        .expect("offline qualification review should exist");
    assert_eq!(plan.target_device, Some(target));
    (contract, app_hash, recipe_hash, plan, review)
}

#[test]
fn live_plan_carries_the_preflight_target_for_executor_identity_guard() {
    let target = TargetDeviceBinding {
        serial: "emulator-5554".to_owned(),
        manufacturer: Some("Google".to_owned()),
        model: Some("sdk_gphone64_arm64".to_owned()),
        android_api_level: Some(36),
    };
    let prepared =
        live_plan(target.clone()).expect("offline production planner should prepare the plan");
    let plan = prepared
        .plan
        .expect("offline qualification plan should exist");

    assert_eq!(plan.target_device, Some(target));
}

fn valid_device_properties() -> DeviceProperties {
    DeviceProperties {
        state: "device".to_owned(),
        boot_completed: "1".to_owned(),
        api_level: "36".to_owned(),
        abi: "arm64-v8a".to_owned(),
        brand: "Google".to_owned(),
        release: "16".to_owned(),
        manufacturer: "Google".to_owned(),
        model: "sdk_gphone64_arm64".to_owned(),
        emulator: "1".to_owned(),
        uid: "2000".to_owned(),
        avd_name: Some(EXPECTED_AVD.to_owned()),
    }
}

#[derive(Default)]
struct FakePreflightAdb {
    calls: Vec<FakePreflightAdbCall>,
    overrides: Vec<(Vec<String>, Result<AdbCommandResult, String>)>,
    package_installed: bool,
}

struct FakePreflightAdbCall {
    serial: String,
    args: Vec<String>,
}

impl FakePreflightAdb {
    fn override_response(&mut self, args: &[&str], returncode: i32, stdout: &str, stderr: &str) {
        self.overrides.push((
            args.iter().map(|arg| (*arg).to_owned()).collect(),
            Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode,
                stdout: stdout.to_owned(),
                stderr: stderr.to_owned(),
            }),
        ));
    }

    fn override_error(&mut self, args: &[&str], error: &str) {
        self.overrides.push((
            args.iter().map(|arg| (*arg).to_owned()).collect(),
            Err(error.to_owned()),
        ));
    }

    fn response(&self, args: &[&str]) -> Result<AdbCommandResult, String> {
        let owned_args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        if let Some((_, response)) = self
            .overrides
            .iter()
            .find(|(expected, _)| *expected == owned_args)
        {
            return response.clone();
        }

        let output = match args {
            ["get-state"] => "device".to_owned(),
            ["shell", "getprop", "sys.boot_completed"] => "1".to_owned(),
            ["shell", "getprop", "ro.build.version.sdk"] => "36".to_owned(),
            ["shell", "getprop", "ro.product.cpu.abi"] => "arm64-v8a".to_owned(),
            ["shell", "getprop", "ro.product.brand"] => "Google".to_owned(),
            ["shell", "getprop", "ro.build.version.release"] => "16".to_owned(),
            ["shell", "getprop", "ro.product.manufacturer"] => "Google".to_owned(),
            ["shell", "getprop", "ro.product.model"] => "sdk_gphone64_arm64".to_owned(),
            ["shell", "getprop", "ro.kernel.qemu"] => "1".to_owned(),
            ["shell", "id", "-u"] => "2000".to_owned(),
            ["emu", "avd", "name"] => format!("{EXPECTED_AVD}\nOK\n"),
            ["shell", "getprop", "ro.boot.qemu.avd_name"] => EXPECTED_AVD.to_owned(),
            ["shell", "pm", "path", PACKAGE] if self.package_installed => {
                "package:/data/app/opaque/base.apk".to_owned()
            }
            ["shell", "pm", "path", PACKAGE] => String::new(),
            ["shell", "dumpsys", "package", PACKAGE] => {
                format!(
                    "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
                )
            }
            _ => return Err("fake adb received an unapproved command".to_owned()),
        };
        Ok(AdbCommandResult {
            args: owned_args,
            returncode: if args == ["shell", "pm", "path", PACKAGE] && !self.package_installed {
                1
            } else {
                0
            },
            stdout: output,
            stderr: String::new(),
        })
    }
}

impl PreflightAdb for FakePreflightAdb {
    fn probe(&mut self, serial: &str, args: &[&str]) -> Result<AdbCommandResult, String> {
        self.calls.push(FakePreflightAdbCall {
            serial: serial.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        });
        self.response(args)
    }
}

fn run_fake_preflight_diagnostic(
    adb: &mut FakePreflightAdb,
    avd_name: Option<&str>,
) -> PreflightDiagnosticEvidence {
    run_fake_preflight_diagnostic_for_serial(adb, avd_name, Some("emulator-5554"))
}

fn run_fake_preflight_diagnostic_for_serial(
    adb: &mut FakePreflightAdb,
    avd_name: Option<&str>,
    serial: Option<&str>,
) -> PreflightDiagnosticEvidence {
    run_preflight_diagnostic_with(
        Some("1"),
        serial,
        avd_name,
        Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
        Some(PACKAGE),
        adb,
    )
}

fn assert_fake_preflight_only_used_read_only_commands(adb: &FakePreflightAdb) {
    for call in &adb.calls {
        assert_eq!(call.serial, "emulator-5554");
        let args = call.args.iter().map(String::as_str).collect::<Vec<_>>();
        assert!(
            read_only_preflight_operation(&args).is_some(),
            "diagnostic issued a non-read-only command: {args:?}"
        );
    }
}

#[test]
fn read_only_preflight_allowlist_rejects_mutation_commands_before_dispatch() {
    let mut dispatched = false;
    for args in [
        &["install", "/tmp/armsx.apk"][..],
        &["uninstall", PACKAGE],
        &["shell", "pm", "clear", PACKAGE],
        &["shell", "rm", "-f", "/data/local/tmp/armsx.apk"],
        &["shell", "input", "tap", "10", "10"],
    ] {
        assert!(read_only_preflight_operation(args).is_none());
        let result = dispatch_read_only_preflight_probe("emulator-5554", args, |_, _| {
            dispatched = true;
            Ok(AdbCommandResult {
                args: Vec::new(),
                returncode: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        });
        assert_eq!(result, Err("preflight_probe_not_allowlisted".to_owned()));
    }
    assert!(!dispatched, "mutation-class requests reached ADB dispatch");
}

#[test]
fn read_only_preflight_allowlist_dispatches_only_defined_probes() {
    let allowed: [&[&str]; 14] = [
        &["get-state"],
        &["shell", "getprop", "sys.boot_completed"],
        &["shell", "getprop", "ro.build.version.sdk"],
        &["shell", "getprop", "ro.product.cpu.abi"],
        &["shell", "getprop", "ro.product.brand"],
        &["shell", "getprop", "ro.build.version.release"],
        &["shell", "getprop", "ro.product.manufacturer"],
        &["shell", "getprop", "ro.product.model"],
        &["shell", "getprop", "ro.kernel.qemu"],
        &["shell", "id", "-u"],
        &["emu", "avd", "name"],
        &["shell", "getprop", "ro.boot.qemu.avd_name"],
        &["shell", "pm", "path", PACKAGE],
        &["shell", "dumpsys", "package", PACKAGE],
    ];

    for args in allowed {
        assert!(read_only_preflight_operation(args).is_some());
        let mut dispatched = false;
        let result = dispatch_read_only_preflight_probe("emulator-5554", args, |serial, actual| {
            dispatched = true;
            assert_eq!(serial, "emulator-5554");
            assert_eq!(actual, args);
            Ok(AdbCommandResult {
                args: actual
                    .iter()
                    .map(|argument| (*argument).to_owned())
                    .collect(),
                returncode: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        });

        assert!(dispatched);
        assert_eq!(
            result
                .expect("allowlisted read-only probe should dispatch")
                .returncode,
            0
        );
    }
}

#[test]
fn live_invocation_validator_rejects_any_serial_except_emulator_5554() {
    for serial in ["emulator-5556", "emulator-5558"] {
        assert_eq!(
            validate_invocation(
                Some("1"),
                Some(serial),
                Some(EXPECTED_AVD),
                Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
                Some(PACKAGE),
            ),
            Err("exact_emulator_serial_invalid"),
            "{serial} must be rejected"
        );
    }
}

#[test]
fn diagnostic_rejects_a_different_emulator_serial_before_any_probe() {
    let mut adb = FakePreflightAdb::default();
    let evidence = run_fake_preflight_diagnostic_for_serial(
        &mut adb,
        Some(EXPECTED_AVD),
        Some("emulator-5556"),
    );

    assert_eq!(evidence.failure_code, Some("invocation_serial_invalid"));
    assert_eq!(evidence.failure_category, Some("invocation"));
    assert!(adb.calls.is_empty(), "rejected serial must not reach ADB");
}

#[test]
fn read_only_diagnostic_classifies_missing_and_mismatched_avd_input_without_adb() {
    let cases = [
        (
            None,
            Some("emulator-5554"),
            Some(EXPECTED_AVD),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE),
            "invocation_opt_in_missing",
        ),
        (
            Some("1"),
            None,
            Some(EXPECTED_AVD),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE),
            "invocation_serial_missing",
        ),
        (
            Some("1"),
            Some("phone-1"),
            Some(EXPECTED_AVD),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE),
            "invocation_serial_invalid",
        ),
        (
            Some("1"),
            Some("emulator-5554"),
            None,
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE),
            "invocation_avd_name_missing",
        ),
        (
            Some("1"),
            Some("emulator-5554"),
            Some(""),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE),
            "invocation_avd_name_missing",
        ),
        (
            Some("1"),
            Some("emulator-5554"),
            Some("other-avd"),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some(PACKAGE),
            "invocation_avd_name_mismatch",
        ),
        (
            Some("1"),
            Some("emulator-5554"),
            Some(EXPECTED_AVD),
            None,
            Some(PACKAGE),
            "invocation_attestation_missing",
        ),
        (
            Some("1"),
            Some("emulator-5554"),
            Some(EXPECTED_AVD),
            Some("I_ATTEST_THIS_AVD_IS_DISPOSABLE_OR_EXCLUSIVELY_DEDICATED"),
            Some("com.other.app"),
            "invocation_package_allowlist_mismatch",
        ),
    ];
    for (opt_in, serial, avd_name, attestation, allowlist, expected_code) in cases {
        let mut adb = FakePreflightAdb::default();
        let evidence = run_preflight_diagnostic_with(
            opt_in,
            serial,
            avd_name,
            attestation,
            allowlist,
            &mut adb,
        );
        assert_eq!(evidence.failure_code, Some(expected_code));
        assert_eq!(evidence.failure_category, Some("invocation"));
        assert!(adb.calls.is_empty());
    }
}

#[test]
fn read_only_diagnostic_classifies_each_device_identity_mismatch() {
    let cases: [(&[&str], &str, &str); 10] = [
        (&["get-state"], "offline", "identity_state_mismatch"),
        (
            &["shell", "getprop", "sys.boot_completed"],
            "0",
            "identity_boot_completed_mismatch",
        ),
        (
            &["shell", "getprop", "ro.build.version.sdk"],
            "35",
            "identity_api_level_mismatch",
        ),
        (
            &["shell", "getprop", "ro.product.cpu.abi"],
            "x86_64",
            "identity_abi_mismatch",
        ),
        (
            &["shell", "getprop", "ro.product.brand"],
            "Samsung",
            "identity_brand_mismatch",
        ),
        (
            &["shell", "getprop", "ro.build.version.release"],
            "15",
            "identity_android_release_mismatch",
        ),
        (
            &["shell", "getprop", "ro.product.manufacturer"],
            "Other",
            "identity_manufacturer_mismatch",
        ),
        (
            &["shell", "getprop", "ro.product.model"],
            "other-model",
            "identity_model_mismatch",
        ),
        (
            &["shell", "getprop", "ro.kernel.qemu"],
            "0",
            "identity_emulator_marker_mismatch",
        ),
        (&["shell", "id", "-u"], "0", "identity_shell_uid_mismatch"),
    ];

    for (command, output, expected_code) in cases {
        let mut adb = FakePreflightAdb::default();
        adb.override_response(command, 0, output, "");
        let evidence = run_fake_preflight_diagnostic(&mut adb, Some(EXPECTED_AVD));
        assert_eq!(evidence.failure_code, Some(expected_code));
        assert_eq!(evidence.failure_category, Some("device_identity"));
        assert_fake_preflight_only_used_read_only_commands(&adb);
    }
}

#[test]
fn read_only_diagnostic_classifies_each_identity_read_failure_without_retaining_diagnostics() {
    let cases: [(&[&str], &str); 10] = [
        (&["get-state"], "identity_state_read_failed"),
        (
            &["shell", "getprop", "sys.boot_completed"],
            "identity_boot_completed_read_failed",
        ),
        (
            &["shell", "getprop", "ro.build.version.sdk"],
            "identity_api_level_read_failed",
        ),
        (
            &["shell", "getprop", "ro.product.cpu.abi"],
            "identity_abi_read_failed",
        ),
        (
            &["shell", "getprop", "ro.product.brand"],
            "identity_brand_read_failed",
        ),
        (
            &["shell", "getprop", "ro.build.version.release"],
            "identity_android_release_read_failed",
        ),
        (
            &["shell", "getprop", "ro.product.manufacturer"],
            "identity_manufacturer_read_failed",
        ),
        (
            &["shell", "getprop", "ro.product.model"],
            "identity_model_read_failed",
        ),
        (
            &["shell", "getprop", "ro.kernel.qemu"],
            "identity_emulator_marker_read_failed",
        ),
        (&["shell", "id", "-u"], "identity_shell_uid_read_failed"),
    ];

    for (command, expected_code) in cases {
        let mut adb = FakePreflightAdb::default();
        adb.override_response(
            command,
            1,
            "",
            "serial=emulator-5554 token=private-test-token",
        );
        let evidence = run_fake_preflight_diagnostic(&mut adb, Some(EXPECTED_AVD));
        assert_eq!(evidence.failure_code, Some(expected_code));
        assert_eq!(evidence.failure_category, Some("device_identity"));
        let serialized = serde_json::to_string(&evidence).expect("evidence should serialize");
        for secret in [
            "emulator-5554",
            EXPECTED_AVD,
            "private-test-token",
            "serial=",
        ] {
            assert!(!serialized.contains(secret));
        }
        assert_fake_preflight_only_used_read_only_commands(&adb);
    }
}

#[test]
fn diagnostic_rejects_successful_package_path_with_stderr() {
    const SECRET: &str = "synthetic-stderr-secret-9f3c";
    for stderr in [SECRET, " \t\n"] {
        let mut adb = FakePreflightAdb {
            package_installed: true,
            ..FakePreflightAdb::default()
        };
        adb.override_response(
            &["shell", "pm", "path", PACKAGE],
            0,
            "package:/data/app/opaque/base.apk",
            stderr,
        );

        let evidence = run_fake_preflight_diagnostic(&mut adb, Some(EXPECTED_AVD));

        assert_eq!(
            evidence.failure_code,
            Some("initial_package_observation_path_ambiguous")
        );
        let serialized = serde_json::to_string(&evidence)
            .expect("sanitized diagnostic evidence should serialize");
        assert!(!serialized.contains(SECRET));
        assert_fake_preflight_only_used_read_only_commands(&adb);
    }
}

#[test]
fn diagnostic_rejects_successful_package_metadata_with_stderr() {
    const SECRET: &str = "synthetic-stderr-secret-9f3c";
    for stderr in [SECRET, " \t\n"] {
        let mut adb = FakePreflightAdb {
            package_installed: true,
            ..FakePreflightAdb::default()
        };
        adb.override_response(
            &["shell", "dumpsys", "package", PACKAGE],
            0,
            &format!(
                "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
            ),
            stderr,
        );

        let evidence = run_fake_preflight_diagnostic(&mut adb, Some(EXPECTED_AVD));

        assert_eq!(
            evidence.failure_code,
            Some("initial_package_observation_metadata_ambiguous")
        );
        let serialized = serde_json::to_string(&evidence)
            .expect("sanitized diagnostic evidence should serialize");
        assert!(!serialized.contains(SECRET));
        assert_fake_preflight_only_used_read_only_commands(&adb);
    }
}

#[test]
fn live_package_observation_rejects_successful_commands_with_stderr() {
    const SECRET: &str = "synthetic-stderr-secret-9f3c";
    for stderr in [SECRET, " \t\n"] {
        let mut path_calls = 0_u8;
        let path_observation = package_version_observation_with_probe(
            PackageObservationPhase::Before,
            |args| {
                path_calls += 1;
                match path_calls {
                    1 => {
                        assert_eq!(args, &["shell", "pm", "path", PACKAGE]);
                        Ok(AdbCommandResult {
                            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                            returncode: 0,
                            stdout: "package:/data/app/opaque/base.apk".to_owned(),
                            stderr: stderr.to_owned(),
                        })
                    }
                    2 => {
                        assert_eq!(args, &["shell", "dumpsys", "package", PACKAGE]);
                        Ok(AdbCommandResult {
                            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                            returncode: 0,
                            stdout: format!(
                                "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
                            ),
                            stderr: String::new(),
                        })
                    }
                    _ => panic!("package observation issued an unexpected command"),
                }
            },
        );
        assert_eq!(
            path_observation,
            Err("package_path_observation_ambiguous".to_owned())
        );
        assert_eq!(path_calls, 1);

        let mut metadata_calls = 0_u8;
        let metadata_observation = package_version_observation_with_probe(
            PackageObservationPhase::Before,
            |args| {
                metadata_calls += 1;
                match metadata_calls {
                    1 => {
                        assert_eq!(args, &["shell", "pm", "path", PACKAGE]);
                        Ok(AdbCommandResult {
                            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                            returncode: 0,
                            stdout: "package:/data/app/opaque/base.apk".to_owned(),
                            stderr: String::new(),
                        })
                    }
                    2 => {
                        assert_eq!(args, &["shell", "dumpsys", "package", PACKAGE]);
                        Ok(AdbCommandResult {
                            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                            returncode: 0,
                            stdout: format!(
                                "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
                            ),
                            stderr: stderr.to_owned(),
                        })
                    }
                    _ => panic!("package observation issued an unexpected command"),
                }
            },
        );
        assert_eq!(
            metadata_observation,
            Err("package_metadata_observation_ambiguous".to_owned())
        );
        assert_eq!(metadata_calls, 2);
    }
}

fn persisted_live_preflight_failure(
    observe: impl FnOnce(&str, PackageObservationPhase) -> Result<PackageObservation, String>,
) -> serde_json::Value {
    let code = verify_package_observation(DIAGNOSTIC_SERIAL, observe)
        .expect_err("fake ADB observer should return a package observation failure");
    let mut evidence = QualificationEvidence::initial();
    assert_eq!(
        record_live_preflight_failure(&mut evidence, &code),
        OutcomeCategory::Preflight
    );
    let directory = tempfile::tempdir().expect("evidence directory should exist");
    let path = directory.path().join("live-preflight-evidence.json");
    persist_evidence(&path, &evidence).expect("live preflight evidence should persist");
    serde_json::from_slice(&fs::read(path).expect("live preflight evidence should be readable"))
        .expect("live preflight evidence should be JSON")
}

fn persisted_live_package_admission_failure(
    observe: impl FnOnce(&str, PackageObservationPhase) -> Result<PackageObservation, String>,
) -> serde_json::Value {
    let observation = verify_package_observation(DIAGNOSTIC_SERIAL, observe)
        .expect("fake ADB observer should return a package observation");
    let code = admit_initial_install(observation.installed)
        .expect_err("an installed package should be rejected before mutation");
    let mut evidence = QualificationEvidence::initial();
    assert_eq!(
        record_live_preflight_failure(&mut evidence, code),
        OutcomeCategory::Preflight
    );
    let directory = tempfile::tempdir().expect("evidence directory should exist");
    let path = directory
        .path()
        .join("live-package-admission-evidence.json");
    persist_evidence(&path, &evidence).expect("live admission evidence should persist");
    serde_json::from_slice(&fs::read(path).expect("live admission evidence should be readable"))
        .expect("live admission evidence should be JSON")
}

#[test]
fn live_preflight_evidence_preserves_package_transport_timeout_code() {
    let evidence = persisted_live_preflight_failure(|serial, phase| {
        assert_eq!(serial, DIAGNOSTIC_SERIAL);
        assert_eq!(phase, PackageObservationPhase::Before);
        package_version_observation_with_probe(phase, |args| {
            assert_eq!(args, &["shell", "pm", "path", PACKAGE]);
            Err("adb_preflight_timed_out".to_owned())
        })
    });

    assert_eq!(evidence["failure_code"], "adb_preflight_timed_out");
    assert_eq!(evidence["live_status"], "unverified");
    assert_eq!(evidence["execution"], "NOT_RUN");
    assert!(!evidence.to_string().contains(DIAGNOSTIC_SERIAL));
}

#[test]
fn live_preflight_evidence_preserves_generic_command_failure_codes_without_diagnostics() {
    const SECRET: &str = "synthetic-generic-command-diagnostic-7d21";

    let path_evidence = persisted_live_preflight_failure(|serial, phase| {
        assert_eq!(serial, DIAGNOSTIC_SERIAL);
        assert_eq!(phase, PackageObservationPhase::Before);
        package_version_observation_with_probe(phase, |args| {
            assert_eq!(args, &["shell", "pm", "path", PACKAGE]);
            Err(format!("{SECRET} path transport detail"))
        })
    });
    assert_eq!(
        path_evidence["failure_code"],
        "adb_preflight_command_failed"
    );
    assert_eq!(path_evidence["live_status"], "unverified");
    assert_eq!(path_evidence["execution"], "NOT_RUN");
    let path_serialized = path_evidence.to_string();
    assert!(!path_serialized.contains(SECRET));
    assert!(!path_serialized.contains(DIAGNOSTIC_SERIAL));

    let metadata_evidence = persisted_live_preflight_failure(|serial, phase| {
        assert_eq!(serial, DIAGNOSTIC_SERIAL);
        assert_eq!(phase, PackageObservationPhase::Before);
        package_version_observation_with_probe(phase, |args| match args {
            ["shell", "pm", "path", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: "package:/data/app/opaque/base.apk".to_owned(),
                stderr: String::new(),
            }),
            ["shell", "dumpsys", "package", PACKAGE] => {
                Err(format!("{SECRET} metadata transport detail"))
            }
            _ => panic!("package observation issued an unexpected command"),
        })
    });
    assert_eq!(
        metadata_evidence["failure_code"],
        "package_metadata_observation_failed"
    );
    assert_eq!(metadata_evidence["live_status"], "unverified");
    assert_eq!(metadata_evidence["execution"], "NOT_RUN");
    let metadata_serialized = metadata_evidence.to_string();
    assert!(!metadata_serialized.contains(SECRET));
    assert!(!metadata_serialized.contains(DIAGNOSTIC_SERIAL));
}

#[test]
fn live_preflight_evidence_preserves_ambiguous_package_code_without_stderr() {
    const SECRET: &str = "synthetic-live-stderr-secret-4c7e";
    let path_evidence = persisted_live_preflight_failure(|serial, phase| {
        assert_eq!(serial, DIAGNOSTIC_SERIAL);
        assert_eq!(phase, PackageObservationPhase::Before);
        package_version_observation_with_probe(phase, |args| match args {
            ["shell", "pm", "path", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: "package:/data/app/opaque/base.apk".to_owned(),
                stderr: SECRET.to_owned(),
            }),
            ["shell", "dumpsys", "package", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: format!(
                    "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
                ),
                stderr: String::new(),
            }),
            _ => panic!("package observation issued an unexpected command"),
        })
    });

    assert_eq!(
        path_evidence["failure_code"],
        "package_path_observation_ambiguous"
    );
    assert_eq!(path_evidence["live_status"], "unverified");
    assert_eq!(path_evidence["execution"], "NOT_RUN");
    let path_serialized = path_evidence.to_string();
    assert!(!path_serialized.contains(SECRET));
    assert!(!path_serialized.contains(DIAGNOSTIC_SERIAL));

    let metadata_evidence = persisted_live_preflight_failure(|serial, phase| {
        assert_eq!(serial, DIAGNOSTIC_SERIAL);
        assert_eq!(phase, PackageObservationPhase::Before);
        package_version_observation_with_probe(phase, |args| match args {
            ["shell", "pm", "path", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: "package:/data/app/opaque/base.apk".to_owned(),
                stderr: String::new(),
            }),
            ["shell", "dumpsys", "package", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: format!(
                    "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
                ),
                stderr: SECRET.to_owned(),
            }),
            _ => panic!("package observation issued an unexpected command"),
        })
    });

    assert_eq!(
        metadata_evidence["failure_code"],
        "package_metadata_observation_ambiguous"
    );
    assert_eq!(metadata_evidence["live_status"], "unverified");
    assert_eq!(metadata_evidence["execution"], "NOT_RUN");
    let metadata_serialized = metadata_evidence.to_string();
    assert!(!metadata_serialized.contains(SECRET));
    assert!(!metadata_serialized.contains(DIAGNOSTIC_SERIAL));
}

#[test]
fn live_preflight_evidence_preserves_preexisting_package_rejection_code() {
    let evidence = persisted_live_package_admission_failure(|serial, phase| {
        assert_eq!(serial, DIAGNOSTIC_SERIAL);
        assert_eq!(phase, PackageObservationPhase::Before);
        package_version_observation_with_probe(phase, |args| match args {
            ["shell", "pm", "path", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: "package:/data/app/opaque/base.apk".to_owned(),
                stderr: String::new(),
            }),
            ["shell", "dumpsys", "package", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: format!(
                    "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
                ),
                stderr: String::new(),
            }),
            _ => panic!("package observation issued an unexpected command"),
        })
    });

    assert_eq!(
        evidence["failure_code"],
        "preexisting_package_not_owned_by_qualification"
    );
    assert_eq!(evidence["live_status"], "unverified");
    assert_eq!(evidence["execution"], "NOT_RUN");
    assert!(!evidence.to_string().contains(DIAGNOSTIC_SERIAL));
}

#[test]
fn read_only_diagnostic_classifies_avd_name_verification_failures() {
    let mut console_failure = FakePreflightAdb::default();
    console_failure.override_response(&["emu", "avd", "name"], 1, "KO: console unavailable\n", "");
    assert_eq!(
        run_fake_preflight_diagnostic(&mut console_failure, Some(EXPECTED_AVD)).failure_code,
        Some("avd_name_console_invalid")
    );

    let mut fallback_mismatch = FakePreflightAdb::default();
    fallback_mismatch.override_response(&["emu", "avd", "name"], 1, "", "");
    fallback_mismatch.override_response(
        &["shell", "getprop", "ro.boot.qemu.avd_name"],
        0,
        "other-avd\n",
        "",
    );
    assert_eq!(
        run_fake_preflight_diagnostic(&mut fallback_mismatch, Some(EXPECTED_AVD)).failure_code,
        Some("avd_name_mismatch")
    );

    let mut fallback_unavailable = FakePreflightAdb::default();
    fallback_unavailable.override_response(&["emu", "avd", "name"], 1, "", "");
    fallback_unavailable.override_error(
        &["shell", "getprop", "ro.boot.qemu.avd_name"],
        "private adb detail",
    );
    assert_eq!(
        run_fake_preflight_diagnostic(&mut fallback_unavailable, Some(EXPECTED_AVD)).failure_code,
        Some("avd_name_property_unavailable")
    );

    let mut fallback_command_failed = FakePreflightAdb::default();
    fallback_command_failed.override_response(&["emu", "avd", "name"], 1, "", "");
    fallback_command_failed.override_response(
        &["shell", "getprop", "ro.boot.qemu.avd_name"],
        1,
        "private property output",
        "private property error",
    );
    let fallback_command_failure =
        run_fake_preflight_diagnostic(&mut fallback_command_failed, Some(EXPECTED_AVD));
    assert_eq!(
        fallback_command_failure.failure_code,
        Some("avd_name_property_unavailable")
    );
    assert_fake_preflight_only_used_read_only_commands(&fallback_command_failed);

    let mut fallback_invalid = FakePreflightAdb::default();
    fallback_invalid.override_response(&["emu", "avd", "name"], 1, "", "");
    fallback_invalid.override_response(
        &["shell", "getprop", "ro.boot.qemu.avd_name"],
        0,
        &format!("{EXPECTED_AVD}\nunexpected-property-output"),
        "",
    );
    assert_eq!(
        run_fake_preflight_diagnostic(&mut fallback_invalid, Some(EXPECTED_AVD)).failure_code,
        Some("avd_name_property_invalid")
    );
    assert_fake_preflight_only_used_read_only_commands(&fallback_invalid);
}

#[test]
fn read_only_diagnostic_rejects_a_valid_console_response_for_a_different_avd_without_fallback() {
    let mut adb = FakePreflightAdb::default();
    adb.override_response(&["emu", "avd", "name"], 0, "Other-AVD\nOK\n", "");

    let evidence = run_fake_preflight_diagnostic(&mut adb, Some(EXPECTED_AVD));

    assert_eq!(evidence.failure_code, Some("avd_name_mismatch"));
    assert!(!adb.calls.iter().any(|call| {
        call.args
            == [
                "shell".to_owned(),
                "getprop".to_owned(),
                "ro.boot.qemu.avd_name".to_owned(),
            ]
    }));
    assert_fake_preflight_only_used_read_only_commands(&adb);
}

#[test]
fn read_only_diagnostic_accepts_confirmed_package_absence() {
    let mut adb = FakePreflightAdb::default();
    adb.override_response(&["shell", "pm", "path", PACKAGE], 1, "", "");

    let evidence = run_fake_preflight_diagnostic(&mut adb, Some(EXPECTED_AVD));

    assert_eq!(evidence.preflight_status, "passed");
    assert_eq!(evidence.failure_code, None);
    assert!(!adb.calls.iter().any(|call| {
        call.args
            == [
                "shell".to_owned(),
                "dumpsys".to_owned(),
                "package".to_owned(),
                PACKAGE.to_owned(),
            ]
    }));
    assert_fake_preflight_only_used_read_only_commands(&adb);
}

#[test]
fn live_package_observation_accepts_confirmed_package_absence() {
    let observation =
        package_version_observation_with_probe(PackageObservationPhase::Before, |args| {
            assert_eq!(args, &["shell", "pm", "path", PACKAGE]);
            Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 1,
                stdout: String::new(),
                stderr: String::new(),
            })
        });

    assert_eq!(
        observation,
        Ok(PackageObservation {
            phase: PackageObservationPhase::Before,
            package: PACKAGE.to_owned(),
            installed: false,
            version_code: None,
            version_name: None,
        })
    );
}

#[test]
fn live_package_observation_accepts_one_exact_path_with_a_terminating_newline() {
    let observation = package_version_observation_with_probe(
        PackageObservationPhase::Before,
        |args| match args {
            ["shell", "pm", "path", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: "package:/data/app/opaque/base.apk\n".to_owned(),
                stderr: String::new(),
            }),
            ["shell", "dumpsys", "package", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: format!(
                    "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
                ),
                stderr: String::new(),
            }),
            _ => panic!("package observation issued an unexpected command"),
        },
    );

    assert_eq!(
        observation,
        Ok(PackageObservation {
            phase: PackageObservationPhase::Before,
            package: PACKAGE.to_owned(),
            installed: true,
            version_code: Some("120".to_owned()),
            version_name: Some("1.2.0".to_owned()),
        })
    );
}

#[test]
fn live_package_observation_rejects_ambiguous_path_responses() {
    for (returncode, stdout, stderr) in [
        (0, String::new(), String::new()),
        (0, "com.nanodata.armsx\n".to_owned(), String::new()),
        (
            0,
            "package:/data/app/com.nanodata.armsx/base.apk\npackage:/data/app/split/base.apk"
                .to_owned(),
            String::new(),
        ),
        (
            1,
            "package:/data/app/com.nanodata.armsx/base.apk".to_owned(),
            String::new(),
        ),
        (1, String::new(), "adb: device offline".to_owned()),
        (
            0,
            "package:/data/app/com.nanodata.armsx/base.apk with spaces".to_owned(),
            String::new(),
        ),
        (
            0,
            " package:/data/app/com.nanodata.armsx/base.apk".to_owned(),
            String::new(),
        ),
        (
            0,
            "package:/data/app/com.nanodata.armsx/base.apk ".to_owned(),
            String::new(),
        ),
        (
            0,
            "\tpackage:/data/app/com.nanodata.armsx/base.apk\n".to_owned(),
            String::new(),
        ),
        (
            0,
            "package:/data/app/com.nanodata.armsx/base.apk\n\n".to_owned(),
            String::new(),
        ),
    ] {
        let observation =
            package_version_observation_with_probe(PackageObservationPhase::Before, |args| {
                assert_eq!(args, &["shell", "pm", "path", PACKAGE]);
                Ok(AdbCommandResult {
                    args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                    returncode,
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                })
            });
        assert_eq!(
            observation,
            Err("package_path_observation_ambiguous".to_owned())
        );
    }
}

#[test]
fn live_package_metadata_rejects_prefix_collisions_and_ambiguous_secondary_observations() {
    for (metadata, expected_error) in [
        (
            format!(
                "Package [{PACKAGE}.extra] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
            ),
            "package_metadata_observation_ambiguous",
        ),
        (
            format!(
                "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0\nPackage [other.package] (opaque):\n    versionCode=999"
            ),
            "package_metadata_observation_ambiguous",
        ),
        (
            format!(
                "Package [{PACKAGE}] (opaque):\n    versionCode=120\n    versionCode=121"
            ),
            "package_metadata_observation_ambiguous",
        ),
        (
            format!("Package [{PACKAGE}] (opaque):\n    versionCode=120\n    versionCode="),
            "package_metadata_observation_ambiguous",
        ),
        (
            format!(
                "Package [{PACKAGE}] (opaque):\n    versionCode=120 versionName=1.2.0\n    versionName="
            ),
            "package_metadata_observation_ambiguous",
        ),
        (
            format!(
                "Package [{PACKAGE}] (opaque):\n    versionCode=120\n    versionName=1.0 versionCode=121"
            ),
            "package_metadata_observation_ambiguous",
        ),
        (
            format!(
                "Package [{PACKAGE}] (opaque):\n    versionCode=120 versionName=1.0 versionName=2.0"
            ),
            "package_metadata_observation_ambiguous",
        ),
        (
            format!("Package [{PACKAGE}] (opaque):\n    codePath=/data/app/opaque"),
            "package_version_code_unavailable",
        ),
    ] {
        let mut calls = 0_u8;
        let observation = package_version_observation_with_probe(
            PackageObservationPhase::Before,
            |args| {
                calls += 1;
                match calls {
                    1 => {
                        assert_eq!(args, &["shell", "pm", "path", PACKAGE]);
                        Ok(AdbCommandResult {
                            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                            returncode: 0,
                            stdout: "package:/data/app/opaque/base.apk".to_owned(),
                            stderr: String::new(),
                        })
                    }
                    2 => {
                        assert_eq!(args, &["shell", "dumpsys", "package", PACKAGE]);
                        Ok(AdbCommandResult {
                            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                            returncode: 0,
                            stdout: metadata.clone(),
                            stderr: String::new(),
                        })
                    }
                    _ => panic!("package observation issued an unexpected command"),
                }
            },
        );
        assert_eq!(observation, Err(expected_error.to_owned()));
        assert_eq!(calls, 2);
    }

    let failed_secondary = package_version_observation_with_probe(
        PackageObservationPhase::Before,
        |args| match args {
            ["shell", "pm", "path", PACKAGE] => Ok(AdbCommandResult {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                returncode: 0,
                stdout: "package:/data/app/opaque/base.apk".to_owned(),
                stderr: String::new(),
            }),
            ["shell", "dumpsys", "package", PACKAGE] => Err("adb_preflight_timed_out".to_owned()),
            _ => panic!("package observation issued an unexpected command"),
        },
    );
    assert_eq!(failed_secondary, Err("adb_preflight_timed_out".to_owned()));
}

#[test]
fn read_only_diagnostic_classifies_initial_package_observation_and_admission() {
    let mut path_ambiguous = FakePreflightAdb::default();
    path_ambiguous.override_response(
        &["shell", "pm", "path", PACKAGE],
        1,
        "",
        "private package manager detail",
    );
    let path_ambiguous_evidence =
        run_fake_preflight_diagnostic(&mut path_ambiguous, Some(EXPECTED_AVD));
    assert_eq!(
        path_ambiguous_evidence.failure_code,
        Some("initial_package_observation_path_ambiguous")
    );
    assert!(!serde_json::to_string(&path_ambiguous_evidence)
        .expect("sanitized diagnostic evidence should serialize")
        .contains("private package manager detail"));

    let mut path_transport_failure = FakePreflightAdb::default();
    path_transport_failure
        .override_error(&["shell", "pm", "path", PACKAGE], "adb_preflight_timed_out");
    assert_eq!(
        run_fake_preflight_diagnostic(&mut path_transport_failure, Some(EXPECTED_AVD)).failure_code,
        Some("initial_package_observation_path_timed_out")
    );

    let mut metadata_failure = FakePreflightAdb {
        package_installed: true,
        ..FakePreflightAdb::default()
    };
    metadata_failure.override_response(&["shell", "dumpsys", "package", PACKAGE], 1, "", "");
    assert_eq!(
        run_fake_preflight_diagnostic(&mut metadata_failure, Some(EXPECTED_AVD)).failure_code,
        Some("initial_package_observation_metadata_failed")
    );

    let mut metadata_ambiguous = FakePreflightAdb {
        package_installed: true,
        ..FakePreflightAdb::default()
    };
    metadata_ambiguous.override_response(
        &["shell", "dumpsys", "package", PACKAGE],
        1,
        "serial=emulator-5554 token=private-package-token",
        "private package metadata detail",
    );
    let metadata_ambiguous_evidence =
        run_fake_preflight_diagnostic(&mut metadata_ambiguous, Some(EXPECTED_AVD));
    assert_eq!(
        metadata_ambiguous_evidence.failure_code,
        Some("initial_package_observation_metadata_ambiguous")
    );
    assert!(!serde_json::to_string(&metadata_ambiguous_evidence)
        .expect("sanitized diagnostic evidence should serialize")
        .contains("private-package-token"));

    let mut version_unavailable = FakePreflightAdb {
        package_installed: true,
        ..FakePreflightAdb::default()
    };
    version_unavailable.override_response(
        &["shell", "dumpsys", "package", PACKAGE],
        0,
        &format!("Package [{PACKAGE}] (opaque):\n    codePath=/data/app/opaque"),
        "",
    );
    assert_eq!(
        run_fake_preflight_diagnostic(&mut version_unavailable, Some(EXPECTED_AVD)).failure_code,
        Some("initial_package_observation_version_unavailable")
    );

    let mut preexisting = FakePreflightAdb {
        package_installed: true,
        ..FakePreflightAdb::default()
    };
    let evidence = run_fake_preflight_diagnostic(&mut preexisting, Some(EXPECTED_AVD));
    assert_eq!(evidence.failure_code, Some("initial_package_preexisting"));
    assert_eq!(evidence.failure_category, Some("package_admission"));
    assert_fake_preflight_only_used_read_only_commands(&preexisting);
}

#[test]
fn successful_read_only_diagnostic_records_live_and_mutation_as_unverified_not_run() {
    let mut adb = FakePreflightAdb::default();
    let evidence = run_fake_preflight_diagnostic(&mut adb, Some(EXPECTED_AVD));
    assert_eq!(evidence.status, "completed");
    assert_eq!(evidence.preflight_status, "passed");
    assert_eq!(evidence.failure_code, None);
    assert_eq!(evidence.live_qualification, "unverified");
    assert_eq!(evidence.planning, "NOT_RUN");
    assert_eq!(evidence.release_resolution, "NOT_RUN");
    assert_eq!(evidence.network, "NOT_RUN");
    assert_eq!(evidence.apk_download, "NOT_RUN");
    assert_eq!(evidence.installation, "NOT_RUN");
    assert_eq!(evidence.package_mutation, "NOT_RUN");
    assert!(evidence
        .checks
        .iter()
        .all(|check| check.outcome == "passed"));
    assert_fake_preflight_only_used_read_only_commands(&adb);
}

#[test]
fn diagnostic_evidence_writer_persists_only_sanitized_preflight_outcomes() {
    let directory = tempfile::tempdir().expect("diagnostic evidence directory should exist");
    let path = directory.path().join("preflight-diagnostic-evidence.json");
    let mut adb = FakePreflightAdb::default();
    adb.override_response(
        &["shell", "getprop", "ro.build.version.sdk"],
        1,
        "",
        "serial=emulator-5554 token=private-test-token",
    );
    let evidence = run_fake_preflight_diagnostic(&mut adb, Some(EXPECTED_AVD));

    persist_diagnostic_evidence(&path, &evidence)
        .expect("sanitized diagnostic evidence should be persisted");
    let serialized = fs::read_to_string(path).expect("diagnostic evidence should be readable");
    assert!(serialized.contains("identity_api_level_read_failed"));
    assert!(serialized.contains("\"live_qualification\": \"unverified\""));
    assert!(serialized.contains("\"package_mutation\": \"NOT_RUN\""));
    for secret in [
        "emulator-5554",
        EXPECTED_AVD,
        "private-test-token",
        "serial=",
    ] {
        assert!(!serialized.contains(secret));
    }
}

#[cfg(unix)]
#[test]
fn adb_preflight_times_out_and_reaps_a_fake_sleeping_child() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    let directory = tempfile::tempdir().expect("fake ADB directory should be created");
    let executable = directory.path().join("sleeping-adb");
    fs::write(&executable, "#!/bin/sh\nexec /bin/sleep 5\n")
        .expect("fake ADB script should be written");
    let mut permissions = fs::metadata(&executable)
        .expect("fake ADB script metadata should be readable")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).expect("fake ADB script should be executable");
    let _deadline = crate::owned_process::arm_test_process_deadline(
        ProcessOperation::Probe,
        Duration::from_millis(100),
    );

    assert_eq!(
        adb_with_executable(
            executable
                .to_str()
                .expect("fake executable path should be UTF-8"),
            "emulator-5554",
            &["get-state"],
        ),
        Err("adb_preflight_timed_out".to_owned())
    );
}

#[test]
fn live_admission_accepts_the_contract_bound_reviewed_plan() {
    let (contract, app_hash, recipe_hash, plan, review) = admission_fixture();

    assert_eq!(
        admit_live_mutation(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &plan,
            &review,
        ),
        Ok(())
    );
}

#[test]
fn live_admission_rejects_remote_release_policy_drift_before_runner_setup() {
    use crate::planner::ExecutionArtifactSource::RemoteRelease;
    use std::cell::Cell;

    let (contract, app_hash, recipe_hash, plan, review) = admission_fixture();
    let mutations: [fn(&mut crate::planner::ExecutionArtifactSource); 6] = [
        |source| match source {
            RemoteRelease { provider, .. } => *provider = "gitlab".to_owned(),
            _ => panic!("reviewed ARMSX1 artifact should use a remote release"),
        },
        |source| match source {
            RemoteRelease { service_origin, .. } => {
                *service_origin = "http://github.com".to_owned();
            }
            _ => panic!("reviewed ARMSX1 artifact should use a remote release"),
        },
        |source| match source {
            RemoteRelease { repository, .. } => *repository = "other/app".to_owned(),
            _ => panic!("reviewed ARMSX1 artifact should use a remote release"),
        },
        |source| match source {
            RemoteRelease {
                include_prereleases,
                ..
            } => *include_prereleases = true,
            _ => panic!("reviewed ARMSX1 artifact should use a remote release"),
        },
        |source| match source {
            RemoteRelease { asset_pattern, .. } => {
                *asset_pattern = Some(".*".to_owned());
            }
            _ => panic!("reviewed ARMSX1 artifact should use a remote release"),
        },
        |source| match source {
            RemoteRelease {
                invert_asset_pattern,
                ..
            } => *invert_asset_pattern = true,
            _ => panic!("reviewed ARMSX1 artifact should use a remote release"),
        },
    ];
    let isolated_runtime = tempfile::tempdir().expect("isolated runtime root should be created");
    let cache_root = isolated_runtime.path().join("cache");

    for (index, mutate_source) in mutations.into_iter().enumerate() {
        let mut drifted_plan = plan.clone();
        mutate_source(&mut drifted_plan.artifacts[0].source);
        let adapter_constructed = Cell::new(false);
        let result = after_admission(
            admit_live_runner(
                &contract,
                &app_hash,
                &recipe_hash,
                Some(PACKAGE),
                &drifted_plan,
                &review,
                IsolatedCacheRoots {
                    runtime: isolated_runtime.path(),
                    cache: &cache_root,
                },
            ),
            || {
                adapter_constructed.set(true);
                Ok::<_, ()>(())
            },
        );

        assert_eq!(
            result,
            Err(OutcomeCategory::ContractDrift),
            "release source mutation {index} should fail admission"
        );
        assert!(
            !adapter_constructed.get(),
            "release source mutation {index} must not construct adapters"
        );
    }

    let mut wrong_cache_policy = plan;
    wrong_cache_policy.artifacts[0].cache = "shared".to_owned();
    let adapter_constructed = Cell::new(false);
    let result = after_admission(
        admit_live_runner(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &wrong_cache_policy,
            &review,
            IsolatedCacheRoots {
                runtime: isolated_runtime.path(),
                cache: &cache_root,
            },
        ),
        || {
            adapter_constructed.set(true);
            Ok::<_, ()>(())
        },
    );
    assert_eq!(result, Err(OutcomeCategory::ContractDrift));
    assert!(!adapter_constructed.get());
}

#[test]
fn live_admission_rejects_reused_or_nonisolated_cache_before_runner_setup() {
    use std::cell::Cell;

    let (contract, app_hash, recipe_hash, plan, review) = admission_fixture();
    let isolated_runtime = tempfile::tempdir().expect("isolated runtime root should be created");
    let fresh_cache = isolated_runtime.path().join("cache");
    assert_eq!(
        admit_live_runner(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &plan,
            &review,
            IsolatedCacheRoots {
                runtime: isolated_runtime.path(),
                cache: &fresh_cache,
            },
        ),
        Ok(())
    );

    let reused_cache = isolated_runtime.path().join("cache");
    fs::create_dir_all(&reused_cache).expect("reused cache fixture should be created");
    let adapter_constructed = Cell::new(false);
    let result = after_admission(
        admit_live_runner(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &plan,
            &review,
            IsolatedCacheRoots {
                runtime: isolated_runtime.path(),
                cache: &reused_cache,
            },
        ),
        || {
            adapter_constructed.set(true);
            Ok::<_, ()>(())
        },
    );
    assert_eq!(result, Err(OutcomeCategory::ContractDrift));
    assert!(!adapter_constructed.get());

    let shared_cache = isolated_runtime.path().join("shared-cache");
    let adapter_constructed = Cell::new(false);
    let result = after_admission(
        admit_live_runner(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &plan,
            &review,
            IsolatedCacheRoots {
                runtime: isolated_runtime.path(),
                cache: &shared_cache,
            },
        ),
        || {
            adapter_constructed.set(true);
            Ok::<_, ()>(())
        },
    );
    assert_eq!(result, Err(OutcomeCategory::ContractDrift));
    assert!(!adapter_constructed.get());
}

#[test]
fn preexisting_package_is_rejected_before_any_mutation() {
    assert_eq!(
        admit_initial_install(true),
        Err("preexisting_package_not_owned_by_qualification")
    );
    assert_eq!(admit_initial_install(false), Ok(()));

    let mut calls = 0_u8;
    let observation = package_version_observation_with_probe(
        PackageObservationPhase::Before,
        |args| {
            calls += 1;
            match calls {
                1 => {
                    assert_eq!(args, &["shell", "pm", "path", PACKAGE]);
                    Ok(AdbCommandResult {
                        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                        returncode: 0,
                        stdout: "package:/data/app/opaque/base.apk".to_owned(),
                        stderr: String::new(),
                    })
                }
                2 => {
                    assert_eq!(args, &["shell", "dumpsys", "package", PACKAGE]);
                    Ok(AdbCommandResult {
                        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                        returncode: 0,
                        stdout: format!(
                            "Package [{PACKAGE}] (opaque):\n    versionCode=120 minSdk=23 versionName=1.2.0"
                        ),
                        stderr: String::new(),
                    })
                }
                _ => panic!("package observation issued an unexpected command"),
            }
        },
    )
    .expect("installed package should be observed")
    .installed;
    assert!(observation);
    assert_eq!(
        admit_initial_install(observation),
        Err("preexisting_package_not_owned_by_qualification")
    );
    assert_eq!(calls, 2);
}

#[test]
fn live_admission_rejects_app_definition_and_recipe_hash_drift() {
    let (contract, app_hash, recipe_hash, plan, review) = admission_fixture();

    assert_eq!(
        admit_live_mutation(
            &contract,
            "changed-app-definition-hash",
            &recipe_hash,
            Some(PACKAGE),
            &plan,
            &review,
        ),
        Err(OutcomeCategory::ContractDrift)
    );
    assert_eq!(
        admit_live_mutation(
            &contract,
            &app_hash,
            "changed-recipe-hash",
            Some(PACKAGE),
            &plan,
            &review,
        ),
        Err(OutcomeCategory::ContractDrift)
    );
}

#[test]
fn live_admission_rejects_unexpected_steps_and_dependency_edges() {
    let (contract, app_hash, recipe_hash, plan, review) = admission_fixture();
    let mut extra_step = plan.clone();
    extra_step.steps.push(extra_step.steps[0].clone());
    assert_eq!(
        admit_live_mutation(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &extra_step,
            &review,
        ),
        Err(OutcomeCategory::UnsafePlan)
    );

    let mut missing_edge = plan;
    let install = missing_edge
        .steps
        .iter_mut()
        .find(|step| step.id.ends_with("/install_armsx1"))
        .expect("reviewed plan should contain the Armsx1 install step");
    install.dependencies.clear();
    assert_eq!(
        admit_live_mutation(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &missing_edge,
            &review,
        ),
        Err(OutcomeCategory::UnsafePlan)
    );
}

#[test]
fn live_admission_binds_the_app_owned_package_to_the_exact_allowlist() {
    let (contract, app_hash, recipe_hash, plan, review) = admission_fixture();
    let mut wrong_package = plan.clone();
    wrong_package.apps[0].package_id = "com.example.unreviewed".to_owned();
    assert_eq!(
        admit_live_mutation(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &wrong_package,
            &review,
        ),
        Err(OutcomeCategory::MutationTarget)
    );
    assert_eq!(
        admit_live_mutation(
            &contract,
            &app_hash,
            &recipe_hash,
            Some("com.example.unreviewed"),
            &plan,
            &review,
        ),
        Err(OutcomeCategory::MutationTarget)
    );
}

#[test]
fn live_admission_rejects_replacement_disabled_or_unexecutable_review() {
    let (contract, app_hash, recipe_hash, plan, review) = admission_fixture();
    let mut replacement_disabled = plan.clone();
    let install = replacement_disabled
        .steps
        .iter_mut()
        .find(|step| step.id.ends_with("/install_armsx1"))
        .expect("reviewed plan should contain the Armsx1 install step");
    install.params.insert(
        "replace_existing".to_owned(),
        crate::planner::ExecutionParamValue::Literal {
            value: serde_json::Value::Bool(false),
        },
    );
    assert_eq!(
        admit_live_mutation(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &replacement_disabled,
            &review,
        ),
        Err(OutcomeCategory::ReplaceExistingDisabled)
    );

    let mut blocked_review = review.clone();
    blocked_review.can_execute = false;
    assert_eq!(
        admit_live_mutation(
            &contract,
            &app_hash,
            &recipe_hash,
            Some(PACKAGE),
            &plan,
            &blocked_review,
        ),
        Err(OutcomeCategory::UnsafePlan)
    );
}

#[test]
fn device_preflight_accepts_ascii_case_insensitive_google_brand() {
    let mut properties = valid_device_properties();
    properties.brand = "google".to_owned();
    assert_eq!(
        validate_device_properties(&properties, EXPECTED_AVD),
        Ok(())
    );

    let mut properties = valid_device_properties();
    properties.brand = "Samsung".to_owned();
    assert_eq!(
        validate_device_properties(&properties, EXPECTED_AVD),
        Err("identity_brand_mismatch")
    );

    let mut properties = valid_device_properties();
    properties.brand.clear();
    assert_eq!(
        validate_device_properties(&properties, EXPECTED_AVD),
        Err("identity_brand_mismatch")
    );
}

#[test]
fn device_preflight_requires_exact_google_brand_and_android_release() {
    let mut properties = valid_device_properties();
    properties.brand = "Google LLC".to_owned();
    assert_eq!(
        validate_device_properties(&properties, EXPECTED_AVD),
        Err("identity_brand_mismatch")
    );

    let mut properties = valid_device_properties();
    properties.release = "15".to_owned();
    assert_eq!(
        validate_device_properties(&properties, EXPECTED_AVD),
        Err("identity_android_release_mismatch")
    );
}

#[test]
fn device_preflight_reports_the_individual_failed_identity_check() {
    let mut properties = valid_device_properties();
    properties.state = "offline".to_owned();
    assert_eq!(
        validate_device_properties(&properties, EXPECTED_AVD),
        Err("identity_state_mismatch")
    );

    let mut properties = valid_device_properties();
    properties.release = "15".to_owned();
    assert_eq!(
        validate_device_properties(&properties, EXPECTED_AVD),
        Err("identity_android_release_mismatch")
    );
}

#[test]
fn avd_name_preflight_requires_the_complete_emulator_console_response() {
    assert_eq!(
        parse_avd_name_response("EmuChef-ARMSX1-Android16-ARM64\r\nOK\r\n"),
        Ok(Some(EXPECTED_AVD))
    );
    assert_eq!(parse_avd_name_response(""), Ok(None));
    for malformed in [
        "EmuChef-ARMSX1-Android16-ARM64\nKO: console unavailable\n",
        "ERROR: console unavailable\n",
        "EmuChef-ARMSX1-Android16-ARM64\n",
        "EmuChef-ARMSX1-Android16-ARM64\nOK\nextra\n",
    ] {
        assert!(parse_avd_name_response(malformed).is_err());
    }
}

#[test]
fn avd_name_property_fallback_requires_a_valid_exact_observation() {
    let properties = valid_device_properties();
    let empty_console = Ok(AdbCommandResult {
        args: Vec::new(),
        returncode: 1,
        stdout: String::new(),
        stderr: String::new(),
    });
    let expected_property = || {
        Ok(AdbCommandResult {
            args: Vec::new(),
            returncode: 0,
            stdout: format!("{EXPECTED_AVD}\n"),
            stderr: String::new(),
        })
    };
    assert_eq!(
        resolve_device_avd_name(
            &properties,
            empty_console.clone(),
            expected_property,
            EXPECTED_AVD,
        ),
        Ok(EXPECTED_AVD.to_owned())
    );
    assert_eq!(
        resolve_device_avd_name(
            &properties,
            Err("console process unavailable".to_owned()),
            || Ok(AdbCommandResult {
                args: Vec::new(),
                returncode: 0,
                stdout: format!("{EXPECTED_AVD}\nOther-AVD\n"),
                stderr: String::new(),
            }),
            EXPECTED_AVD,
        ),
        Err("emulator_avd_name_property_invalid")
    );
    assert_eq!(
        resolve_device_avd_name(
            &properties,
            empty_console.clone(),
            || Ok(AdbCommandResult {
                args: Vec::new(),
                returncode: 0,
                stdout: "Other-AVD\n".to_owned(),
                stderr: String::new(),
            }),
            EXPECTED_AVD,
        ),
        Err("designated_avd_name_mismatch")
    );
    assert_eq!(
        resolve_device_avd_name(
            &properties,
            empty_console,
            || Ok(AdbCommandResult {
                args: Vec::new(),
                returncode: 0,
                stdout: String::new(),
                stderr: String::new(),
            }),
            EXPECTED_AVD,
        ),
        Err("emulator_avd_name_property_invalid")
    );
}

#[test]
fn avd_name_console_errors_and_malformed_responses_never_use_property_fallback() {
    let properties = valid_device_properties();
    for console_result in [
        Ok(AdbCommandResult {
            args: Vec::new(),
            returncode: 1,
            stdout: "KO: console unavailable\n".to_owned(),
            stderr: String::new(),
        }),
        Ok(AdbCommandResult {
            args: Vec::new(),
            returncode: 1,
            stdout: String::new(),
            stderr: "error: console unavailable\n".to_owned(),
        }),
        Ok(AdbCommandResult {
            args: Vec::new(),
            returncode: 0,
            stdout: format!("{EXPECTED_AVD}\n"),
            stderr: String::new(),
        }),
    ] {
        let mut property_queried = false;
        assert_eq!(
            resolve_device_avd_name(
                &properties,
                console_result,
                || {
                    property_queried = true;
                    Ok(AdbCommandResult {
                        args: Vec::new(),
                        returncode: 0,
                        stdout: format!("{EXPECTED_AVD}\n"),
                        stderr: String::new(),
                    })
                },
                EXPECTED_AVD,
            ),
            Err("emulator_avd_name_protocol_invalid")
        );
        assert!(!property_queried);
    }
}

#[test]
fn avd_name_console_name_plus_ok_does_not_query_the_property_fallback() {
    let mut properties = valid_device_properties();
    properties.avd_name = None;
    let mut property_queried = false;
    assert_eq!(
        resolve_device_avd_name(
            &properties,
            Ok(AdbCommandResult {
                args: Vec::new(),
                returncode: 0,
                stdout: format!("{EXPECTED_AVD}\r\nOK\r\n"),
                stderr: String::new(),
            }),
            || {
                property_queried = true;
                Err("property fallback should not be used".to_owned())
            },
            EXPECTED_AVD,
        ),
        Ok(EXPECTED_AVD.to_owned())
    );
    assert!(!property_queried);
}

#[test]
fn avd_property_fallback_still_requires_the_complete_emulator_profile() {
    let mut properties = valid_device_properties();
    properties.state = "offline".to_owned();
    let mut property_queried = false;
    assert_eq!(
        resolve_device_avd_name(
            &properties,
            Ok(AdbCommandResult {
                args: Vec::new(),
                returncode: 1,
                stdout: String::new(),
                stderr: String::new(),
            }),
            || {
                property_queried = true;
                Ok(AdbCommandResult {
                    args: Vec::new(),
                    returncode: 0,
                    stdout: format!("{EXPECTED_AVD}\n"),
                    stderr: String::new(),
                })
            },
            EXPECTED_AVD,
        ),
        Err("identity_state_mismatch")
    );
    assert!(!property_queried);
}

#[test]
fn qualification_failure_categories_are_sanitized_and_distinguish_signing() {
    let cases = [
        (
            EvidenceStage::NetworkResolution,
            "remote_release_not_found: repository and asset details",
            OutcomeCategory::NoEligibleRelease,
        ),
        (
            EvidenceStage::NetworkResolution,
            "remote_release_client_failed: credential=private-token",
            OutcomeCategory::NetworkResolution,
        ),
        (
            EvidenceStage::ApkIdentity,
            "apk_package_mismatch: expected private.package actual other.package",
            OutcomeCategory::ApkIdentity,
        ),
        (
            EvidenceStage::Install,
            "android_signing_failed: certificate fingerprint=private-fingerprint",
            OutcomeCategory::InstallSigning,
        ),
        (
            EvidenceStage::Install,
            "android_install_failed: device serial=emulator-5554",
            OutcomeCategory::Install,
        ),
        (
            EvidenceStage::Version,
            "version_observation_failed: raw command output",
            OutcomeCategory::Version,
        ),
        (
            EvidenceStage::Cleanup,
            "incomplete_cleanup: child process diagnostics",
            OutcomeCategory::IncompleteCleanup,
        ),
    ];

    for (stage, diagnostic, expected) in cases {
        assert_eq!(classify_failure(stage, diagnostic), expected);
    }
}

#[test]
fn qualification_failure_marks_all_later_checks_not_run() {
    let evidence = QualificationEvidence::failed(OutcomeCategory::NoEligibleRelease);

    assert_eq!(evidence.status, "blocked");
    assert_eq!(evidence.execution, "NOT_RUN");
    let failed = evidence
        .outcomes
        .iter()
        .find(|outcome| outcome.stage == EvidenceStage::NetworkResolution)
        .expect("network resolution outcome should be recorded");
    assert_eq!(failed.status, OutcomeStatus::Failed);
    assert_eq!(failed.category, Some(OutcomeCategory::NoEligibleRelease));
    assert!(evidence
        .outcomes
        .iter()
        .filter(|outcome| outcome.stage != EvidenceStage::NetworkResolution)
        .all(|outcome| outcome.status == OutcomeStatus::NotRun));
}

#[test]
fn cleanup_failure_does_not_mark_unobserved_stages_as_passed() {
    let evidence = QualificationEvidence::failed(OutcomeCategory::IncompleteCleanup);

    assert!(evidence.outcomes[..evidence.outcomes.len() - 1]
        .iter()
        .all(|outcome| outcome.status == OutcomeStatus::NotRun));
    let cleanup = evidence
        .outcomes
        .iter()
        .find(|outcome| outcome.stage == EvidenceStage::Cleanup)
        .expect("cleanup outcome should be recorded");
    assert_eq!(cleanup.status, OutcomeStatus::Failed);
    assert_eq!(cleanup.category, Some(OutcomeCategory::IncompleteCleanup));
    assert!(evidence
        .install_runs
        .iter()
        .all(|run| run.status == OutcomeStatus::NotRun));
}

#[test]
fn confirmed_process_cleanup_does_not_complete_runtime_cleanup() {
    let expected_step_id = format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}");
    let result = crate::executor::ExecutionRunResult {
        success: true,
        cancelled: false,
        total_steps: 1,
        steps: vec![crate::executor::StepRunRecord {
            step_id: expected_step_id.clone(),
            status: crate::executor::StepRunStatus::Executed,
            message: None,
            outputs: crate::model::OrderedMap::new(),
            failure_kind: None,
            cleanup: Some(crate::owned_process::ProcessCleanup::Confirmed),
        }],
        resolved_releases: Vec::new(),
        resolved_artifacts: Vec::new(),
    };
    let mut evidence = QualificationEvidence::initial();

    record_execution_result(&mut evidence, 1, &expected_step_id, false, true, &result);

    assert_eq!(evidence.cleanup.status, CleanupStatus::NotRun);
    assert_eq!(
        evidence.cleanup.process_observations,
        vec![ProcessCleanupEvidence {
            run: 1,
            step_id: expected_step_id,
            status: ProcessCleanupStatus::Confirmed,
        }]
    );

    let temporary = tempfile::tempdir().expect("isolated runtime should be created");
    let device = crate::executor::adb::RealAdbDevice::new("adb", Some("emulator-5554".into()));
    let adapters = crate::executor::ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        temporary.path().join("runtime"),
        temporary.path().join("cache"),
        temporary.path().join("device"),
        Vec::new(),
        false,
    );
    let runner = crate::executor::ExecutorRunner::new(adapters);
    close_live_runtime(&mut evidence, runner, temporary);

    assert_eq!(evidence.cleanup.status, CleanupStatus::Complete);
}

#[test]
fn failed_runtime_cleanup_never_marks_overall_cleanup_complete() {
    let temporary = tempfile::tempdir().expect("isolated runtime should be created");
    let temporary_path = temporary.path().to_path_buf();
    let device = crate::executor::adb::RealAdbDevice::new("adb", Some("emulator-5554".into()));
    let adapters = crate::executor::ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        temporary.path().join("runtime"),
        temporary.path().join("cache"),
        temporary.path().join("device"),
        Vec::new(),
        false,
    );
    let runner = crate::executor::ExecutorRunner::new(adapters);
    fs::remove_dir(&temporary_path).expect("temporary runtime directory should be removable");
    fs::write(&temporary_path, b"cleanup failure fixture")
        .expect("replacement file should prevent directory cleanup");
    let mut evidence = QualificationEvidence::initial();

    close_live_runtime(&mut evidence, runner, temporary);

    assert_eq!(evidence.cleanup.status, CleanupStatus::Incomplete);
    assert_eq!(evidence.status, "blocked");
    assert_eq!(evidence.live_status, "unverified");
    assert_eq!(evidence.execution, "NOT_RUN");
}

#[test]
fn cleanup_uncertainty_preserves_the_primary_failure() {
    let mut evidence = QualificationEvidence::initial();
    evidence.mark_passed(EvidenceStage::Preflight);
    evidence.mark_passed(EvidenceStage::PlanValidation);
    evidence.mark_passed(EvidenceStage::RunnerSetup);
    evidence.mark_passed(EvidenceStage::NetworkResolution);
    evidence.mark_passed(EvidenceStage::ApkIdentity);
    evidence.mark_install_run(
        1,
        Some(format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}")),
        InstallStepStatus::Failed,
        Some(true),
    );
    evidence.fail(OutcomeCategory::InstallSigning);
    evidence.mark_passed(EvidenceStage::Version);
    evidence.mark_cleanup_incomplete();
    evidence.fail(OutcomeCategory::IncompleteCleanup);

    assert_eq!(evidence.reason, Some(OutcomeCategory::InstallSigning));
    let install = evidence
        .outcomes
        .iter()
        .find(|outcome| outcome.stage == EvidenceStage::Install)
        .expect("install outcome should retain the primary failure");
    assert_eq!(install.status, OutcomeStatus::Failed);
    assert_eq!(install.category, Some(OutcomeCategory::InstallSigning));
    assert!(evidence.outcomes[..EvidenceStage::Install.index()]
        .iter()
        .all(|outcome| outcome.status == OutcomeStatus::Passed));
    let cleanup = evidence
        .outcomes
        .iter()
        .find(|outcome| outcome.stage == EvidenceStage::Cleanup)
        .expect("cleanup outcome should report the secondary uncertainty");
    assert_eq!(cleanup.status, OutcomeStatus::Failed);
    assert_eq!(cleanup.category, Some(OutcomeCategory::IncompleteCleanup));
    let version = evidence
        .outcomes
        .iter()
        .find(|outcome| outcome.stage == EvidenceStage::Version)
        .expect("version outcome should remain unobserved after install failure");
    assert_eq!(version.status, OutcomeStatus::NotRun);
    assert_eq!(version.category, None);
    assert_eq!(evidence.cleanup.status, CleanupStatus::Incomplete);
}

#[test]
fn primary_failure_replaces_cleanup_only_reason_without_changing_cleanup() {
    let mut evidence = QualificationEvidence::initial();
    evidence.mark_cleanup_incomplete();
    evidence.fail(OutcomeCategory::IncompleteCleanup);
    evidence.mark_passed(EvidenceStage::Preflight);
    evidence.mark_passed(EvidenceStage::PlanValidation);
    evidence.mark_passed(EvidenceStage::RunnerSetup);
    evidence.mark_passed(EvidenceStage::NetworkResolution);
    evidence.mark_passed(EvidenceStage::ApkIdentity);
    evidence.mark_install_run(
        1,
        Some(format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}")),
        InstallStepStatus::Failed,
        Some(true),
    );
    evidence.fail(OutcomeCategory::InstallSigning);

    assert_eq!(evidence.reason, Some(OutcomeCategory::InstallSigning));
    assert_eq!(evidence.cleanup.status, CleanupStatus::Incomplete);
    assert_eq!(
        evidence
            .outcomes
            .iter()
            .find(|outcome| outcome.stage == EvidenceStage::Cleanup)
            .expect("cleanup outcome should remain independently failed")
            .category,
        Some(OutcomeCategory::IncompleteCleanup)
    );
    assert_eq!(
        evidence
            .outcomes
            .iter()
            .find(|outcome| outcome.stage == EvidenceStage::Version)
            .expect("later version outcome should remain unobserved")
            .status,
        OutcomeStatus::NotRun
    );
}

#[test]
fn qualification_evidence_serialization_excludes_identifiers_and_diagnostics() {
    let evidence = QualificationEvidence::failed(OutcomeCategory::InstallSigning);
    let serialized = serde_json::to_string(&evidence).expect("evidence should serialize");

    for forbidden in [
        "emulator-5554",
        "private.package",
        "private-token",
        "private-fingerprint",
        "raw command output",
        "apk bytes",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    assert!(serialized.contains("install_signing"));
    assert!(serialized.contains("NOT_RUN"));
}

#[test]
fn qualification_evidence_write_is_atomic_and_replaces_prior_json() {
    let directory = tempfile::tempdir().expect("temporary evidence directory should be available");
    let path = directory.path().join("armsx1-evidence.json");
    let first = QualificationEvidence::failed(OutcomeCategory::NetworkResolution);
    let second = QualificationEvidence::failed(OutcomeCategory::InstallSigning);

    write_evidence_atomic(&path, &first).expect("first evidence write should succeed");
    assert_eq!(
        serde_json::from_slice::<QualificationEvidence>(
            &fs::read(&path).expect("first evidence file should exist")
        )
        .expect("first evidence should parse"),
        first
    );

    write_evidence_atomic(&path, &second).expect("replacement evidence write should succeed");
    assert_eq!(
        serde_json::from_slice::<QualificationEvidence>(
            &fs::read(&path).expect("replacement evidence file should exist")
        )
        .expect("replacement evidence should parse"),
        second
    );
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("evidence directory should remain readable")
            .count(),
        1,
        "temporary siblings should not be left behind after atomic replacement"
    );
}

#[test]
fn evidence_destination_directory_is_rejected_as_unsafe() {
    let directory = tempfile::tempdir().expect("evidence directory should be available");
    let path = directory.path().join("existing-directory");
    fs::create_dir(&path).expect("unwritable evidence destination should be a directory");
    let evidence = QualificationEvidence::initial();

    assert_eq!(
        persist_evidence(&path, &evidence),
        Err("evidence_path_unsafe")
    );
    assert!(path.is_dir());
}

#[test]
fn repeat_install_requires_a_same_version_or_monotonic_upgrade_observation() {
    let first = PackageObservation {
        phase: PackageObservationPhase::AfterFirstInstall,
        package: PACKAGE.to_owned(),
        installed: true,
        version_code: Some("120".to_owned()),
        version_name: Some("1.2.0".to_owned()),
    };
    let same = PackageObservation {
        phase: PackageObservationPhase::AfterSecondInstall,
        version_code: Some("120".to_owned()),
        ..first.clone()
    };
    let upgraded = PackageObservation {
        phase: PackageObservationPhase::AfterSecondInstall,
        version_code: Some("121".to_owned()),
        ..first.clone()
    };
    let downgraded = PackageObservation {
        phase: PackageObservationPhase::AfterSecondInstall,
        version_code: Some("119".to_owned()),
        ..first.clone()
    };
    let wrong_package = PackageObservation {
        package: "com.other.app".to_owned(),
        ..same.clone()
    };
    let wrong_phase = PackageObservation {
        phase: PackageObservationPhase::Before,
        ..same.clone()
    };

    assert_eq!(
        compare_repeat_versions(&first, &same),
        Ok("same_version_replacement")
    );
    assert_eq!(
        compare_repeat_versions(&first, &upgraded),
        Ok("version_upgrade")
    );
    assert_eq!(
        compare_repeat_versions(&first, &downgraded),
        Err(OutcomeCategory::Version)
    );
    assert_eq!(
        compare_repeat_versions(&first, &wrong_package),
        Err(OutcomeCategory::Version)
    );
    assert_eq!(
        compare_repeat_versions(&first, &wrong_phase),
        Err(OutcomeCategory::Version)
    );
}

#[test]
fn live_install_run_records_require_two_executed_replacements() {
    let expected_step_id = format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}");
    let mut evidence = QualificationEvidence::initial();
    evidence.mark_install_run(
        1,
        Some(expected_step_id.clone()),
        InstallStepStatus::Executed,
        Some(true),
    );
    evidence.mark_install_run(
        2,
        Some(expected_step_id.clone()),
        InstallStepStatus::Executed,
        Some(true),
    );
    assert_eq!(validate_install_run_records(&evidence), Ok(()));

    evidence.mark_install_run(
        2,
        Some(expected_step_id.clone()),
        InstallStepStatus::Skipped,
        Some(true),
    );
    assert_eq!(
        validate_install_run_records(&evidence),
        Err(OutcomeCategory::Install)
    );

    evidence.mark_install_run(
        2,
        Some(expected_step_id),
        InstallStepStatus::Executed,
        Some(false),
    );
    assert_eq!(
        validate_install_run_records(&evidence),
        Err(OutcomeCategory::ReplaceExistingDisabled)
    );
}

#[test]
fn evidence_path_defaults_to_persistent_run_artifact() {
    assert_eq!(
        default_evidence_path(),
        repository_root().join("docs/testing/recipe-qualification/armsx1/live-evidence.json")
    );
}

#[test]
fn evidence_destinations_reject_forbidden_or_unapproved_repository_paths() {
    let default = default_evidence_path();
    assert_eq!(
        resolve_safe_evidence_destination(&default),
        Ok(default.clone())
    );
    assert_eq!(
        resolve_safe_evidence_destination(
            &repository_root().join(".chatgpt/not-created-evidence.json")
        ),
        Err("evidence_path_unsafe")
    );
    assert_eq!(
        resolve_safe_evidence_destination(&repository_root().join("unapproved-evidence.json")),
        Err("evidence_path_unsafe")
    );
    assert_eq!(
        resolve_safe_evidence_destination(Path::new("relative-evidence.json")),
        Err("evidence_path_unsafe")
    );

    let external = tempfile::tempdir().expect("external evidence directory should exist");
    let external_path = external.path().join("armsx1-evidence.json");
    let canonical_external_path = external
        .path()
        .canonicalize()
        .expect("external evidence directory should resolve")
        .join("armsx1-evidence.json");
    assert_eq!(
        resolve_safe_evidence_destination(&external_path),
        Ok(canonical_external_path)
    );
}

#[cfg(unix)]
#[test]
fn evidence_destination_rejects_external_symlink_into_unapproved_repository_path() {
    use std::os::unix::fs::symlink;

    let external = tempfile::tempdir().expect("external evidence directory should exist");
    let repository_alias = external.path().join("repository-alias");
    symlink(repository_root(), &repository_alias).expect("repository alias should be created");
    assert_eq!(
        resolve_safe_evidence_destination(&repository_alias.join("unsafe-evidence.json")),
        Err("evidence_path_unsafe")
    );
}

#[test]
#[ignore = "read-only ADB preflight diagnostic requires explicit operator selection"]
fn armsx1_read_only_preflight_diagnostic() {
    let mut adb = SystemPreflightAdb;
    let evidence = run_preflight_diagnostic_with(
        env::var(OPT_IN).ok().as_deref(),
        env::var(SERIAL).ok().as_deref(),
        env::var(AVD_NAME).ok().as_deref(),
        env::var(ATTESTATION).ok().as_deref(),
        env::var(PACKAGE_ALLOWLIST).ok().as_deref(),
        &mut adb,
    );
    let path = diagnostic_evidence_path().expect("diagnostic evidence path must be safe");
    persist_diagnostic_evidence(&path, &evidence)
        .expect("sanitized preflight diagnostic evidence must be durable");
}

#[test]
#[ignore = "live network and package installation require an operator-attested dedicated AVD"]
fn armsx1_live_emulator_qualification() {
    let evidence_file = evidence_path().expect("qualification evidence path must be safe");
    let mut evidence = QualificationEvidence::initial();
    persist_or_abort(&evidence_file, &evidence);

    let opt_in = env::var(OPT_IN).ok();
    let serial = env::var(SERIAL).ok();
    let avd_name = env::var(AVD_NAME).ok();
    let attestation = env::var(ATTESTATION).ok();
    let package_allowlist = env::var(PACKAGE_ALLOWLIST).ok();
    let identity = validate_invocation(
        opt_in.as_deref(),
        serial.as_deref(),
        avd_name.as_deref(),
        attestation.as_deref(),
        package_allowlist.as_deref(),
    );
    let identity = match identity {
        Ok(identity) => identity,
        Err(_) => fail_with_evidence(&evidence_file, &mut evidence, OutcomeCategory::Preflight),
    };

    let (api_abi, before_version, target_device) = match verify_device(&identity) {
        Ok(observation) => observation,
        Err(code) => fail_with_preflight_code(&evidence_file, &mut evidence, &code),
    };
    evidence.api_abi = Some(api_abi);
    evidence.package_observations.push(before_version.clone());
    if let Err(code) = admit_initial_install(before_version.installed) {
        fail_with_preflight_code(&evidence_file, &mut evidence, code);
    }
    evidence.mark_passed(EvidenceStage::Preflight);
    persist_or_abort(&evidence_file, &evidence);

    let contract = match load_source_hash_contract() {
        Ok(contract) => contract,
        Err(category) => fail_with_evidence(&evidence_file, &mut evidence, category),
    };
    evidence.contract_app_definition_sha256 = Some(contract.app_definition_sha256.clone());
    evidence.contract_recipe_sha256 = Some(contract.recipe_sha256.clone());
    let mut prepared = match live_plan(target_device) {
        Ok(prepared) => prepared,
        Err(category) => fail_with_evidence(&evidence_file, &mut evidence, category),
    };
    let app_sha256 = prepared.app_definition_sha256.clone();
    let recipe_sha256 = prepared.recipe_sha256.clone();
    evidence.app_definition_sha256 = Some(app_sha256.clone());
    evidence.recipe_sha256 = Some(recipe_sha256.clone());
    let plan = match prepared.plan.take() {
        Some(plan) => plan,
        None => fail_with_evidence(&evidence_file, &mut evidence, OutcomeCategory::UnsafePlan),
    };
    let review = match prepared.review.take() {
        Some(review) => review,
        None => fail_with_evidence(&evidence_file, &mut evidence, OutcomeCategory::UnsafePlan),
    };
    let plan_digest = match prepared.plan_digest.take() {
        Some(digest) => digest,
        None => fail_with_evidence(&evidence_file, &mut evidence, OutcomeCategory::UnsafePlan),
    };
    let review_digest = match serde_json::to_vec(&review) {
        Ok(review) => sha256_hex(&review),
        Err(_) => fail_with_evidence(&evidence_file, &mut evidence, OutcomeCategory::UnsafePlan),
    };
    evidence.plan_digest = Some(plan_digest);
    evidence.review_digest = Some(review_digest);

    let temporary = match tempfile::tempdir() {
        Ok(temporary) => temporary,
        Err(_) => fail_with_evidence(&evidence_file, &mut evidence, OutcomeCategory::RunnerSetup),
    };
    let cache = temporary.path().join("cache");
    let admission = admit_live_runner(
        &contract,
        &app_sha256,
        &recipe_sha256,
        package_allowlist.as_deref(),
        &plan,
        &review,
        IsolatedCacheRoots {
            runtime: temporary.path(),
            cache: &cache,
        },
    );
    if let Err(category) = &admission {
        fail_with_evidence(&evidence_file, &mut evidence, *category);
    }
    evidence.mark_passed(EvidenceStage::PlanValidation);
    persist_or_abort(&evidence_file, &evidence);

    // Construct the production ADB adapter only after source and reviewed-plan admission.
    let runtime = after_admission(admission, || {
        fs::create_dir_all(&cache).map_err(|_| OutcomeCategory::RunnerSetup)?;
        let device = VerifiedInstallDevice::new(
            crate::executor::adb::RealAdbDevice::new("adb", Some(identity.serial.clone())),
            contract.expected_apk_sha256.clone(),
            system_apk_signature_verifier,
        );
        let adapters = crate::executor::ExecutorAdapters::with_device_and_sandbox_roots(
            device,
            temporary.path().join("runtime"),
            cache.clone(),
            temporary.path().join("device"),
            vec![],
            false,
        );
        let runner = crate::executor::ExecutorRunner::new(adapters);
        Ok::<_, OutcomeCategory>((runner, temporary))
    });
    let (mut runner, temporary) = match runtime {
        Err(category) => fail_with_evidence(&evidence_file, &mut evidence, category),
        Ok(Err(category)) => fail_with_evidence(&evidence_file, &mut evidence, category),
        Ok(Ok(runtime)) => runtime,
    };
    evidence.mark_passed(EvidenceStage::RunnerSetup);
    persist_or_abort(&evidence_file, &evidence);

    let install_step_id = format!("{EXPECTED_RECIPE}{EXPECTED_INSTALL_STEP_SUFFIX}");
    evidence.execution = "ATTEMPTED".to_owned();
    persist_or_abort(&evidence_file, &evidence);
    let first = runner.run(&plan);
    let first_failure =
        record_execution_result(&mut evidence, 1, &install_step_id, true, true, &first);
    persist_or_abort(&evidence_file, &evidence);
    if let Some(category) = first_failure.primary {
        fail_with_runtime_evidence(&evidence_file, &mut evidence, category, runner, temporary);
    }
    if first_failure.cleanup_incomplete {
        fail_with_runtime_evidence(
            &evidence_file,
            &mut evidence,
            OutcomeCategory::IncompleteCleanup,
            runner,
            temporary,
        );
    }

    let after_first = match package_version_observation(
        &identity.serial,
        PackageObservationPhase::AfterFirstInstall,
    ) {
        Ok(observation)
            if observation.package == PACKAGE
                && observation.phase == PackageObservationPhase::AfterFirstInstall
                && observation.installed =>
        {
            observation
        }
        _ => fail_with_runtime_evidence(
            &evidence_file,
            &mut evidence,
            OutcomeCategory::Version,
            runner,
            temporary,
        ),
    };
    evidence.package_observations.push(after_first.clone());
    persist_or_abort(&evidence_file, &evidence);

    let second = runner.run(&plan);
    let second_failure =
        record_execution_result(&mut evidence, 2, &install_step_id, true, false, &second);
    persist_or_abort(&evidence_file, &evidence);
    if let Some(category) = second_failure.primary {
        fail_with_runtime_evidence(&evidence_file, &mut evidence, category, runner, temporary);
    }
    if second_failure.cleanup_incomplete {
        fail_with_runtime_evidence(
            &evidence_file,
            &mut evidence,
            OutcomeCategory::IncompleteCleanup,
            runner,
            temporary,
        );
    }
    if let Err(category) = validate_install_run_records(&evidence) {
        fail_with_runtime_evidence(&evidence_file, &mut evidence, category, runner, temporary);
    }

    let after_second = match package_version_observation(
        &identity.serial,
        PackageObservationPhase::AfterSecondInstall,
    ) {
        Ok(observation)
            if observation.package == PACKAGE
                && observation.phase == PackageObservationPhase::AfterSecondInstall
                && observation.installed =>
        {
            observation
        }
        _ => fail_with_runtime_evidence(
            &evidence_file,
            &mut evidence,
            OutcomeCategory::Version,
            runner,
            temporary,
        ),
    };
    evidence.package_observations.push(after_second.clone());
    let repeat_kind = match compare_repeat_versions(&after_first, &after_second) {
        Ok(kind) => kind,
        Err(category) => {
            fail_with_runtime_evidence(&evidence_file, &mut evidence, category, runner, temporary)
        }
    };
    evidence.version_outcome = Some(repeat_kind.to_owned());
    evidence.mark_passed(EvidenceStage::Version);
    persist_or_abort(&evidence_file, &evidence);

    close_live_runtime(&mut evidence, runner, temporary);
    if evidence.cleanup.status != CleanupStatus::Complete {
        persist_or_abort(&evidence_file, &evidence);
        panic!(
            "qualification_blocked:{:?}",
            OutcomeCategory::IncompleteCleanup
        );
    }
    if let Err(category) = complete_live_qualification(&mut evidence) {
        evidence.fail(category);
        persist_or_abort(&evidence_file, &evidence);
        panic!("qualification_blocked:{category:?}");
    }
    persist_or_abort(&evidence_file, &evidence);
    eprintln!("{}", json!({"qualification_evidence": evidence}));
}
