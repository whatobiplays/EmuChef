//! Automated qualification for the authored ROM-library copy workflow.
//!
//! The qualification uses the real authored catalog and runtime-configuration
//! planner, prepares directory-copy snapshots before execution, and runs each
//! reviewed plan through deterministic sandbox-root adapters. An ignored
//! qualification test exercises the shared-storage route on the authorized
//! AVD; no case uses a physical device or live network request.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::executor::{
    DeviceOperationError, ExecutorAdapters, ExecutorDevice, ExecutorRunner, FakeDryRunDevice,
    StepRunStatus,
};
use crate::planner::BindingSource;
use crate::planner::{ExecutionParamValue, ExecutionPlan, ExecutionStep};
use crate::runtime_configuration::PlanConfigurationResult;

const TARGET_RECIPE: &str = "feature.copy_roms";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.generic.base";
const SOURCE_INPUT_KEY: &str = "feature.copy_roms/source";
const DESTINATION_INPUT_KEY: &str = "feature.copy_roms/destination";
const POLICY_INPUT_KEY: &str = "feature.copy_roms/policy";
const DEFAULT_DESTINATION: &str = "/sdcard/ROMs";
const DEFAULT_POLICY: &str = "merge";
const COPY_STEP_ID_SUFFIX: &str = "copy_rom_library";
const COPY_STEP_PATH_SUFFIX: &str = "/copy_rom_library";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomQualificationContract {
    schema_version: i64,
    target_recipe: String,
    planning_device_plan: String,
    authored_source: AuthoredSourceContract,
    selected_recipes: Vec<String>,
    expanded_recipes: Vec<String>,
    recipe_constraint_capabilities: Vec<String>,
    qualification_context_capabilities: Vec<String>,
    required_operation_families: Vec<String>,
    inputs: RomInputsContract,
    copy_step_id_suffix: String,
    live_network_required_for_automated_qualification: bool,
    automated_status: String,
    physical_status: String,
    physical_cleanup_authority: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthoredSourceContract {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomInputsContract {
    source: RomInputContract,
    destination: RomInputContract,
    policy: RomInputContract,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RomInputContract {
    key: String,
    required: bool,
    #[serde(rename = "type")]
    type_name: String,
    role: String,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    options: Vec<String>,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("backend crate should live beneath the repository root")
        .to_path_buf()
}

fn authored_root() -> PathBuf {
    repository_root().join("authored")
}

fn contract_path() -> PathBuf {
    repository_root().join("tests/fixtures/recipe-qualification/roms/qualification-contract.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_contract() -> RomQualificationContract {
    let text =
        fs::read_to_string(contract_path()).expect("ROM qualification contract should be readable");
    let contract: RomQualificationContract = serde_json::from_str(&text)
        .expect("ROM qualification contract should deserialize strictly");
    assert_eq!(contract.schema_version, 1);
    assert_eq!(contract.target_recipe, TARGET_RECIPE);
    assert_eq!(contract.planning_device_plan, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(contract.selected_recipes, vec![TARGET_RECIPE]);
    assert_eq!(contract.expanded_recipes, vec![TARGET_RECIPE]);
    assert_eq!(
        contract.recipe_constraint_capabilities,
        vec!["shared_storage_write"]
    );
    assert_eq!(
        contract.qualification_context_capabilities,
        vec!["shared_storage_write"]
    );
    assert_eq!(contract.required_operation_families, vec!["copy_files"]);
    assert_eq!(contract.inputs.source.key, SOURCE_INPUT_KEY);
    assert!(contract.inputs.source.required);
    assert_eq!(contract.inputs.source.type_name, "directory");
    assert_eq!(contract.inputs.source.role, "rom_library");
    assert_eq!(contract.inputs.source.default, None);
    assert!(contract.inputs.source.options.is_empty());
    assert_eq!(contract.inputs.destination.key, DESTINATION_INPUT_KEY);
    assert!(contract.inputs.destination.required);
    assert_eq!(contract.inputs.destination.type_name, "device_path");
    assert_eq!(contract.inputs.destination.role, "rom_destination");
    assert_eq!(
        contract.inputs.destination.default.as_deref(),
        Some(DEFAULT_DESTINATION)
    );
    assert!(contract.inputs.destination.options.is_empty());
    assert_eq!(contract.inputs.policy.key, POLICY_INPUT_KEY);
    assert!(!contract.inputs.policy.required);
    assert_eq!(contract.inputs.policy.type_name, "enum");
    assert_eq!(contract.inputs.policy.role, "copy_policy");
    assert_eq!(
        contract.inputs.policy.default.as_deref(),
        Some(DEFAULT_POLICY)
    );
    assert_eq!(
        contract.inputs.policy.options,
        vec!["merge", "replace", "sync"]
    );
    assert_eq!(contract.copy_step_id_suffix, COPY_STEP_ID_SUFFIX);
    assert_eq!(contract.automated_status, "qualified");
    assert_eq!(contract.physical_status, "deferred");
    assert_eq!(
        contract.physical_cleanup_authority,
        "not_authorized_for_recipe_qualification"
    );
    assert!(!contract.live_network_required_for_automated_qualification);
    contract
}

/// Prepare the real authored ROM-copy configuration through the production
/// runtime-configuration path. Explicit selection keeps the device plan's
/// default recipe membership out of this standalone qualification.
fn plan_roms(
    source: Option<&Path>,
    destination: Option<&str>,
    policy: Option<&str>,
) -> PlanConfigurationResult {
    use crate::catalog_source::CatalogSnapshot;
    use crate::model::OrderedMap;
    use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    let mut explicit_bindings = OrderedMap::new();
    if let Some(path) = source {
        explicit_bindings.insert(
            SOURCE_INPUT_KEY.to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
    }
    if let Some(destination) = destination {
        explicit_bindings.insert(
            DESTINATION_INPUT_KEY.to_string(),
            Value::String(destination.to_string()),
        );
    }
    if let Some(policy) = policy {
        explicit_bindings.insert(
            POLICY_INPUT_KEY.to_string(),
            Value::String(policy.to_string()),
        );
    }

    plan_configuration(ConfigurationContextRequest {
        catalog,
        configuration_root: None,
        user_configuration: None,
        device_plan: Some(QUALIFICATION_DEVICE_PLAN.to_string()),
        selected_recipes: Some(vec![TARGET_RECIPE.to_string()]),
        explicit_bindings,
        device_context: None,
        target_device: None,
        runtime_capability_availability: None,
    })
    .expect("real authored ROM configuration should prepare")
}

fn generated_copy_step(plan: &ExecutionPlan) -> &ExecutionStep {
    let mut matches = plan.steps.iter().filter(|step| {
        step.recipe_ref == TARGET_RECIPE && step.id.ends_with(COPY_STEP_PATH_SUFFIX)
    });
    let step = matches.next().expect("ROM copy step should be generated");
    assert!(matches.next().is_none(), "ROM copy step must be unique");
    step
}

fn runtime_capability_enabled(plan: &ExecutionPlan, capability: &str) -> bool {
    match capability {
        "shared_storage_write" => plan.runtime_capabilities.shared_storage_write,
        other => panic!("unexpected ROM runtime capability {other}"),
    }
}

fn resolved_input<'a>(
    result: &'a PlanConfigurationResult,
    key: &str,
) -> &'a crate::planner::ResolvedInputBinding {
    result
        .resolved_inputs
        .iter()
        .find(|binding| binding.key == key)
        .unwrap_or_else(|| panic!("resolved input {key} should be present"))
}

struct RomQualificationWorkspace {
    _temp: tempfile::TempDir,
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    host_input_root: PathBuf,
    rom_source_dir: PathBuf,
}

impl RomQualificationWorkspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("qualification tempdir should be created");
        let runtime_root = temp.path().join("runtime");
        let cache_root = temp.path().join("cache");
        let fake_device_root = temp.path().join("fake-device");
        let host_input_root = temp.path().join("host-input");
        let rom_source_dir = host_input_root.join("rom-source");
        let gba_path = rom_source_dir.join("Nintendo/GBA/Metroid Fusion.gba");
        let psx_path = rom_source_dir.join("Sony/PSX/Crash Bandicoot.chd");
        fs::create_dir_all(gba_path.parent().expect("GBA source parent should exist"))
            .expect("GBA source parent should be created");
        fs::create_dir_all(psx_path.parent().expect("PSX source parent should exist"))
            .expect("PSX source parent should be created");
        fs::write(&gba_path, b"source-gba\n").expect("GBA source ROM should be written");
        fs::write(&psx_path, b"source-psx\n").expect("PSX source ROM should be written");
        Self {
            _temp: temp,
            runtime_root,
            cache_root,
            fake_device_root,
            host_input_root,
            rom_source_dir,
        }
    }

    fn source_file(&self, relative: &str) -> PathBuf {
        self.rom_source_dir.join(relative)
    }

    fn fake_device_file(&self, relative: &str) -> PathBuf {
        self.fake_device_root.join(relative)
    }
}

fn normal_rom_adapters(workspace: &RomQualificationWorkspace) -> ExecutorAdapters {
    ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
    )
}

#[test]
fn rom_sync_generated_plan_mirrors_source_directory() {
    let workspace = RomQualificationWorkspace::new();
    let stale_file = workspace.fake_device_file("sdcard/ROMs/destination-only.txt");
    fs::create_dir_all(
        stale_file
            .parent()
            .expect("destination parent should exist"),
    )
    .expect("destination parent should be created");
    fs::write(&stale_file, b"stale\n").expect("stale destination file should be written");

    let prepared = plan_roms(
        Some(&workspace.rom_source_dir),
        Some(DEFAULT_DESTINATION),
        Some("sync"),
    );
    let mut plan = prepared
        .plan
        .expect("production-generated ROM sync plan should exist");
    let step = generated_copy_step(&plan);
    assert_eq!(step.type_name, "copy_files");
    assert_eq!(
        step.params.get("copy_policy"),
        Some(&ExecutionParamValue::Ref {
            ref_value: "inputs.feature.copy_roms/policy".to_string(),
        })
    );

    let mut runner = ExecutorRunner::new(normal_rom_adapters(&workspace));
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("ROM sync review should prepare trusted copy snapshots");
    assert_eq!(plan.copy_commit_snapshots.len(), 1);
    assert!(plan.copy_commit_snapshots[0]
        .reviewed_deletion_set
        .iter()
        .any(|path| path.ends_with("destination-only.txt")));
    let result = runner.run(&plan);

    assert!(result.success, "generated ROM sync plan should execute");
    assert_eq!(
        result
            .steps
            .iter()
            .find(|record| record.step_id.ends_with(COPY_STEP_PATH_SUFFIX))
            .map(|record| &record.status),
        Some(&StepRunStatus::Executed)
    );
    assert!(
        !stale_file.exists(),
        "directory-style sync must remove destination-only files"
    );
    assert_eq!(
        fs::read(workspace.fake_device_file("sdcard/ROMs/Nintendo/GBA/Metroid Fusion.gba"))
            .expect("GBA ROM should be copied"),
        fs::read(workspace.source_file("Nintendo/GBA/Metroid Fusion.gba"))
            .expect("source GBA ROM should be readable")
    );
    assert_eq!(
        fs::read(workspace.fake_device_file("sdcard/ROMs/Sony/PSX/Crash Bandicoot.chd"))
            .expect("PSX ROM should be copied"),
        fs::read(workspace.source_file("Sony/PSX/Crash Bandicoot.chd"))
            .expect("source PSX ROM should be readable")
    );
}

#[test]
fn rom_contract_binds_current_source_and_deferred_physical_status() {
    let contract = load_contract();
    let source_path = Path::new(&contract.authored_source.path);
    assert!(source_path.is_relative());
    assert_eq!(
        source_path,
        Path::new("authored/recipes/feature.copy_roms.yaml")
    );
    let resolved = repository_root().join(source_path);
    let canonical_root = repository_root()
        .canonicalize()
        .expect("repo root should canonicalize");
    let canonical_source = resolved
        .canonicalize()
        .expect("authored ROM recipe should resolve");
    assert!(canonical_source.starts_with(canonical_root));
    let raw = fs::read(&resolved).expect("authored ROM recipe should be readable");
    assert_eq!(sha256_hex(&raw), contract.authored_source.sha256);
}

#[test]
fn rom_real_authored_plan_and_review_match_qualification_contract() {
    let contract = load_contract();
    let workspace = RomQualificationWorkspace::new();
    let result = plan_roms(Some(&workspace.rom_source_dir), None, None);
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.severity != "error"));
    let plan = result.plan.as_ref().expect("ROM plan should be generated");
    assert!(result.copy_commit_preparation_required);
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    assert_eq!(plan.source.selected_recipe_refs, contract.selected_recipes);
    assert_eq!(plan.source.expanded_recipe_refs, contract.expanded_recipes);
    assert_eq!(plan.source.device_plan_ref, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(plan.source.device_profile_ref, "ayaneo.generic");
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.recipe_ref.as_str())
            .collect::<Vec<_>>(),
        vec![TARGET_RECIPE]
    );

    let step = generated_copy_step(plan);
    assert_eq!(step.type_name, "copy_files");
    assert_eq!(
        step.constraints.capabilities,
        contract.recipe_constraint_capabilities
    );
    for capability in &contract.qualification_context_capabilities {
        assert!(runtime_capability_enabled(plan, capability));
    }
    assert_eq!(
        contract.required_operation_families,
        vec![step.type_name.clone()]
    );
    assert!(
        step.verify.is_empty(),
        "the authored ROM step has verify: []"
    );

    assert_eq!(result.resolved_inputs.len(), 3);
    let source_binding = resolved_input(&result, SOURCE_INPUT_KEY);
    assert_eq!(source_binding.type_name, "directory");
    assert_eq!(source_binding.source, Some(BindingSource::Explicit));
    assert_eq!(
        source_binding.value,
        Some(Value::String(
            workspace.rom_source_dir.to_string_lossy().into_owned()
        ))
    );
    let destination_binding = resolved_input(&result, DESTINATION_INPUT_KEY);
    assert_eq!(destination_binding.type_name, "device_path");
    assert_eq!(
        destination_binding.source,
        Some(BindingSource::RecipeDefault)
    );
    assert_eq!(
        destination_binding.value,
        Some(Value::String(DEFAULT_DESTINATION.to_string()))
    );
    let policy_binding = resolved_input(&result, POLICY_INPUT_KEY);
    assert_eq!(policy_binding.type_name, "enum");
    assert_eq!(policy_binding.source, Some(BindingSource::RecipeDefault));
    assert_eq!(
        policy_binding.value,
        Some(Value::String(DEFAULT_POLICY.to_string()))
    );

    for (param, expected_ref) in [
        ("source", "inputs.feature.copy_roms/source"),
        ("dest", "inputs.feature.copy_roms/destination"),
        ("copy_policy", "inputs.feature.copy_roms/policy"),
    ] {
        assert_eq!(
            step.params.get(param),
            Some(&ExecutionParamValue::Ref {
                ref_value: expected_ref.to_string(),
            })
        );
    }

    fs::create_dir_all(&workspace.fake_device_root)
        .expect("fake device root should exist before snapshot preparation");
    let mut reviewed_plan = plan.clone();
    let mut runner = ExecutorRunner::new(normal_rom_adapters(&workspace));
    runner
        .prepare_copy_commit_snapshots(&mut reviewed_plan)
        .expect("reviewed ROM destination snapshot should be prepared");
    let plan_digest = crate::plan_digest::execution_plan_digest(&reviewed_plan)
        .expect("prepared ROM plan digest should be available");
    assert!(!plan_digest.is_empty());
    let review = crate::review_projection::project_review(&reviewed_plan, &result.diagnostics);
    assert!(review.can_execute);
    assert_eq!(review.features.len(), 1);
    assert_eq!(review.features[0].name, "Copy ROM library");
    assert!(!review.features[0].automatically_added);
    let copies = review.features[0]
        .sections
        .iter()
        .find(|section| section.kind == "copies")
        .expect("ROM review should expose a copies section");
    assert_eq!(copies.actions.len(), 1);
    assert_eq!(copies.actions[0].title, "Copy ROM library");
    assert_eq!(review.work.action_count, 1);
    assert!(review
        .notices
        .iter()
        .all(|notice| notice.severity != "blocker"));
    let serialized = serde_json::to_string(&review).expect("ROM review should serialize");
    assert!(serialized.contains("rom-source"));
    assert!(serialized.contains(DEFAULT_DESTINATION));
    assert!(!serialized.contains(&workspace._temp.path().to_string_lossy().to_string()));
    assert!(!serialized.contains("serial"));
    assert!(!serialized.contains("runtimeAuthority"));
}

