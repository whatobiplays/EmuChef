use std::cell::Cell;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::artifact_resolver::artifact_local_filename;
use crate::executor::{
    adb::{FakeAdbCommandExecutor, RealAdbDevice},
    DryRunExecutorAdapters, ExecutorAdapters, ExecutorRunner,
};
use crate::model::OrderedMap;
use crate::planner::{
    plan_execution, DeviceContext, ExecutionArtifact, ExecutionParamValue, ExecutionPlan,
    ExecutionPlanSource, ExecutionStep, ExecutionStepCondition, ExecutionStepConstraints,
    PlannerInput, RuntimeCapabilities,
};

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("compatibility_goldens_v1")
        .join(name)
}

fn read_golden(name: &str) -> Value {
    let text =
        fs::read_to_string(golden_path(name)).expect("Compatibility fixture should be readable");
    serde_json::from_str(&text).expect("Compatibility fixture should be valid JSON")
}

fn fixture_device_context() -> DeviceContext {
    DeviceContext {
        manufacturer: "Example".to_string(),
        model: "Example".to_string(),
        android_version: 13,
        android_api_level: Some(33),
        device_tags: Vec::new(),
    }
}

fn fixture_runtime_capabilities() -> RuntimeCapabilities {
    RuntimeCapabilities {
        adb_available: true,
        apk_install: true,
        shared_storage_write: true,
        app_launch: true,
        shell_command: true,
        package_remove_for_user: false,
        root_shell: true,
        app_data_write: true,
    }
}

fn plan(steps: Vec<ExecutionStep>) -> ExecutionPlan {
    plan_with_artifacts(Vec::new(), steps)
}

fn plan_with_artifacts(
    artifacts: Vec<ExecutionArtifact>,
    steps: Vec<ExecutionStep>,
) -> ExecutionPlan {
    ExecutionPlan {
        id: "plan.test".to_string(),
        source: ExecutionPlanSource {
            device_profile_ref: "example.device_profile".to_string(),
            device_plan_ref: "example.device_plan".to_string(),
            selected_recipe_refs: vec!["example.recipe".to_string()],
            expanded_recipe_refs: vec!["example.recipe".to_string()],
            catalog: None,
        },
        recipes: Vec::new(),
        target_device: None,
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
        inputs: Vec::new(),
        artifacts,
        steps,
        schema_version: 1,
        kind: "execution_plan",
    }
}

fn constraints() -> ExecutionStepConstraints {
    ExecutionStepConstraints {
        capabilities: Vec::new(),
        conflicts_with: Vec::new(),
    }
}

fn literal(value: Value) -> ExecutionParamValue {
    ExecutionParamValue::Literal { value }
}

fn wait_step(id: &str, name: &str, duration_ms: i64) -> ExecutionStep {
    let mut params = OrderedMap::new();
    params.insert("duration_ms".to_string(), literal(json!(duration_ms)));
    ExecutionStep {
        id: id.to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "wait".to_string(),
        name: name.to_string(),
        note: name.to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }
}

fn condition(type_name: &str, params: Value) -> ExecutionStepCondition {
    let mut condition_params = OrderedMap::new();
    for (key, value) in params
        .as_object()
        .expect("condition params should be object")
    {
        condition_params.insert(key.clone(), value.clone());
    }
    ExecutionStepCondition {
        type_name: type_name.to_string(),
        params: condition_params,
    }
}

fn run_value(plan: &ExecutionPlan, adapters: DryRunExecutorAdapters) -> (Value, ExecutorRunner) {
    let mut runner = ExecutorRunner::new(adapters);
    let result = runner.run(plan);
    (
        serde_json::to_value(result).expect("execution result should serialize"),
        runner,
    )
}

fn run_real_adb_value(
    plan: &ExecutionPlan,
    device: RealAdbDevice<FakeAdbCommandExecutor>,
) -> (Value, ExecutorRunner<RealAdbDevice<FakeAdbCommandExecutor>>) {
    let mut runner = ExecutorRunner::new(ExecutorAdapters::with_device(device));
    let result = runner.run(plan);
    (
        serde_json::to_value(result).expect("execution result should serialize"),
        runner,
    )
}

fn sandbox_adapters(
    runtime_root: &Path,
    cache_root: &Path,
    fake_device_root: &Path,
    read_only_roots: Vec<PathBuf>,
) -> DryRunExecutorAdapters {
    DryRunExecutorAdapters::with_sandbox_roots(
        runtime_root.to_path_buf(),
        cache_root.to_path_buf(),
        fake_device_root.to_path_buf(),
        read_only_roots,
    )
}

#[test]
fn planned_rom_copy_input_refs_resolve_to_typed_executor_values() {
    let tmp = tempfile::tempdir().expect("runtime configuration temp root should be created");
    let source = tmp.path().join("roms");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("game.rom"), b"rom").unwrap();
    let mut input = PlannerInput::from_authored_root(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("authored"),
        vec!["feature.copy_roms".to_string()],
        "plan.feature.copy_roms.001".to_string(),
        "test.plan".to_string(),
        "test.profile".to_string(),
        fixture_device_context(),
        fixture_runtime_capabilities(),
    )
    .unwrap();
    input.explicit_input_bindings.insert(
        "feature.copy_roms/source".to_string(),
        json!(source.to_string_lossy()),
    );
    input.explicit_input_bindings.insert(
        "feature.copy_roms/destination".to_string(),
        json!("/sdcard/Games"),
    );
    input
        .explicit_input_bindings
        .insert("feature.copy_roms/policy".to_string(), json!("sync"));
    let result = plan_execution(input);
    let execution_plan = result.execution_plan.expect("ROM copy should plan");
    let step = execution_plan
        .steps
        .iter()
        .find(|step| step.id == "feature.copy_roms/copy_rom_library")
        .unwrap();
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

    let runtime_root = tmp.path().join("runtime");
    let cache_root = tmp.path().join("cache");
    let fake_device_root = tmp.path().join("device");
    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![source.clone()],
        ),
    );
    assert_eq!(actual["success"], true, "{actual:#}");
    assert_eq!(
        fs::read(fake_device_root.join("sdcard/Games/game.rom")).unwrap(),
        b"rom"
    );
}

fn normalize_tmp_paths(value: Value, tmp_root: &Path) -> Value {
    fn normalize_string(mut value: String, tmp_root: &Path) -> String {
        let mut roots = vec![tmp_root.to_path_buf()];
        if let Ok(canonical) = tmp_root.canonicalize() {
            roots.push(canonical);
        }
        roots.sort_by_key(|path| std::cmp::Reverse(path.as_os_str().len()));
        for root in roots {
            value = value.replace(&root.to_string_lossy().to_string(), "$TMP");
        }
        value
    }

    match value {
        Value::String(value) => Value::String(normalize_string(value, tmp_root)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| normalize_tmp_paths(value, tmp_root))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, normalize_tmp_paths(value, tmp_root)))
                .collect(),
        ),
        other => other,
    }
}

fn runtime_value(type_name: &str, value: Value, location: Option<&str>) -> Value {
    json!({
        "type": type_name,
        "value": value,
        "location": location,
    })
}

fn write_zip(path: &Path, members: &[(&str, &str)]) {
    let file = fs::File::create(path).expect("zip fixture should be writable");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, contents) in members {
        zip.start_file(*name, options)
            .expect("zip member should start");
        zip.write_all(contents.as_bytes())
            .expect("zip member should be writable");
    }
    zip.finish().expect("zip fixture should finish");
}

fn extract_archive_step(id: &str, archive_path: &Path) -> ExecutionStep {
    let mut params = OrderedMap::new();
    params.insert(
        "archive".to_string(),
        literal(runtime_value(
            "file_path",
            json!(archive_path.to_string_lossy().to_string()),
            Some("host"),
        )),
    );
    params.insert("extract_on".to_string(), literal(json!("host")));
    ExecutionStep {
        id: id.to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "extract_archive".to_string(),
        name: "Extract Archive".to_string(),
        note: "Extract Archive".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }
}

fn install_apk_step(id: &str, apk_path: &Path, replace_existing: bool) -> ExecutionStep {
    let mut params = OrderedMap::new();
    params.insert(
        "app".to_string(),
        literal(runtime_value(
            "file_path",
            json!(apk_path.to_string_lossy().to_string()),
            Some("host"),
        )),
    );
    if replace_existing {
        params.insert("replace_existing".to_string(), literal(json!(true)));
    }
    ExecutionStep {
        id: id.to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "install_apk".to_string(),
        name: "Install APK".to_string(),
        note: "Install APK".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }
}

fn install_apk_step_with_expected_package(
    id: &str,
    apk_path: &Path,
    replace_existing: bool,
    expected_package_name: &str,
) -> ExecutionStep {
    let mut step = install_apk_step(id, apk_path, replace_existing);
    step.params.insert(
        "expected_package_name".to_string(),
        literal(json!(expected_package_name)),
    );
    step
}

