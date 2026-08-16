//! Phase 6E.1 automated recipe-qualification foundation for the real
//! `app.retroarch.provision` workflow.
//!
//! These tests bind qualification expectations to the authored source digest,
//! load the real authored catalog through production runtime configuration,
//! exercise the production review projection, and execute the unchanged
//! generated plan through the deterministic sandbox-root executor adapters.
//! No test in this module invokes ADB, a physical device, a real network
//! request, or an ignored physical-qualification harness.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::artifact_resolver::artifact_local_filename;
use crate::executor::{ExecutorAdapters, ExecutorRunner, StepRunStatus};
use crate::planner::{BindingSource, ExecutionArtifact, ExecutionPlan, ExecutionStep};
use crate::runtime_configuration::PlanConfigurationResult;

const TARGET_RECIPE: &str = "app.retroarch.provision";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.konkr_pocket_fit.base";
const OPTIONAL_INPUT_KEY: &str = "app.retroarch.provision/retroarch_cfg";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetroArchQualificationContract {
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
    material_dependency_edges: Vec<Vec<String>>,
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
    repository_root().join("tests/fixtures/phase-6e/retroarch/qualification-contract.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_contract() -> RetroArchQualificationContract {
    let text = fs::read_to_string(contract_path())
        .expect("Phase 6E.1 qualification contract should be readable");
    let contract: RetroArchQualificationContract = serde_json::from_str(&text)
        .expect("Phase 6E.1 qualification contract should deserialize strictly");
    assert_eq!(
        contract.schema_version, 1,
        "contract schema version must be 1"
    );
    assert_eq!(contract.target_recipe, TARGET_RECIPE);
    assert_eq!(contract.planning_device_plan, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(contract.automated_status, "foundation");
    assert_eq!(contract.physical_status, "deferred");
    assert_eq!(
        contract.physical_cleanup_authority, "not_authorized_in_phase_6e1",
        "Phase 6E.1 grants no physical cleanup authority"
    );
    assert!(!contract.live_network_required_for_automated_qualification);
    contract
}

/// Prepare the real authored RetroArch configuration through the production
/// runtime-configuration path. No target serial and no device probe is used.
fn plan_retroarch(config_path: Option<&Path>) -> PlanConfigurationResult {
    use crate::catalog_source::CatalogSnapshot;
    use crate::model::OrderedMap;
    use crate::runtime_configuration::{plan_configuration, ConfigurationContextRequest};

    let catalog = CatalogSnapshot::legacy_local(authored_root())
        .expect("real authored catalog should be admitted");
    let mut explicit_bindings = OrderedMap::new();
    if let Some(path) = config_path {
        explicit_bindings.insert(
            OPTIONAL_INPUT_KEY.to_string(),
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
    .expect("real authored RetroArch configuration should prepare")
}

/// Map an authored step id to exactly one generated step for the target recipe.
fn generated_step_for_authored_id<'a>(
    plan: &'a ExecutionPlan,
    authored_step_id: &str,
) -> &'a ExecutionStep {
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
    let capabilities = &plan.runtime_capabilities;
    match capability {
        "adb_available" => capabilities.adb_available,
        "apk_install" => capabilities.apk_install,
        "shared_storage_write" => capabilities.shared_storage_write,
        "app_launch" => capabilities.app_launch,
        "shell_command" => capabilities.shell_command,
        "package_remove_for_user" => capabilities.package_remove_for_user,
        "root_shell" => capabilities.root_shell,
        "app_data_write" => capabilities.app_data_write,
        other => panic!("unknown runtime capability {other}"),
    }
}

#[test]
fn phase_6e1_contract_binds_current_retroarch_source_and_deferred_physical_status() {
    let contract = load_contract();
    let source_path = Path::new(&contract.authored_source.path);
    assert!(
        source_path.is_relative(),
        "contract source path must be repository-relative"
    );
    assert_eq!(
        source_path,
        Path::new("authored/recipes/app.retroarch.provision.yaml")
    );
    let resolved = repository_root().join(source_path);
    let canonical_root = repository_root()
        .canonicalize()
        .expect("repo root should canonicalize");
    let canonical_source = resolved
        .canonicalize()
        .expect("authored recipe should resolve");
    assert!(
        canonical_source.starts_with(canonical_root),
        "contract source path must not escape the repository root"
    );
    let raw = fs::read(&resolved).expect("authored recipe should be readable");
    assert_eq!(
        sha256_hex(&raw),
        contract.authored_source.sha256,
        "authored RetroArch recipe changed; qualification expectations must be reviewed"
    );
}

#[test]
fn phase_6e1_real_authored_retroarch_plan_matches_qualification_contract() {
    let contract = load_contract();
    let result = plan_retroarch(None);
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != "error"),
        "planning must produce no error diagnostics"
    );
    let plan = result.plan.expect("plan should be generated");
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
    assert_eq!(plan.source.device_profile_ref, "ayaneo.konkr_pocket_fit");
    assert!(
        plan.recipes.iter().all(|recipe| recipe.id == TARGET_RECIPE),
        "plan recipe snapshots must contain only app.retroarch.provision"
    );

    let mut families = plan
        .steps
        .iter()
        .map(|step| step.type_name.clone())
        .collect::<Vec<_>>();
    families.sort();
    families.dedup();
    for family in &contract.required_operation_families {
        assert!(
            families.contains(family),
            "plan is missing required operation family {family}"
        );
    }

    let mut capabilities = plan
        .steps
        .iter()
        .flat_map(|step| step.constraints.capabilities.iter().cloned())
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    let mut expected_capabilities = contract.recipe_constraint_capabilities.clone();
    expected_capabilities.sort();
    assert_eq!(capabilities, expected_capabilities);

    for capability in &contract.qualification_context_capabilities {
        assert!(
            runtime_capability_enabled(&plan, capability),
            "plan runtime capabilities must enable {capability}"
        );
    }
    assert!(!plan.runtime_capabilities.package_remove_for_user);

    assert!(contract.required_inputs.is_empty());
    assert_eq!(result.resolved_inputs.len(), 1);
    assert_eq!(result.resolved_inputs[0].key, OPTIONAL_INPUT_KEY);
    assert!(result.resolved_inputs[0].value.is_none());
    assert_eq!(
        contract.optional_inputs,
        vec![OPTIONAL_INPUT_KEY.to_string()]
    );

    for edge in &contract.material_dependency_edges {
        let earlier = generated_step_for_authored_id(&plan, &edge[0]);
        let later = generated_step_for_authored_id(&plan, &edge[1]);
        assert!(
            later.dependencies.contains(&earlier.id),
            "material dependency edge {edge:?} must be a direct generated dependency"
        );
    }
}

