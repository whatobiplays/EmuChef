//! Automated qualification for the authored XaniteOG installation workflow.
//!
//! These tests bind expectations to the authored recipe bytes, plan the real
//! catalog through runtime configuration, inspect the production review, and
//! execute the unchanged generated plan through deterministic sandbox-root
//! adapters. No test in this module invokes ADB, a physical device, a live
//! network request, or an ignored physical-qualification harness.

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

const TARGET_RECIPE: &str = "app.xaniteog.install";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.pocket_s2.base";
const INPUT_KEY: &str = "app.xaniteog.install/xaniteog_apk";
const TARGET_PACKAGE: &str = "Ali.Xanite";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualificationContract {
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
    input: InputContract,
    install: InstallContract,
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
struct InputContract {
    key: String,
    required: bool,
    #[serde(rename = "type")]
    type_name: String,
    role: String,
    must_exist: bool,
    allowed_extensions: Vec<String>,
    path_kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallContract {
    step_id_suffix: String,
    package_name: String,
    input_ref: String,
    replace_existing: bool,
    skip_condition_type: String,
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
        .join("tests/fixtures/recipe-qualification/xaniteog/qualification-contract.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_contract() -> QualificationContract {
    let text = fs::read_to_string(contract_path())
        .expect("XaniteOG qualification contract should be readable");
    let contract: QualificationContract = serde_json::from_str(&text)
        .expect("XaniteOG qualification contract should deserialize strictly");
    assert_eq!(contract.schema_version, 1);
    assert_eq!(contract.target_recipe, TARGET_RECIPE);
    assert_eq!(contract.planning_device_plan, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(contract.selected_recipes, vec![TARGET_RECIPE]);
    assert_eq!(contract.expanded_recipes, vec![TARGET_RECIPE]);
    assert_eq!(contract.recipe_constraint_capabilities, vec!["apk_install"]);
    assert_eq!(
        contract.qualification_context_capabilities,
        vec!["apk_install"]
    );
    assert_eq!(contract.required_inputs, vec![INPUT_KEY]);
    assert!(contract.optional_inputs.is_empty());
    assert_eq!(contract.required_operation_families, vec!["install_apk"]);
    assert_eq!(contract.input.key, INPUT_KEY);
    assert!(contract.input.required);
    assert_eq!(contract.input.type_name, "file");
    assert_eq!(contract.input.role, "apk");
    assert!(contract.input.must_exist);
    assert_eq!(contract.input.allowed_extensions, vec!["apk"]);
    assert_eq!(contract.input.path_kind, "file");
    assert_eq!(contract.install.step_id_suffix, "install_xaniteog");
    assert_eq!(contract.install.package_name, TARGET_PACKAGE);
    assert_eq!(contract.install.input_ref, "inputs.xaniteog_apk");
    assert!(!contract.install.replace_existing);
    assert_eq!(contract.install.skip_condition_type, "package_installed");
    assert!(!contract.live_network_required_for_automated_qualification);
    assert_eq!(contract.automated_status, "qualified");
    assert_eq!(contract.physical_status, "deferred");
    assert_eq!(
        contract.physical_cleanup_authority,
        "not_authorized_for_recipe_qualification"
    );
    contract
}

/// Prepare the real authored XaniteOG configuration through the production
/// runtime-configuration path. Explicit selection prevents the device plan's
/// other default recipes from entering this standalone qualification.
fn plan_xaniteog(apk_path: Option<&Path>) -> PlanConfigurationResult {
    use crate::catalog_source::CatalogSnapshot;
    use crate::model::OrderedMap;
    use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    let mut explicit_bindings = OrderedMap::new();
    if let Some(path) = apk_path {
        explicit_bindings.insert(
            INPUT_KEY.to_string(),
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
    .expect("real authored XaniteOG configuration should prepare")
}

fn generated_step<'a>(plan: &'a ExecutionPlan, authored_step_id: &str) -> &'a ExecutionStep {
    let suffix = format!("/{authored_step_id}");
    let mut matches = plan
        .steps
        .iter()
        .filter(|step| step.recipe_ref == TARGET_RECIPE && step.id.ends_with(&suffix));
    let step = matches
        .next()
        .unwrap_or_else(|| panic!("no generated step for authored id {authored_step_id}"));
    assert!(
        matches.next().is_none(),
        "multiple generated steps for authored id {authored_step_id}"
    );
    step
}

fn runtime_capability_enabled(plan: &ExecutionPlan, capability: &str) -> bool {
    match capability {
        "apk_install" => plan.runtime_capabilities.apk_install,
        other => panic!("unexpected XaniteOG runtime capability {other}"),
    }
}

#[test]
fn xaniteog_contract_binds_current_source_and_deferred_physical_status() {
    let contract = load_contract();
    let source_path = Path::new(&contract.authored_source.path);
    assert!(
        source_path.is_relative(),
        "contract source path must be repository-relative"
    );
    assert_eq!(
        source_path,
        Path::new("authored/recipes/app.xaniteog.install.yaml")
    );
    let resolved = repository_root().join(source_path);
    let canonical_root = repository_root()
        .canonicalize()
        .expect("repo root should canonicalize");
    let canonical_source = resolved
        .canonicalize()
        .expect("authored XaniteOG recipe should resolve");
    assert!(
        canonical_source.starts_with(canonical_root),
        "contract source path must not escape the repository root"
    );
    let raw = fs::read(&resolved).expect("authored XaniteOG recipe should be readable");
    assert_eq!(
        sha256_hex(&raw),
        contract.authored_source.sha256,
        "authored XaniteOG recipe changed; qualification expectations must be reviewed"
    );
}

#[test]
fn xaniteog_real_authored_plan_and_review_match_qualification_contract() {
    let contract = load_contract();
    let temp = tempfile::tempdir().expect("qualification tempdir should be created");
    let apk_path = temp.path().join("xaniteog.apk");
    fs::write(&apk_path, b"deterministic XaniteOG APK fixture\n")
        .expect("XaniteOG APK fixture should be written");
    let result = plan_xaniteog(Some(&apk_path));
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != "error"),
        "planning must produce no error diagnostics"
    );
    let plan = result
        .plan
        .as_ref()
        .expect("XaniteOG plan should be generated");
    assert!(
        result
            .plan_digest
            .as_deref()
            .is_some_and(|digest| !digest.is_empty()),
        "plan digest should be present"
    );
    assert_eq!(plan.source.selected_recipe_refs, contract.selected_recipes);
    assert_eq!(plan.source.expanded_recipe_refs, contract.expanded_recipes);
    assert_eq!(plan.source.device_plan_ref, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(plan.source.device_profile_ref, "ayaneo.pocket_s2");
    assert!(plan.recipes.iter().all(|recipe| recipe.id == TARGET_RECIPE));
    assert_eq!(plan.steps.len(), 1);

    let step = generated_step(plan, &contract.install.step_id_suffix);
    assert_eq!(step.type_name, "install_apk");
    assert_eq!(step.constraints.capabilities, vec!["apk_install"]);
    for capability in &contract.qualification_context_capabilities {
        assert!(runtime_capability_enabled(plan, capability));
    }
    assert_eq!(
        contract.required_operation_families,
        vec![step.type_name.clone()]
    );
    assert_eq!(result.resolved_inputs.len(), 1);
    let binding = &result.resolved_inputs[0];
    assert_eq!(binding.key, INPUT_KEY);
    assert_eq!(binding.source, Some(BindingSource::Explicit));
    assert_eq!(
        binding.value,
        Some(Value::String(apk_path.to_string_lossy().into_owned()))
    );
    assert_eq!(
        step.params.get("app"),
        Some(&ExecutionParamValue::Ref {
            ref_value: format!("inputs.{INPUT_KEY}"),
        })
    );
    assert_eq!(
        step.params.get("replace_existing"),
        Some(&ExecutionParamValue::Literal {
            value: Value::Bool(contract.install.replace_existing),
        })
    );
    assert!(step.verify.is_empty());
    assert_eq!(step.skip_if.len(), 1);
    assert_eq!(
        step.skip_if[0].type_name,
        contract.install.skip_condition_type
    );
    assert_eq!(
        step.skip_if[0].params.get("package_name"),
        Some(&Value::String(contract.install.package_name.clone()))
    );

    let review = result
        .review
        .as_ref()
        .expect("production review should exist");
    assert!(review.can_execute);
    assert_eq!(review.features.len(), 1);
    assert_eq!(review.features[0].name, "Install XaniteOG");
    assert!(!review.features[0].automatically_added);
    assert!(review.features[0]
        .sections
        .iter()
        .any(|section| section.kind == "installs"));
    assert_eq!(review.inputs.len(), 1);
    assert_eq!(review.inputs[0].label, "XaniteOG APK");
    assert_eq!(review.inputs[0].summary, "xaniteog.apk");
    assert!(review.inputs[0].required);
    assert_eq!(review.work.action_count, plan.steps.len());
    assert!(review
        .notices
        .iter()
        .all(|notice| notice.severity != "blocker"));

    let serialized = serde_json::to_string(review).expect("review should serialize");
    assert!(!serialized.contains(&temp.path().to_string_lossy().to_string()));
    assert!(!serialized.contains("serial"));
    assert!(!serialized.contains("unauthorized"));
}

fn assert_invalid_input(result: PlanConfigurationResult, expected_code: &str) {
    assert!(result.plan.is_none());
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == expected_code)
        .unwrap_or_else(|| panic!("expected {expected_code} diagnostic"));
    assert_eq!(diagnostic.key.as_deref(), Some(INPUT_KEY));
}

#[test]
fn xaniteog_missing_required_input_prevents_executable_plan_and_review() {
    assert_invalid_input(plan_xaniteog(None), "binding_missing");
}

#[test]
fn xaniteog_nonexistent_required_apk_prevents_executable_plan_and_review() {
    let temp = tempfile::tempdir().expect("missing APK test root should be created");
    let missing = temp.path().join("xaniteog.apk");
    assert_invalid_input(plan_xaniteog(Some(&missing)), "binding_path_missing");
}

#[test]
fn xaniteog_wrong_extension_prevents_executable_plan_and_review() {
    let temp = tempfile::tempdir().expect("extension test root should be created");
    let wrong_extension = temp.path().join("xaniteog.txt");
    fs::write(&wrong_extension, b"not an APK").expect("wrong-extension fixture should exist");
    assert_invalid_input(
        plan_xaniteog(Some(&wrong_extension)),
        "binding_extension_unsupported",
    );
}

#[test]
fn xaniteog_wrong_path_kind_prevents_executable_plan_and_review() {
    let temp = tempfile::tempdir().expect("path-kind test root should be created");
    let directory_with_apk_extension = temp.path().join("xaniteog.apk");
    fs::create_dir(&directory_with_apk_extension)
        .expect("wrong-kind fixture directory should be created");
    assert_invalid_input(
        plan_xaniteog(Some(&directory_with_apk_extension)),
        "binding_path_kind_mismatch",
    );
}

struct QualificationWorkspace {
    _temp: tempfile::TempDir,
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    host_input_root: PathBuf,
    apk_path: PathBuf,
}

impl QualificationWorkspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("qualification tempdir should be created");
        let host_input_root = temp.path().join("host-input");
        fs::create_dir_all(&host_input_root).expect("host input root should be created");
        let apk_path = host_input_root.join("xaniteog.apk");
        fs::write(&apk_path, b"deterministic XaniteOG APK fixture\n")
            .expect("XaniteOG APK fixture should be written");
        Self {
            runtime_root: temp.path().join("runtime"),
            cache_root: temp.path().join("cache"),
            fake_device_root: temp.path().join("fake-device"),
            host_input_root,
            apk_path,
            _temp: temp,
        }
    }
}

