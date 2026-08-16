//! Automated qualification for the authored BIOS-copy workflow.
//!
//! These tests bind expectations to the authored recipe bytes, plan the real
//! catalog through runtime configuration, inspect the production review, and
//! execute the unchanged generated plan through deterministic device adapters.
//! No test in this module invokes ADB, a physical device, a live network
//! request, or an ignored physical-qualification harness.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::executor::{
    DeviceOperationError, ExecutorAdapters, ExecutorDevice, ExecutorRunner, FakeDryRunDevice,
    StepRunStatus,
};
use crate::planner::{BindingSource, ExecutionParamValue, ExecutionPlan, ExecutionStep};
use crate::runtime_configuration::PlanConfigurationResult;

const TARGET_RECIPE: &str = "feature.copy_bios";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.generic.base";
const BIOS_INPUT_KEY: &str = "feature.copy_bios/bios_source_dir";
const BIOS_DESTINATION: &str = "/sdcard/RetroArch/system";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BiosQualificationContract {
    schema_version: i64,
    target_recipe: String,
    planning_device_plan: String,
    authored_source: AuthoredSourceContract,
    selected_recipes: Vec<String>,
    expanded_recipes: Vec<String>,
    recipe_constraint_capabilities: Vec<String>,
    qualification_context_capabilities: Vec<String>,
    required_inputs: Vec<String>,
    optional_inputs: Vec<String>,
    required_operation_families: Vec<String>,
    copy_policy: String,
    destination: String,
    verification: VerificationContract,
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
struct VerificationContract {
    #[serde(rename = "type")]
    type_name: String,
    path: String,
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
    repository_root().join("tests/fixtures/recipe-qualification/bios/qualification-contract.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_contract() -> BiosQualificationContract {
    let text = fs::read_to_string(contract_path())
        .expect("BIOS qualification contract should be readable");
    let contract: BiosQualificationContract = serde_json::from_str(&text)
        .expect("BIOS qualification contract should deserialize strictly");
    assert_eq!(contract.schema_version, 1);
    assert_eq!(contract.target_recipe, TARGET_RECIPE);
    assert_eq!(contract.planning_device_plan, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(contract.selected_recipes, vec![TARGET_RECIPE]);
    assert_eq!(contract.expanded_recipes, vec![TARGET_RECIPE]);
    assert_eq!(contract.destination, BIOS_DESTINATION);
    assert_eq!(contract.verification.type_name, "path_exists");
    assert_eq!(contract.verification.path, BIOS_DESTINATION);
    assert_eq!(contract.automated_status, "qualified");
    assert_eq!(contract.physical_status, "deferred");
    assert_eq!(
        contract.physical_cleanup_authority,
        "not_authorized_for_recipe_qualification"
    );
    assert!(!contract.live_network_required_for_automated_qualification);
    contract
}

/// Prepare the real authored BIOS configuration through the production
/// runtime-configuration path. Explicit selection prevents the device plan's
/// default RetroArch recipe from entering this standalone qualification.
fn plan_bios(bios_dir: Option<&Path>) -> PlanConfigurationResult {
    use crate::catalog_source::CatalogSnapshot;
    use crate::model::OrderedMap;
    use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    let mut explicit_bindings = OrderedMap::new();
    if let Some(path) = bios_dir {
        explicit_bindings.insert(
            BIOS_INPUT_KEY.to_string(),
            Value::String(path.to_string_lossy().into_owned()),
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
    .expect("real authored BIOS configuration should prepare")
}

fn generated_step(plan: &ExecutionPlan) -> &ExecutionStep {
    let suffix = "/copy_bios_dir";
    let mut matches = plan
        .steps
        .iter()
        .filter(|step| step.recipe_ref == TARGET_RECIPE && step.id.ends_with(suffix));
    let step = matches.next().expect("BIOS copy step should be generated");
    assert!(matches.next().is_none(), "BIOS copy step must be unique");
    step
}

fn runtime_capability_enabled(plan: &ExecutionPlan, capability: &str) -> bool {
    match capability {
        "shared_storage_write" => plan.runtime_capabilities.shared_storage_write,
        other => panic!("unexpected BIOS runtime capability {other}"),
    }
}

#[test]
fn bios_contract_binds_current_source_and_deferred_physical_status() {
    let contract = load_contract();
    let source_path = Path::new(&contract.authored_source.path);
    assert!(source_path.is_relative());
    assert_eq!(
        source_path,
        Path::new("authored/recipes/feature.copy_bios.yaml")
    );
    let resolved = repository_root().join(source_path);
    let canonical_root = repository_root()
        .canonicalize()
        .expect("repo root should canonicalize");
    let canonical_source = resolved
        .canonicalize()
        .expect("authored BIOS recipe should resolve");
    assert!(canonical_source.starts_with(canonical_root));
    let raw = fs::read(&resolved).expect("authored BIOS recipe should be readable");
    assert_eq!(sha256_hex(&raw), contract.authored_source.sha256);
}

#[test]
fn bios_real_authored_plan_and_review_match_qualification_contract() {
    let contract = load_contract();
    let temp = tempfile::tempdir().expect("tempdir should be created");
    let bios_dir = temp.path().join("bios-source");
    fs::create_dir(&bios_dir).expect("BIOS source directory should be created");
    let result = plan_bios(Some(&bios_dir));
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.severity != "error"));
    let plan = result.plan.as_ref().expect("BIOS plan should be generated");
    assert!(result
        .plan_digest
        .as_deref()
        .is_some_and(|digest| !digest.is_empty()));
    assert_eq!(plan.source.selected_recipe_refs, contract.selected_recipes);
    assert_eq!(plan.source.expanded_recipe_refs, contract.expanded_recipes);
    assert_eq!(plan.source.device_plan_ref, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(plan.source.device_profile_ref, "ayaneo.generic");
    assert!(plan.recipes.iter().all(|recipe| recipe.id == TARGET_RECIPE));
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.recipe_ref.as_str())
            .collect::<Vec<_>>(),
        vec![TARGET_RECIPE]
    );

    let step = generated_step(plan);
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
    assert_eq!(contract.required_inputs, vec![BIOS_INPUT_KEY]);
    assert_eq!(contract.optional_inputs, Vec::<String>::new());
    assert_eq!(result.resolved_inputs.len(), 1);
    let binding = &result.resolved_inputs[0];
    assert_eq!(binding.key, BIOS_INPUT_KEY);
    assert_eq!(binding.source, Some(BindingSource::Explicit));
    assert_eq!(
        binding.value,
        Some(Value::String(bios_dir.to_string_lossy().into_owned()))
    );

    assert_eq!(
        step.params.get("dest"),
        Some(&ExecutionParamValue::Literal {
            value: Value::String(BIOS_DESTINATION.to_string()),
        })
    );
    assert_eq!(
        step.params.get("copy_policy"),
        Some(&ExecutionParamValue::Literal {
            value: Value::String(contract.copy_policy.clone()),
        })
    );
    assert_eq!(step.verify.len(), 1);
    assert_eq!(step.verify[0].type_name, contract.verification.type_name);
    assert_eq!(
        step.verify[0].params.get("path"),
        Some(&Value::String(BIOS_DESTINATION.to_string()))
    );

    let review = result
        .review
        .as_ref()
        .expect("production review should exist");
    assert!(review.can_execute);
    assert_eq!(review.features.len(), 1);
    assert!(!review.features[0].automatically_added);
    assert!(review.features[0]
        .sections
        .iter()
        .any(|section| section.kind == "copies"));
    assert_eq!(review.work.action_count, plan.steps.len());
    assert!(review
        .notices
        .iter()
        .all(|notice| notice.severity != "blocker"));
    let serialized = serde_json::to_string(review).expect("review should serialize");
    assert!(serialized.contains("bios-source"));
    assert!(!serialized.contains(&temp.path().to_string_lossy().to_string()));
    assert!(!serialized.contains("serial"));
}

#[test]
fn bios_missing_required_input_prevents_executable_plan_and_review() {
    let result = plan_bios(None);
    assert!(result.plan.is_none());
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "binding_missing")
        .expect("missing BIOS input should produce binding_missing");
    assert_eq!(diagnostic.key.as_deref(), Some(BIOS_INPUT_KEY));
    let binding = result
        .resolved_inputs
        .iter()
        .find(|binding| binding.key == BIOS_INPUT_KEY)
        .expect("missing BIOS binding should remain visible");
    assert!(binding.value.is_none());
    assert!(binding.source.is_none());
}

#[test]
fn bios_nonexistent_required_directory_prevents_executable_plan_and_review() {
    let temp = tempfile::tempdir().expect("tempdir should be created");
    let missing = temp.path().join("missing-bios-source");
    let result = plan_bios(Some(&missing));
    assert!(result.plan.is_none());
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "binding_path_missing")
        .expect("missing BIOS directory should produce binding_path_missing");
    assert_eq!(diagnostic.key.as_deref(), Some(BIOS_INPUT_KEY));
    assert_eq!(diagnostic.provenance, Some(BindingSource::Explicit));
}