#[test]
fn phase_6e1_optional_retroarch_cfg_is_not_required_for_planning() {
    let result = plan_retroarch(None);
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != "error"),
        "omitting the optional config must not produce error diagnostics"
    );
    let plan = result
        .plan
        .expect("plan should be generated without the optional config");
    assert_eq!(result.resolved_inputs.len(), 1);
    assert_eq!(result.resolved_inputs[0].key, OPTIONAL_INPUT_KEY);
    assert!(
        result.resolved_inputs[0].value.is_none(),
        "unbound optional input must resolve to no value"
    );
    assert!(!plan.steps.is_empty());
    assert!(
        plan.steps.iter().all(|step| !step.id.is_empty()),
        "no emitted step may carry an empty id"
    );
}

#[test]
fn phase_6e1_supplied_retroarch_cfg_is_bound_and_reviewed_without_parent_path_leakage() {
    let temp = tempfile::tempdir().expect("tempdir should be created");
    let config_path = temp.path().join("retroarch.cfg");
    fs::write(&config_path, b"video_driver = \"vulkan\"\n").expect("config should be written");
    let parent = config_path
        .parent()
        .expect("config path should have a parent")
        .to_string_lossy()
        .to_string();

    let result = plan_retroarch(Some(&config_path));
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != "error"),
        "supplying the config must not produce error diagnostics"
    );
    let plan = result
        .plan
        .expect("plan should be generated with the supplied config");
    let binding = result
        .resolved_inputs
        .iter()
        .find(|input| input.key == OPTIONAL_INPUT_KEY)
        .expect("optional config binding should exist");
    assert_eq!(binding.source, Some(BindingSource::Explicit));
    assert_eq!(
        binding.value,
        Some(Value::String(config_path.to_string_lossy().into_owned()))
    );
    assert!(
        plan.steps
            .iter()
            .any(|step| step.id.ends_with("/seed_retroarch_cfg")),
        "supplied config must produce the seed_retroarch_cfg copy step"
    );

    let review = result.review.expect("production review should be produced");
    assert!(review.can_execute, "review must remain executable");
    assert_eq!(review.features.len(), 1, "exactly one feature is qualified");
    assert!(!review.features[0].automatically_added);
    assert!(!review.features[0].name.is_empty());
    let section_kinds = review.features[0]
        .sections
        .iter()
        .map(|section| section.kind)
        .collect::<HashSet<_>>();
    for kind in [
        "preparation",
        "downloads",
        "installs",
        "copies",
        "permissions",
        "launches",
        "device_changes",
    ] {
        assert!(
            section_kinds.contains(kind),
            "review is missing section kind {kind}"
        );
    }
    assert_eq!(review.work.action_count, plan.steps.len());
    assert_eq!(review.work.known_wait_seconds, Some(7));
    assert!(
        review
            .notices
            .iter()
            .all(|notice| notice.severity != "blocker"),
        "review must contain no blocker notices"
    );
    assert!(
        review
            .inputs
            .iter()
            .any(|input| input.summary == "retroarch.cfg"),
        "review input summary must contain the config basename"
    );

    let serialized = serde_json::to_string(&review).expect("review should serialize");
    assert!(serialized.contains("retroarch.cfg"));
    assert!(
        !serialized.contains(&parent),
        "review must not leak the config parent directory"
    );
    assert!(
        !serialized.contains("serial"),
        "review must not expose an ADB serial"
    );
}

