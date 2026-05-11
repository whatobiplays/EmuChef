use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::executor::{artifact_local_filename, DryRunExecutorAdapters, ExecutorRunner};
use crate::model::OrderedMap;
use crate::planner::{
    DeviceContext, ExecutionArtifact, ExecutionParamValue, ExecutionPlan, ExecutionPlanSource,
    ExecutionStep, ExecutionStepCondition, ExecutionStepConstraints, RuntimeCapabilities,
};

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("python_goldens")
        .join(name)
}

fn read_golden(name: &str) -> Value {
    let text = fs::read_to_string(golden_path(name))
        .expect("Python executor parity fixture should be readable");
    serde_json::from_str(&text).expect("Python executor parity fixture should be valid JSON")
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
        },
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
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }
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
        dependencies: Vec::new(),
        constraints: constraints(),
        params,
        skip_if: Vec::new(),
        verify: Vec::new(),
    }
}

#[test]
fn wait_success_matches_python_executor_dry_run_result() {
    let execution_plan = plan(vec![wait_step("example.recipe/wait", "Wait", 10)]);
    let original_plan = execution_plan.clone();

    let (actual, runner) = run_value(&execution_plan, DryRunExecutorAdapters::default());

    assert_eq!(actual, read_golden("phase6o_executor_wait_success.json"));
    assert_eq!(runner.adapters().sleep_calls(), &[0.01]);
    assert_eq!(execution_plan, original_plan);
}

#[test]
fn failures_block_dependents_but_not_unrelated_steps_like_python() {
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
fn skip_if_uses_python_dry_run_device_state_and_does_not_block_dependents() {
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
fn verify_uses_only_python_backed_condition_types_and_fails_after_execution() {
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
fn grant_permissions_dry_run_result_matches_python_without_exposing_recorded_commands() {
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
fn grant_permissions_dry_run_failure_preserves_python_step_outputs_and_blocks_dependents() {
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
fn file_exists_condition_uses_python_dry_run_path_and_directory_state() {
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
fn phase6q_install_apk_dry_run_matches_python_outputs_and_keeps_replace_existing_internal() {
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
fn phase6q_install_apk_validation_stays_at_python_executor_layer_with_python_messages() {
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
fn phase6q_install_apk_does_not_mutate_package_state_for_later_package_installed_checks() {
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
fn phase6q_launch_and_force_stop_dry_run_match_python_empty_outputs_and_internal_logs() {
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
fn phase6q_device_app_failures_block_dependents_but_unrelated_steps_continue() {
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
fn phase6q_force_stop_rejects_blank_package_name_with_python_executor_message() {
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
fn phase6q_permission_required_failure_preserves_partial_permission_results_like_python() {
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
fn phase6q_permission_policy_matrix_covers_appops_api_root_and_failure_policies() {
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
fn phase6q_path_exists_and_file_exists_match_dry_run_remote_file_dir_and_missing_state() {
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
fn phase6p_artifact_filename_algorithm_matches_python_reference() {
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
fn phase6p_resolve_extract_and_copy_flow_matches_python_and_stays_in_sandbox() {
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
        dependencies: vec!["example.recipe/extract".to_string()],
        constraints: constraints(),
        params: copy_params,
        skip_if: Vec::new(),
        verify: Vec::new(),
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
fn phase6p_extract_archive_success_matches_python_golden() {
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
fn phase6p_remote_artifact_resolution_fails_without_network_download_attempt() {
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
            url: "https://example.invalid/archive.zip".to_string(),
            cache: "none".to_string(),
        }],
        vec![
            ExecutionStep {
                id: "example.recipe/resolve".to_string(),
                recipe_ref: "example.recipe".to_string(),
                type_name: "resolve_artifacts".to_string(),
                name: "Resolve".to_string(),
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
        .contains("artifact_download_failed"));
    assert!(actual["steps"][0]["message"]
        .as_str()
        .unwrap()
        .contains("network downloads are disabled"));
    assert_eq!(actual["steps"][1]["status"], "blocked");
    assert!(!runtime_root.join("downloads").exists());
}

#[test]
fn phase6p_extract_archive_invalid_failure_blocks_dependent_like_python() {
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
fn phase6p_extract_archive_rejects_traversal_entries_without_writing_outside_temp_root() {
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
fn phase6p_extract_archive_rejects_absolute_entries_before_writing() {
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
fn phase6p_extract_archive_prescans_entries_before_writing_any_member() {
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
fn phase6p_extract_archive_rejects_symlinked_extract_parent_before_writing() {
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
fn phase6p_extract_archive_rejects_symlinked_runtime_root_before_writing() {
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
fn phase6p_copy_replace_deletes_only_inside_fake_device_root() {
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
fn phase6p_copy_sync_preserves_stale_files_like_python_push_sync() {
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
fn phase6p_copy_rejects_destination_traversal_before_writing() {
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
fn phase6p_copy_rejects_fake_device_symlink_escape() {
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
fn phase6p_copy_rejects_symlinked_fake_device_root_before_writing() {
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