fn install_apk_step_with_expected_sha256(
    id: &str,
    apk_path: &Path,
    expected_sha256: Value,
) -> ExecutionStep {
    let mut step = install_apk_step(id, apk_path, false);
    step.params
        .insert("expected_sha256".to_string(), literal(expected_sha256));
    step
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

fn launch_app_step(id: &str, package_name: &str, activity: Option<&str>) -> ExecutionStep {
    let mut params = OrderedMap::new();
    params.insert("package_name".to_string(), literal(json!(package_name)));
    if let Some(activity) = activity {
        params.insert("activity".to_string(), literal(json!(activity)));
    }
    ExecutionStep {
        id: id.to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "launch_app".to_string(),
        name: "Launch App".to_string(),
        note: "Launch App".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }
}

fn force_stop_app_step(id: &str, package_name: &str) -> ExecutionStep {
    let mut params = OrderedMap::new();
    params.insert("package_name".to_string(), literal(json!(package_name)));
    ExecutionStep {
        id: id.to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "force_stop_app".to_string(),
        name: "Force Stop App".to_string(),
        note: "Force Stop App".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }
}

#[test]
fn wait_success_matches_compatibility_executor_dry_run_result() {
    let execution_plan = plan(vec![wait_step("example.recipe/wait", "Wait", 10)]);
    let original_plan = execution_plan.clone();

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(actual, read_golden("phase6o_executor_wait_success.json"));
    assert_eq!(runner.adapters().sleep_calls(), &[0.01]);
    assert_eq!(execution_plan, original_plan);
}

#[test]
fn failures_block_dependents_but_not_unrelated_steps_like_compatibility() {
    let mut downstream = wait_step("example.recipe/downstream", "Downstream", 1);
    downstream.dependencies = vec!["example.recipe/fail".to_string()];
    let execution_plan = plan(vec![
        wait_step("example.recipe/fail", "Fail", 0),
        downstream,
        wait_step("example.recipe/unrelated", "Unrelated", 1),
    ]);
    let original_plan = execution_plan.clone();

    let (actual, _) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(
        actual,
        read_golden("phase6o_executor_failure_blocking.json")
    );
    assert_eq!(execution_plan, original_plan);
}

#[test]
fn skip_if_uses_compatibility_dry_run_device_state_and_does_not_block_dependents() {
    let mut skipped = wait_step("example.recipe/skipped", "Skipped", 1);
    skipped.skip_if = vec![condition(
        "package_installed",
        json!({"package_name": "com.example.skip"}),
    )];
    let mut downstream = wait_step("example.recipe/downstream", "Downstream", 1);
    downstream.dependencies = vec!["example.recipe/skipped".to_string()];
    let execution_plan = plan(vec![skipped, downstream]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters
        .device_mut()
        .installed_packages_mut()
        .insert("com.example.skip".to_string());

    let (actual, _) = run_value(&execution_plan, adapters);

    assert_eq!(actual, read_golden("phase6o_executor_skip_if.json"));
}

#[test]
fn verify_uses_only_compatibility_backed_condition_types_and_fails_after_execution() {
    let mut step = wait_step("example.recipe/verify", "Verify", 1);
    step.verify = vec![condition("path_exists", json!({"path": "/sdcard/missing"}))];
    let execution_plan = plan(vec![step]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert_eq!(actual["steps"][0]["message"], "verify failed: path_exists");
    assert_eq!(
        runner.adapters().device().commands(),
        &[vec![
            "path_exists".to_string(),
            "/sdcard/missing".to_string(),
            "False".to_string(),
        ]]
    );
}

#[test]
fn grant_permissions_dry_run_result_matches_compatibility_without_exposing_recorded_commands() {
    let mut params = OrderedMap::new();
    params.insert(
        "runtime".to_string(),
        literal(json!([{
            "package_name": "com.example.app",
            "name": "android.permission.POST_NOTIFICATIONS",
            "required": false
        }])),
    );
    params.insert(
        "appops".to_string(),
        literal(json!([{
            "package_name": "com.example.app",
            "op": "MANAGE_EXTERNAL_STORAGE",
            "mode": "allow",
            "required": false,
            "when": {"rooted": false}
        }])),
    );
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/grant".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "grant_permissions".to_string(),
        name: "Grant".to_string(),
        note: "Grant".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(
        actual,
        read_golden("phase6o_executor_grant_permissions.json")
    );
    assert_eq!(runner.adapters().device().commands().len(), 1);
    assert!(actual.to_string().contains("permission_results"));
    assert!(!actual.to_string().contains("run_plan_command"));
}

#[test]
fn grant_permissions_dry_run_failure_preserves_compatibility_step_outputs_and_blocks_dependents() {
    let mut params = OrderedMap::new();
    params.insert(
        "runtime".to_string(),
        literal(json!([{
            "package_name": "com.example.fail",
            "name": "android.permission.CAMERA",
            "required": true
        }])),
    );
    params.insert(
        "policy".to_string(),
        literal(json!({"on_failure": "warn", "require_all": false})),
    );
    let mut dependent = wait_step("example.recipe/dependent", "Dependent", 1);
    dependent.dependencies = vec!["example.recipe/grant_fail".to_string()];
    let execution_plan = plan(vec![
        ExecutionStep {
            id: "example.recipe/grant_fail".to_string(),
            recipe_ref: "example.recipe".to_string(),
            type_name: "grant_permissions".to_string(),
            name: "Grant Fail".to_string(),
            note: "Grant Fail".to_string(),
            dependencies: Vec::new(),
            constraints: constraints(),
            params,
            skip_if: Vec::new(),
            verify: Vec::new(),
        },
        dependent,
        wait_step("example.recipe/unrelated", "Unrelated", 1),
    ]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters.device_mut().fail_run_plan_command(
        vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            "com.example.fail".to_string(),
            "android.permission.CAMERA".to_string(),
        ],
        "permission denied",
    );

    let (actual, _) = run_value(&execution_plan, adapters);

    assert_eq!(
        actual,
        read_golden("phase6o_executor_grant_permissions_failure.json")
    );
}

#[test]
fn file_exists_condition_uses_compatibility_dry_run_path_and_directory_state() {
    let mut step = wait_step("example.recipe/verify_file", "Verify File", 1);
    step.verify = vec![condition("file_exists", json!({"path": "/sdcard/config"}))];
    let execution_plan = plan(vec![step]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters
        .device_mut()
        .remote_paths_mut()
        .insert("/sdcard/config".to_string());
    adapters
        .device_mut()
        .remote_dirs_mut()
        .insert("/sdcard/config".to_string());

    let (actual, runner) = run_value(&execution_plan, adapters);

    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["message"], "verify failed: file_exists");
    assert_eq!(
        runner.adapters().device().commands(),
        &[
            vec![
                "path_exists".to_string(),
                "/sdcard/config".to_string(),
                "False".to_string(),
            ],
            vec![
                "path_is_dir".to_string(),
                "/sdcard/config".to_string(),
                "False".to_string(),
            ],
        ]
    );
}

#[test]
fn optional_permission_command_failure_is_reported_without_failing_the_step() {
    let mut params = OrderedMap::new();
    params.insert(
        "runtime".to_string(),
        literal(json!([{
            "package_name": "com.example.optional",
            "name": "android.permission.CAMERA",
            "required": false
        }])),
    );
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/grant_optional".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "grant_permissions".to_string(),
        name: "Grant Optional".to_string(),
        note: "Grant Optional".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters.device_mut().fail_run_plan_command(
        vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            "com.example.optional".to_string(),
            "android.permission.CAMERA".to_string(),
        ],
        "permission denied",
    );

    let (actual, _) = run_value(&execution_plan, adapters);

    assert_eq!(actual["success"], true);
    assert_eq!(actual["steps"][0]["status"], "executed");
    assert_eq!(
        actual["steps"][0]["outputs"]["permission_results"]["value"]["actions"][0]["status"],
        "failed"
    );
    assert_eq!(
        actual["steps"][0]["outputs"]["permission_results"]["value"]["actions"][0]["message"],
        "permission denied"
    );
}

#[test]
fn require_all_policy_promotes_optional_permission_failure_to_step_failure() {
    let mut params = OrderedMap::new();
    params.insert(
        "runtime".to_string(),
        literal(json!([{
            "package_name": "com.example.require_all",
            "name": "android.permission.CAMERA",
            "required": false
        }])),
    );
    params.insert(
        "policy".to_string(),
        literal(json!({"on_failure": "warn", "require_all": true})),
    );
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/grant_require_all".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "grant_permissions".to_string(),
        name: "Grant Require All".to_string(),
        note: "Grant Require All".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters.device_mut().fail_run_plan_command(
        vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            "com.example.require_all".to_string(),
            "android.permission.CAMERA".to_string(),
        ],
        "permission denied",
    );

    let (actual, _) = run_value(&execution_plan, adapters);

    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert_eq!(actual["steps"][0]["message"], "permission denied");
}

#[test]
fn install_apk_dry_run_matches_compatibility_outputs_and_keeps_replace_existing_internal() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let apk = tmp.path().join("example.apk");
    fs::write(&apk, "apk").expect("apk fixture should be writable");
    let execution_plan = plan(vec![install_apk_step("example.recipe/install", &apk, true)]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(
        normalize_tmp_paths(actual.clone(), tmp.path()),
        read_golden("phase6q_executor_install_apk_replace_existing.json")
    );
    assert_eq!(actual["success"], true);
    assert_eq!(actual["steps"][0]["status"], "executed");
    assert_eq!(actual["steps"][0]["outputs"], json!({}));
    assert_eq!(
        runner.adapters().device().commands(),
        &[vec![
            "install_apk".to_string(),
            apk.to_string_lossy().to_string(),
            "True".to_string(),
        ]]
    );
    assert!(!actual.to_string().contains("install_apk"));
    assert!(!actual.to_string().contains("replace_existing"));
}

#[test]
fn install_apk_with_matching_expected_package_installs_once() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = crate::apk_manifest::tests::write_valid_test_apk(&workspace);
    let execution_plan = plan(vec![install_apk_step_with_expected_package(
        "example.recipe/install",
        &apk,
        true,
        "com.example.qualified",
    )]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(actual["success"], true);
    assert_eq!(actual["steps"][0]["status"], "executed");
    assert_eq!(
        runner.adapters().device().commands(),
        &[vec![
            "install_apk".to_string(),
            apk.to_string_lossy().to_string(),
            "True".to_string(),
        ]]
    );
}

#[test]
fn install_apk_with_mismatched_expected_package_fails_before_install() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = crate::apk_manifest::tests::write_valid_test_apk(&workspace);
    let execution_plan = plan(vec![install_apk_step_with_expected_package(
        "example.recipe/install",
        &apk,
        false,
        "Com.example.qualified",
    )]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());
    let message = actual["steps"][0]["message"]
        .as_str()
        .expect("failure should include a message");

    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert_eq!(
        message,
        "apk_package_mismatch: expected package 'Com.example.qualified', actual package 'com.example.qualified'."
    );
    assert!(!message.contains(&apk.to_string_lossy().to_string()));
    assert!(runner.adapters().device().commands().is_empty());
}