#[derive(Clone, Copy)]
enum SystemFixtureMode {
    Complete,
    MissingPpsspp,
}

struct QualificationWorkspace {
    _temp: tempfile::TempDir,
    runtime_root: PathBuf,
    cache_root: PathBuf,
    fake_device_root: PathBuf,
    host_input_root: PathBuf,
    config_path: PathBuf,
}

impl QualificationWorkspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("qualification tempdir should be created");
        let runtime_root = temp.path().join("runtime");
        let cache_root = temp.path().join("cache");
        let fake_device_root = temp.path().join("fake-device");
        let host_input_root = temp.path().join("host-input");
        fs::create_dir_all(&host_input_root).expect("host input root should be created");
        let config_path = host_input_root.join("retroarch.cfg");
        fs::write(&config_path, b"video_driver = \"vulkan\"\n")
            .expect("retroarch.cfg should be written");
        Self {
            _temp: temp,
            runtime_root,
            cache_root,
            fake_device_root,
            host_input_root,
            config_path,
        }
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
        zip.write_all(b"phase-6e1\n")
            .expect("zip entry should write");
    }
    zip.finish().expect("zip should finish");
}

fn seed_artifact_cache(cache_root: &Path, plan: &ExecutionPlan, system_mode: SystemFixtureMode) {
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
            fs::write(&path, b"phase-6e1 deterministic apk fixture\n")
                .expect("apk fixture should be written");
            continue;
        }
        let entries: &[&str] = match leaf {
            "core_files_dolphin_zip" => &["dolphin-emu/marker.txt"],
            "core_files_fbneo_zip" => &["fbneo/marker.txt"],
            "core_files_ppsspp_zip" if matches!(system_mode, SystemFixtureMode::Complete) => {
                &["PPSSPP/marker.txt"]
            }
            "core_files_ppsspp_zip" => &["marker.txt"],
            _ => &["marker.txt"],
        };
        write_zip(&path, entries);
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

