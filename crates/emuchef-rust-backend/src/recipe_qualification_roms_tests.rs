//! Automated qualification for the authored ROM-library copy workflow.
//!
//! The qualification uses the real authored catalog and runtime-configuration
//! planner, then executes its unchanged generated plan through deterministic
//! sandbox-root adapters. The tests intentionally do not invoke ADB, a physical
//! device, a live network request, or an ignored physical-qualification
//! harness.

use std::fs;
use std::path::{Path, PathBuf};

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
    let plan = prepared
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
    assert!(result
        .plan_digest
        .as_deref()
        .is_some_and(|digest| !digest.is_empty()));
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

    let review = result
        .review
        .as_ref()
        .expect("production ROM review should exist");
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
    let serialized = serde_json::to_string(review).expect("ROM review should serialize");
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
    let plan = prepared.plan.expect("default ROM merge plan should exist");
    let plan_before = plan.clone();
    let mut runner = ExecutorRunner::new(normal_rom_adapters(&workspace));
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
    let plan = prepared.plan.expect("ROM replace plan should exist");
    let mut runner = ExecutorRunner::new(normal_rom_adapters(&workspace));
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
}

impl ExecutorDevice for CopyFailureDevice {
    fn uses_fake_device_filesystem(&self) -> bool {
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
        let _ = <FakeDryRunDevice as ExecutorDevice>::push(&mut self.inner, source, dest, sync);
        Err(DeviceOperationError::other(
            "deterministic ROM copy failure",
        ))
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

    fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), DeviceOperationError> {
        <FakeDryRunDevice as ExecutorDevice>::run_plan_command(&mut self.inner, command)
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
    let plan = prepared.plan.expect("ROM merge plan should be generated");
    let plan_before = plan.clone();
    assert!(generated_copy_step(&plan).verify.is_empty());

    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        CopyFailureDevice::default(),
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
        false,
    );
    let mut runner = ExecutorRunner::new(adapters);
    let result = runner.run(&plan);

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