#[test]
fn install_apk_with_expected_package_redacts_inspection_failure_and_does_not_install() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = workspace.path().join("malformed.apk");
    fs::write(&apk, "not a zip").expect("malformed APK fixture should be writable");
    let execution_plan = plan(vec![install_apk_step_with_expected_package(
        "example.recipe/install",
        &apk,
        false,
        "com.example.expected",
    )]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());
    let message = actual["steps"][0]["message"]
        .as_str()
        .expect("failure should include a message");

    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert_eq!(
        message,
        "apk_package_inspection_failed: manifest inspection failed with reason 'apk_zip_invalid'."
    );
    assert!(!message.contains(&apk.to_string_lossy().to_string()));
    for internal in ["ZIP", "AXML", "rusty_axml", "not a zip"] {
        assert!(!message.contains(internal));
    }
    assert!(runner.adapters().device().commands().is_empty());
}

#[test]
fn install_apk_rejects_invalid_expected_package_value_before_inspection_or_install() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = workspace.path().join("malformed.apk");
    fs::write(&apk, "not a zip").expect("malformed APK fixture should be writable");
    let mut step = install_apk_step("example.recipe/install", &apk, false);
    step.params
        .insert("expected_package_name".to_string(), literal(json!(false)));

    let (actual, runner) = run_value(&plan(vec![step]), DryRunExecutorAdapters::default());

    assert_eq!(actual["success"], false);
    assert_eq!(
        actual["steps"][0]["message"],
        "install_apk expected_package_name must be a non-empty string literal."
    );
    assert!(runner.adapters().device().commands().is_empty());
}

#[test]
fn install_apk_with_matching_expected_sha256_installs_once() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = workspace.path().join("matching.apk");
    let bytes = b"trusted APK bytes";
    fs::write(&apk, bytes).expect("APK fixture should be writable");
    let expected = format!(" \t{}\r\n", sha256(bytes).to_ascii_lowercase());

    let (actual, runner) = run_value(
        &plan(vec![install_apk_step_with_expected_sha256(
            "example.recipe/install",
            &apk,
            json!(expected),
        )]),
        DryRunExecutorAdapters::default(),
    );

    assert_eq!(actual["success"], true);
    assert_eq!(runner.adapters().device().commands().len(), 1);
    assert_eq!(runner.adapters().device().commands()[0][0], "install_apk");
}

#[test]
fn install_apk_checksum_mismatch_is_redacted_and_does_not_install() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = workspace.path().join("mismatch.apk");
    fs::write(&apk, b"actual APK bytes").expect("APK fixture should be writable");
    let expected = "0".repeat(64);

    let (actual, runner) = run_value(
        &plan(vec![install_apk_step_with_expected_sha256(
            "example.recipe/install",
            &apk,
            json!(expected.clone()),
        )]),
        DryRunExecutorAdapters::default(),
    );
    let message = actual["steps"][0]["message"].as_str().unwrap();

    assert_eq!(actual["success"], false);
    assert!(message.starts_with(&format!(
        "apk_checksum_mismatch: expected SHA-256 '{expected}', actual SHA-256 '"
    )));
    assert!(!message.contains(&apk.to_string_lossy().to_string()));
    assert!(runner.adapters().device().commands().is_empty());
}

#[test]
fn install_apk_checksum_read_failure_is_redacted_and_does_not_install() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = workspace.path().join("unreadable.apk");
    fs::create_dir(&apk).expect("directory fixture should be created");

    let (actual, runner) = run_value(
        &plan(vec![install_apk_step_with_expected_sha256(
            "example.recipe/install",
            &apk,
            json!("0".repeat(64)),
        )]),
        DryRunExecutorAdapters::default(),
    );
    let message = actual["steps"][0]["message"].as_str().unwrap();

    assert_eq!(
        message,
        "apk_checksum_read_failed: APK checksum could not be calculated."
    );
    assert!(!message.contains(&apk.to_string_lossy().to_string()));
    assert!(runner.adapters().device().commands().is_empty());
}

#[test]
fn install_apk_rejects_invalid_runtime_checksum_before_hashing_or_installation() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = workspace.path().join("invalid.apk");
    fs::create_dir(&apk).expect("directory fixture should be created");

    let (actual, runner) = run_value(
        &plan(vec![install_apk_step_with_expected_sha256(
            "example.recipe/install",
            &apk,
            json!("sha256:not-a-digest"),
        )]),
        DryRunExecutorAdapters::default(),
    );

    assert_eq!(
        actual["steps"][0]["message"],
        "install_apk expected_sha256 must be a 64-character hexadecimal string literal."
    );
    assert!(runner.adapters().device().commands().is_empty());
}

#[test]
fn install_apk_package_enforcement_precedes_checksum_enforcement() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = crate::apk_manifest::tests::write_valid_test_apk(&workspace);
    let mut step = install_apk_step_with_expected_package(
        "example.recipe/install",
        &apk,
        false,
        "com.example.wrong",
    );
    step.params.insert(
        "expected_sha256".to_string(),
        literal(json!("0".repeat(64))),
    );

    let (actual, runner) = run_value(&plan(vec![step]), DryRunExecutorAdapters::default());

    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .starts_with("apk_package_mismatch:"));
    assert!(runner.adapters().device().commands().is_empty());
}

#[test]
fn install_apk_matching_package_proceeds_to_matching_checksum() {
    let workspace = tempfile::tempdir().expect("temp root should be created");
    let apk = crate::apk_manifest::tests::write_valid_test_apk(&workspace);
    let mut step = install_apk_step_with_expected_package(
        "example.recipe/install",
        &apk,
        false,
        "com.example.qualified",
    );
    step.params.insert(
        "expected_sha256".to_string(),
        literal(json!(sha256(&fs::read(&apk).unwrap()))),
    );

    let (actual, runner) = run_value(&plan(vec![step]), DryRunExecutorAdapters::default());

    assert_eq!(actual["success"], true);
    assert_eq!(runner.adapters().device().commands().len(), 1);
    assert_eq!(runner.adapters().device().commands()[0][0], "install_apk");
}

