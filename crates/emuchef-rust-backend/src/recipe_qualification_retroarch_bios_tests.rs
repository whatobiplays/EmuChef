use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::artifact_resolver::artifact_local_filename;
use crate::executor::{
    DeviceOperationError, ExecutorAdapters, ExecutorDevice, ExecutorRunner, FakeDryRunDevice,
    StepRunStatus,
};
use crate::model::OrderedMap;
use crate::planner::{BindingSource, ExecutionArtifact, ExecutionParamValue, ExecutionPlan};
use crate::runtime_configuration::PlanConfigurationResult;

const KONKR_DEVICE_PLAN: &str = "ayaneo.konkr_pocket_fit.base";
const KONKR_DEVICE_PROFILE: &str = "ayaneo.konkr_pocket_fit";
const RETROARCH_RECIPE: &str = "app.retroarch.provision";
const BIOS_RECIPE: &str = "feature.copy_bios";
const RETROARCH_INPUT_KEY: &str = "app.retroarch.provision/retroarch_cfg";
const BIOS_INPUT_KEY: &str = "feature.copy_bios/bios_source_dir";
const BIOS_DESTINATION: &str = "/sdcard/RetroArch/system";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CombinedQualificationContract {
    schema_version: i64,
    planning_device_plan: String,
    device_profile: String,
    authored_sources: Vec<AuthoredSourceContract>,
    selected_recipes: Vec<String>,
    expanded_recipes: Vec<String>,
    recipe_constraint_capabilities: Vec<String>,
    qualification_context_capabilities: Vec<String>,
    required_inputs: Vec<String>,
    optional_inputs: Vec<String>,
    required_operation_families: Vec<String>,
    fake_storage_aliases_qualified: bool,
    live_network_required_for_automated_qualification: bool,
    automated_status: String,
    physical_status: String,
    physical_cleanup_authority: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthoredSourceContract {
    source_kind: String,
    id: String,
    path: String,
    sha256: String,
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
    repository_root()
        .join("tests/fixtures/recipe-qualification/retroarch-bios/qualification-contract.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_contract() -> CombinedQualificationContract {
    let text = fs::read_to_string(contract_path())
        .expect("combined qualification contract should be readable");
    let contract: CombinedQualificationContract = serde_json::from_str(&text)
        .expect("combined qualification contract should deserialize strictly");
    assert_eq!(contract.schema_version, 1);
    assert_eq!(contract.planning_device_plan, KONKR_DEVICE_PLAN);
    assert_eq!(contract.device_profile, KONKR_DEVICE_PROFILE);
    assert_eq!(contract.selected_recipes, contract.expanded_recipes);
    assert_eq!(contract.selected_recipes.len(), 2);
    assert_eq!(contract.required_inputs, vec![BIOS_INPUT_KEY]);
    assert_eq!(contract.optional_inputs, vec![RETROARCH_INPUT_KEY]);
    assert!(!contract.fake_storage_aliases_qualified);
    assert!(!contract.live_network_required_for_automated_qualification);
    assert_eq!(contract.automated_status, "qualified");
    assert_eq!(contract.physical_status, "deferred");
    assert_eq!(
        contract.physical_cleanup_authority,
        "not_authorized_for_recipe_qualification"
    );
    contract
}

fn plan_combined(
    bios_source_dir: Option<&Path>,
    retroarch_config: Option<&Path>,
) -> PlanConfigurationResult {
    plan_configuration_for(KONKR_DEVICE_PLAN, None, bios_source_dir, retroarch_config)
}

fn plan_configuration_for(
    device_plan: &str,
    selected_recipes: Option<Vec<String>>,
    bios_source_dir: Option<&Path>,
    retroarch_config: Option<&Path>,
) -> PlanConfigurationResult {
    use crate::catalog_source::CatalogSnapshot;
    use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    let mut explicit_bindings = OrderedMap::new();
    if let Some(path) = bios_source_dir {
        explicit_bindings.insert(
            BIOS_INPUT_KEY.to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
    }
    if let Some(path) = retroarch_config {
        explicit_bindings.insert(
            RETROARCH_INPUT_KEY.to_string(),
            Value::String(path.to_string_lossy().into_owned()),
        );
    }

    plan_configuration(ConfigurationContextRequest {
        catalog,
        configuration_root: None,
        user_configuration: None,
        device_plan: Some(device_plan.to_string()),
        selected_recipes,
        explicit_bindings,
        device_context: None,
        target_device: None,
        runtime_capability_availability: None,
    })
    .expect("real authored combined configuration should prepare")
}

#[test]
fn combined_contract_binds_all_authored_sources_and_defers_physical_aliases() {
    let contract = load_contract();
    let expected = [
        (
            "recipe",
            RETROARCH_RECIPE,
            "authored/recipes/app.retroarch.provision.yaml",
        ),
        (
            "recipe",
            BIOS_RECIPE,
            "authored/recipes/feature.copy_bios.yaml",
        ),
        (
            "devicePlan",
            KONKR_DEVICE_PLAN,
            "authored/device_plans/ayaneo.konkr_pocket_fit.base.yaml",
        ),
        (
            "deviceProfile",
            KONKR_DEVICE_PROFILE,
            "authored/device_profiles/ayaneo.konkr_pocket_fit.yaml",
        ),
    ];
    assert_eq!(contract.authored_sources.len(), expected.len());

    let canonical_root = repository_root()
        .canonicalize()
        .expect("repo root should canonicalize");
    for ((source_kind, id, expected_path), source) in
        expected.into_iter().zip(contract.authored_sources)
    {
        assert_eq!(source.source_kind, source_kind);
        assert_eq!(source.id, id);
        let source_path = Path::new(&source.path);
        assert!(source_path.is_relative());
        assert_eq!(source_path, Path::new(expected_path));
        let resolved = repository_root().join(source_path);
        let canonical_source = resolved
            .canonicalize()
            .expect("contract source should resolve");
        assert!(canonical_source.starts_with(&canonical_root));
        let raw = fs::read(resolved).expect("contract source should be readable");
        assert_eq!(sha256_hex(&raw), source.sha256);
    }
}

#[test]
fn combined_default_plan_and_review_match_the_strict_contract() {
    let contract = load_contract();
    let temp = tempfile::tempdir().expect("qualification tempdir should be created");
    let bios_source_dir = temp.path().join("bios-source");
    let retroarch_config = temp.path().join("retroarch.cfg");
    fs::create_dir_all(&bios_source_dir).expect("BIOS source directory should be created");
    fs::write(&retroarch_config, b"video_driver = \"vulkan\"\n")
        .expect("RetroArch config should be written");

    let result = plan_combined(Some(&bios_source_dir), Some(&retroarch_config));
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.severity != "error"));
    let plan = result
        .plan
        .as_ref()
        .expect("combined plan should be generated");
    assert!(result
        .plan_digest
        .as_deref()
        .is_some_and(|digest| !digest.is_empty()));
    assert_eq!(plan.source.selected_recipe_refs, contract.selected_recipes);
    assert_eq!(plan.source.expanded_recipe_refs, contract.expanded_recipes);
    assert_eq!(plan.source.device_plan_ref, KONKR_DEVICE_PLAN);
    assert_eq!(plan.source.device_profile_ref, KONKR_DEVICE_PROFILE);
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.recipe_ref.as_str())
            .collect::<HashSet<_>>(),
        HashSet::from([RETROARCH_RECIPE, BIOS_RECIPE])
    );
    let bios_step = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with("/copy_bios_dir"))
        .expect("combined plan should contain the BIOS copy step");
    assert_eq!(
        bios_step.params.get("dest"),
        Some(&ExecutionParamValue::Literal {
            value: Value::String(BIOS_DESTINATION.to_string()),
        })
    );
    assert_eq!(
        bios_step.params.get("copy_policy"),
        Some(&ExecutionParamValue::Literal {
            value: Value::String("sync".to_string()),
        })
    );
    assert_eq!(bios_step.verify.len(), 1);
    assert_eq!(bios_step.verify[0].type_name, "path_exists");

    let mut operation_families = plan
        .steps
        .iter()
        .map(|step| step.type_name.clone())
        .collect::<Vec<_>>();
    operation_families.sort();
    operation_families.dedup();
    let mut expected_operation_families = contract.required_operation_families.clone();
    expected_operation_families.sort();
    assert_eq!(operation_families, expected_operation_families);

    let mut constraint_capabilities = plan
        .steps
        .iter()
        .flat_map(|step| step.constraints.capabilities.iter().cloned())
        .collect::<Vec<_>>();
    constraint_capabilities.sort();
    constraint_capabilities.dedup();
    let mut expected_constraint_capabilities = contract.recipe_constraint_capabilities.clone();
    expected_constraint_capabilities.sort();
    expected_constraint_capabilities.dedup();
    assert_eq!(constraint_capabilities, expected_constraint_capabilities);
    for capability in &contract.qualification_context_capabilities {
        assert!(runtime_capability_enabled(plan, capability));
    }

    assert_eq!(result.resolved_inputs.len(), 2);
    let bios_binding = result
        .resolved_inputs
        .iter()
        .find(|binding| binding.key == BIOS_INPUT_KEY)
        .expect("BIOS binding should be resolved");
    assert_eq!(bios_binding.source, Some(BindingSource::Explicit));
    assert_eq!(
        bios_binding.value,
        Some(Value::String(
            bios_source_dir.to_string_lossy().into_owned()
        ))
    );
    let config_binding = result
        .resolved_inputs
        .iter()
        .find(|binding| binding.key == RETROARCH_INPUT_KEY)
        .expect("RetroArch config binding should be resolved");
    assert_eq!(config_binding.source, Some(BindingSource::Explicit));

    let review = result
        .review
        .as_ref()
        .expect("production review should exist");
    assert!(review.can_execute);
    assert_eq!(review.features.len(), 2);
    assert!(review
        .features
        .iter()
        .all(|feature| !feature.automatically_added));
    assert!(review
        .features
        .iter()
        .any(|feature| feature.name == "Provision RetroArch (AArch64)"));
    assert!(review
        .features
        .iter()
        .any(|feature| feature.name == "Copy BIOS Files"));
    assert!(review
        .features
        .iter()
        .flat_map(|feature| feature.sections.iter())
        .any(|section| section.kind == "copies"));
    assert_eq!(review.work.action_count, plan.steps.len());
    assert!(review
        .notices
        .iter()
        .all(|notice| notice.severity != "blocker"));

    let serialized = serde_json::to_string(review).expect("review should serialize");
    assert!(serialized.contains("retroarch.cfg"));
    assert!(serialized.contains("bios-source"));
    assert!(!serialized.contains(&temp.path().to_string_lossy().to_string()));
    assert!(!serialized.contains("serial"));
}