fn dry_run_adapters(workspace: &QualificationWorkspace) -> ExecutorAdapters {
    ExecutorAdapters::with_sandbox_roots(
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
    )
}

fn install_status(result: &crate::executor::ExecutionRunResult) -> StepRunStatus {
    result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/install_xaniteog"))
        .expect("XaniteOG install record should exist")
        .status
        .clone()
}

#[test]
fn xaniteog_generated_plan_executes_successfully_without_network_or_adb() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_xaniteog(Some(&workspace.apk_path));
    let plan = prepared.plan.expect("XaniteOG plan should be generated");
    let plan_before = plan.clone();

    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));
    let result = runner.run(&plan);

    assert!(
        result.success,
        "generated XaniteOG plan should execute successfully"
    );
    assert_eq!(plan, plan_before);
    assert_eq!(result.total_steps, plan.steps.len());
    assert_eq!(result.steps.len(), result.total_steps);
    assert_eq!(install_status(&result), StepRunStatus::Executed);
    assert!(result.steps.iter().all(|record| !matches!(
        record.status,
        StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
    )));
}

#[test]
fn xaniteog_install_skips_on_repeated_deterministic_run() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_xaniteog(Some(&workspace.apk_path));
    let plan = prepared.plan.expect("XaniteOG plan should be generated");
    let plan_before = plan.clone();
    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));

    let first = runner.run(&plan);
    assert!(first.success, "first deterministic run should succeed");
    assert_eq!(install_status(&first), StepRunStatus::Executed);

    let second = runner.run(&plan);
    assert!(
        second.success,
        "repeated deterministic run should remain successful"
    );
    assert_eq!(install_status(&second), StepRunStatus::Skipped);
    assert_eq!(plan, plan_before);
    assert!(second.steps.iter().all(|record| !matches!(
        record.status,
        StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
    )));
}