#[test]
fn install_apk_validation_stays_at_compatibility_executor_layer_with_compatibility_messages() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let non_apk = tmp.path().join("example.txt");
    fs::write(&non_apk, "not an apk").expect("non-apk fixture should be writable");
    let missing_apk = tmp.path().join("missing.apk");

    let (non_apk_result, non_apk_runner) = run_value(
        &plan(vec![install_apk_step(
            "example.recipe/non_apk",
            &non_apk,
            false,
        )]),
        DryRunExecutorAdapters::default(),
    );
    assert_eq!(non_apk_result["success"], false);
    assert_eq!(
        non_apk_result["steps"][0]["message"],
        format!(
            "install_apk requires an .apk file, got: {}",
            non_apk.display()
        )
    );
    assert!(non_apk_runner.adapters().device().commands().is_empty());

    let (missing_result, missing_runner) = run_value(
        &plan(vec![install_apk_step(
            "example.recipe/missing",
            &missing_apk,
            false,
        )]),
        DryRunExecutorAdapters::default(),
    );
    assert_eq!(missing_result["success"], false);
    assert_eq!(
        missing_result["steps"][0]["message"],
        format!("APK file not found: {}", missing_apk.display())
    );
    assert!(missing_runner.adapters().device().commands().is_empty());

    let mut params = OrderedMap::new();
    params.insert(
        "app".to_string(),
        literal(runtime_value(
            "directory_path",
            json!("/sdcard/app.apk"),
            Some("device"),
        )),
    );
    let invalid_runtime_plan = plan(vec![ExecutionStep {
        id: "example.recipe/invalid_runtime".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "install_apk".to_string(),
        name: "Invalid Runtime".to_string(),
        note: "Invalid Runtime".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);
    let (invalid_runtime_result, invalid_runtime_runner) =
        run_value(&invalid_runtime_plan, DryRunExecutorAdapters::default());
    assert_eq!(invalid_runtime_result["success"], false);
    assert_eq!(
        invalid_runtime_result["steps"][0]["message"],
        "install_apk requires a host-side file_path runtime value."
    );
    assert!(invalid_runtime_runner
        .adapters()
        .device()
        .commands()
        .is_empty());
}

#[test]
fn install_apk_does_not_mutate_package_state_for_later_package_installed_checks() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let apk = tmp.path().join("example.apk");
    fs::write(&apk, "apk").expect("apk fixture should be writable");
    let mut downstream = wait_step("example.recipe/downstream", "Downstream", 1);
    downstream.dependencies = vec!["example.recipe/install".to_string()];
    downstream.skip_if = vec![condition(
        "package_installed",
        json!({"package_name": "com.example.app"}),
    )];
    let execution_plan = plan(vec![
        install_apk_step("example.recipe/install", &apk, false),
        downstream,
    ]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(actual["success"], true);
    assert_eq!(actual["steps"][0]["status"], "executed");
    assert_eq!(actual["steps"][1]["status"], "executed");
    assert_eq!(
        runner.adapters().device().commands(),
        &[
            vec![
                "install_apk".to_string(),
                apk.to_string_lossy().to_string(),
                "False".to_string(),
            ],
            vec![
                "package_installed".to_string(),
                "com.example.app".to_string(),
            ],
        ]
    );
}

#[test]
fn launch_and_force_stop_dry_run_match_compatibility_empty_outputs_and_internal_logs() {
    let execution_plan = plan(vec![
        launch_app_step(
            "example.recipe/launch",
            "com.example.missing",
            Some(".MainActivity"),
        ),
        force_stop_app_step("example.recipe/force_stop", "com.example.missing"),
    ]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(
        actual,
        read_golden("phase6q_executor_launch_force_stop.json")
    );
    assert_eq!(actual["success"], true);
    assert_eq!(
        actual["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["outputs"].clone())
            .collect::<Vec<_>>(),
        vec![json!({}), json!({})]
    );
    assert_eq!(
        runner.adapters().device().commands(),
        &[
            vec![
                "launch_app".to_string(),
                "com.example.missing".to_string(),
                ".MainActivity".to_string(),
            ],
            vec![
                "force_stop_app".to_string(),
                "com.example.missing".to_string(),
            ],
        ]
    );
    assert!(!actual.to_string().contains("launch_app"));
    assert!(!actual.to_string().contains("force_stop_app"));
}

#[test]
fn device_app_failures_block_dependents_but_unrelated_steps_continue() {
    let mut dependent = wait_step("example.recipe/dependent", "Dependent", 1);
    dependent.dependencies = vec!["example.recipe/launch".to_string()];
    let execution_plan = plan(vec![
        launch_app_step("example.recipe/launch", "com.example.fail", None),
        dependent,
        wait_step("example.recipe/unrelated", "Unrelated", 1),
    ]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters
        .device_mut()
        .fail_launch_app("com.example.fail", None, "launch denied");

    let (actual, runner) = run_value(&execution_plan, adapters);

    assert_eq!(
        actual,
        read_golden("phase6q_executor_device_app_failure_blocking.json")
    );
    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert_eq!(actual["steps"][0]["message"], "launch denied");
    assert_eq!(actual["steps"][1]["status"], "blocked");
    assert_eq!(actual["steps"][2]["status"], "executed");
    assert_eq!(
        runner.adapters().device().commands(),
        &[vec![
            "launch_app".to_string(),
            "com.example.fail".to_string(),
            "".to_string(),
        ]]
    );
}

#[test]
fn force_stop_rejects_blank_package_name_with_compatibility_executor_message() {
    let execution_plan = plan(vec![force_stop_app_step(
        "example.recipe/force_stop",
        "   ",
    )]);

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert_eq!(
        actual["steps"][0]["message"],
        "force_stop_app step requires a non-empty package_name."
    );
    assert!(runner.adapters().device().commands().is_empty());
}

#[test]
fn permission_required_failure_preserves_partial_permission_results_like_compatibility() {
    let mut params = OrderedMap::new();
    params.insert(
        "runtime".to_string(),
        literal(json!([
            {
                "package_name": "com.example.first",
                "name": "android.permission.POST_NOTIFICATIONS",
                "required": false
            },
            {
                "package_name": "com.example.fail",
                "name": "android.permission.CAMERA",
                "required": true
            },
            {
                "package_name": "com.example.after",
                "name": "android.permission.RECORD_AUDIO",
                "required": false
            }
        ])),
    );
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/grant_partial".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "grant_permissions".to_string(),
        name: "Grant Partial".to_string(),
        note: "Grant Partial".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters.device_mut().fail_run_plan_command(
        vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            "com.example.fail".to_string(),
            "android.permission.CAMERA".to_string(),
        ],
        "permission denied",
    );

    let (actual, _) = run_value(&execution_plan, adapters);

    assert_eq!(
        actual,
        read_golden("phase6q_executor_permission_partial_failure.json")
    );
    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert_eq!(actual["steps"][0]["message"], "permission denied");
    let actions = actual["steps"][0]["outputs"]["permission_results"]["value"]["actions"]
        .as_array()
        .unwrap();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0]["status"], "executed");
    assert_eq!(actions[1]["status"], "failed");
    assert_eq!(actions[1]["message"], "permission denied");
}

#[test]
fn permission_policy_matrix_covers_appops_api_root_and_failure_policies() {
    let mut params = OrderedMap::new();
    params.insert(
        "runtime".to_string(),
        literal(json!([
            {
                "package_name": "com.example.optional",
                "name": "android.permission.CAMERA",
                "required": false
            },
            {
                "package_name": "com.example.api",
                "name": "android.permission.POST_NOTIFICATIONS",
                "required": false,
                "when": {"android_api_min": 34}
            }
        ])),
    );
    params.insert(
        "appops".to_string(),
        literal(json!([
            {
                "package_name": "com.example.app",
                "op": "RUN_IN_BACKGROUND",
                "mode": "ignore",
                "required": false
            },
            {
                "package_name": "com.example.root",
                "op": "MANAGE_EXTERNAL_STORAGE",
                "mode": "allow",
                "required": false,
                "when": {"rooted": false}
            }
        ])),
    );
    params.insert(
        "policy".to_string(),
        literal(json!({"on_failure": "warn", "require_all": false})),
    );
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/grant_matrix".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "grant_permissions".to_string(),
        name: "Grant Matrix".to_string(),
        note: "Grant Matrix".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters.device_mut().fail_run_plan_command(
        vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            "com.example.optional".to_string(),
            "android.permission.CAMERA".to_string(),
        ],
        "permission denied",
    );

    let (actual, _) = run_value(&execution_plan, adapters);

    assert_eq!(actual["success"], true);
    let actions = actual["steps"][0]["outputs"]["permission_results"]["value"]["actions"]
        .as_array()
        .unwrap();
    assert_eq!(
        actions
            .iter()
            .map(|action| action["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["failed", "not_applicable", "executed", "not_applicable"]
    );
    assert_eq!(actions[0]["message"], "permission denied");
    assert_eq!(actions[1]["reason_code"], "android_api_out_of_range");
    assert_eq!(actions[2]["kind"], "appop");
    assert_eq!(actions[2]["desired_mode"], "ignore");
    assert_eq!(actions[3]["reason_code"], "requires_unrooted");
}

#[test]
fn path_exists_and_file_exists_match_dry_run_remote_file_dir_and_missing_state() {
    let mut file_step = wait_step("example.recipe/file", "File", 1);
    file_step.verify = vec![condition(
        "file_exists",
        json!({"path": "/sdcard/file.bin"}),
    )];
    let mut directory_step = wait_step("example.recipe/directory", "Directory", 1);
    directory_step.verify = vec![condition("file_exists", json!({"path": "/sdcard/dir"}))];
    let mut path_step = wait_step("example.recipe/path", "Path", 1);
    path_step.verify = vec![condition("path_exists", json!({"path": "/sdcard/dir"}))];
    let mut missing_step = wait_step("example.recipe/missing", "Missing", 1);
    missing_step.verify = vec![condition("path_exists", json!({"path": "/sdcard/missing"}))];
    let execution_plan = plan(vec![file_step, directory_step, path_step, missing_step]);
    let mut adapters = DryRunExecutorAdapters::default();
    adapters
        .device_mut()
        .remote_paths_mut()
        .insert("/sdcard/file.bin".to_string());
    adapters
        .device_mut()
        .remote_paths_mut()
        .insert("/sdcard/dir".to_string());
    adapters
        .device_mut()
        .remote_dirs_mut()
        .insert("/sdcard/dir".to_string());

    let (actual, runner) = run_value(&execution_plan, adapters);

    assert_eq!(
        actual,
        read_golden("phase6q_executor_file_dir_conditions.json")
    );
    assert_eq!(actual["success"], false);
    assert_eq!(
        actual["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["executed", "failed", "executed", "failed"]
    );
    assert_eq!(actual["steps"][1]["message"], "verify failed: file_exists");
    assert_eq!(actual["steps"][3]["message"], "verify failed: path_exists");
    assert_eq!(
        runner.adapters().device().commands(),
        &[
            vec![
                "path_exists".to_string(),
                "/sdcard/file.bin".to_string(),
                "False".to_string(),
            ],
            vec![
                "path_is_dir".to_string(),
                "/sdcard/file.bin".to_string(),
                "False".to_string(),
            ],
            vec![
                "path_exists".to_string(),
                "/sdcard/dir".to_string(),
                "False".to_string(),
            ],
            vec![
                "path_is_dir".to_string(),
                "/sdcard/dir".to_string(),
                "False".to_string(),
            ],
            vec![
                "path_exists".to_string(),
                "/sdcard/dir".to_string(),
                "False".to_string(),
            ],
            vec![
                "path_exists".to_string(),
                "/sdcard/missing".to_string(),
                "False".to_string(),
            ],
        ]
    );
}

#[test]
fn real_adb_device_construction_does_not_run_commands() {
    let executor = FakeAdbCommandExecutor::default();
    let device = RealAdbDevice::with_executor("adb", Some("emulator-5554"), executor);

    assert!(device.command_executor().calls().is_empty());
}

#[test]
fn adb_shell_payload_quoting_matches_compatibility_shlex_join() {
    let mut executor = FakeAdbCommandExecutor::default();
    for _ in 0..5 {
        executor.push_completed(0, "", "");
    }
    let mut device = RealAdbDevice::with_executor("adb", Some("emulator-5554"), executor);

    assert!(device.path_exists("/sdcard/My File.txt").unwrap());
    assert!(device.path_exists("/sdcard/has'quote.txt").unwrap());
    assert!(device
        .path_exists("/sdcard/double\"quote $dollar *glob [x] semi; slash\\")
        .unwrap());
    assert!(device.path_exists("-leading-dash").unwrap());
    assert!(device
        .path_exists("/data/data/com.example/weird path")
        .unwrap());

    assert_eq!(
        device.command_executor().calls(),
        &[
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "test -e '/sdcard/My File.txt'".to_string(),
            ],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                r#"test -e '/sdcard/has'"'"'quote.txt'"#.to_string(),
            ],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                r#"test -e '/sdcard/double"quote $dollar *glob [x] semi; slash\'"#.to_string(),
            ],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "test -e -leading-dash".to_string(),
            ],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                r#"su -c 'test -e '"'"'/data/data/com.example/weird path'"'"''"#.to_string(),
            ],
        ]
    );
}