#[test]
fn combined_default_missing_bios_binding_fails_closed_before_planning() {
    let result = plan_combined(None, None);
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
fn generic_default_keeps_retroarch_available_without_selecting_it() {
    let temp = tempfile::tempdir().expect("qualification tempdir should be created");
    let bios_source_dir = temp.path().join("bios-source");
    fs::create_dir_all(&bios_source_dir).expect("BIOS source directory should be created");
    let result = plan_configuration_for("ayaneo.generic.base", None, Some(&bios_source_dir), None);
    let plan = result
        .plan
        .expect("generic BIOS default should be executable");
    assert_eq!(
        plan.source.selected_recipe_refs,
        vec![BIOS_RECIPE.to_string()]
    );
    assert_eq!(
        plan.source.expanded_recipe_refs,
        vec![BIOS_RECIPE.to_string()]
    );
    assert!(!plan.runtime_capabilities.root_shell);
    assert!(!plan.runtime_capabilities.app_data_write);
}

fn combined_plan(result: PlanConfigurationResult) -> ExecutionPlan {
    result.plan.unwrap_or_else(|| {
        panic!(
            "combined plan should be generated: {:?}",
            result.diagnostics
        )
    })
}

fn runtime_capability_enabled(plan: &ExecutionPlan, capability: &str) -> bool {
    match capability {
        "adb_available" => plan.runtime_capabilities.adb_available,
        "apk_install" => plan.runtime_capabilities.apk_install,
        "shared_storage_write" => plan.runtime_capabilities.shared_storage_write,
        "app_launch" => plan.runtime_capabilities.app_launch,
        "shell_command" => plan.runtime_capabilities.shell_command,
        "package_remove_for_user" => plan.runtime_capabilities.package_remove_for_user,
        "root_shell" => plan.runtime_capabilities.root_shell,
        "app_data_write" => plan.runtime_capabilities.app_data_write,
        other => panic!("unknown runtime capability {other}"),
    }
}

struct CombinedQualificationWorkspace {
    _temp: tempfile::TempDir,
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    host_input_root: PathBuf,
    bios_source_dir: PathBuf,
    retroarch_config: PathBuf,
}

impl CombinedQualificationWorkspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("qualification tempdir should be created");
        let runtime_root = temp.path().join("runtime");
        let cache_root = temp.path().join("cache");
        let fake_device_root = temp.path().join("fake-device");
        let host_input_root = temp.path().join("host-input");
        let bios_source_dir = host_input_root.join("bios-source");
        let retroarch_config = host_input_root.join("retroarch.cfg");
        let psx_path = bios_source_dir.join("sony/psx/scph5501.bin");
        let gba_path = bios_source_dir.join("nintendo/gba/gba_bios.bin");
        fs::create_dir_all(psx_path.parent().expect("PSX BIOS parent should exist"))
            .expect("PSX BIOS parent should be created");
        fs::create_dir_all(gba_path.parent().expect("GBA BIOS parent should exist"))
            .expect("GBA BIOS parent should be created");
        fs::write(&psx_path, b"psx-bios\n").expect("PSX BIOS fixture should be written");
        fs::write(&gba_path, b"gba-bios\n").expect("GBA BIOS fixture should be written");
        fs::write(&retroarch_config, b"video_driver = \"vulkan\"\n")
            .expect("RetroArch config fixture should be written");
        Self {
            _temp: temp,
            runtime_root,
            cache_root,
            fake_device_root,
            host_input_root,
            bios_source_dir,
            retroarch_config,
        }
    }

    fn source_file(&self, relative: &str) -> PathBuf {
        self.bios_source_dir.join(relative)
    }

    fn fake_device_file(&self, relative: &str) -> PathBuf {
        self.fake_device_root.join(relative)
    }
}