#[test]
fn rom_missing_required_source_prevents_executable_plan_and_review() {
    let result = plan_roms(None, None, None);
    assert!(result.plan.is_none());
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "binding_missing")
        .expect("missing ROM source should produce binding_missing");
    assert_eq!(diagnostic.key.as_deref(), Some(SOURCE_INPUT_KEY));
    let binding = resolved_input(&result, SOURCE_INPUT_KEY);
    assert!(binding.value.is_none());
    assert!(binding.source.is_none());
}

#[test]
fn rom_nonexistent_required_source_prevents_executable_plan_and_review() {
    let temp = tempfile::tempdir().expect("tempdir should be created");
    let missing = temp.path().join("missing-rom-source");
    let result = plan_roms(Some(&missing), None, None);
    assert!(result.plan.is_none());
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "binding_path_missing")
        .expect("missing ROM directory should produce binding_path_missing");
    assert_eq!(diagnostic.key.as_deref(), Some(SOURCE_INPUT_KEY));
    assert_eq!(diagnostic.provenance, Some(BindingSource::Explicit));
}

#[test]
fn rom_destination_validation_is_fail_closed_and_accepts_storage_prefix() {
    let workspace = RomQualificationWorkspace::new();
    let invalid = plan_roms(
        Some(&workspace.rom_source_dir),
        Some("/data/local/tmp/roms"),
        None,
    );
    assert!(invalid.plan.is_none());
    assert!(invalid.plan_digest.is_none());
    assert!(invalid.review.is_none());
    let diagnostic = invalid
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "binding_validation_failed")
        .expect("disallowed ROM destination should fail validation");
    assert_eq!(diagnostic.key.as_deref(), Some(DESTINATION_INPUT_KEY));
    assert_eq!(diagnostic.provenance, Some(BindingSource::Explicit));

    let accepted = plan_roms(
        Some(&workspace.rom_source_dir),
        Some("/storage/emulated/0/Emulation/ROMs"),
        None,
    );
    assert!(accepted.plan.is_some());
    let destination = resolved_input(&accepted, DESTINATION_INPUT_KEY);
    assert_eq!(destination.source, Some(BindingSource::Explicit));
    assert_eq!(
        destination.value,
        Some(Value::String(
            "/storage/emulated/0/Emulation/ROMs".to_string()
        ))
    );
}