#[test]
fn run_plan_command_serial_injection_matches_compatibility() {
    let mut executor = FakeAdbCommandExecutor::default();
    executor.push_completed(0, "", "");
    executor.push_completed(0, "", "");
    let mut device = RealAdbDevice::with_executor("adb", Some("emulator-5554"), executor);

    device
        .run_plan_command(vec![
            "adb".to_string(),
            "shell".to_string(),
            "pm".to_string(),
            "grant".to_string(),
            "com.example.app".to_string(),
            "android.permission.CAMERA".to_string(),
        ])
        .unwrap();
    device
        .run_plan_command(vec![
            "adb".to_string(),
            "-s".to_string(),
            "already-selected".to_string(),
            "shell".to_string(),
            "appops".to_string(),
            "set".to_string(),
            "com.example.app".to_string(),
            "RUN_IN_BACKGROUND".to_string(),
            "ignore".to_string(),
        ])
        .unwrap();

    assert_eq!(
        device.command_executor().calls(),
        &[
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "pm".to_string(),
                "grant".to_string(),
                "com.example.app".to_string(),
                "android.permission.CAMERA".to_string(),
            ],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "already-selected".to_string(),
                "shell".to_string(),
                "appops".to_string(),
                "set".to_string(),
                "com.example.app".to_string(),
                "RUN_IN_BACKGROUND".to_string(),
                "ignore".to_string(),
            ],
        ]
    );

    let err = device
        .run_plan_command(vec!["pm".to_string(), "path".to_string()])
        .unwrap_err();
    assert_eq!(err, "Plan command must start with 'adb': ['pm', 'path']");

    let err = device.run_plan_command(Vec::new()).unwrap_err();
    assert_eq!(err, "Plan command must not be empty.");
}

#[test]
fn adb_result_mapping_uses_fake_executor_without_launching_processes() {
    let mut executor = FakeAdbCommandExecutor::default();
    executor.push_completed(1, "", "Failure [INSTALL_FAILED_ALREADY_EXISTS]\n");
    let mut device = RealAdbDevice::with_executor("adb", None, executor);

    let err = device
        .install_apk(Path::new("/tmp/example app.apk"), true)
        .unwrap_err();

    assert_eq!(
        err,
        "ADB command failed (1): adb install -r /tmp/example app.apk\nFailure [INSTALL_FAILED_ALREADY_EXISTS]"
    );
    assert_eq!(
        device.command_executor().calls(),
        &[vec![
            "adb".to_string(),
            "install".to_string(),
            "-r".to_string(),
            "/tmp/example app.apk".to_string(),
        ]]
    );

    let mut executor = FakeAdbCommandExecutor::default();
    executor.push_missing_binary();
    let mut device = RealAdbDevice::with_executor("adb", None, executor);
    let err = device.force_stop_app("com.example.app").unwrap_err();

    assert_eq!(
        err,
        "The configured ADB executable could not be started. Ensure adb is available on PATH or pass an explicit executable when constructing RealAdbDevice."
    );
    assert_eq!(
        device.command_executor().calls(),
        &[vec![
            "adb".to_string(),
            "shell".to_string(),
            "am".to_string(),
            "force-stop".to_string(),
            "com.example.app".to_string(),
        ]]
    );
}

#[test]
fn package_installed_maps_compatibility_stdout_and_exit_code() {
    let mut executor = FakeAdbCommandExecutor::default();
    executor.push_completed(0, "package:/data/app/com.example/base.apk\n", "");
    executor.push_completed(1, "", "package not found");
    let mut device = RealAdbDevice::with_executor("adb", Some("emulator-5554"), executor);

    assert!(device.package_installed("com.example.present").unwrap());
    assert!(!device.package_installed("com.example.missing").unwrap());
    assert_eq!(
        device.command_executor().calls(),
        &[
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "pm".to_string(),
                "path".to_string(),
                "com.example.present".to_string(),
            ],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "pm".to_string(),
                "path".to_string(),
                "com.example.missing".to_string(),
            ],
        ]
    );
}