struct BiosQualificationWorkspace {
    _temp: tempfile::TempDir,
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    host_input_root: PathBuf,
    bios_source_dir: PathBuf,
}

impl BiosQualificationWorkspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("qualification tempdir should be created");
        let runtime_root = temp.path().join("runtime");
        let cache_root = temp.path().join("cache");
        let fake_device_root = temp.path().join("fake-device");
        let host_input_root = temp.path().join("host-input");
        let bios_source_dir = host_input_root.join("bios-source");
        let psx_path = bios_source_dir.join("sony/psx/scph5501.bin");
        let gba_path = bios_source_dir.join("nintendo/gba/gba_bios.bin");
        fs::create_dir_all(psx_path.parent().expect("PSX BIOS parent should exist"))
            .expect("PSX BIOS parent should be created");
        fs::create_dir_all(gba_path.parent().expect("GBA BIOS parent should exist"))
            .expect("GBA BIOS parent should be created");
        fs::write(&psx_path, b"psx-bios\n").expect("PSX BIOS fixture should be written");
        fs::write(&gba_path, b"gba-bios\n").expect("GBA BIOS fixture should be written");
        Self {
            _temp: temp,
            runtime_root,
            cache_root,
            fake_device_root,
            host_input_root,
            bios_source_dir,
        }
    }

    fn source_file(&self, relative: &str) -> PathBuf {
        self.bios_source_dir.join(relative)
    }

    fn fake_device_file(&self, relative: &str) -> PathBuf {
        self.fake_device_root.join(relative)
    }
}