#[test]
fn rom_copy_policy_accepts_authored_options() {
    let workspace = RomQualificationWorkspace::new();
    for policy in ["merge", "replace", "sync"] {
        let result = plan_roms(Some(&workspace.rom_source_dir), None, Some(policy));
        assert!(result.plan.is_some(), "policy {policy} should plan");
        let binding = resolved_input(&result, POLICY_INPUT_KEY);
        assert_eq!(binding.source, Some(BindingSource::Explicit));
        assert_eq!(binding.value, Some(Value::String(policy.to_string())));
    }
}

#[test]
fn rom_copy_policy_rejects_unsupported_values() {
    let workspace = RomQualificationWorkspace::new();
    let result = plan_roms(Some(&workspace.rom_source_dir), None, Some("overwrite"));
    assert!(result.plan.is_none());
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "binding_validation_failed")
        .expect("unsupported ROM policy should fail validation");
    assert_eq!(diagnostic.key.as_deref(), Some(POLICY_INPUT_KEY));
    assert_eq!(diagnostic.provenance, Some(BindingSource::Explicit));
}

#[test]
fn rom_default_merge_preserves_unrelated_content_and_plan_identity() {
    let workspace = RomQualificationWorkspace::new();
    let keep = workspace.fake_device_file("sdcard/ROMs/Existing/keep.txt");
    let collision = workspace.fake_device_file("sdcard/ROMs/Nintendo/GBA/Metroid Fusion.gba");
    fs::create_dir_all(keep.parent().expect("keep parent should exist"))
        .expect("keep parent should be created");
    fs::create_dir_all(collision.parent().expect("collision parent should exist"))
        .expect("collision parent should be created");
    fs::write(&keep, b"keep-me\n").expect("unrelated destination file should be written");
    fs::write(&collision, b"old-gba\n").expect("collision destination file should be written");

    let prepared = plan_roms(Some(&workspace.rom_source_dir), None, None);
    let mut plan = prepared.plan.expect("default ROM merge plan should exist");
    let mut runner = ExecutorRunner::new(normal_rom_adapters(&workspace));
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("reviewed ROM destination snapshot should be prepared");
    let plan_before = plan.clone();
    let result = runner.run(&plan);

    assert!(result.success, "default ROM merge plan should execute");
    assert_eq!(
        plan, plan_before,
        "executor must not mutate the generated plan"
    );
    assert_eq!(fs::read(&keep).unwrap(), b"keep-me\n");
    assert_eq!(fs::read(&collision).unwrap(), b"source-gba\n");
    assert_eq!(
        fs::read(workspace.fake_device_file("sdcard/ROMs/Sony/PSX/Crash Bandicoot.chd")).unwrap(),
        b"source-psx\n"
    );
    assert_eq!(
        result
            .steps
            .iter()
            .filter(|record| record.status == StepRunStatus::Executed)
            .count(),
        1
    );
}