#[test]
fn launch_app_command_shapes_match_compatibility_explicit_resolved_and_fallback_paths() {
    let mut explicit_executor = FakeAdbCommandExecutor::default();
    explicit_executor.push_completed(0, "", "");
    let mut explicit_device = RealAdbDevice::with_executor("adb", None, explicit_executor);

    explicit_device
        .launch_app("com.example.app", Some(".MainActivity"))
        .unwrap();
    assert_eq!(
        explicit_device.command_executor().calls(),
        &[vec![
            "adb".to_string(),
            "shell".to_string(),
            "am".to_string(),
            "start".to_string(),
            "-n".to_string(),
            "com.example.app/.MainActivity".to_string(),
        ]]
    );

    let mut resolved_executor = FakeAdbCommandExecutor::default();
    resolved_executor.push_completed(0, "priority=0\ncom.example.app/.ResolvedActivity\n", "");
    resolved_executor.push_completed(0, "", "");
    let mut resolved_device =
        RealAdbDevice::with_executor("adb", Some("emulator-5554"), resolved_executor);

    resolved_device.launch_app("com.example.app", None).unwrap();
    assert_eq!(
        resolved_device.command_executor().calls(),
        &[
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "cmd".to_string(),
                "package".to_string(),
                "resolve-activity".to_string(),
                "--brief".to_string(),
                "com.example.app".to_string(),
            ],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "am".to_string(),
                "start".to_string(),
                "-n".to_string(),
                "com.example.app/.ResolvedActivity".to_string(),
            ],
        ]
    );

    let mut fallback_executor = FakeAdbCommandExecutor::default();
    fallback_executor.push_completed(1, "", "cmd failed");
    fallback_executor.push_completed(0, "no component here\n", "");
    fallback_executor.push_completed(0, "", "");
    let mut fallback_device = RealAdbDevice::with_executor("adb", None, fallback_executor);

    fallback_device.launch_app("com.example.app", None).unwrap();
    assert_eq!(
        fallback_device.command_executor().calls(),
        &[
            vec![
                "adb".to_string(),
                "shell".to_string(),
                "cmd".to_string(),
                "package".to_string(),
                "resolve-activity".to_string(),
                "--brief".to_string(),
                "com.example.app".to_string(),
            ],
            vec![
                "adb".to_string(),
                "shell".to_string(),
                "pm".to_string(),
                "resolve-activity".to_string(),
                "--brief".to_string(),
                "com.example.app".to_string(),
            ],
            vec![
                "adb".to_string(),
                "shell".to_string(),
                "monkey".to_string(),
                "-p".to_string(),
                "com.example.app".to_string(),
                "-c".to_string(),
                "android.intent.category.LAUNCHER".to_string(),
                "1".to_string(),
            ],
        ]
    );
}

#[test]
fn executor_can_use_explicit_real_adb_device_for_selected_handlers() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let apk = tmp.path().join("example app.apk");
    fs::write(&apk, "apk").expect("apk fixture should be writable");

    let mut params = OrderedMap::new();
    params.insert(
        "runtime".to_string(),
        literal(json!([
            {
                "package_name": "com.example.app",
                "name": "android.permission.POST_NOTIFICATIONS"
            }
        ])),
    );
    let grant_step = ExecutionStep {
        id: "example.recipe/grant".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "grant_permissions".to_string(),
        name: "Grant".to_string(),
        note: "Grant".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    };
    let execution_plan = plan(vec![
        install_apk_step("example.recipe/install", &apk, true),
        grant_step,
    ]);

    let mut executor = FakeAdbCommandExecutor::default();
    executor.push_completed(0, "", "");
    executor.push_completed(0, "", "");
    let device = RealAdbDevice::with_executor("adb", Some("emulator-5554"), executor);

    let (actual, runner) = run_real_adb_value(&execution_plan, device);

    assert_eq!(actual["success"], true);
    assert_eq!(actual["steps"][0]["status"], "executed");
    assert_eq!(actual["steps"][1]["status"], "executed");
    assert_eq!(
        runner.adapters().device().command_executor().calls(),
        &[
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "install".to_string(),
                "-r".to_string(),
                apk.to_string_lossy().to_string(),
            ],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "emulator-5554".to_string(),
                "shell".to_string(),
                "pm".to_string(),
                "grant".to_string(),
                "com.example.app".to_string(),
                "android.permission.POST_NOTIFICATIONS".to_string(),
            ],
        ]
    );
}

#[test]
fn rust_real_adb_file_operations_forward_exact_executable_and_serial() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let source = tmp.path().join("payload.bin");
    fs::write(&source, "payload").expect("source should be writable");
    let mut device = RealAdbDevice::with_executor(
        "/opt/android/adb",
        Some("device-123"),
        FakeAdbCommandExecutor::default(),
    );

    device.push(&source, "/sdcard/payload.bin", false).unwrap();
    device.push(&source, "/sdcard/payload.bin", true).unwrap();
    device.mkdir_p("/sdcard/EmuChef").unwrap();
    device.remove_file("/sdcard/payload.bin").unwrap();
    device.remove_tree("/sdcard/EmuChef").unwrap();
    device
        .copy_on_device(
            "/data/local/tmp/emuchef/payload.bin",
            "/data/data/com.example/payload.bin",
            false,
            true,
        )
        .unwrap();

    let calls = device.command_executor().calls();
    assert_eq!(calls.len(), 6);
    assert!(calls.iter().all(|call| {
        call.starts_with(&[
            "/opt/android/adb".to_string(),
            "-s".to_string(),
            "device-123".to_string(),
        ])
    }));
    assert_eq!(calls[0][3..5], ["push", source.to_string_lossy().as_ref()]);
    assert_eq!(
        calls[1][3..6],
        ["push", "--sync", source.to_string_lossy().as_ref()]
    );
    assert!(calls[5]
        .last()
        .expect("privileged copy should have a shell payload")
        .starts_with("su -c "));
}

#[test]
fn device_archive_extraction_happens_on_host_then_pushes_without_unzip_command() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let runtime_root = tmp.path().join("runtime");
    let cache_root = tmp.path().join("cache");
    let fake_device_root = tmp.path().join("fake-device");
    let archive = tmp.path().join("archive.zip");
    write_zip(&archive, &[("nested/file.txt", "hello")]);
    let mut step = extract_archive_step("example.recipe/extract-device", &archive);
    step.params
        .insert("extract_on".to_string(), literal(json!("device")));
    step.params.insert(
        "dest".to_string(),
        literal(json!("/sdcard/EmuChef/extracted")),
    );

    let (actual, runner) = run_value(
        &plan(vec![step]),
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![tmp.path().to_path_buf()],
        ),
    );

    assert_eq!(actual["success"], true);
    assert_eq!(
        actual["steps"][0]["outputs"]["extracted_path"]["location"],
        "device"
    );
    let commands = runner.adapters().device().commands();
    assert!(commands.iter().any(|command| command[0] == "mkdir_p"));
    assert!(commands.iter().any(|command| command[0] == "push"));
    assert!(!commands.iter().flatten().any(|token| token == "unzip"));
}

#[test]
fn artifact_filename_algorithm_matches_compatibility_reference() {
    assert_eq!(
        artifact_local_filename("example.recipe/archive", "file:///tmp/archive.zip", "none"),
        "b8fa707d78f0719a711d8ecb5c37a8cc5f2f4e56f69f9f8acd40c7eccd063c79-archive.zip"
    );
    assert_eq!(
        artifact_local_filename(
            "example.recipe/archive",
            "file:///tmp/archive.zip",
            "default"
        ),
        "b92b76f1245b8ee58e3187c04b57d954864c55557c4bf885ebad66c9c5aba8a0-archive.zip"
    );
    assert_eq!(
        artifact_local_filename("example.recipe/archive", "file:///tmp/", "none"),
        "e3d2cb6c819416e48eaaed5ddf2bdee5e099beb502106d7b6ecdfadb4712594a-tmp"
    );
    assert_eq!(
        artifact_local_filename(
            "example.recipe/archive",
            "https://example.com/downloads/archive.zip?token=1",
            "none"
        ),
        "6a3cbf4fa3962ac24d057186af9cad0a34b95466596e5393a1f4d53a8b72dfaf-archive.zip"
    );
}

