//! Automated RetroArch recipe qualification for the real
//! `app.retroarch.provision` workflow.
//!
//! These tests bind qualification expectations to the authored source digest,
//! load the real authored catalog through production runtime configuration,
//! exercise the production review projection, and execute the unchanged
//! generated plan through the deterministic sandbox-root executor adapters.
//! No test in this module invokes ADB, a physical device, a real network
//! request, or an ignored physical-qualification harness.

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
use crate::planner::{
    BindingSource, ExecutionArtifact, ExecutionArtifactSource, ExecutionParamValue, ExecutionPlan,
    ExecutionStep,
};
use crate::runtime_configuration::{
    DeviceContextOverride, PlanConfigurationResult, RuntimeCapabilityAvailability,
};

const TARGET_RECIPE: &str = "app.retroarch.provision";
const QUALIFICATION_DEVICE_PLAN: &str = "ayaneo.konkr_pocket_fit.base";
const OPTIONAL_INPUT_KEY: &str = "app.retroarch.provision/retroarch_cfg";
const RETROARCH_PACKAGE: &str = "com.retroarch.aarch64";
const RETROARCH_LAUNCHER: &str = "com.retroarch.browser.mainmenu.MainMenuActivity";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetroArchQualificationContract {
    schema_version: i64,
    target_recipe: String,
    planning_device_plan: String,
    authored_source: AuthoredSourceContract,
    app_source: AuthoredSourceContract,
    app_lifecycle: AppLifecycleContract,
    selected_recipes: Vec<String>,
    expanded_recipes: Vec<String>,
    recipe_constraint_capabilities: Vec<String>,
    qualification_context_capabilities: Vec<String>,
    required_inputs: Vec<String>,
    optional_inputs: Vec<String>,
    required_operation_families: Vec<String>,
    first_launch_sequences: Vec<FirstLaunchSequenceContract>,
    material_dependency_edges: Vec<Vec<String>>,
    live_network_required_for_automated_qualification: bool,
    automated_status: String,
    physical_status: String,
    physical_cleanup_authority: String,
    permission_android_api_level: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthoredSourceContract {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppLifecycleContract {
    app_id: String,
    package_id: String,
    launcher_activity: String,
    launch_steps: Vec<String>,
    force_stop_steps: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FirstLaunchSequenceContract {
    launch_step: String,
    wait_step: String,
    wait_duration_ms: i64,
    force_stop_step: String,
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
        .join("tests/fixtures/recipe-qualification/retroarch/qualification-contract.json")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_contract() -> RetroArchQualificationContract {
    let text = fs::read_to_string(contract_path())
        .expect("RetroArch qualification contract should be readable");
    let contract: RetroArchQualificationContract = serde_json::from_str(&text)
        .expect("RetroArch qualification contract should deserialize strictly");
    assert_eq!(
        contract.schema_version, 1,
        "contract schema version must be 1"
    );
    assert_eq!(contract.target_recipe, TARGET_RECIPE);
    assert_eq!(contract.planning_device_plan, QUALIFICATION_DEVICE_PLAN);
    assert_eq!(contract.automated_status, "foundation");
    assert_eq!(contract.physical_status, "deferred");
    assert_eq!(
        contract.physical_cleanup_authority, "not_authorized_for_recipe_qualification",
        "RetroArch recipe qualification grants no physical cleanup authority"
    );
    assert!(contract.permission_android_api_level > 0);
    assert!(!contract.live_network_required_for_automated_qualification);
    contract
}

/// Prepare the real authored RetroArch configuration through the production
/// runtime-configuration path. No target serial and no device probe is used.
fn plan_retroarch(config_path: Option<&Path>) -> PlanConfigurationResult {
    plan_retroarch_with_permissions(
        config_path,
        None,
        Some(load_contract().permission_android_api_level),
    )
}

fn plan_retroarch_with_permissions(
    config_path: Option<&Path>,
    root_shell: Option<bool>,
    android_api_level: Option<i64>,
) -> PlanConfigurationResult {
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
        device_context: Some(DeviceContextOverride {
            android_api_level,
            ..DeviceContextOverride::default()
        }),
        target_device: None,
        runtime_capability_availability: root_shell.map(|root_shell| {
            RuntimeCapabilityAvailability {
                root_shell,
                app_data_write: root_shell,
                root_shell_route: root_shell.then_some(crate::planner::RootShellRoute::SuCommand),
            }
        }),
    })
    .expect("real authored RetroArch configuration should prepare")
}

fn block_unsupported_runtime_app_target_sources_before_mutation(
    plan: &ExecutionPlan,
    workspace: &QualificationWorkspace,
) -> bool {
    let Err(reason) = crate::executor::validate_supported_copy_plan(plan) else {
        return false;
    };
    assert!(
        reason.contains("App Target sources from runtime step outputs cannot be reviewed"),
        "unexpected preflight rejection: {reason}"
    );

    let mut runner = ExecutorRunner::new(dry_run_adapters(workspace));
    let execution = runner.run(plan);
    assert!(!execution.success);
    assert!(execution
        .steps
        .iter()
        .all(|record| record.status != StepRunStatus::Executed));
    assert!(runner.adapters().device().commands().is_empty());
    true
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
fn retroarch_contract_binds_current_source_and_deferred_physical_status() {
    let contract = load_contract();
    let canonical_root = repository_root()
        .canonicalize()
        .expect("repo root should canonicalize");
    for (source, expected_path) in [
        (
            &contract.authored_source,
            "authored/recipes/app.retroarch.provision.yaml",
        ),
        (&contract.app_source, "authored/apps/retroarch.yaml"),
    ] {
        let source_path = Path::new(&source.path);
        assert!(source_path.is_relative());
        assert_eq!(source_path, Path::new(expected_path));
        let resolved = repository_root().join(source_path);
        let canonical_source = resolved
            .canonicalize()
            .expect("authored source should resolve");
        assert!(canonical_source.starts_with(&canonical_root));
        assert_eq!(sha256_hex(&fs::read(resolved).unwrap()), source.sha256);
    }
}

#[test]
fn retroarch_real_authored_plan_matches_qualification_contract() {
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
    assert!(result.copy_commit_preparation_required);
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    let workspace = QualificationWorkspace::new();
    assert!(block_unsupported_runtime_app_target_sources_before_mutation(&plan, &workspace));
    let lifecycle = &contract.app_lifecycle;
    assert_eq!(lifecycle.app_id, "retroarch");
    assert_eq!(lifecycle.package_id, RETROARCH_PACKAGE);
    assert_eq!(lifecycle.launcher_activity, RETROARCH_LAUNCHER);
    assert_eq!(
        lifecycle.launch_steps,
        [
            "launch_retroarch_bootstrap",
            "launch_retroarch_permissions",
            "launch_retroarch"
        ]
    );
    assert_eq!(
        lifecycle.force_stop_steps,
        [
            "stop_retroarch_after_bootstrap",
            "stop_retroarch_after_permissions"
        ]
    );
    assert_eq!(
        plan.apps
            .iter()
            .filter(|app| app.id == lifecycle.app_id)
            .count(),
        1
    );
    for (ids, step_type, expected_activity) in [
        (
            &lifecycle.launch_steps,
            "launch_app",
            Some(RETROARCH_LAUNCHER),
        ),
        (&lifecycle.force_stop_steps, "force_stop_app", None),
    ] {
        for id in ids {
            let step = generated_step_for_authored_id(&plan, id);
            assert_eq!(step.type_name, step_type);
            assert_eq!(step.app_id.as_deref(), Some(lifecycle.app_id.as_str()));
            assert_eq!(
                crate::planner::validated_plan_lifecycle_target(&plan, step).unwrap(),
                (
                    RETROARCH_PACKAGE.to_string(),
                    expected_activity.map(str::to_string)
                )
            );
        }
    }
    assert!(result.plan_digest.is_none());

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

    assert_eq!(
        contract
            .first_launch_sequences
            .iter()
            .map(|sequence| {
                (
                    sequence.launch_step.as_str(),
                    sequence.wait_step.as_str(),
                    sequence.wait_duration_ms,
                    sequence.force_stop_step.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                "launch_retroarch_bootstrap",
                "wait_for_retroarch_bootstrap",
                1500,
                "stop_retroarch_after_bootstrap",
            ),
            (
                "launch_retroarch_permissions",
                "wait_for_retroarch_permissions",
                5000,
                "stop_retroarch_after_permissions",
            ),
        ]
    );
    for sequence in &contract.first_launch_sequences {
        let launch = generated_step_for_authored_id(&plan, &sequence.launch_step);
        let wait = generated_step_for_authored_id(&plan, &sequence.wait_step);
        let force_stop = generated_step_for_authored_id(&plan, &sequence.force_stop_step);
        assert_eq!(launch.type_name, "launch_app");
        assert_eq!(wait.type_name, "wait");
        assert_eq!(force_stop.type_name, "force_stop_app");
        assert_eq!(
            wait.params.get("duration_ms"),
            Some(&ExecutionParamValue::Literal {
                value: Value::from(sequence.wait_duration_ms),
            })
        );
        assert!(wait.dependencies.contains(&launch.id));
        assert!(force_stop.dependencies.contains(&wait.id));
    }
}

#[test]
fn retroarch_permission_sets_follow_rooted_and_unrooted_ordering_contract() {
    let contract = load_contract();
    let unrooted = plan_retroarch_with_permissions(
        None,
        Some(false),
        Some(contract.permission_android_api_level),
    )
    .plan
    .expect("unrooted baseline provisioning should remain plan-able");
    let rooted = plan_retroarch_with_permissions(
        None,
        Some(true),
        Some(contract.permission_android_api_level),
    )
    .plan
    .expect("rooted permission provisioning should be plan-able");

    let unrooted_steps = serde_json::to_value(&unrooted).unwrap();
    let unrooted_ids = unrooted_steps["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(unrooted_ids
        .iter()
        .any(|id| id.ends_with("/grant_retroarch_baseline")));
    // The App Definition assigns all-files access to elevated, so an
    // unrooted plan retains baseline grants and omits that root-only action.
    assert!(!unrooted_ids
        .iter()
        .any(|id| id.ends_with("/grant_retroarch_elevated")));
    let baseline = generated_step_for_authored_id(&unrooted, "grant_retroarch_baseline");
    assert_eq!(
        baseline.params.len(),
        0,
        "actions live only in permission_plan"
    );
    assert_eq!(
        serde_json::to_value(&baseline.permission_plan).unwrap()["permission_set"],
        "baseline"
    );
    let baseline_actions = serde_json::to_value(&baseline.permission_plan).unwrap()["actions"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(baseline_actions.len(), 7);
    assert!(baseline_actions.iter().all(|action| {
        action["kind"] == "runtime_permission"
            && action["package_name"] == RETROARCH_PACKAGE
            && action["applicability"]["status"] == "applicable"
    }));
    let unrooted_launch = generated_step_for_authored_id(&unrooted, "launch_retroarch_permissions");
    assert!(unrooted_launch
        .dependencies
        .iter()
        .any(|dependency| dependency.ends_with("/grant_retroarch_baseline")));

    let rooted_steps = serde_json::to_value(&rooted).unwrap();
    let rooted_ids = rooted_steps["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    let baseline_index = rooted_ids
        .iter()
        .position(|id| id.ends_with("/grant_retroarch_baseline"))
        .unwrap();
    let elevated_index = rooted_ids
        .iter()
        .position(|id| id.ends_with("/grant_retroarch_elevated"))
        .unwrap();
    let launch_index = rooted_ids
        .iter()
        .position(|id| id.ends_with("/launch_retroarch_permissions"))
        .unwrap();
    // Baseline is a dependency of both following steps. The stable
    // topological sort preserves their authored order when elevated and
    // launch become ready together.
    assert!(baseline_index < elevated_index && elevated_index < launch_index);
    let elevated = generated_step_for_authored_id(&rooted, "grant_retroarch_elevated");
    assert_eq!(
        elevated.params.len(),
        0,
        "actions live only in permission_plan"
    );
    assert_eq!(
        serde_json::to_value(&elevated.permission_plan).unwrap()["permission_set"],
        "elevated"
    );
    assert_eq!(
        serde_json::to_value(&elevated.permission_plan).unwrap()["actions"][0],
        serde_json::json!({
            "kind": "app_op",
            "package_name": RETROARCH_PACKAGE,
            "op": "MANAGE_EXTERNAL_STORAGE",
            "mode": "allow",
            "applicability": { "status": "applicable" }
        })
    );
    assert!(elevated
        .constraints
        .capabilities
        .iter()
        .any(|capability| capability == "root_shell"));
    let rooted_launch = generated_step_for_authored_id(&rooted, "launch_retroarch_permissions");
    assert!(rooted_launch
        .dependencies
        .iter()
        .any(|dependency| dependency.ends_with("/grant_retroarch_baseline")));
    assert!(!rooted_launch
        .dependencies
        .iter()
        .any(|dependency| dependency.ends_with("/grant_retroarch_elevated")));

    let isolated_steps = |source: &ExecutionPlan, authored_ids: &[&str]| {
        let mut isolated = source.clone();
        isolated.steps = authored_ids
            .iter()
            .map(|authored_id| generated_step_for_authored_id(source, authored_id).clone())
            .collect();
        let baseline_id = isolated.steps[0].id.clone();
        isolated.steps[0].dependencies.clear();
        for step in isolated.steps.iter_mut().skip(1) {
            step.dependencies = vec![baseline_id.clone()];
        }
        isolated
    };
    let unrooted_execution = isolated_steps(
        &unrooted,
        &["grant_retroarch_baseline", "launch_retroarch_permissions"],
    );
    let mut unrooted_runner = ExecutorRunner::new(ExecutorAdapters::default());
    let unrooted_result = unrooted_runner.run(&unrooted_execution);
    assert!(unrooted_result.success);
    let unrooted_calls = unrooted_runner.adapters().device().commands();
    let unrooted_permission_calls = unrooted_calls
        .iter()
        .filter(|call| {
            call.first()
                .is_some_and(|operation| operation == "permission_operation")
        })
        .collect::<Vec<_>>();
    assert_eq!(unrooted_permission_calls.len(), 7);
    assert!(unrooted_permission_calls
        .iter()
        .all(|call| call.get(1).is_some_and(|set| set == "baseline")));
    assert!(!unrooted_calls.iter().any(|call| call
        .first()
        .is_some_and(|operation| operation == "root_preflight")));
    assert!(unrooted_calls
        .iter()
        .position(|call| call
            .first()
            .is_some_and(|operation| operation == "launch_app"))
        .is_some_and(|launch_index| unrooted_calls
            .iter()
            .rposition(|call| call
                .first()
                .is_some_and(|operation| operation == "permission_operation"))
            .is_some_and(|permission_index| permission_index < launch_index)));

    let rooted_execution = isolated_steps(
        &rooted,
        &[
            "grant_retroarch_baseline",
            "grant_retroarch_elevated",
            "launch_retroarch_permissions",
        ],
    );
    let mut rooted_runner = ExecutorRunner::new(ExecutorAdapters::default());
    let rooted_result = rooted_runner.run(&rooted_execution);
    assert!(rooted_result.success);
    let rooted_calls = rooted_runner.adapters().device().commands();
    let rooted_operation_sequence = rooted_calls
        .iter()
        .filter_map(|call| match call.first().map(String::as_str) {
            Some("permission_operation") => call.get(1).cloned(),
            Some("root_preflight") => Some("root_preflight".to_string()),
            Some("launch_app") => Some("launch_app".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut expected_rooted_sequence = vec!["baseline".to_string(); 7];
    expected_rooted_sequence.extend([
        "root_preflight".to_string(),
        "elevated".to_string(),
        "launch_app".to_string(),
    ]);
    assert_eq!(rooted_operation_sequence, expected_rooted_sequence);
}

#[test]
fn bounded_app_permission_actions_require_a_known_android_api_level() {
    let result = plan_retroarch_with_permissions(None, Some(false), None);
    assert!(result.plan.is_none());
    let serialized = serde_json::to_value(result).unwrap();
    let diagnostics = serialized["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "permission_api_level_unknown"
            && diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("Android API level"))
    }));
}

#[test]
fn retroarch_optional_cfg_is_not_required_for_planning() {
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
fn retroarch_supplied_cfg_does_not_bypass_runtime_app_target_source_rejection() {
    let temp = tempfile::tempdir().expect("tempdir should be created");
    let config_path = temp.path().join("retroarch.cfg");
    fs::write(&config_path, b"video_driver = \"vulkan\"\n").expect("config should be written");
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

    assert!(result.copy_commit_preparation_required);
    assert!(result.plan_digest.is_none());
    assert!(result.review.is_none());
    let workspace = QualificationWorkspace::new();
    assert!(block_unsupported_runtime_app_target_sources_before_mutation(&plan, &workspace));
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

/// Compute a deterministic cache path from a concrete URL in the execution plan.
fn artifact_cache_path(cache_root: &Path, artifact: &ExecutionArtifact) -> PathBuf {
    let url = match &artifact.source {
        ExecutionArtifactSource::RemoteFile { url }
        | ExecutionArtifactSource::DirectUrl { url, .. } => url,
        source => {
            panic!("offline RetroArch qualification requires a concrete URL source, got {source:?}")
        }
    };
    cache_root.join(artifact_local_filename(&artifact.id, url, &artifact.cache))
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

fn seed_artifact_cache(cache_root: &Path, plan: &ExecutionPlan, system_mode: SystemFixtureMode) {
    fs::create_dir_all(cache_root).expect("cache root should be created");
    for artifact in &plan.artifacts {
        assert_eq!(
            artifact.cache, "default",
            "every qualified artifact must use the default cache mode"
        );
        let is_app_apk = if let Some(provenance) = &artifact.app_provenance {
            assert_eq!(provenance.app_id, "retroarch");
            let expected_kind = if provenance.artifact_id == "apk" {
                "apk"
            } else {
                "file"
            };
            assert_eq!(provenance.kind, expected_kind);
            assert!(matches!(
                &artifact.source,
                ExecutionArtifactSource::DirectUrl { .. }
            ));
            provenance.artifact_id == "apk"
        } else {
            false
        };
        let path = artifact_cache_path(cache_root, artifact);
        let leaf = artifact
            .id
            .rsplit('/')
            .next()
            .expect("artifact id should have a leaf");
        if is_app_apk {
            let fixture_workspace =
                tempfile::tempdir().expect("RetroArch APK fixture root should be created");
            let fixture_apk = crate::apk_manifest::tests::write_valid_test_apk_with_package(
                &fixture_workspace,
                RETROARCH_PACKAGE,
            );
            fs::copy(fixture_apk, &path)
                .expect("valid RetroArch APK fixture should be copied into the cache");
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

/// Test-only device adapter that fails the first authored RetroArch force-stop
/// while delegating every other operation to the deterministic dry-run device.
#[derive(Debug, Default)]
struct FirstLaunchFailureDevice {
    inner: FakeDryRunDevice,
    fail_bootstrap_force_stop: bool,
}

impl FirstLaunchFailureDevice {
    fn fail_bootstrap_force_stop() -> Self {
        Self {
            inner: FakeDryRunDevice::default(),
            fail_bootstrap_force_stop: true,
        }
    }
}

impl ExecutorDevice for FirstLaunchFailureDevice {
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
        if self.fail_bootstrap_force_stop {
            self.fail_bootstrap_force_stop = false;
            return Err(DeviceOperationError::other(
                "deterministic RetroArch bootstrap stop failure",
            ));
        }
        <FakeDryRunDevice as ExecutorDevice>::force_stop_app(&mut self.inner, package_name)
    }
}

#[test]
fn retroarch_generated_plan_blocks_unsupported_runtime_sources_before_execution() {
    let workspace = QualificationWorkspace::new();
    let contract = load_contract();
    let prepared = plan_retroarch(Some(&workspace.config_path));
    let plan = prepared.plan.expect("plan should be generated");
    if block_unsupported_runtime_app_target_sources_before_mutation(&plan, &workspace) {
        return;
    }
    seed_artifact_cache(&workspace.cache_root, &plan, SystemFixtureMode::Complete);
    let mut runner = ExecutorRunner::new(dry_run_adapters(&workspace));
    let result = runner.run(&plan);

    assert!(
        result.success,
        "generated RetroArch plan should execute successfully"
    );
    assert_eq!(result.total_steps, plan.steps.len());
    assert_eq!(result.steps.len(), result.total_steps);
    let lifecycle_commands = runner
        .adapters()
        .device()
        .commands()
        .iter()
        .filter(|command| {
            matches!(
                command.first().map(String::as_str),
                Some("launch_app" | "force_stop_app")
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle_commands,
        vec![
            vec!["launch_app", RETROARCH_PACKAGE, RETROARCH_LAUNCHER],
            vec!["force_stop_app", RETROARCH_PACKAGE],
            vec!["launch_app", RETROARCH_PACKAGE, RETROARCH_LAUNCHER],
            vec!["force_stop_app", RETROARCH_PACKAGE],
            vec!["launch_app", RETROARCH_PACKAGE, RETROARCH_LAUNCHER],
        ]
        .into_iter()
        .map(|command| command.into_iter().map(str::to_string).collect::<Vec<_>>())
        .collect::<Vec<_>>()
    );
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
        "/grant_retroarch_baseline",
        "/grant_retroarch_elevated",
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
    for sequence in &contract.first_launch_sequences {
        for suffix in [
            &sequence.launch_step,
            &sequence.wait_step,
            &sequence.force_stop_step,
        ] {
            let record = result
                .steps
                .iter()
                .find(|record| record.step_id.ends_with(&format!("/{suffix}")))
                .unwrap_or_else(|| panic!("missing execution record for /{suffix}"));
            assert_eq!(
                record.status,
                StepRunStatus::Executed,
                "first-launch lifecycle step /{suffix} must execute"
            );
        }
    }
}

#[test]
fn retroarch_first_launch_scenario_preflights_runtime_app_target_sources() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_retroarch(Some(&workspace.config_path));
    let plan = prepared.plan.expect("plan should be generated");
    if block_unsupported_runtime_app_target_sources_before_mutation(&plan, &workspace) {
        return;
    }
    let plan_before = plan.clone();
    seed_artifact_cache(&workspace.cache_root, &plan, SystemFixtureMode::Complete);
    let adapters = ExecutorAdapters::with_device_and_sandbox_roots(
        FirstLaunchFailureDevice::fail_bootstrap_force_stop(),
        workspace.runtime_root.clone(),
        workspace.cache_root.clone(),
        workspace.fake_device_root.clone(),
        vec![workspace.host_input_root.clone()],
        false,
    );
    let mut runner = ExecutorRunner::new(adapters);
    let result = runner.run(&plan);
    let status = |suffix: &str| {
        result
            .steps
            .iter()
            .find(|record| record.step_id.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing execution record for {suffix}"))
            .status
            .clone()
    };

    assert_eq!(
        plan, plan_before,
        "execution must not mutate the generated plan"
    );
    assert!(
        !result.success,
        "forced bootstrap stop failure must fail the run"
    );
    assert_eq!(
        status("/install_retroarch"),
        StepRunStatus::Executed,
        "prior installation evidence must remain successful"
    );
    assert_eq!(
        status("/launch_retroarch_bootstrap"),
        StepRunStatus::Executed
    );
    assert_eq!(
        status("/wait_for_retroarch_bootstrap"),
        StepRunStatus::Executed
    );
    assert_eq!(
        status("/stop_retroarch_after_bootstrap"),
        StepRunStatus::Failed
    );
    assert!(result
        .steps
        .iter()
        .find(|record| record.step_id.ends_with("/stop_retroarch_after_bootstrap"))
        .and_then(|record| record.message.as_deref())
        .is_some_and(|message| message.contains("deterministic RetroArch bootstrap stop failure")));
    for suffix in [
        "/grant_retroarch_baseline",
        "/grant_retroarch_elevated",
        "/launch_retroarch_permissions",
        "/wait_for_retroarch_permissions",
        "/stop_retroarch_after_permissions",
        "/copy_core_system_files",
        "/launch_retroarch",
    ] {
        assert_eq!(
            status(suffix),
            StepRunStatus::Blocked,
            "dependent step {suffix} must be blocked after the lifecycle failure"
        );
    }
}

#[test]
fn retroarch_install_scenario_preflights_runtime_app_target_sources() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_retroarch(Some(&workspace.config_path));
    let plan = prepared.plan.expect("plan should be generated");
    if block_unsupported_runtime_app_target_sources_before_mutation(&plan, &workspace) {
        return;
    }
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
fn retroarch_missing_core_scenario_preflights_runtime_app_target_sources() {
    let workspace = QualificationWorkspace::new();
    let prepared = plan_retroarch(Some(&workspace.config_path));
    let plan = prepared.plan.expect("plan should be generated");
    if block_unsupported_runtime_app_target_sources_before_mutation(&plan, &workspace) {
        return;
    }
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
