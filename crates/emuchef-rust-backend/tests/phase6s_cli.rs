use std::fs;
use std::path::{Path, PathBuf};

use emuchef_rust_backend::run_with_args_and_input;
use serde_json::{json, Value};
use tempfile::TempDir;

// Phase 6S mirrors Python CLI behavior from `src/emuchef/cli.py` and
// `src/emuchef/io/execution_plan_io.py`. Normal Rust tests keep these
// expectations checked in instead of invoking Python at test runtime.

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn parse_stdout_json(stdout: &str) -> Value {
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "expected one JSON stdout line: {stdout:?}");
    serde_json::from_str(lines[0]).expect("stdout should contain valid JSON")
}

fn fixture_path(relative: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative)
        .to_string_lossy()
        .into_owned()
}

fn minimal_plan_yaml() -> String {
    r#"schema_version: 1
kind: execution_plan
id: plan.test
source:
  device_profile_ref: example.device_profile
  device_plan_ref: example.device_plan
  selected_recipe_refs:
  - example.recipe
  expanded_recipe_refs:
  - example.recipe
device_context:
  manufacturer: Example
  model: Example
  android_version: 13
  android_api_level: 33
  device_tags: []
runtime_capabilities:
  adb_available: true
  apk_install: true
  shared_storage_write: true
  app_launch: true
  shell_command: true
  package_remove_for_user: false
  root_shell: true
  app_data_write: true
inputs: []
artifacts: []
steps:
- id: example.recipe/wait
  recipe_ref: example.recipe
  type: wait
  name: Wait
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  params:
    duration_ms:
      value: 10
  skip_if: []
  verify: []
"#
    .to_string()
}

fn minimal_planning_result_yaml() -> String {
    let indented_plan = minimal_plan_yaml()
        .lines()
        .map(|line| format!("  {line}\n"))
        .collect::<String>();
    format!(
        "schema_version: 1\nkind: planning_result\nstatus: success\nwarnings: []\nerrors: []\nexecution_plan:\n{indented_plan}"
    )
}

fn blocked_plan_yaml() -> String {
    minimal_plan_yaml().replace(
        r#"steps:
- id: example.recipe/wait
  recipe_ref: example.recipe
  type: wait
  name: Wait
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  params:
    duration_ms:
      value: 10
  skip_if: []
  verify: []
"#,
        r#"steps:
- id: example.recipe/fail
  recipe_ref: example.recipe
  type: wait
  name: Fail
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  params:
    duration_ms:
      value: 0
  skip_if: []
  verify: []
- id: example.recipe/downstream
  recipe_ref: example.recipe
  type: wait
  name: Downstream
  dependencies:
  - example.recipe/fail
  constraints:
    capabilities: []
    conflicts_with: []
  params:
    duration_ms:
      value: 1
  skip_if: []
  verify: []
"#,
    )
}

fn permission_plan_yaml() -> String {
    minimal_plan_yaml().replace(
        r#"steps:
- id: example.recipe/wait
  recipe_ref: example.recipe
  type: wait
  name: Wait
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  params:
    duration_ms:
      value: 10
  skip_if: []
  verify: []
"#,
        r#"steps:
- id: example.recipe/grant
  recipe_ref: example.recipe
  type: grant_permissions
  name: Grant
  dependencies: []
  constraints:
    capabilities: []
    conflicts_with: []
  params:
    runtime:
      value:
      - package_name: com.example.app
        name: android.permission.POST_NOTIFICATIONS
        required: false
    appops:
      value:
      - package_name: com.example.app
        op: MANAGE_EXTERNAL_STORAGE
        mode: allow
        required: false
        when:
          rooted: false
  skip_if: []
  verify: []
"#,
    )
}

fn plan_with_input_yaml() -> String {
    minimal_plan_yaml().replace(
        "inputs: []",
        "inputs:\n- id: example.recipe/source\n  value:\n    type: file_path\n    value: /tmp/example\n    location: null",
    )
}

fn write_plan(temp_dir: &TempDir, name: &str, yaml: &str) -> PathBuf {
    let path = temp_dir.path().join(name);
    fs::write(&path, yaml).expect("plan fixture should be writable");
    path
}