#[test]
fn resolve_extract_and_copy_flow_matches_compatibility_and_stays_in_sandbox() {
    // Regression for UX-039: verification must observe files materialized in the
    // simulated-device sandbox, not only the fake adapter's in-memory path sets.
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let zip_a = fixture_root.join("a.zip");
    let zip_b = fixture_root.join("b.zip");
    write_zip(&zip_a, &[("core_a.so", "alpha")]);
    write_zip(&zip_b, &[("core_b.so", "beta")]);
    let original_zip_a = fs::read(&zip_a).expect("fixture should be readable");
    let original_zip_b = fs::read(&zip_b).expect("fixture should be readable");

    let mut resolve_params = OrderedMap::new();
    resolve_params.insert(
        "artifacts".to_string(),
        literal(json!(["example.recipe/a_zip", "example.recipe/b_zip"])),
    );
    let mut extract_params = OrderedMap::new();
    extract_params.insert(
        "artifacts".to_string(),
        literal(json!(["example.recipe/a_zip", "example.recipe/b_zip"])),
    );
    extract_params.insert("extract_on".to_string(), literal(json!("host")));
    let mut copy_params = OrderedMap::new();
    copy_params.insert(
        "source".to_string(),
        ExecutionParamValue::Ref {
            ref_value: "steps.example.recipe/extract.outputs.extracted_paths".to_string(),
        },
    );
    copy_params.insert(
        "dest".to_string(),
        literal(json!("/sdcard/RetroArch/cores")),
    );
    copy_params.insert("copy_policy".to_string(), literal(json!("sync")));

    let mut extract = ExecutionStep {
        id: "example.recipe/extract".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "extract_artifacts".to_string(),
        name: "Extract".to_string(),
        note: "Extract".to_string(),
        dependencies: vec!["example.recipe/resolve".to_string()],
        constraints: constraints(),
        params: extract_params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    };
    let mut copy = ExecutionStep {
        id: "example.recipe/copy".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "copy_files".to_string(),
        name: "Copy".to_string(),
        note: "Copy".to_string(),
        dependencies: vec!["example.recipe/extract".to_string()],
        constraints: constraints(),
        params: copy_params,
        skip_if: Vec::new(),
        verify: vec![
            condition(
                "file_exists",
                json!({ "path": "/sdcard/RetroArch/cores/core_a.so" }),
            ),
            condition(
                "file_exists",
                json!({ "path": "/sdcard/RetroArch/cores/core_b.so" }),
            ),
        ],
    };
    extract.dependencies = vec!["example.recipe/resolve".to_string()];
    copy.dependencies = vec!["example.recipe/extract".to_string()];
    let execution_plan = plan_with_artifacts(
        vec![
            ExecutionArtifact {
                id: "example.recipe/a_zip".to_string(),
                type_name: "remote_file".to_string(),
                url: format!("file://{}", zip_a.canonicalize().unwrap().to_string_lossy()),
                cache: "none".to_string(),
            },
            ExecutionArtifact {
                id: "example.recipe/b_zip".to_string(),
                type_name: "remote_file".to_string(),
                url: format!("file://{}", zip_b.canonicalize().unwrap().to_string_lossy()),
                cache: "none".to_string(),
            },
        ],
        vec![
            ExecutionStep {
                id: "example.recipe/resolve".to_string(),
                recipe_ref: "example.recipe".to_string(),
                type_name: "resolve_artifacts".to_string(),
                name: "Resolve".to_string(),
                note: "Resolve".to_string(),
                dependencies: Vec::new(),
                constraints: constraints(),
                params: resolve_params,
                skip_if: Vec::new(),
                verify: Vec::new(),
            },
            extract,
            copy,
        ],
    );
    let original_plan = execution_plan.clone();

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root.clone()],
        ),
    );

    assert_eq!(
        normalize_tmp_paths(actual, tmp.path()),
        read_golden("phase6p_executor_resolve_extract_copy_flow.json")
    );
    assert_eq!(execution_plan, original_plan);
    assert_eq!(fs::read(&zip_a).unwrap(), original_zip_a);
    assert_eq!(fs::read(&zip_b).unwrap(), original_zip_b);
    assert_eq!(
        fs::read_to_string(fake_device_root.join("sdcard/RetroArch/cores/core_a.so")).unwrap(),
        "alpha"
    );
    assert_eq!(
        fs::read_to_string(fake_device_root.join("sdcard/RetroArch/cores/core_b.so")).unwrap(),
        "beta"
    );
    assert!(runtime_root.join("downloads").exists());
    assert!(!tmp.path().join("sdcard").exists());
}

#[test]
fn extract_archive_success_matches_compatibility_golden() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let archive = fixture_root.join("archive.zip");
    write_zip(&archive, &[("single.txt", "hello")]);

    let execution_plan = plan(vec![extract_archive_step(
        "example.recipe/extract_archive",
        &archive,
    )]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(
        normalize_tmp_paths(actual, tmp.path()),
        read_golden("phase6p_executor_extract_archive_success.json")
    );
    assert_eq!(
        fs::read_to_string(runtime_root.join("extract/example.recipe_extract_archive/single.txt"))
            .unwrap(),
        "hello"
    );
}

#[test]
fn unsupported_artifact_scheme_fails_without_network_download_attempt() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    let mut resolve_params = OrderedMap::new();
    resolve_params.insert(
        "artifacts".to_string(),
        literal(json!(["example.recipe/archive"])),
    );
    let mut dependent = wait_step("example.recipe/downstream", "Downstream", 1);
    dependent.dependencies = vec!["example.recipe/resolve".to_string()];
    let execution_plan = plan_with_artifacts(
        vec![ExecutionArtifact {
            id: "example.recipe/archive".to_string(),
            type_name: "remote_file".to_string(),
            url: "ftp://example.invalid/archive.zip".to_string(),
            cache: "none".to_string(),
        }],
        vec![
            ExecutionStep {
                id: "example.recipe/resolve".to_string(),
                recipe_ref: "example.recipe".to_string(),
                type_name: "resolve_artifacts".to_string(),
                name: "Resolve".to_string(),
                note: "Resolve".to_string(),
                dependencies: Vec::new(),
                constraints: constraints(),
                params: resolve_params,
                skip_if: Vec::new(),
                verify: Vec::new(),
            },
            dependent,
        ],
    );

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(&runtime_root, &cache_root, &fake_device_root, Vec::new()),
    );

    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("artifact_scheme_unsupported"));
    assert_eq!(actual["steps"][1]["status"], "blocked");
    assert!(!runtime_root.join("downloads").exists());
}

#[test]
fn http_status_failure_blocks_dependents_allows_unrelated_steps_and_cleans_partial() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        let _ = std::io::Read::read(&mut stream, &mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 11\r\nConnection: close\r\n\r\nsecret body")
            .unwrap();
    });
    let tmp = tempfile::tempdir().unwrap();
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    let url = format!("http://{address}/archive.zip?token=secret");
    let mut resolve_params = OrderedMap::new();
    resolve_params.insert(
        "artifacts".to_string(),
        literal(json!(["example.recipe/archive"])),
    );
    let mut dependent = wait_step("example.recipe/downstream", "Downstream", 1);
    dependent.dependencies = vec!["example.recipe/resolve".to_string()];
    let execution_plan = plan_with_artifacts(
        vec![ExecutionArtifact {
            id: "example.recipe/archive".to_string(),
            type_name: "remote_file".to_string(),
            url,
            cache: "default".to_string(),
        }],
        vec![
            ExecutionStep {
                id: "example.recipe/resolve".to_string(),
                recipe_ref: "example.recipe".to_string(),
                type_name: "resolve_artifacts".to_string(),
                name: "Resolve".to_string(),
                note: "Resolve".to_string(),
                dependencies: Vec::new(),
                constraints: constraints(),
                params: resolve_params,
                skip_if: Vec::new(),
                verify: Vec::new(),
            },
            dependent,
            wait_step("example.recipe/unrelated", "Unrelated", 1),
        ],
    );
    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(&runtime_root, &cache_root, &fake_device_root, Vec::new()),
    );
    server.join().unwrap();

    assert_eq!(actual["steps"][0]["status"], "failed");
    let message = actual["steps"][0]["message"].as_str().unwrap();
    assert!(message.starts_with("artifact_http_status"));
    assert!(message.contains("HTTP 500"));
    assert!(!message.contains("secret"));
    assert_eq!(actual["steps"][1]["status"], "blocked");
    assert_eq!(actual["steps"][2]["status"], "executed");
    if cache_root.exists() {
        assert!(fs::read_dir(cache_root).unwrap().next().is_none());
    }
}

#[test]
fn extract_archive_invalid_failure_blocks_dependent_like_compatibility() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let archive = fixture_root.join("invalid.zip");
    fs::write(&archive, "not a zip").expect("invalid archive fixture should be writable");
    let mut dependent = wait_step("example.recipe/downstream", "Downstream", 1);
    dependent.dependencies = vec!["example.recipe/extract_archive".to_string()];

    let execution_plan = plan(vec![
        extract_archive_step("example.recipe/extract_archive", &archive),
        dependent,
        wait_step("example.recipe/unrelated", "Unrelated", 1),
    ]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(
        normalize_tmp_paths(actual, tmp.path()),
        read_golden("phase6p_executor_extract_archive_invalid_failure.json")
    );
}

#[test]
fn extract_archive_rejects_traversal_entries_without_writing_outside_temp_root() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let archive = fixture_root.join("malicious.zip");
    write_zip(&archive, &[("../escape.txt", "owned")]);

    let execution_plan = plan(vec![extract_archive_step(
        "example.recipe/extract_archive",
        &archive,
    )]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], false);
    assert_eq!(actual["steps"][0]["status"], "failed");
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("unsafe archive entry"));
    assert!(!tmp.path().join("escape.txt").exists());
}