fn normal_bios_adapters(workspace: &BiosQualificationWorkspace) -> ExecutorAdapters {
    ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
    )
}

#[test]
fn bios_generated_plan_copies_nested_files_successfully_without_network_or_adb() {
    let workspace = BiosQualificationWorkspace::new();
    let prepared = plan_bios(Some(&workspace.bios_source_dir));
    let plan = prepared.plan.expect("BIOS plan should be generated");
    let step = generated_step(&plan);
    assert_eq!(step.type_name, "copy_files");
    assert_eq!(
        step.params.get("dest"),
        Some(&ExecutionParamValue::Literal {
            value: Value::String(BIOS_DESTINATION.to_string()),
        })
    );
    assert_eq!(
        step.params.get("copy_policy"),
        Some(&ExecutionParamValue::Literal {
            value: Value::String("sync".to_string()),
        })
    );
    assert_eq!(step.verify.len(), 1);
    assert_eq!(step.verify[0].type_name, "path_exists");
    assert_eq!(
        step.verify[0].params.get("path"),
        Some(&Value::String(BIOS_DESTINATION.to_string()))
    );

    let mut runner = ExecutorRunner::new(normal_bios_adapters(&workspace));
    let result = runner.run(&plan);

    assert!(
        result.success,
        "generated BIOS plan should execute successfully"
    );
    assert_eq!(result.total_steps, plan.steps.len());
    assert_eq!(result.total_steps, 1);
    let record = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/copy_bios_dir"))
        .expect("BIOS copy execution record should exist");
    assert_eq!(record.status, StepRunStatus::Executed);
    assert!(result.steps.iter().all(|record| {
        !matches!(
            record.status,
            StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
        )
    }));

    let psx_destination =
        workspace.fake_device_file("sdcard/RetroArch/system/sony/psx/scph5501.bin");
    let gba_destination =
        workspace.fake_device_file("sdcard/RetroArch/system/nintendo/gba/gba_bios.bin");
    assert_eq!(
        fs::read(psx_destination).expect("nested PSX BIOS should be copied"),
        fs::read(workspace.source_file("sony/psx/scph5501.bin"))
            .expect("source PSX BIOS should be readable")
    );
    assert_eq!(
        fs::read(gba_destination).expect("nested GBA BIOS should be copied"),
        fs::read(workspace.source_file("nintendo/gba/gba_bios.bin"))
            .expect("source GBA BIOS should be readable")
    );
}