#[test]
fn phase_6e1_retroarch_generated_plan_executes_successfully_without_network_or_adb() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_retroarch(Some(&workspace.config_path));
    let plan = prepared.plan.expect("plan should be generated");
    seed_artifact_cache(&workspace.cache_root, &plan, SystemFixtureMode::Complete);
    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));
    let result = runner.run(&plan);

    assert!(
        result.success,
        "generated RetroArch plan should execute successfully"
    );
    assert_eq!(result.total_steps, plan.steps.len());
    assert_eq!(result.steps.len(), result.total_steps);
    for record in &result.steps {
        assert!(
            !matches!(
                record.status,
                StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
            ),
            "step {} must not fail: {:?}",
            record.step_id,
            record.status
        );
    }
    for suffix in [
        "/resolve_artifacts",
        "/install_retroarch",
        "/grant_retroarch_permissions",
        "/copy_core_system_files",
        "/seed_retroarch_cfg",
        "/launch_retroarch",
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
}

#[test]
fn phase_6e1_retroarch_install_skip_on_repeated_deterministic_run() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_retroarch(Some(&workspace.config_path));
    let plan = prepared.plan.expect("plan should be generated");
    seed_artifact_cache(&workspace.cache_root, &plan, SystemFixtureMode::Complete);
    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));

    let first = runner.run(&plan);
    assert!(first.success, "first deterministic run should succeed");
    let install_status = |result: &crate::executor::ExecutionRunResult| {
        result
            .steps
            .iter()
            .find(|record| record.step_id.ends_with("/install_retroarch"))
            .expect("install record should exist")
            .status
            .clone()
    };
    assert_eq!(install_status(&first), StepRunStatus::Executed);

    let second = runner.run(&plan);
    assert!(
        second.success,
        "repeated deterministic run should remain successful"
    );
    assert_eq!(
        install_status(&second),
        StepRunStatus::Skipped,
        "the authored package_installed predicate must skip the repeated install"
    );
    for record in &second.steps {
        assert!(
            !matches!(
                record.status,
                StepRunStatus::Failed | StepRunStatus::Blocked | StepRunStatus::Cancelled
            ),
            "repeated run step {} must not fail: {:?}",
            record.step_id,
            record.status
        );
    }
}

#[test]
fn phase_6e1_missing_core_system_verification_fails_and_stops_later_recipe_work() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_retroarch(Some(&workspace.config_path));
    let plan = prepared.plan.expect("plan should be generated");
    seed_artifact_cache(
        &workspace.cache_root,
        &plan,
        SystemFixtureMode::MissingPpsspp,
    );
    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));
    let result = runner.run(&plan);

    assert!(
        !result.success,
        "missing PPSSPP core files must fail the run"
    );
    let copy = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/copy_core_system_files"))
        .expect("copy_core_system_files record should exist");
    assert_eq!(copy.status, StepRunStatus::Failed);
    assert!(
        copy.message
            .as_deref()
            .is_some_and(|message| !message.is_empty()),
        "failed step must carry a message"
    );

    let install = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/install_retroarch"))
        .expect("install record should exist");
    assert_eq!(
        install.status,
        StepRunStatus::Executed,
        "prior completed results must be retained after the verification failure"
    );

    let launch = result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/launch_retroarch"))
        .expect("launch record should exist");
    assert_ne!(
        launch.status,
        StepRunStatus::Executed,
        "final launch must not report success after the verification failure"
    );
}