#[test]
fn rom_shared_directory_snapshots_preserve_selected_spelling() {
    for destination in ["/sdcard/ROMs", "/storage/emulated/0/RetroArch/system"] {
        let workspace = RomQualificationWorkspace::new();
        let stale_file = workspace
            .fake_device_file(
                destination
                    .strip_prefix('/')
                    .expect("shared destination should be absolute"),
            )
            .join("destination-only.txt");
        fs::create_dir_all(
            stale_file
                .parent()
                .expect("destination parent should exist"),
        )
        .expect("destination parent should be created");
        fs::write(&stale_file, b"stale\n").expect("stale entry should be written");

        let prepared = plan_roms(
            Some(&workspace.rom_source_dir),
            Some(destination),
            Some("sync"),
        );
        let mut plan = prepared.plan.expect("ROM sync plan should exist");
        let mut runner = ExecutorRunner::new(normal_rom_adapters(&workspace));
        runner
            .prepare_copy_commit_snapshots(&mut plan)
            .expect("shared destination should receive a trusted snapshot");

        let snapshot = plan
            .copy_commit_snapshots
            .first()
            .expect("shared destination snapshot should be present");
        assert_eq!(snapshot.destination_identity, destination);
        assert_eq!(
            snapshot.destination_kind,
            crate::executor::copy_commit::CopyCommitDestinationKind::SharedDevicePath
        );
        assert_eq!(snapshot.reviewed_deletion_set, ["destination-only.txt"]);

        let result = runner.run(&plan);
        assert!(result.success, "reviewed shared-path sync should execute");
        assert!(
            !stale_file.exists(),
            "sync should remove only reviewed stale content"
        );
    }
}