fn artifact_cache_path(cache_root: &Path, artifact: &ExecutionArtifact) -> PathBuf {
    cache_root.join(artifact_local_filename(
        &artifact.id,
        &artifact.url,
        &artifact.cache,
    ))
}

fn write_zip(path: &Path, entries: &[&str]) {
    let file = fs::File::create(path).expect("cache zip should be created");
    let mut zip = zip::ZipWriter::new(file);
    for entry in entries {
        zip.start_file(*entry, zip::write::SimpleFileOptions::default())
            .expect("zip entry should start");
        zip.write_all(b"retroarch qualification fixture\n")
            .expect("zip entry should write");
    }
    zip.finish().expect("zip should finish");
}

fn seed_artifact_cache(cache_root: &Path, plan: &ExecutionPlan) {
    fs::create_dir_all(cache_root).expect("cache root should be created");
    for artifact in &plan.artifacts {
        assert_eq!(
            artifact.cache, "default",
            "every qualified artifact must use the default cache mode"
        );
        let path = artifact_cache_path(cache_root, artifact);
        let leaf = artifact
            .id
            .rsplit('/')
            .next()
            .expect("artifact id should have a leaf");
        if artifact.type_name == "remote_file" && leaf == "retroarch_apk" {
            fs::write(&path, b"retroarch deterministic apk fixture\n")
                .expect("APK fixture should be written");
            continue;
        }
        let entries: &[&str] = match leaf {
            "core_files_dolphin_zip" => &["dolphin-emu/marker.txt"],
            "core_files_fbneo_zip" => &["fbneo/marker.txt"],
            "core_files_ppsspp_zip" => &["PPSSPP/marker.txt"],
            _ => &["marker.txt"],
        };
        write_zip(&path, entries);
    }
}