#[derive(Debug, Default)]
struct VerificationFailDevice {
    inner: FakeDryRunDevice,
    missing_path: String,
}

impl VerificationFailDevice {
    fn for_missing_path(path: &str) -> Self {
        Self {
            inner: FakeDryRunDevice::default(),
            missing_path: path.to_string(),
        }
    }

    fn commands(&self) -> &[Vec<String>] {
        self.inner.commands()
    }
}

impl ExecutorDevice for VerificationFailDevice {
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
        <FakeDryRunDevice as ExecutorDevice>::push(&mut self.inner, source, dest, sync)
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
        let observed = <FakeDryRunDevice as ExecutorDevice>::path_exists(&mut self.inner, path)?;
        if path == self.missing_path {
            Ok(false)
        } else {
            Ok(observed)
        }
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
fn verification_fail_device_delegates_non_verification_operations() {
    let mut device = VerificationFailDevice::for_missing_path(BIOS_DESTINATION);
    assert!(!device.uses_fake_device_filesystem());
    device
        .install_apk(Path::new("/tmp/qualification.apk"), true)
        .expect("install should delegate");
    device
        .push(
            Path::new("/tmp/qualification.bin"),
            "/sdcard/file.bin",
            true,
        )
        .expect("push should delegate");
    device
        .mkdir_p("/sdcard/RetroArch/system")
        .expect("mkdir should delegate");
    device
        .remove_file("/sdcard/file.bin")
        .expect("remove file should delegate");
    device
        .remove_tree("/sdcard/RetroArch/system")
        .expect("remove tree should delegate");
    device
        .copy_on_device("/sdcard/source", "/sdcard/destination", true, false)
        .expect("device copy should delegate");
    assert!(!device
        .package_installed("com.example.qualification")
        .expect("package predicate should delegate"));
    assert!(!device
        .path_exists("/sdcard/other")
        .expect("path predicate should delegate"));
    assert!(!device
        .path_is_dir("/sdcard/other")
        .expect("directory predicate should delegate"));
    device
        .run_plan_command(vec!["qualification".to_string()])
        .expect("plan command should delegate");
    device
        .launch_app("com.example.qualification", Some(".MainActivity"))
        .expect("launch should delegate");
    device
        .force_stop_app("com.example.qualification")
        .expect("force stop should delegate");
}

#[test]
fn bios_destination_verification_failure_fails_the_unchanged_generated_plan() {
    let workspace = BiosQualificationWorkspace::new();
    let prepared = plan_bios(Some(&workspace.bios_source_dir));
    let plan = prepared.plan.expect("BIOS plan should be generated");
    let plan_before = plan.clone();
    let device = VerificationFailDevice::for_missing_path(BIOS_DESTINATION);
    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        device,
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
        "executor qualification must not mutate the plan"
    );
    assert!(!result.success);
    let record = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/copy_bios_dir"))
        .expect("BIOS copy execution record should exist");
    assert_eq!(record.status, StepRunStatus::Failed);
    assert!(record
        .message
        .as_deref()
        .is_some_and(|message| !message.is_empty()));

    let commands = runner.adapters().device().commands();
    let mkdir_index = commands
        .iter()
        .position(|command| {
            command.first().map(String::as_str) == Some("mkdir_p")
                && command.get(1).map(String::as_str) == Some(BIOS_DESTINATION)
        })
        .expect("copy should create the authored destination");
    let push_index = commands
        .iter()
        .position(|command| command.first().map(String::as_str) == Some("push_sync"))
        .expect("copy should delegate at least one synced push");
    let verify_index = commands
        .iter()
        .position(|command| {
            command.first().map(String::as_str) == Some("path_exists")
                && command.get(1).map(String::as_str) == Some(BIOS_DESTINATION)
        })
        .expect("authored destination verification should be observed");
    assert!(mkdir_index < push_index);
    assert!(push_index < verify_index);
    assert_ne!(record.status, StepRunStatus::Executed);
}