#[test]
fn rom_replace_generated_plan_removes_stale_destination_content() {
    let workspace = RomQualificationWorkspace::new();
    let stale = workspace.fake_device_file("sdcard/ROMs/stale-only.txt");
    fs::create_dir_all(stale.parent().expect("stale parent should exist"))
        .expect("stale parent should be created");
    fs::write(&stale, b"stale\n").expect("stale ROM should be written");

    let prepared = plan_roms(
        Some(&workspace.rom_source_dir),
        Some(DEFAULT_DESTINATION),
        Some("replace"),
    );
    let mut plan = prepared.plan.expect("ROM replace plan should exist");
    let mut runner = ExecutorRunner::new(normal_rom_adapters(&workspace));
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("ROM replace review should prepare trusted copy snapshots");
    assert_eq!(plan.copy_commit_snapshots.len(), 1);
    assert!(plan.copy_commit_snapshots[0]
        .reviewed_deletion_set
        .iter()
        .any(|path| path.ends_with("stale-only.txt")));
    let result = runner.run(&plan);

    assert!(result.success, "ROM replace plan should execute");
    assert!(!stale.exists());
    assert_eq!(
        fs::read(workspace.fake_device_file("sdcard/ROMs/Nintendo/GBA/Metroid Fusion.gba"))
            .unwrap(),
        b"source-gba\n"
    );
    assert_eq!(
        fs::read(workspace.fake_device_file("sdcard/ROMs/Sony/PSX/Crash Bandicoot.chd")).unwrap(),
        b"source-psx\n"
    );
}

#[derive(Debug, Default)]
struct CopyFailureDevice {
    inner: FakeDryRunDevice,
    push_attempts: Arc<Mutex<usize>>,
}

impl ExecutorDevice for CopyFailureDevice {
    fn uses_fake_device_filesystem(&self) -> bool {
        false
    }

    fn uses_fake_copy_commit_adapter(&self) -> bool {
        false
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::install_apk(
            &mut self.inner,
            apk_path,
            replace_existing,
        )
    }

    fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), DeviceOperationError> {
        let _ = (source, dest, sync);
        *self
            .push_attempts
            .lock()
            .expect("copy failure push counter should remain available") += 1;
        Err(DeviceOperationError::other(
            "deterministic ROM copy failure",
        ))
    }

    fn copy_commit_tree_manifest(
        &mut self,
        _root: &str,
    ) -> Result<Vec<crate::executor::copy_commit::CopyManifestEntry>, DeviceOperationError> {
        Ok(Vec::new())
    }

    fn copy_commit_path_binding(&mut self, path: &str) -> Result<String, DeviceOperationError> {
        Ok(format!("rom-copy-failure:{path}"))
    }

    fn copy_commit_create_directory(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::mkdir_p(&mut self.inner, path)
    }

    fn copy_commit_remove_empty_directory(
        &mut self,
        _path: &str,
    ) -> Result<(), DeviceOperationError> {
        Ok(())
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::mkdir_p(&mut self.inner, path)
    }

    fn remove_file(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::remove_file(&mut self.inner, path)
    }

    fn remove_tree(&mut self, path: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::remove_tree(&mut self.inner, path)
    }

    fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::copy_on_device(
            &mut self.inner,
            source,
            dest,
            recursive,
            privileged,
        )
    }

    fn package_installed(&mut self, package_name: &str) -> Result<bool, DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::package_installed(&mut self.inner, package_name)
    }

    fn path_exists(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::path_exists(&mut self.inner, path)
    }

    fn path_is_dir(&mut self, path: &str) -> Result<bool, DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::path_is_dir(&mut self.inner, path)
    }

    fn apply_permission_operation(
        &mut self,
        operation: crate::executor::PermissionOperation,
    ) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::apply_permission_operation(&mut self.inner, operation)
    }

    fn launch_app(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::launch_app(&mut self.inner, package_name, activity)
    }

    fn force_stop_app(&mut self, package_name: &str) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::force_stop_app(&mut self.inner, package_name)
    }
}

