//! Contract checks for the deferred KONKR Pocket Advance operation-evidence matrix.
//!
//! The fixture defines what a future qualification run must record. It does not
//! contain device measurements or authorize capability changes.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

const DEVICE_PROFILE: &str = "konkr.pocket_advance";
const DEVICE_PLAN: &str = "konkr.pocket_advance.base";
const MATRIX_PATH: &str =
    "tests/fixtures/recipe-qualification/konkr-pocket-advance/operation-evidence-matrix.json";

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualificationScenarioContract {
    id: QualificationScenarioId,
    device_plan_ref: String,
    selected_recipe_refs: Vec<String>,
    preinstalled_app_prerequisite: PreinstalledAppPrerequisiteContract,
    operation_identity: OperationIdentityContract,
    copy_policy: CopyPolicy,
    required_postconditions: Vec<Postcondition>,
    storage_binding: ScenarioStorageBindingContract,
    destination_before_content_manifest: ContentManifestContract,
    source_content_manifest: ContentManifestContract,
    destination_after_content_manifest: ExpectedAfterManifestContract,
    sentinel_preservation: SentinelPreservationContract,
    emulator_side_usability: EmulatorSideUsabilityContract,
    device_evidence_status: EvidenceCaptureStatus,
    physical_status: PhysicalRunStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum QualificationScenarioId {
    InternalStorageRomMerge,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationIdentityContract {
    operation_id: OperationId,
    recipe_ref: String,
    step_id: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreinstalledAppPrerequisiteContract {
    required_before_rom_copy: bool,
    app_ref: String,
    package_name: String,
    pinned_apk_evidence_required: bool,
    installed_package_must_match_pinned_apk: bool,
    capture_status: EvidenceCaptureStatus,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioStorageBindingContract {
    setup_location_id: String,
    volume_kind: ScenarioStorageVolumeKind,
    relative_path: String,
    review_required: bool,
    required_fields: Vec<StorageBindingField>,
    capture_status: EvidenceCaptureStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ScenarioStorageVolumeKind {
    InternalShared,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StorageBindingField {
    SetupLocationId,
    DevicePlan,
    SessionEpoch,
    Name,
    RelativePath,
    VolumeHandle,
    VolumeKind,
    MountRoot,
    VolumeIdentity,
    VolumeContinuity,
    ObservationToken,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContentManifestContract {
    required_fields: Vec<ContentManifestField>,
    capture_status: EvidenceCaptureStatus,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedAfterManifestContract {
    required_fields: Vec<ContentManifestField>,
    capture_status: EvidenceCaptureStatus,
    must_include_source_entries: bool,
    preserve_destination_only_entries: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ContentManifestField {
    RelativePath,
    Sha256,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SentinelPreservationContract {
    relative_path: String,
    required: bool,
    must_exist_before: bool,
    must_be_absent_from_source: bool,
    must_exist_after: bool,
    compared_fields: Vec<ContentManifestField>,
    capture_status: EvidenceCaptureStatus,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmulatorSideUsabilityContract {
    required_checks: Vec<EmulatorSideUsabilityCheck>,
    capture_status: EvidenceCaptureStatus,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EmulatorSideUsabilityCheck {
    ContentDirectoryMatchesReviewedBinding,
    RomAppearsInEmulatorLibrary,
    RomLaunchesSuccessfully,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationEvidenceMatrix {
    schema_version: u32,
    target: TargetContract,
    qualification: QualificationContract,
    device_evidence: EvidenceFieldsContract,
    build_evidence: EvidenceFieldsContract,
    apk_evidence: ApkEvidenceContract,
    execution_strategy: ExecutionStrategyContract,
    default_recipe_selection: Vec<RecipeSelectionContract>,
    operations: Vec<OperationContract>,
    qualification_scenarios: Vec<QualificationScenarioContract>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TargetContract {
    device_profile: String,
    device_plan: String,
    manufacturer: String,
    model: String,
    ambiguous_match_rule: AmbiguousMatchRule,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(PartialEq, Eq)]
enum AmbiguousMatchRule {
    UniqueOrSessionBoundExplicitChoice,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualificationContract {
    automated_evidence: AutomatedEvidenceKind,
    physical_status: PhysicalQualificationStatus,
    physical_tests_run: bool,
    cleanup_authority: String,
    production_capability_promotion: bool,
    release_claims: bool,
    operation_membership_only: bool,
    execution_ready_full_setup: bool,
    cross_recipe_ordering_guaranteed: bool,
    actual_app_usability_qualified: bool,
    full_journey_qualified: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AutomatedEvidenceKind {
    HostOnly,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PhysicalQualificationStatus {
    Deferred,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvidenceFieldsContract {
    required_fields: Vec<String>,
    capture_status: EvidenceCaptureStatus,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EvidenceCaptureStatus {
    NotCaptured,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApkEvidenceContract {
    app_ref: String,
    package_name: String,
    required_fields: Vec<String>,
    capture_status: EvidenceCaptureStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExecutionStrategyContract {
    required_strategy: ExecutionStrategy,
    required_fields: Vec<String>,
    forbidden_authorities: Vec<String>,
    capture_status: EvidenceCaptureStatus,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ExecutionStrategy {
    Unprivileged,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecipeSelectionContract {
    recipe_ref: String,
    selected_by_default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationContract {
    id: OperationId,
    recipe_refs: Vec<String>,
    step_ids: Vec<String>,
    mandatory_step_ids: Vec<String>,
    optional_step_ids: Vec<String>,
    conditional_step_ids: Vec<String>,
    copy_policy: Option<CopyPolicy>,
    copy_policy_options: Vec<CopyPolicy>,
    destination_kind: Option<DestinationKind>,
    destination_ref: Option<String>,
    permission_set: Option<PermissionSet>,
    failure_policy: Option<PermissionFailurePolicy>,
    required_postconditions: Vec<Postcondition>,
    device_evidence_status: EvidenceCaptureStatus,
    physical_status: PhysicalRunStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OperationId {
    ArtifactResolution,
    ApkInstall,
    Initialization,
    ElevatedPermission,
    ResourceProvisioning,
    BiosCopy,
    RomCopy,
    Configuration,
    FinalLaunch,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum CopyPolicy {
    Merge,
    Replace,
    Sync,
    CreateOnly,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DestinationKind {
    AppTarget,
    SetupLocationRef,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PermissionSet {
    Elevated,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PermissionFailurePolicy {
    Warn,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Postcondition {
    ArtifactProvenanceAndSha256Recorded,
    InstalledPackageMatchesApkEvidence,
    BootstrapAndPermissionLaunchesComplete,
    ElevatedPermissionOmittedWhenInapplicable,
    SelectedResourceCopiesMatchReviewedResults,
    BiosSyncMatchesReviewedResultAndDeletionSet,
    SetupLocationAndVolumeIdentityMatchReview,
    RomCopyMatchesSelectedPolicyAndReviewedResult,
    ConfigurationPublicationMeetsExistingProtectedContract,
    FinalLaunchReportsSuccess,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PhysicalRunStatus {
    NotRun,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate should live beneath the repository root")
        .to_path_buf()
}

fn load_matrix() -> OperationEvidenceMatrix {
    serde_json::from_str(&read_matrix_text())
        .expect("KONKR operation-evidence matrix should deserialize strictly")
}

fn read_matrix_value() -> Value {
    serde_json::from_str(&read_matrix_text())
        .expect("KONKR operation-evidence matrix should be valid JSON")
}

fn read_matrix_text() -> String {
    let text = fs::read_to_string(repository_root().join(MATRIX_PATH))
        .expect("KONKR operation-evidence matrix should be readable");
    text
}

fn operation(matrix: &OperationEvidenceMatrix, id: OperationId) -> &OperationContract {
    matrix
        .operations
        .iter()
        .find(|operation| operation.id == id)
        .expect("operation should be present in the evidence matrix")
}

fn validate_rom_merge_scenario(matrix: &OperationEvidenceMatrix) -> Result<(), &'static str> {
    let [scenario] = matrix.qualification_scenarios.as_slice() else {
        return Err("expected exactly one qualification scenario");
    };
    if scenario.device_plan_ref != matrix.target.device_plan
        || scenario.operation_identity.operation_id != OperationId::RomCopy
        || scenario.selected_recipe_refs != [scenario.operation_identity.recipe_ref.as_str()]
    {
        return Err("scenario identity or recipe selection does not match its target");
    }

    let mut rom_operations = matrix
        .operations
        .iter()
        .filter(|operation| operation.id == OperationId::RomCopy);
    let Some(rom_copy) = rom_operations.next() else {
        return Err("catalog ROM copy operation is missing");
    };
    if rom_operations.next().is_some() {
        return Err("catalog ROM copy operation is duplicated");
    }
    if rom_copy.recipe_refs != [scenario.operation_identity.recipe_ref.as_str()]
        || rom_copy.step_ids != [scenario.operation_identity.step_id.as_str()]
        || rom_copy.mandatory_step_ids != [scenario.operation_identity.step_id.as_str()]
        || rom_copy.copy_policy != Some(scenario.copy_policy.clone())
    {
        return Err("scenario recipe, step, or copy policy does not match the ROM operation");
    }

    if scenario.required_postconditions.is_empty()
        || scenario
            .required_postconditions
            .iter()
            .any(|postcondition| !rom_copy.required_postconditions.contains(postcondition))
    {
        return Err("scenario postconditions do not match the catalog ROM operation");
    }

    Ok(())
}

#[test]
fn matrix_binds_future_evidence_to_the_exact_device_build_apk_and_strategy() {
    let matrix = load_matrix();

    assert_eq!(matrix.schema_version, 1);
    assert_eq!(matrix.target.device_profile, DEVICE_PROFILE);
    assert_eq!(matrix.target.device_plan, DEVICE_PLAN);
    assert_eq!(matrix.target.manufacturer, "ARBOR");
    assert_eq!(matrix.target.model, "GT78-VN");
    assert_eq!(
        matrix.target.ambiguous_match_rule,
        AmbiguousMatchRule::UniqueOrSessionBoundExplicitChoice
    );
    assert_eq!(
        matrix.device_evidence.required_fields,
        [
            "exact_device_identity",
            "device_session_id",
            "selected_profile_id",
            "profile_match_resolution",
        ]
    );
    assert_eq!(
        matrix.device_evidence.capture_status,
        EvidenceCaptureStatus::NotCaptured
    );
    assert_eq!(
        matrix.build_evidence.required_fields,
        ["android_version", "android_api_level", "build_fingerprint"]
    );
    assert_eq!(
        matrix.build_evidence.capture_status,
        EvidenceCaptureStatus::NotCaptured
    );
    assert_eq!(matrix.apk_evidence.app_ref, "retroarch");
    assert_eq!(matrix.apk_evidence.package_name, "com.retroarch.aarch64");
    assert_eq!(
        matrix.apk_evidence.required_fields,
        [
            "apk_sha256",
            "artifact_source",
            "artifact_version_or_build_id"
        ]
    );
    assert_eq!(
        matrix.apk_evidence.capture_status,
        EvidenceCaptureStatus::NotCaptured
    );
    assert_eq!(
        matrix.execution_strategy.required_strategy,
        ExecutionStrategy::Unprivileged
    );
    assert_eq!(
        matrix.execution_strategy.required_fields,
        [
            "strategy_id",
            "strategy_digest",
            "root_authority_used",
            "oem_compiler_or_stage_used"
        ]
    );
    assert_eq!(
        matrix.execution_strategy.forbidden_authorities,
        ["root_shell", "oem_compiler", "oem_stage"]
    );
    assert_eq!(
        matrix.execution_strategy.capture_status,
        EvidenceCaptureStatus::NotCaptured
    );
}

#[test]
fn matrix_preserves_default_membership_and_operation_level_requirements() {
    let matrix = load_matrix();

    assert_eq!(
        matrix.default_recipe_selection,
        [
            RecipeSelectionContract {
                recipe_ref: "app.retroarch.provision".to_string(),
                selected_by_default: true,
            },
            RecipeSelectionContract {
                recipe_ref: "feature.copy_bios".to_string(),
                selected_by_default: true,
            },
            RecipeSelectionContract {
                recipe_ref: "feature.copy_roms".to_string(),
                selected_by_default: false,
            },
            RecipeSelectionContract {
                recipe_ref: "app.retroarch.configure".to_string(),
                selected_by_default: false,
            },
        ]
    );

    let artifacts = operation(&matrix, OperationId::ArtifactResolution);
    assert_eq!(artifacts.recipe_refs, ["app.retroarch.provision"]);
    assert_eq!(artifacts.mandatory_step_ids, ["resolve_artifacts"]);

    let install = operation(&matrix, OperationId::ApkInstall);
    assert_eq!(install.recipe_refs, ["app.retroarch.provision"]);
    assert_eq!(install.mandatory_step_ids, ["install_retroarch"]);
    assert_eq!(
        install.required_postconditions,
        [
            Postcondition::ArtifactProvenanceAndSha256Recorded,
            Postcondition::InstalledPackageMatchesApkEvidence,
        ]
    );

    let initialization = operation(&matrix, OperationId::Initialization);
    assert_eq!(initialization.recipe_refs, ["app.retroarch.provision"]);
    assert_eq!(
        initialization.mandatory_step_ids,
        [
            "launch_retroarch_bootstrap",
            "wait_for_retroarch_bootstrap",
            "stop_retroarch_after_bootstrap",
            "grant_retroarch_baseline",
            "launch_retroarch_permissions",
            "wait_for_retroarch_permissions",
            "stop_retroarch_after_permissions",
        ]
    );

    let elevated = operation(&matrix, OperationId::ElevatedPermission);
    assert_eq!(elevated.recipe_refs, ["app.retroarch.provision"]);
    assert_eq!(elevated.conditional_step_ids, ["grant_retroarch_elevated"]);
    assert_eq!(elevated.permission_set, Some(PermissionSet::Elevated));
    assert_eq!(elevated.failure_policy, Some(PermissionFailurePolicy::Warn));

    let resources = operation(&matrix, OperationId::ResourceProvisioning);
    assert_eq!(resources.recipe_refs, ["app.retroarch.provision"]);
    assert_eq!(
        resources.mandatory_step_ids,
        [
            "extract_assets",
            "copy_assets",
            "extract_autoconfig",
            "copy_autoconfig",
            "extract_cheats",
            "copy_cheats",
            "extract_database_rdb",
            "copy_database_rdb",
            "extract_info",
            "copy_info",
            "extract_overlays",
            "copy_overlays",
            "extract_shaders_glsl",
            "copy_shaders_glsl",
        ]
    );
    assert_eq!(
        resources.optional_step_ids,
        [
            "extract_cores",
            "copy_cores",
            "extract_core_system_files",
            "copy_core_system_files",
        ]
    );

    let bios = operation(&matrix, OperationId::BiosCopy);
    assert_eq!(bios.recipe_refs, ["feature.copy_bios"]);
    assert_eq!(bios.optional_step_ids, ["copy_bios_dir"]);
    assert_eq!(bios.copy_policy, Some(CopyPolicy::Sync));
    assert_eq!(bios.destination_kind, Some(DestinationKind::AppTarget));
    assert_eq!(bios.destination_ref.as_deref(), Some("system"));
    assert!(bios
        .required_postconditions
        .contains(&Postcondition::BiosSyncMatchesReviewedResultAndDeletionSet));

    let roms = operation(&matrix, OperationId::RomCopy);
    assert_eq!(roms.recipe_refs, ["feature.copy_roms"]);
    assert_eq!(roms.mandatory_step_ids, ["copy_rom_library"]);
    assert_eq!(
        roms.destination_kind,
        Some(DestinationKind::SetupLocationRef)
    );
    assert_eq!(
        roms.destination_ref.as_deref(),
        Some("feature.copy_roms/destination")
    );
    assert_eq!(roms.copy_policy, Some(CopyPolicy::Merge));
    assert_eq!(
        roms.copy_policy_options
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        [CopyPolicy::Merge, CopyPolicy::Replace, CopyPolicy::Sync]
            .into_iter()
            .collect()
    );
    assert!(roms
        .required_postconditions
        .contains(&Postcondition::SetupLocationAndVolumeIdentityMatchReview));

    let configuration = operation(&matrix, OperationId::Configuration);
    assert_eq!(
        configuration.recipe_refs,
        ["app.retroarch.provision", "app.retroarch.configure"]
    );
    assert_eq!(
        configuration.optional_step_ids,
        [
            "app.retroarch.provision/seed_retroarch_cfg",
            "app.retroarch.configure/seed_retroarch_cfg",
        ]
    );
    assert_eq!(configuration.copy_policy, Some(CopyPolicy::CreateOnly));
    assert!(configuration
        .required_postconditions
        .contains(&Postcondition::ConfigurationPublicationMeetsExistingProtectedContract));

    let launch = operation(&matrix, OperationId::FinalLaunch);
    assert_eq!(launch.recipe_refs, ["app.retroarch.provision"]);
    assert_eq!(
        launch.optional_step_ids,
        ["app.retroarch.provision/launch_retroarch"]
    );
    assert!(launch
        .required_postconditions
        .contains(&Postcondition::FinalLaunchReportsSuccess));
}

#[test]
fn matrix_keeps_host_evidence_separate_from_deferred_physical_qualification() {
    let matrix = load_matrix();

    assert_eq!(
        matrix.qualification.automated_evidence,
        AutomatedEvidenceKind::HostOnly
    );
    assert_eq!(
        matrix.qualification.physical_status,
        PhysicalQualificationStatus::Deferred
    );
    assert!(!matrix.qualification.physical_tests_run);
    assert_eq!(
        matrix.qualification.cleanup_authority,
        "not_authorized_for_recipe_qualification"
    );
    assert!(!matrix.qualification.production_capability_promotion);
    assert!(!matrix.qualification.release_claims);
    assert!(matrix.qualification.operation_membership_only);
    assert!(!matrix.qualification.execution_ready_full_setup);
    assert!(!matrix.qualification.cross_recipe_ordering_guaranteed);
    assert!(!matrix.qualification.actual_app_usability_qualified);
    assert!(!matrix.qualification.full_journey_qualified);

    for operation in &matrix.operations {
        assert!(
            !operation.step_ids.is_empty(),
            "{:?} needs operation steps",
            operation.id
        );
        assert!(!operation.required_postconditions.is_empty());
        assert_eq!(
            operation.device_evidence_status,
            EvidenceCaptureStatus::NotCaptured
        );
        assert_eq!(operation.physical_status, PhysicalRunStatus::NotRun);

        let mandatory = operation.mandatory_step_ids.iter().collect::<BTreeSet<_>>();
        let optional = operation.optional_step_ids.iter().collect::<BTreeSet<_>>();
        let conditional = operation
            .conditional_step_ids
            .iter()
            .collect::<BTreeSet<_>>();
        assert!(mandatory.is_disjoint(&optional));
        assert!(mandatory.is_disjoint(&conditional));
        assert!(optional.is_disjoint(&conditional));
        assert_eq!(
            mandatory
                .union(&optional)
                .copied()
                .chain(conditional.iter().copied())
                .collect::<BTreeSet<_>>(),
            operation.step_ids.iter().collect()
        );
    }
}

#[test]
fn matrix_contract_rejects_unknown_fields() {
    let mut value = read_matrix_value();
    value["unexpected"] = Value::Bool(true);

    assert!(serde_json::from_value::<OperationEvidenceMatrix>(value.clone()).is_err());

    value.as_object_mut().unwrap().remove("unexpected");
    value["qualificationScenarios"][0]["unexpected"] = Value::Bool(true);
    assert!(serde_json::from_value::<OperationEvidenceMatrix>(value).is_err());
}

#[test]
fn matrix_binds_internal_storage_rom_merge_to_required_operation_and_evidence() {
    let matrix = load_matrix();
    validate_rom_merge_scenario(&matrix)
        .unwrap_or_else(|error| panic!("invalid internal-storage ROM Merge scenario: {error}"));

    let scenario = &matrix.qualification_scenarios[0];
    assert_eq!(
        scenario.id,
        QualificationScenarioId::InternalStorageRomMerge
    );
    assert_eq!(scenario.device_plan_ref, DEVICE_PLAN);
    assert_eq!(scenario.selected_recipe_refs, ["feature.copy_roms"]);
    assert_eq!(
        scenario.operation_identity.operation_id,
        OperationId::RomCopy
    );
    assert_eq!(scenario.operation_identity.recipe_ref, "feature.copy_roms");
    assert_eq!(scenario.operation_identity.step_id, "copy_rom_library");
    assert_eq!(scenario.copy_policy, CopyPolicy::Merge);
    assert_eq!(
        scenario.required_postconditions,
        [
            Postcondition::SetupLocationAndVolumeIdentityMatchReview,
            Postcondition::RomCopyMatchesSelectedPolicyAndReviewedResult,
        ]
    );

    let app = &scenario.preinstalled_app_prerequisite;
    assert!(app.required_before_rom_copy);
    assert_eq!(app.app_ref, matrix.apk_evidence.app_ref);
    assert_eq!(app.package_name, matrix.apk_evidence.package_name);
    assert!(app.pinned_apk_evidence_required);
    assert!(app.installed_package_must_match_pinned_apk);
    assert_eq!(app.capture_status, EvidenceCaptureStatus::NotCaptured);

    let binding = &scenario.storage_binding;
    assert_eq!(binding.setup_location_id, "internal-roms");
    assert_eq!(
        binding.volume_kind,
        ScenarioStorageVolumeKind::InternalShared
    );
    assert_eq!(binding.relative_path, "ROMs");
    assert!(binding.review_required);
    assert_eq!(
        binding.required_fields,
        [
            StorageBindingField::SetupLocationId,
            StorageBindingField::DevicePlan,
            StorageBindingField::SessionEpoch,
            StorageBindingField::Name,
            StorageBindingField::RelativePath,
            StorageBindingField::VolumeHandle,
            StorageBindingField::VolumeKind,
            StorageBindingField::MountRoot,
            StorageBindingField::VolumeIdentity,
            StorageBindingField::VolumeContinuity,
            StorageBindingField::ObservationToken,
        ]
    );
    assert_eq!(binding.capture_status, EvidenceCaptureStatus::NotCaptured);

    let expected_manifest_fields = [
        ContentManifestField::RelativePath,
        ContentManifestField::Sha256,
    ];
    assert_eq!(
        scenario.destination_before_content_manifest.required_fields,
        expected_manifest_fields
    );
    assert_eq!(
        scenario.destination_before_content_manifest.capture_status,
        EvidenceCaptureStatus::NotCaptured
    );
    assert_eq!(
        scenario.source_content_manifest.required_fields,
        expected_manifest_fields
    );
    assert_eq!(
        scenario.source_content_manifest.capture_status,
        EvidenceCaptureStatus::NotCaptured
    );
    assert_eq!(
        scenario.destination_after_content_manifest.required_fields,
        expected_manifest_fields
    );
    assert_eq!(
        scenario.destination_after_content_manifest.capture_status,
        EvidenceCaptureStatus::NotCaptured
    );
    assert!(
        scenario
            .destination_after_content_manifest
            .must_include_source_entries
    );
    assert!(
        scenario
            .destination_after_content_manifest
            .preserve_destination_only_entries
    );

    let sentinel = &scenario.sentinel_preservation;
    assert_eq!(sentinel.relative_path, "qualification-sentinel.keep");
    assert!(sentinel.required);
    assert!(sentinel.must_exist_before);
    assert!(sentinel.must_be_absent_from_source);
    assert!(sentinel.must_exist_after);
    assert_eq!(sentinel.compared_fields, expected_manifest_fields);
    assert_eq!(sentinel.capture_status, EvidenceCaptureStatus::NotCaptured);

    assert_eq!(
        scenario.emulator_side_usability.required_checks,
        [
            EmulatorSideUsabilityCheck::ContentDirectoryMatchesReviewedBinding,
            EmulatorSideUsabilityCheck::RomAppearsInEmulatorLibrary,
            EmulatorSideUsabilityCheck::RomLaunchesSuccessfully,
        ]
    );
    assert_eq!(
        scenario.emulator_side_usability.capture_status,
        EvidenceCaptureStatus::NotCaptured
    );
    assert_eq!(
        scenario.device_evidence_status,
        EvidenceCaptureStatus::NotCaptured
    );
    assert_eq!(scenario.physical_status, PhysicalRunStatus::NotRun);

    let authored_root = repository_root().join("authored");
    let recipes = crate::planner::load_top_level_recipes(&authored_root)
        .expect("authored KONKR recipes should load through the planner");
    let plan_parts =
        crate::planner_device_plan::load_planner_input_parts(&authored_root, DEVICE_PLAN, &recipes)
            .expect("the real KONKR Device Plan should load through the planner");
    assert_eq!(plan_parts.device_plan_ref, scenario.device_plan_ref);
    assert!(plan_parts
        .recipe_refs
        .contains(&scenario.operation_identity.recipe_ref));

    let planner_input = crate::planner::PlannerInput::from_authored_device_plan(
        &authored_root,
        DEVICE_PLAN,
        "plan.qualification.matrix-contract.001".to_string(),
        crate::model::OrderedMap::new(),
    )
    .expect("the reviewed KONKR Device Plan should load through the planner path");
    assert_eq!(planner_input.device_plan_ref, scenario.device_plan_ref);
    let rom_recipe = planner_input
        .recipes
        .iter()
        .find(|recipe| recipe.id == scenario.operation_identity.recipe_ref)
        .expect("the scenario Recipe should exist in the real KONKR Device Plan catalog");
    let copy_step = rom_recipe
        .steps
        .iter()
        .find(|step| step.id == scenario.operation_identity.step_id)
        .expect("the scenario operation should exist in the real Recipe");
    assert!(!copy_step.user_toggleable);
    assert!(!planner_input
        .selected_recipe_refs
        .contains(&scenario.operation_identity.recipe_ref));
}

#[test]
fn matrix_rejects_rom_merge_scenarios_detached_from_catalog_or_scope() {
    let mut wrong_plan = load_matrix();
    wrong_plan.qualification_scenarios[0].device_plan_ref =
        "konkr.pocket_air_mini.base".to_string();
    assert!(validate_rom_merge_scenario(&wrong_plan).is_err());

    let mut wrong_step = load_matrix();
    wrong_step.qualification_scenarios[0]
        .operation_identity
        .step_id = "copy_bios_dir".to_string();
    assert!(validate_rom_merge_scenario(&wrong_step).is_err());

    let mut full_setup = load_matrix();
    full_setup.qualification_scenarios[0]
        .selected_recipe_refs
        .push("app.retroarch.provision".to_string());
    assert!(validate_rom_merge_scenario(&full_setup).is_err());

    let mut replacement_copy = load_matrix();
    replacement_copy.qualification_scenarios[0].copy_policy = CopyPolicy::Replace;
    assert!(validate_rom_merge_scenario(&replacement_copy).is_err());

    let mut duplicate_step = load_matrix();
    let rom_copy = duplicate_step
        .operations
        .iter_mut()
        .find(|operation| operation.id == OperationId::RomCopy)
        .unwrap();
    rom_copy
        .mandatory_step_ids
        .push("copy_rom_library".to_string());
    assert!(validate_rom_merge_scenario(&duplicate_step).is_err());

    let mut missing_postcondition = load_matrix();
    let rom_copy = missing_postcondition
        .operations
        .iter_mut()
        .find(|operation| operation.id == OperationId::RomCopy)
        .unwrap();
    rom_copy.required_postconditions.retain(|postcondition| {
        postcondition != &Postcondition::RomCopyMatchesSelectedPolicyAndReviewedResult
    });
    assert!(validate_rom_merge_scenario(&missing_postcondition).is_err());
}

#[test]
fn matrix_rejects_missing_or_duplicate_rom_copy_operations() {
    let mut missing = read_matrix_value();
    missing["operations"]
        .as_array_mut()
        .unwrap()
        .retain(|operation| operation["id"] != "rom_copy");
    let missing: OperationEvidenceMatrix = serde_json::from_value(missing).unwrap();
    assert!(validate_rom_merge_scenario(&missing).is_err());

    let mut duplicate = read_matrix_value();
    let operations = duplicate["operations"].as_array_mut().unwrap();
    let rom_copy = operations
        .iter()
        .find(|operation| operation["id"] == "rom_copy")
        .unwrap()
        .clone();
    operations.push(rom_copy);
    let duplicate: OperationEvidenceMatrix = serde_json::from_value(duplicate).unwrap();
    assert!(validate_rom_merge_scenario(&duplicate).is_err());
}
