use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should resolve from crate manifest dir")
}

fn repo_authored_root() -> PathBuf {
    repo_root().join("authored")
}

fn run_shadow_with_authored_root(device_plan: &str, extra_args: &[&str]) -> Output {
    let authored_root = repo_authored_root();
    run_shadow_with_authored_root_path(&authored_root, device_plan, extra_args)
}

fn run_shadow_with_authored_root_path(
    authored_root: &Path,
    device_plan: &str,
    extra_args: &[&str],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_emuchef-plan-shadow"));
    command
        .arg("--authored-root")
        .arg(authored_root)
        .arg("--device-plan")
        .arg(device_plan);
    for arg in extra_args {
        command.arg(arg);
    }
    command.output().expect("shadow planner process should run")
}

fn temp_authored_root_with_required_input() -> TempDir {
    let temp = TempDir::new().expect("temp authored root should be created");
    let authored = temp.path().join("authored");
    fs::create_dir_all(authored.join("recipes")).expect("recipes dir should be created");
    fs::create_dir_all(authored.join("device_profiles"))
        .expect("device_profiles dir should be created");
    fs::create_dir_all(authored.join("device_plans")).expect("device_plans dir should be created");
    fs::write(
        authored.join("recipes/main.yaml"),
        r#"schema_version: 1
kind: recipe
id: shadow.required_input
name: Required Input
inputs:
  required_source:
    type: file
    role: generic
    label: Required source
    required: true
    multiple: false
    validation:
      must_exist: true
      allowed_extensions: []
      path_kind: file
    default: null
artifacts: {}
artifact_groups: {}
steps:
  - id: copy
    type: copy_files
    name: Copy
    user_toggleable: false
    dependencies: []
    constraints:
      capabilities: []
      conflicts_with: []
    skip_if: []
    params:
      source:
        ref: inputs.required_source
      destination: /sdcard/Shadow
    verify: []
"#,
    )
    .expect("recipe should be written");
    fs::write(
        authored.join("device_profiles/profile.yaml"),
        r#"schema_version: 1
kind: device_profile
id: shadow.profile
name: Shadow Profile
match:
  manufacturer_contains:
    - Shadow
  android_version:
    min: 13
capability_defaults:
  adb_available: true
  apk_install: true
  shared_storage_write: true
  app_launch: true
  shell_command: true
  package_remove_for_user: false
  root_shell: true
  app_data_write: true
device_tags:
  - shadow
"#,
    )
    .expect("device profile should be written");
    fs::write(
        authored.join("device_plans/plan.yaml"),
        r#"schema_version: 1
kind: device_plan
id: shadow.required_input.plan
name: Shadow Required Input Plan
device_profile_ref: shadow.profile
recipes:
  - recipe_ref: shadow.required_input
    selected_by_default: true
defaults: {}
overrides: {}
"#,
    )
    .expect("device plan should be written");
    temp
}

fn stdout_json(output: &Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8");
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("stdout should be valid JSON: {error}\nstdout:\n{stdout}"))
}

fn execution_step_ids(result: &Value) -> Vec<&str> {
    result["execution_plan"]["steps"]
        .as_array()
        .expect("execution_plan.steps should be an array")
        .iter()
        .map(|step| {
            step["id"]
                .as_str()
                .expect("execution step id should be a string")
        })
        .collect()
}

#[test]
fn shadow_plan_success_for_checked_in_device_plan_emits_pretty_json() {
    let output = run_shadow_with_authored_root("ayaneo.pocket_s_mini.base", &[]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8");
    assert!(
        stdout.contains('\n') && stdout.starts_with("{\n"),
        "stdout should be pretty JSON: {stdout:?}"
    );

    let result = stdout_json(&output);
    assert_eq!(result["status"], "success");
    assert!(result["execution_plan"].is_object());
    assert_eq!(
        result["execution_plan"]["source"]["device_plan_ref"],
        "ayaneo.pocket_s_mini.base"
    );
    assert_eq!(
        result["execution_plan"]["source"]["selected_recipe_refs"],
        json!(["app.retroarch.provision"])
    );
    assert_eq!(
        result["execution_plan"]["source"]["expanded_recipe_refs"],
        json!(["app.retroarch.provision"])
    );
    assert!(!result["execution_plan"]
        .as_object()
        .expect("execution_plan should be an object")
        .contains_key("permission_plan"));
}

#[test]
fn shadow_plan_selected_expanded_refs_and_step_order_are_deterministic() {
    let first = stdout_json(&run_shadow_with_authored_root(
        "ayaneo.pocket_s_mini.base",
        &[],
    ));
    let second = stdout_json(&run_shadow_with_authored_root(
        "ayaneo.pocket_s_mini.base",
        &[],
    ));

    assert_eq!(
        first["execution_plan"]["source"]["selected_recipe_refs"],
        second["execution_plan"]["source"]["selected_recipe_refs"]
    );
    assert_eq!(
        first["execution_plan"]["source"]["expanded_recipe_refs"],
        second["execution_plan"]["source"]["expanded_recipe_refs"]
    );
    assert_eq!(execution_step_ids(&first), execution_step_ids(&second));
}

#[test]
fn shadow_plan_missing_required_binding_emits_error_result_json_to_stdout() {
    let temp = temp_authored_root_with_required_input();
    let output = run_shadow_with_authored_root_path(
        &temp.path().join("authored"),
        "shadow.required_input.plan",
        &[],
    );

    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let result = stdout_json(&output);
    assert_eq!(result["status"], "error");
    assert_eq!(result["execution_plan"], Value::Null);
    assert!(result["errors"]
        .as_array()
        .expect("errors should be an array")
        .iter()
        .any(|error| {
            error["code"] == "binding_missing"
                && error["details"]["input_id"] == "shadow.required_input/required_source"
        }));
}

#[test]
fn shadow_plan_repeated_bind_values_follow_python_grouping_behavior() {
    let output = run_shadow_with_authored_root(
        "ayaneo.pocket_s_mini.base",
        &[
            "--bind",
            "app.retroarch.provision/retroarch_cfg=/tmp/one.cfg",
            "--bind",
            "app.retroarch.provision/retroarch_cfg=/tmp/two.cfg",
        ],
    );

    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let result = stdout_json(&output);
    assert_eq!(result["status"], "error");
    assert!(result["errors"]
        .as_array()
        .expect("errors should be an array")
        .iter()
        .any(|error| {
            error["code"] == "binding_validation_failed"
                && error["details"]["input_id"] == "app.retroarch.provision/retroarch_cfg"
        }));
}

#[test]
fn shadow_plan_usage_errors_write_stable_stderr_without_stdout() {
    let output =
        run_shadow_with_authored_root("ayaneo.pocket_s_mini.base", &["--bind", "not-a-binding"]);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("Invalid --bind value"));
    assert!(stderr.contains("<recipe_ref>/<input_id>=<value>"));
}