#[test]
fn rom_copy_failure_fails_the_unchanged_generated_plan_truthfully() {
    let workspace = RomQualificationWorkspace::new();
    let prepared = plan_roms(Some(&workspace.rom_source_dir), None, None);
    let mut plan = prepared.plan.expect("ROM merge plan should be generated");
    assert!(generated_copy_step(&plan).verify.is_empty());

    let push_attempts = Arc::new(Mutex::new(0));
    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        CopyFailureDevice {
            inner: FakeDryRunDevice::default(),
            push_attempts: Arc::clone(&push_attempts),
        },
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
        false,
    );
    let mut runner = ExecutorRunner::new(adapters);
    fs::create_dir_all(&workspace.fake_device_root)
        .expect("fake device root should exist before snapshot preparation");
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("reviewed ROM destination snapshot should be prepared");
    let plan_before = plan.clone();
    let result = runner.run(&plan);

    assert_eq!(
        *push_attempts
            .lock()
            .expect("copy failure push counter should remain available"),
        1,
        "CopyCommit staging should exercise the deterministic device push failure"
    );
    assert_eq!(
        plan, plan_before,
        "executor must not mutate the generated plan"
    );
    assert!(!result.success);
    assert_eq!(result.steps.len(), 1);
    let record = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with(COPY_STEP_PATH_SUFFIX))
        .expect("ROM copy execution record should exist");
    assert_eq!(record.status, StepRunStatus::Failed);
    assert!(record
        .message
        .as_deref()
        .is_some_and(|message| message.contains("deterministic ROM copy failure")));
    assert!(result.steps.iter().all(|record| {
        !matches!(
            record.status,
            StepRunStatus::Blocked | StepRunStatus::Cancelled
        )
    }));
}