#[test]
fn extract_archive_rejects_absolute_entries_before_writing() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let archive = fixture_root.join("absolute.zip");
    write_zip(&archive, &[("/absolute.txt", "owned")]);

    let execution_plan = plan(vec![extract_archive_step(
        "example.recipe/extract_archive",
        &archive,
    )]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], false);
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("unsafe archive entry"));
    assert!(!runtime_root
        .join("extract/example.recipe_extract_archive/absolute.txt")
        .exists());
}

#[test]
fn extract_archive_prescans_entries_before_writing_any_member() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let archive = fixture_root.join("partially_malicious.zip");
    write_zip(
        &archive,
        &[("safe.txt", "safe"), ("../escape.txt", "owned")],
    );

    let execution_plan = plan(vec![extract_archive_step(
        "example.recipe/extract_archive",
        &archive,
    )]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], false);
    assert!(!runtime_root
        .join("extract/example.recipe_extract_archive/safe.txt")
        .exists());
    assert!(!tmp.path().join("escape.txt").exists());
}

#[cfg(unix)]
#[test]
fn extract_archive_rejects_symlinked_extract_parent_before_writing() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let archive = fixture_root.join("archive.zip");
    write_zip(&archive, &[("safe.txt", "safe")]);
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&runtime_root).unwrap();
    std::os::unix::fs::symlink(&outside, runtime_root.join("extract")).unwrap();

    let execution_plan = plan(vec![extract_archive_step(
        "example.recipe/extract_archive",
        &archive,
    )]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], false);
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("symlink"));
    assert!(!outside
        .join("example.recipe_extract_archive/safe.txt")
        .exists());
}

#[cfg(unix)]
#[test]
fn extract_archive_rejects_symlinked_runtime_root_before_writing() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let archive = fixture_root.join("archive.zip");
    write_zip(&archive, &[("safe.txt", "safe")]);
    let outside = tmp.path().join("outside_runtime");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &runtime_root).unwrap();

    let execution_plan = plan(vec![extract_archive_step(
        "example.recipe/extract_archive",
        &archive,
    )]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], false);
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("symlink"));
    assert!(!outside
        .join("extract/example.recipe_extract_archive/safe.txt")
        .exists());
}

#[test]
fn copy_replace_deletes_only_inside_fake_device_root() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    fs::create_dir_all(fake_device_root.join("sdcard/cores")).unwrap();
    fs::write(fake_device_root.join("sdcard/cores/old.so"), "old").unwrap();
    let outside = tmp.path().join("outside/keep.txt");
    fs::create_dir_all(outside.parent().unwrap()).unwrap();
    fs::write(&outside, "keep").unwrap();
    let source_dir = fixture_root.join("cores");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("new.so"), "new").unwrap();

    let mut params = OrderedMap::new();
    params.insert(
        "source".to_string(),
        literal(runtime_value(
            "directory_path",
            json!(source_dir.to_string_lossy().to_string()),
            Some("host"),
        )),
    );
    params.insert("dest".to_string(), literal(json!("/sdcard/cores")));
    params.insert("copy_policy".to_string(), literal(json!("replace")));
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/copy".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "copy_files".to_string(),
        name: "Copy".to_string(),
        note: "Copy".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], true);
    assert!(!fake_device_root.join("sdcard/cores/old.so").exists());
    assert_eq!(
        fs::read_to_string(fake_device_root.join("sdcard/cores/new.so")).unwrap(),
        "new"
    );
    assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
}

#[test]
fn copy_sync_preserves_stale_files_like_compatibility_push_sync() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    fs::create_dir_all(fake_device_root.join("sdcard/cores")).unwrap();
    fs::write(fake_device_root.join("sdcard/cores/stale.so"), "stale").unwrap();
    let source_dir = fixture_root.join("cores");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("new.so"), "new").unwrap();
    let mut params = OrderedMap::new();
    params.insert(
        "source".to_string(),
        literal(runtime_value(
            "directory_path",
            json!(source_dir.to_string_lossy().to_string()),
            Some("host"),
        )),
    );
    params.insert("dest".to_string(), literal(json!("/sdcard/cores")));
    params.insert("copy_policy".to_string(), literal(json!("sync")));
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/copy".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "copy_files".to_string(),
        name: "Copy".to_string(),
        note: "Copy".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], true);
    assert_eq!(
        fs::read_to_string(fake_device_root.join("sdcard/cores/stale.so")).unwrap(),
        "stale"
    );
    assert_eq!(
        fs::read_to_string(fake_device_root.join("sdcard/cores/new.so")).unwrap(),
        "new"
    );
}

#[test]
fn copy_rejects_destination_traversal_before_writing() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let source = fixture_root.join("core.so");
    fs::write(&source, "core").unwrap();
    let mut params = OrderedMap::new();
    params.insert(
        "source".to_string(),
        literal(runtime_value(
            "file_path",
            json!(source.to_string_lossy().to_string()),
            Some("host"),
        )),
    );
    params.insert("dest".to_string(), literal(json!("/sdcard/../escape.so")));

    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/copy".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "copy_files".to_string(),
        name: "Copy".to_string(),
        note: "Copy".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], false);
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("path traversal"));
    assert!(!fake_device_root.join("escape.so").exists());
}

#[cfg(unix)]
#[test]
fn copy_rejects_fake_device_symlink_escape() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let source = fixture_root.join("core.so");
    fs::write(&source, "core").unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(fake_device_root.join("sdcard")).unwrap();
    std::os::unix::fs::symlink(&outside, fake_device_root.join("sdcard/cores")).unwrap();
    let mut params = OrderedMap::new();
    params.insert(
        "source".to_string(),
        literal(runtime_value(
            "file_path",
            json!(source.to_string_lossy().to_string()),
            Some("host"),
        )),
    );
    params.insert("dest".to_string(), literal(json!("/sdcard/cores/core.so")));
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/copy".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "copy_files".to_string(),
        name: "Copy".to_string(),
        note: "Copy".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], false);
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("symlink"));
    assert!(!outside.join("core.so").exists());
}

#[cfg(unix)]
#[test]
fn copy_rejects_symlinked_fake_device_root_before_writing() {
    let tmp = tempfile::tempdir().expect("temp root should be created");
    let fixture_root = tmp.path().join("fixtures");
    let runtime_root = tmp.path().join(".emuchef_runtime");
    let cache_root = tmp.path().join(".emuchef_cache").join("artifacts");
    let fake_device_root = tmp.path().join("fake_device");
    fs::create_dir_all(&fixture_root).expect("fixture root should be created");
    let source = fixture_root.join("core.so");
    fs::write(&source, "core").unwrap();
    let outside = tmp.path().join("outside_device");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &fake_device_root).unwrap();
    let mut params = OrderedMap::new();
    params.insert(
        "source".to_string(),
        literal(runtime_value(
            "file_path",
            json!(source.to_string_lossy().to_string()),
            Some("host"),
        )),
    );
    params.insert("dest".to_string(), literal(json!("/sdcard/core.so")));
    let execution_plan = plan(vec![ExecutionStep {
        id: "example.recipe/copy".to_string(),
        recipe_ref: "example.recipe".to_string(),
        type_name: "copy_files".to_string(),
        name: "Copy".to_string(),
        note: "Copy".to_string(),
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }]);

    let (actual, _) = run_value(
        &execution_plan,
        sandbox_adapters(
            &runtime_root,
            &cache_root,
            &fake_device_root,
            vec![fixture_root],
        ),
    );

    assert_eq!(actual["success"], false);
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("symlink"));
    assert!(!outside.join("sdcard/core.so").exists());
}

#[test]
fn cooperative_cancellation_preserves_completed_work_and_schedules_no_later_steps() {
    let execution_plan = plan(vec![
        wait_step("example.recipe/first", "First", 1),
        wait_step("example.recipe/second", "Second", 1),
        wait_step("example.recipe/third", "Third", 1),
    ]);
    let checks = Cell::new(0usize);
    let mut runner = ExecutorRunner::default();
    let result = runner.run_with_progress_and_cancel(
        &execution_plan,
        |_| {},
        || {
            let current = checks.get();
            checks.set(current + 1);
            current >= 1
        },
    );

    assert!(!result.success);
    assert!(result.cancelled);
    assert_eq!(
        result.steps[0].status,
        crate::executor::StepRunStatus::Executed
    );
    assert_eq!(
        result.steps[1].status,
        crate::executor::StepRunStatus::Cancelled
    );
    assert_eq!(
        result.steps[2].status,
        crate::executor::StepRunStatus::Cancelled
    );
    assert_eq!(runner.adapters().sleep_calls(), &[0.001]);
}
