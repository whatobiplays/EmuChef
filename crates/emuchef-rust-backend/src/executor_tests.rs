use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::executor::{DryRunExecutorAdapters, ExecutorRunner};
use crate::model::OrderedMap;
use crate::planner::{
    DeviceContext, ExecutionParamValue, ExecutionPlan, ExecutionPlanSource, ExecutionStep,
    ExecutionStepCondition, ExecutionStepConstraints, RuntimeCapabilities,
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
        artifacts: Vec::new(),
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