#[derive(Debug, Default)]
struct InstallFailureDevice {
    inner: FakeDryRunDevice,
}

impl InstallFailureDevice {
    fn commands(&self) -> &[Vec<String>] {
        self.inner.commands()
    }
}

impl ExecutorDevice for InstallFailureDevice {
    fn uses_fake_device_filesystem(&self) -> bool {
        true
    }

    fn install_apk(
        &mut self,
        apk_path: &Path,
        replace_existing: bool,
    ) -> Result<(), DeviceOperationError> {
        let _ = <FakeDryRunDevice as ExecutorDevice>::install_apk(
            &mut self.inner,
            apk_path,
            replace_existing,
        );
        Err(DeviceOperationError::other(
            "deterministic XaniteOG install failure",
        ))
    }

    fn record_installed_package(&mut self, package_name: &str) {
        <FakeDryRunDevice as ExecutorDevice>::record_installed_package(
            &mut self.inner,
            package_name,
        );
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
fn xaniteog_install_failure_stops_the_unchanged_generated_plan_truthfully() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_xaniteog(Some(&workspace.apk_path));
    let plan = prepared.plan.expect("XaniteOG plan should be generated");
    let plan_before = plan.clone();
    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        InstallFailureDevice::default(),
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
        false,
    );
    let mut runner = ExecutorRunner::new(adapters);
    let result = runner.run(&plan);

    assert_eq!(plan, plan_before);
    assert!(!result.success);
    assert_eq!(result.steps.len(), 1);
    let record = result
        .steps
        .first()
        .expect("XaniteOG install execution record should exist");
    assert_eq!(record.status, StepRunStatus::Failed);
    assert!(record
        .message
        .as_deref()
        .is_some_and(|message| message.contains("deterministic XaniteOG install failure")));
    assert!(runner
        .adapters()
        .device()
        .commands()
        .iter()
        .any(|command| command.first().map(String::as_str) == Some("install_apk")));
}