#[test]
fn phase6s_preserves_one_shot_json_and_sidecar_dispatch() {
    let one_shot = run_with_args_and_input(&args(&[r#"{"type":"hello"}"#]), "");
    assert_eq!(one_shot.exit_code, 0);
    assert_eq!(one_shot.stderr, "");
    let response = parse_stdout_json(&one_shot.stdout);
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["capabilities"],
        json!([
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
            "openRecipe",
            "createRecipeFromTemplate",
            "getDocument",
            "saveRecipe",
            "saveRecipeAs",
            "closeDocument",
            "applyRecipeCommand",
            "undo",
            "redo",
            "emitYaml",
            "validate",
            "getRefIndex",
            "setDocumentAuthoredRoot",
            "ping"
        ])
    );

    let sidecar = run_with_args_and_input(&args(&["--sidecar"]), r#"{"id":"h","type":"hello"}"#);
    assert_eq!(sidecar.exit_code, 0);
    assert_eq!(sidecar.stderr, "");
    assert_eq!(parse_stdout_json(&sidecar.stdout)["id"], "h");

    let mixed = run_with_args_and_input(&args(&["--sidecar", r#"{"type":"hello"}"#]), "");
    assert_eq!(mixed.exit_code, 2);
    assert_eq!(mixed.stdout, "");
    assert!(mixed
        .stderr
        .contains("usage: emuchef-rust-backend --sidecar"));
}

#[test]
fn phase6s_single_unknown_or_malformed_args_remain_one_shot_errors() {
    for arg in ["foo", "{bad"] {
        let output = run_with_args_and_input(&args(&[arg]), "");
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stderr, "");
        let response = parse_stdout_json(&output.stdout);
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["code"], "invalid_request");
    }
}

#[test]
fn phase6s_validate_without_path_is_cli_usage_error_not_one_shot_json() {
    let output = run_with_args_and_input(&args(&["validate"]), "");

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    assert!(output.stderr.contains("requires an explicit recipe path"));
    assert!(!output.stderr.contains("\"ok\":false"));
}

#[test]
fn phase6s_validate_recipe_path_matches_python_summary_shape() {
    let path = fixture_path("recipes/invalid_top_level_permissions.yaml");
    let output = run_with_args_and_input(&args(&["validate", &path]), "");

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stderr, "");
    assert!(output
        .stdout
        .starts_with("Validation status: error\nValidated paths:\n"));
    assert!(output.stdout.contains(&path));
    assert!(output.stdout.contains("Issues:\n"));
    assert!(output
        .stdout
        .contains("  - validation_context_limited: Cross-file validation was limited because no authored_root was provided."));
    assert!(output
        .stdout
        .contains("  - authored_data_invalid: Recipe top-level 'permissions' is no longer supported; author permissions under grant_permissions.params."));
    assert!(output.stdout.contains("    field: permissions"));
}

#[test]
fn phase6s_validate_missing_file_matches_python_stdout_error_style() {
    let path = fixture_path("recipes/does_not_exist.yaml");
    let output = run_with_args_and_input(&args(&["validate", &path]), "");

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Validation status: error"));
    assert!(output
        .stdout
        .contains(&format!("- {}", Path::new(&path).display())));
    assert!(output.stdout.contains(&format!(
        "  - authored_data_invalid: File {path} was not found."
    )));
}

#[test]
fn phase6s_apply_requires_python_plan_file_flag() {
    let output = run_with_args_and_input(&args(&["apply", "--dry-run"]), "");

    assert_eq!(output.exit_code, 2);
    assert_eq!(output.stdout, "");
    assert!(output.stderr.contains("usage: emuchef apply"));
    assert!(output
        .stderr
        .contains("the following arguments are required: --plan-file"));
}

#[test]
fn phase6s_apply_dry_run_emits_python_progress_and_summary() {
    let temp_dir = TempDir::new().expect("temp dir should be available");
    let plan_path = write_plan(&temp_dir, "plan.yaml", &minimal_plan_yaml());
    let output = run_with_args_and_input(
        &args(&[
            "apply",
            "--plan-file",
            plan_path.to_str().unwrap(),
            "--dry-run",
        ]),
        "",
    );

    assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
    assert_eq!(output.stderr, "");
    assert_eq!(
        output.stdout,
        "[1/1] Wait: checking skip conditions\n\
[1/1] Wait: executing (dry-run)\n\
[1/1] Wait: verifying\n\
[1/1] Wait: succeeded\n\
Dry run: success\n\
- total: 1\n\
- succeeded: 1\n\
- skipped: 0\n\
- blocked: 0\n\
- failed: 0\n\
- not run: 0\n"
    );
}

#[test]
fn phase6s_apply_accepts_python_planning_result_plan_file_wrapper() {
    let temp_dir = TempDir::new().expect("temp dir should be available");
    let plan_path = write_plan(
        &temp_dir,
        "planning_result.yaml",
        &minimal_planning_result_yaml(),
    );
    let output = run_with_args_and_input(
        &args(&[
            "apply",
            "--plan-file",
            plan_path.to_str().unwrap(),
            "--dry-run",
        ]),
        "",
    );

    assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("[1/1] Wait: executing (dry-run)"));
    assert!(output.stdout.contains("Dry run: success"));
}

#[test]
fn phase6s_apply_rejects_broader_plan_files_instead_of_silent_divergence() {
    let temp_dir = TempDir::new().expect("temp dir should be available");
    let plan_path = write_plan(&temp_dir, "with_input.yaml", &plan_with_input_yaml());
    let output = run_with_args_and_input(
        &args(&[
            "apply",
            "--plan-file",
            plan_path.to_str().unwrap(),
            "--dry-run",
        ]),
        "",
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert!(output.stderr.contains("no-input plan fixtures"));
    assert!(!output.stderr.contains("\"ok\":false"));
}

#[test]
fn phase6s_apply_dry_run_reports_blocked_steps_like_python_cli() {
    let temp_dir = TempDir::new().expect("temp dir should be available");
    let plan_path = write_plan(&temp_dir, "blocked.yaml", &blocked_plan_yaml());
    let output = run_with_args_and_input(
        &args(&[
            "apply",
            "--plan-file",
            plan_path.to_str().unwrap(),
            "--dry-run",
        ]),
        "",
    );

    assert_eq!(output.exit_code, 1);
    assert!(output
        .stderr
        .contains("ERROR emuchef.executor.runner: Step failed: example.recipe/fail"));
    assert!(output
        .stderr
        .contains("ValueError: wait step requires a positive integer duration_ms: 0"));
    assert!(output.stdout.contains("[1/2] Fail: failed"));
    assert!(output.stdout.contains("[2/2] Downstream: blocked"));
    assert!(output.stdout.contains("Dry run: failed"));
    assert!(output.stdout.contains("- blocked: 1"));
    assert!(output.stdout.contains("- failed: 1"));
}

#[test]
fn phase6s_apply_dry_run_permission_summary_matches_python_cli_labels() {
    let temp_dir = TempDir::new().expect("temp dir should be available");
    let plan_path = write_plan(&temp_dir, "permissions.yaml", &permission_plan_yaml());
    let output = run_with_args_and_input(
        &args(&[
            "apply",
            "--plan-file",
            plan_path.to_str().unwrap(),
            "--dry-run",
        ]),
        "",
    );

    assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
    assert_eq!(output.stderr, "");
    assert!(output.stdout.contains("Permission actions:\n"));
    assert!(output.stdout.contains("- executed: 1"));
    assert!(output.stdout.contains("- not_applicable: 1"));
    assert!(output.stdout.contains("- failed: 0"));
    assert!(output.stdout.contains(
        "- executed: runtime_permission com.example.app android.permission.POST_NOTIFICATIONS"
    ));
    assert!(output
        .stdout
        .contains("- not_applicable: appop com.example.app MANAGE_EXTERNAL_STORAGE"));
}

#[test]
fn phase6s_apply_missing_plan_file_is_process_error_not_api_envelope() {
    let output = run_with_args_and_input(
        &args(&[
            "apply",
            "--plan-file",
            "/tmp/emuchef_phase6s_missing_plan.yaml",
            "--dry-run",
        ]),
        "",
    );

    assert_eq!(output.exit_code, 1);
    assert_eq!(output.stdout, "");
    assert!(output.stderr.contains("FileNotFoundError"));
    assert!(output
        .stderr
        .contains("/tmp/emuchef_phase6s_missing_plan.yaml"));
    assert!(!output.stderr.contains("\"ok\":false"));
}