fn combined_sandbox_adapters(workspace: &CombinedQualificationWorkspace) -> ExecutorAdapters {
    ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
    )
}

fn prepared_combined_plan(workspace: &CombinedQualificationWorkspace) -> ExecutionPlan {
    let result = plan_combined(
        Some(&workspace.bios_source_dir),
        Some(&workspace.retroarch_config),
    );
    assert!(result
        .diagnostics
        .iter()
        .all(|diagnostic| diagnostic.severity != "error"));
    combined_plan(result)
}

#[test]
fn combined_generated_plan_executes_with_exact_nested_bios_bytes() {
    let workspace = CombinedQualificationWorkspace::new();
    let plan = prepared_combined_plan(&workspace);
    seed_artifact_cache(&workspace.cache_root, &plan);
    let mut runner = ExecutorRunner::new(combined_sandbox_adapters(&workspace));
    let result = runner.run(&plan);

    assert!(result.success, "combined plan should execute successfully");
    assert_eq!(result.total_steps, plan.steps.len());
    assert_eq!(result.steps.len(), result.total_steps);
    assert!(result.steps.iter().all(|record| {
        !matches!(
            record.status,
            StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
        )
    }));
    for suffix in [
        "/install_retroarch",
        "/copy_core_system_files",
        "/copy_bios_dir",
    ] {
        let record = result
            .steps
            .iter()
            .find(|record| record.step_id.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing execution record for {suffix}"));
        assert_eq!(
            record.status,
            StepRunStatus::Executed,
            "{suffix} must execute"
        );
    }

    assert_eq!(
        fs::read(workspace.fake_device_file("sdcard/RetroArch/system/sony/psx/scph5501.bin"))
            .expect("nested PSX BIOS should be copied"),
        fs::read(workspace.source_file("sony/psx/scph5501.bin"))
            .expect("source PSX BIOS should be readable")
    );
    assert_eq!(
        fs::read(workspace.fake_device_file("sdcard/RetroArch/system/nintendo/gba/gba_bios.bin",))
            .expect("nested GBA BIOS should be copied"),
        fs::read(workspace.source_file("nintendo/gba/gba_bios.bin"))
            .expect("source GBA BIOS should be readable")
    );
}

#[test]
fn combined_repeated_run_skips_only_retroarch_install_and_reexecutes_bios_sync() {
    let workspace = CombinedQualificationWorkspace::new();
    let plan = prepared_combined_plan(&workspace);
    seed_artifact_cache(&workspace.cache_root, &plan);
    let mut runner = ExecutorRunner::new(combined_sandbox_adapters(&workspace));

    let first = runner.run(&plan);
    assert!(first.success, "first combined run should succeed");
    let status = |result: &crate::executor::ExecutionRunResult, suffix: &str| {
        result
            .steps
            .iter()
            .find(|record| record.step_id.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing execution record for {suffix}"))
            .status
            .clone()
    };
    assert_eq!(
        status(&first, "/install_retroarch"),
        StepRunStatus::Executed
    );
    assert_eq!(status(&first, "/copy_bios_dir"), StepRunStatus::Executed);

    let second = runner.run(&plan);
    assert!(
        second.success,
        "repeated combined run should remain successful"
    );
    assert_eq!(
        status(&second, "/install_retroarch"),
        StepRunStatus::Skipped,
        "the authored package_installed predicate must skip only repeated RetroArch install"
    );
    assert_eq!(
        status(&second, "/copy_bios_dir"),
        StepRunStatus::Executed,
        "the authored BIOS sync policy must re-execute on the repeated run"
    );
    assert!(second.steps.iter().all(|record| {
        !matches!(
            record.status,
            StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
        )
    }));
}

#[derive(Debug, Default)]
struct CombinedVerificationFailDevice {
    inner: FakeDryRunDevice,
    missing_path: String,
}

impl CombinedVerificationFailDevice {
    fn for_bios_destination() -> Self {
        Self {
            inner: FakeDryRunDevice::default(),
            missing_path: BIOS_DESTINATION.to_string(),
        }
    }

    fn commands(&self) -> &[Vec<String>] {
        self.inner.commands()
    }
}

impl ExecutorDevice for CombinedVerificationFailDevice {
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
fn combined_bios_verification_failure_keeps_prior_results_truthful_without_plan_mutation() {
    let workspace = CombinedQualificationWorkspace::new();
    let plan = prepared_combined_plan(&workspace);
    seed_artifact_cache(&workspace.cache_root, &plan);
    let plan_before = plan.clone();
    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        CombinedVerificationFailDevice::for_bios_destination(),
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
        "execution must not mutate the generated plan"
    );
    assert!(
        !result.success,
        "BIOS verification failure must fail the run"
    );
    let bios_record = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/copy_bios_dir"))
        .expect("BIOS step record should exist");
    assert_eq!(bios_record.status, StepRunStatus::Failed);
    assert!(bios_record
        .message
        .as_deref()
        .is_some_and(|message| !message.is_empty()));

    assert_eq!(result.steps.len(), plan.steps.len());
    assert!(result
        .steps
        .iter()
        .all(|record| { plan.steps.iter().any(|step| step.id == record.step_id) }));
    assert_eq!(
        result
            .steps
            .iter()
            .filter(|record| record.status == StepRunStatus::Failed)
            .count(),
        1
    );
    for record in &result.steps {
        if record.step_id.ends_with("/copy_bios_dir") {
            continue;
        }
        assert!(
            !matches!(record.status, StepRunStatus::Failed),
            "only the authored BIOS verification should fail: {} {:?}",
            record.step_id,
            record.status
        );
    }

    let commands = runner.adapters().device().commands();
    let mkdir_index = commands
        .iter()
        .position(|command| {
            command.first().map(String::as_str) == Some("mkdir_p")
                && command.get(1).map(String::as_str) == Some(BIOS_DESTINATION)
        })
        .expect("BIOS copy should create the authored destination");
    let bios_source_prefix = workspace.bios_source_dir.to_string_lossy().into_owned();
    let push_index = commands
        .iter()
        .position(|command| {
            command.first().map(String::as_str) == Some("push_sync")
                && command
                    .get(1)
                    .is_some_and(|source| source.starts_with(&bios_source_prefix))
        })
        .expect("BIOS copy should delegate a synced push");
    let verify_index = commands
        .iter()
        .position(|command| {
            command.first().map(String::as_str) == Some("path_exists")
                && command.get(1).map(String::as_str) == Some(BIOS_DESTINATION)
        })
        .expect("authored BIOS destination verification should be observed");
    assert!(mkdir_index < push_index);
    assert!(push_index < verify_index);

    assert!(commands
        .iter()
        .any(|command| command.first().map(String::as_str) == Some("install_apk")));
    let install = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/install_retroarch"))
        .expect("RetroArch install record should exist");
    assert_eq!(install.status, StepRunStatus::Executed);

    assert!(commands
        .iter()
        .any(|command| command.first().map(String::as_str) == Some("launch_app")));
    let launch = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/launch_retroarch"))
        .expect("RetroArch launch record should exist");
    assert_eq!(launch.status, StepRunStatus::Executed);
}

#[test]
fn konkr_default_selects_retroarch_and_bios() {
    let temp = tempfile::tempdir().expect("qualification tempdir should be created");
    let bios_source_dir = temp.path().join("bios-source");
    fs::create_dir_all(&bios_source_dir).expect("BIOS source directory should be created");

    let plan = combined_plan(plan_combined(Some(&bios_source_dir), None));

    assert_eq!(
        plan.source.selected_recipe_refs,
        vec![
            "app.retroarch.provision".to_string(),
            "feature.copy_bios".to_string()
        ]
    );
    assert_eq!(
        plan.source.expanded_recipe_refs,
        vec![
            "app.retroarch.provision".to_string(),
            "feature.copy_bios".to_string()
        ]
    );
}