#[derive(Debug, Default)]
struct SharedAvdCommandRecorder {
    inner: crate::executor::adb::ProcessAdbCommandExecutor,
    commands: Arc<Mutex<Vec<crate::executor::adb::AdbCommandResult>>>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl crate::executor::adb::AdbCommandExecutor for SharedAvdCommandRecorder {
    fn run_for(
        &mut self,
        args: &[String],
        operation: crate::owned_process::ProcessOperation,
    ) -> Result<crate::executor::adb::AdbCommandResult, crate::executor::adb::AdbCommandError> {
        match crate::executor::adb::AdbCommandExecutor::run_for(&mut self.inner, args, operation) {
            Ok(result) => {
                self.commands
                    .lock()
                    .expect("ADB command evidence lock should remain available")
                    .push(result.clone());
                Ok(result)
            }
            Err(error) => {
                self.errors
                    .lock()
                    .expect("ADB error evidence lock should remain available")
                    .push(format!("args={args:?}; error={error:?}"));
                Err(error)
            }
        }
    }
}

#[derive(Default)]
struct SharedAvdCleanup {
    serial: String,
    selected_root: Option<String>,
    sibling_sentinel: Option<String>,
}

impl Drop for SharedAvdCleanup {
    fn drop(&mut self) {
        if let Some(selected_root) = &self.selected_root {
            let _ = Command::new("adb")
                .args([
                    "-s",
                    &self.serial,
                    "shell",
                    "rm",
                    "-rf",
                    "--",
                    selected_root,
                ])
                .output();
        }
        if let Some(sibling_sentinel) = &self.sibling_sentinel {
            let _ = Command::new("adb")
                .args([
                    "-s",
                    &self.serial,
                    "shell",
                    "rm",
                    "-rf",
                    "--",
                    sibling_sentinel,
                ])
                .output();
        }
    }
}

fn shared_avd_adb_output(serial: &str, args: &[&str]) -> Output {
    Command::new("adb")
        .arg("-s")
        .arg(serial)
        .args(args)
        .output()
        .expect("ADB should start for the controlled shared-storage qualification")
}

fn assert_shared_avd_shell_uid(serial: &str, phase: &str) {
    let output = shared_avd_adb_output(serial, &["shell", "id", "-u"]);
    assert!(
        output.status.success(),
        "ADB shell UID probe should succeed during {phase}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "2000",
        "shared-storage qualification must use Android shell UID 2000 during {phase}"
    );
}

fn assert_shared_avd_alias_chain(serial: &str) {
    let sdcard_target = shared_avd_adb_output(serial, &["shell", "readlink", "/sdcard"]);
    assert!(sdcard_target.status.success());
    assert_eq!(
        String::from_utf8_lossy(&sdcard_target.stdout).trim(),
        "/storage/self/primary"
    );
    let primary_target =
        shared_avd_adb_output(serial, &["shell", "readlink", "/storage/self/primary"]);
    assert!(primary_target.status.success());
    assert_eq!(
        String::from_utf8_lossy(&primary_target.stdout).trim(),
        "/storage/emulated/0"
    );
    let canonical = shared_avd_adb_output(serial, &["shell", "readlink", "-f", "/sdcard"]);
    assert!(canonical.status.success());
    assert_eq!(
        String::from_utf8_lossy(&canonical.stdout).trim(),
        "/storage/emulated/0"
    );
}

fn assert_shared_avd_path_absent(serial: &str, path: &str) {
    let output = shared_avd_adb_output(serial, &["shell", "test", "!", "-e", path]);
    assert!(
        output.status.success(),
        "ADB path collision check should succeed for {path}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Qualify reviewed ROM Mirror behavior on the authorized non-root AVD.
///
/// The test creates only a unique child directory and sibling sentinel below
/// `/sdcard/ROMs`; it requires the reviewed shared-storage alias chain and
/// shell UID 2000, then verifies the exact stale entry is removed while the
/// sibling sentinel remains unchanged.
#[test]
#[ignore = "requires the authorized EmuChef-Android16-Root AVD on emulator-5554"]
fn avd_shared_copy_commit_mirrors_sdcard_roms_as_shell_uid() {
    use std::time::{SystemTime, UNIX_EPOCH};

    const SERIAL: &str = "emulator-5554";
    let state = shared_avd_adb_output(SERIAL, &["get-state"]);
    assert!(state.status.success());
    assert_eq!(String::from_utf8_lossy(&state.stdout).trim(), "device");
    assert_shared_avd_shell_uid(SERIAL, "before setup");
    assert_shared_avd_alias_chain(SERIAL);

    let unique = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos()
    );
    let selected_root = format!("{DEFAULT_DESTINATION}/.emuchef-copycommit-{unique}");
    let sibling_sentinel = format!("{DEFAULT_DESTINATION}/.emuchef-sibling-{unique}");
    let sibling_sentinel_file = format!("{sibling_sentinel}/sentinel.txt");
    shared_avd_adb_output(SERIAL, &["shell", "mkdir", "-p", DEFAULT_DESTINATION]);
    assert_shared_avd_path_absent(SERIAL, &selected_root);
    assert_shared_avd_path_absent(SERIAL, &sibling_sentinel);

    let mkdir = shared_avd_adb_output(SERIAL, &["shell", "mkdir", &selected_root]);
    assert!(
        mkdir.status.success(),
        "unique selected ROM directory should be created: {}",
        String::from_utf8_lossy(&mkdir.stderr)
    );
    let mut cleanup = SharedAvdCleanup {
        serial: SERIAL.to_string(),
        selected_root: Some(selected_root.clone()),
        sibling_sentinel: None,
    };
    let sentinel_mkdir = shared_avd_adb_output(SERIAL, &["shell", "mkdir", &sibling_sentinel]);
    assert!(
        sentinel_mkdir.status.success(),
        "unique outside sibling sentinel directory should be created: {}",
        String::from_utf8_lossy(&sentinel_mkdir.stderr)
    );
    cleanup.sibling_sentinel = Some(sibling_sentinel.clone());

    let workspace = RomQualificationWorkspace::new();
    fs::create_dir_all(&workspace.fake_device_root)
        .expect("sandbox fake-device root should exist before snapshot preparation");
    let stale_host_file = workspace.host_input_root.join("destination-only.bin");
    let sentinel_host_file = workspace.host_input_root.join("sibling-sentinel.txt");
    fs::write(&stale_host_file, b"stale destination entry\n")
        .expect("stale destination fixture should be written");
    fs::write(&sentinel_host_file, b"outside selected tree\n")
        .expect("outside sibling sentinel should be written");
    let stale_device_file = format!("{selected_root}/destination-only.bin");
    let push_stale = shared_avd_adb_output(
        SERIAL,
        &[
            "push",
            stale_host_file
                .to_str()
                .expect("fixture path should be UTF-8"),
            &stale_device_file,
        ],
    );
    assert!(
        push_stale.status.success(),
        "stale destination fixture should be pushed to the unique selected tree"
    );
    let stale_present = shared_avd_adb_output(SERIAL, &["shell", "test", "-e", &stale_device_file]);
    assert!(
        stale_present.status.success(),
        "stale destination fixture should exist before snapshot preparation"
    );
    let push_sentinel = shared_avd_adb_output(
        SERIAL,
        &[
            "push",
            sentinel_host_file
                .to_str()
                .expect("sentinel fixture path should be UTF-8"),
            &sibling_sentinel_file,
        ],
    );
    assert!(
        push_sentinel.status.success(),
        "outside sibling sentinel fixture should be pushed to its unique directory"
    );

    let prepared = plan_roms(
        Some(&workspace.rom_source_dir),
        Some(&selected_root),
        Some("sync"),
    );
    let mut plan = prepared
        .plan
        .expect("production-generated ROM Mirror plan should exist");
    let commands = Arc::new(Mutex::new(Vec::new()));
    let errors = Arc::new(Mutex::new(Vec::new()));
    let device = crate::executor::adb::RealAdbDevice::with_executor(
        "adb",
        Some(SERIAL),
        SharedAvdCommandRecorder {
            inner: crate::executor::adb::ProcessAdbCommandExecutor::default(),
            commands: commands.clone(),
            errors: errors.clone(),
        },
    );
    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        device,
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
        false,
    );
    let mut runner = ExecutorRunner::new(adapters);
    assert_shared_avd_shell_uid(SERIAL, "before snapshot preparation");
    runner
        .prepare_copy_commit_snapshots(&mut plan)
        .expect("shared ROM destination snapshot should bind under shell UID 2000");
    let snapshot = plan
        .copy_commit_snapshots
        .first()
        .expect("shared ROM plan should retain its prepared CopyCommit snapshot");
    assert_eq!(
        snapshot.operation,
        crate::executor::copy_commit::CopyCommitOperation::Mirror
    );
    assert_eq!(snapshot.reviewed_deletion_set, ["destination-only.bin"]);
    assert_shared_avd_shell_uid(SERIAL, "after snapshot preparation and before mutation");
    let result = runner.run(&plan);
    assert_shared_avd_shell_uid(SERIAL, "after CopyCommit execution");

    let observed = commands
        .lock()
        .expect("ADB command evidence lock should remain available")
        .clone();
    let errors = errors
        .lock()
        .expect("ADB error evidence lock should remain available")
        .clone();
    assert!(
        result.success,
        "shared ROM Mirror should complete through normal shell: steps={:#?}, errors={:#?}, nonzero_commands={:#?}, recent_commands={:#?}",
        result.steps,
        errors,
        observed
            .iter()
            .filter(|command| command.returncode != 0)
            .collect::<Vec<_>>(),
        observed.iter().rev().take(20).collect::<Vec<_>>()
    );
    assert!(
        errors.is_empty(),
        "ADB route recorded command errors: {errors:#?}"
    );
    let copy_record = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with(COPY_STEP_PATH_SUFFIX))
        .expect("ROM CopyCommit should produce an execution record");
    let copy_report = copy_record
        .outputs
        .get("copy_commit_report")
        .expect("successful CopyCommit should expose its execution report");
    assert_eq!(
        copy_report.value["reviewedDeletionSet"],
        serde_json::json!(["destination-only.bin"])
    );
    assert_eq!(
        copy_report.value["deletedPaths"],
        serde_json::json!(["destination-only.bin"])
    );
    assert_eq!(
        copy_report.value["destinationIdentity"].as_str(),
        Some(selected_root.as_str())
    );
    assert_eq!(
        copy_report.value["destinationBindingVerified"].as_bool(),
        Some(true)
    );
    assert_eq!(copy_report.value["stagingVerified"].as_bool(), Some(true));
    assert_eq!(
        copy_report.value["afterPaths"],
        serde_json::json!([
            "Nintendo",
            "Nintendo/GBA",
            "Nintendo/GBA/Metroid Fusion.gba",
            "Sony",
            "Sony/PSX",
            "Sony/PSX/Crash Bandicoot.chd"
        ])
    );
    let uid_results = observed
        .iter()
        .filter(|command| {
            let joined = command.args.join(" ");
            joined.contains(" shell id -u") || joined.contains(" shell 'id -u")
        })
        .map(|command| command.stdout.trim().to_string())
        .collect::<Vec<_>>();
    assert!(
        uid_results.len() >= 2,
        "CopyCommit should verify shell UID during snapshot and immediately before mutation; observed commands: {:?}",
        observed
            .iter()
            .map(|command| &command.args)
            .collect::<Vec<_>>()
    );
    assert!(
        uid_results.iter().all(|uid| uid == "2000"),
        "all in-route shared-storage UID probes must return 2000: {uid_results:?}"
    );
    assert!(
        observed.iter().all(|command| {
            !command.args.iter().any(|arg| arg == "su") && !command.args.join(" ").contains("su -c")
        }),
        "shared CopyCommit must not fall back to root commands"
    );
    let helper_commands = observed
        .iter()
        .filter(|command| {
            command.args.iter().any(|arg| {
                arg.contains("-helper ")
                    && arg.contains(&selected_root)
                    && (arg.contains("remove-file")
                        || arg.contains("ensure-dir")
                        || arg.contains("publish"))
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        helper_commands.len(),
        7,
        "one reviewed deletion, four directory creations, and two file publishes should use the descriptor helper"
    );
    let last_helper_index = observed
        .iter()
        .rposition(|command| {
            command.args.iter().any(|arg| {
                arg.contains("-helper ")
                    && arg.contains(&selected_root)
                    && (arg.contains("remove-file")
                        || arg.contains("ensure-dir")
                        || arg.contains("publish"))
            })
        })
        .expect("selected-root mutation helper should run");
    let final_binding_probe = observed
        .iter()
        .enumerate()
        .skip(last_helper_index + 1)
        .find(|(_, command)| {
            command
                .args
                .join(" ")
                .contains(&format!("readlink -f {selected_root}"))
        })
        .map(|(_, command)| command)
        .expect("reviewed spelling and alias identity should be rechecked after mutation");

    let stale_check =
        shared_avd_adb_output(SERIAL, &["shell", "test", "!", "-e", &stale_device_file]);
    assert!(
        stale_check.status.success(),
        "the exact reviewed destination-only entry should be deleted; execution report: {}",
        serde_json::to_string_pretty(&copy_report.value)
            .expect("CopyCommit report should be JSON serializable")
    );
    for (relative, expected) in [
        (
            "Nintendo/GBA/Metroid Fusion.gba",
            b"source-gba\n".as_slice(),
        ),
        ("Sony/PSX/Crash Bandicoot.chd", b"source-psx\n".as_slice()),
    ] {
        let remote_file = format!("{selected_root}/{relative}");
        let local_file = workspace
            ._temp
            .path()
            .join(format!("pulled-{}", relative.replace('/', "_")));
        let pull = shared_avd_adb_output(
            SERIAL,
            &[
                "pull",
                &remote_file,
                local_file
                    .to_str()
                    .expect("pull destination should be UTF-8"),
            ],
        );
        assert!(pull.status.success());
        assert_eq!(
            fs::read(local_file).expect("mirrored payload should be readable"),
            expected
        );
    }
    let sentinel_check = shared_avd_adb_output(SERIAL, &["shell", "cat", &sibling_sentinel_file]);
    assert!(sentinel_check.status.success());
    assert_eq!(sentinel_check.stdout, b"outside selected tree\n");
    eprintln!(
        "shared-storage CopyCommit report: {}",
        serde_json::to_string_pretty(&copy_report.value)
            .expect("CopyCommit report should be JSON serializable")
    );
    eprintln!(
        "shared-storage exact reviewed binding used by helper: {}",
        helper_commands[0].args.join(" ")
    );
    eprintln!(
        "shared-storage final post-mutation identity probe: {}",
        final_binding_probe.args.join(" ")
    );
    eprintln!("shared-storage in-route shell UID probes: {uid_results:?}");
}
