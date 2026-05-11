use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::planner::{plan_execution, DeviceContext, PlannerInput, RuntimeCapabilities};

fn authored_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("authored_root")
        .join(name)
        .join("authored")
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("python_goldens")
        .join(name)
}

fn read_golden(name: &str) -> Value {
    let text = fs::read_to_string(golden_path(name))
        .expect("Python planner parity fixture should be readable");
    serde_json::from_str(&text).expect("Python planner parity fixture should be valid JSON")
}

fn fixture_device_context() -> DeviceContext {
    // Mirrors the Python test fixture profile/device context shape used by the
    // Phase 6M parity fixtures; no probing or profile resolution happens in Rust.
    DeviceContext {
        manufacturer: "Example".to_string(),
        model: "Example".to_string(),
        android_version: 13,
        android_api_level: Some(33),
        device_tags: vec!["example_tag".to_string()],
    }
}

fn fixture_runtime_capabilities() -> RuntimeCapabilities {
    // Mirrors tests/support.py capability_defaults used by Python planner tests.
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

fn planner_input(fixture: &str, selected_recipe_refs: &[&str]) -> PlannerInput {
    PlannerInput::from_authored_root(
        authored_root(fixture),
        selected_recipe_refs
            .iter()
            .map(|item| item.to_string())
            .collect(),
        "plan.example.device_plan.001".to_string(),
        "example.device_plan".to_string(),
        "example.device_profile".to_string(),
        fixture_device_context(),
        fixture_runtime_capabilities(),
    )
    .expect("planner fixture recipes should load")
}

fn planning_result_value(input: PlannerInput) -> Value {
    serde_json::to_value(plan_execution(input)).expect("planning result should serialize")
}

#[test]
fn minimal_plan_matches_python_planning_result_shape() {
    let actual = planning_result_value(planner_input("planner_minimal", &["planner.minimal"]));
    let expected = read_golden("phase6m_planner_minimal.json");

    assert_eq!(actual, expected);
    assert_eq!(
        actual["execution_plan"]["steps"][0]["params"]["duration_ms"],
        json!({"value": 1})
    );
}

#[test]
fn recipe_and_step_dependencies_match_python_ordering() {
    let actual = planning_result_value(planner_input(
        "planner_dependencies",
        &["planner.dependencies"],
    ));
    let expected = read_golden("phase6m_planner_dependencies.json");

    assert_eq!(actual, expected);
    let step_ids = actual["execution_plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        step_ids,
        vec![
            "planner.dependency/dependency_step",
            "planner.dependencies/first",
            "planner.dependencies/second",
        ]
    );
}

#[test]
fn refs_artifacts_defaults_and_conditions_match_python_execution_plan() {
    let actual = planning_result_value(planner_input(
        "planner_refs_artifacts",
        &["planner.refs_artifacts"],
    ));
    let expected = read_golden("phase6m_planner_refs_artifacts.json");

    assert_eq!(actual, expected);
    let steps = actual["execution_plan"]["steps"].as_array().unwrap();
    let resolve = steps
        .iter()
        .find(|step| step["id"] == "planner.refs_artifacts/resolve")
        .unwrap();
    let extract = steps
        .iter()
        .find(|step| step["id"] == "planner.refs_artifacts/extract")
        .unwrap();
    let copy = steps
        .iter()
        .find(|step| step["id"] == "planner.refs_artifacts/copy")
        .unwrap();

    assert_eq!(
        resolve["params"]["artifacts"],
        json!({"value": [
            "planner.refs_artifacts/app_apk",
            "planner.refs_artifacts/core_zip",
            "planner.refs_artifacts/shader_zip",
        ]})
    );
    assert_eq!(
        extract["params"]["artifacts"],
        json!({"value": [
            "planner.refs_artifacts/core_zip",
            "planner.refs_artifacts/shader_zip",
        ]})
    );
    assert_eq!(extract["params"]["extract_on"], json!({"value": "host"}));
    assert_eq!(
        copy["params"]["source"],
        json!({"ref": "steps.planner.refs_artifacts/extract.outputs.extracted_paths"})
    );
    assert_eq!(copy["params"]["copy_policy"], json!({"value": "merge"}));
    assert_eq!(
        copy["verify"][0]["params"]["nested"],
        json!({"ref": "nested.ref.literal"})
    );
}

#[test]
fn stepspec_defaults_do_not_mutate_source_recipe_model() {
    let input = planner_input("planner_refs_artifacts", &["planner.refs_artifacts"]);
    let original_recipes = input.recipes.clone();

    let actual = planning_result_value(input.clone());

    assert_eq!(actual["status"], "success");
    assert_eq!(input.recipes, original_recipes);
}

#[test]
fn required_input_bindings_match_python_success_and_missing_error() {
    let mut bound = planner_input("planner_inputs", &["planner.inputs"]);
    bound.input_bindings.insert(
        "planner.inputs/required_cfg".to_string(),
        json!("/tmp/example.cfg"),
    );
    let actual_bound = planning_result_value(bound);
    assert_eq!(
        actual_bound,
        read_golden("phase6m_planner_inputs_bound.json")
    );
    assert_eq!(
        actual_bound["execution_plan"]["inputs"][0],
        json!({
            "id": "planner.inputs/required_cfg",
            "value": {"type": "file_path", "value": "/tmp/example.cfg", "location": "host"}
        })
    );

    let actual_missing =
        planning_result_value(planner_input("planner_inputs", &["planner.inputs"]));
    let expected_missing = read_golden("phase6m_planner_inputs_missing.json");
    assert_eq!(actual_missing, expected_missing);
    assert_eq!(actual_missing["status"], "error");
    assert_eq!(actual_missing["execution_plan"], Value::Null);
    assert_eq!(
        actual_missing["errors"][0],
        json!({
            "code": "binding_missing",
            "message": "Required binding 'planner.inputs/required_cfg' is missing.",
            "details": {"input_id": "planner.inputs/required_cfg"}
        })
    );
}

#[test]
fn grant_permissions_stays_step_local_without_permission_plan() {
    let actual = planning_result_value(planner_input(
        "planner_grant_permissions",
        &["planner.grant_permissions"],
    ));
    let expected = read_golden("phase6m_planner_grant_permissions.json");

    assert_eq!(actual, expected);
    let plan = actual["execution_plan"].as_object().unwrap();
    assert!(!plan.contains_key("permission_plan"));
    let grant = &actual["execution_plan"]["steps"][0];
    assert_eq!(grant["id"], "planner.grant_permissions/grant");
    assert_eq!(
        grant["params"]["runtime"]["value"][0]["package_name"],
        "com.example.app"
    );
    assert_eq!(
        grant["params"]["policy"]["value"],
        json!({"on_failure": "warn", "require_all": false})
    );
}

#[test]
fn planner_does_not_mutate_authored_fixture_files() {
    let root = authored_root("planner_refs_artifacts");
    let before = snapshot_files(&root);

    let actual = planning_result_value(planner_input(
        "planner_refs_artifacts",
        &["planner.refs_artifacts"],
    ));

    assert_eq!(actual["status"], "success");
    assert_eq!(snapshot_files(&root), before);
}

fn snapshot_files(root: &Path) -> BTreeMap<String, String> {
    let mut snapshot = BTreeMap::new();
    collect_file_snapshot(root, root, &mut snapshot);
    snapshot
}

fn collect_file_snapshot(root: &Path, current: &Path, snapshot: &mut BTreeMap<String, String>) {
    let mut entries = fs::read_dir(current)
        .expect("fixture directory should be readable")
        .map(|entry| entry.expect("fixture entry should be readable").path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_file_snapshot(root, &path, snapshot);
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("fixture path should be under root")
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read_to_string(&path).expect("fixture file should be UTF-8");
        snapshot.insert(relative, contents);
    }
}
