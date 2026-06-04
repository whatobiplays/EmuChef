use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::model::{
    OrderedMap, ParamValue, Recipe, RecipeProvides, RemoteFileArtifact, Step, StepCondition,
    StepConstraints,
};
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

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("python_goldens")
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

fn normalized_planning_result_value(input: PlannerInput) -> Value {
    normalize_paths(planning_result_value(input))
}

fn normalize_paths(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(normalize_paths).collect()),
        Value::Object(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, normalize_paths(value)))
                .collect(),
        ),
        Value::String(value) => Value::String(normalize_path_string(value)),
        other => other,
    }
}

fn normalize_path_string(value: String) -> String {
    for marker in ["/relative/", "/dependency-a.cfg", "/main.cfg"] {
        if let Some(index) = value.find(marker) {
            return format!("$REPO_ROOT{}", &value[index..]);
        }
    }
    value
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
fn phase6n_builtin_planner_mappings_match_python_goldens() {
    let actual = normalized_planning_result_value(planner_input(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    ));
    let expected = read_golden("phase6n_planner_builtins_all.json");

    assert_eq!(actual, expected);
    let steps = actual["execution_plan"]["steps"].as_array().unwrap();
    let step_ids = steps
        .iter()
        .map(|step| step["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        step_ids,
        vec![
            "planner.phase6n.builtins_all/resolve",
            "planner.phase6n.builtins_all/extract_artifacts",
            "planner.phase6n.builtins_all/extract_archive",
            "planner.phase6n.builtins_all/install",
            "planner.phase6n.builtins_all/copy",
            "planner.phase6n.builtins_all/grant",
            "planner.phase6n.builtins_all/launch",
            "planner.phase6n.builtins_all/wait",
            "planner.phase6n.builtins_all/force_stop",
        ]
    );
    let install = steps
        .iter()
        .find(|step| step["id"] == "planner.phase6n.builtins_all/install")
        .unwrap();
    assert_eq!(
        install["params"]["replace_existing"],
        json!({"value": false})
    );
    let archive = steps
        .iter()
        .find(|step| step["id"] == "planner.phase6n.builtins_all/extract_archive")
        .unwrap();
    assert_eq!(archive["params"]["cleanup"], json!({"value": true}));
}

#[test]
fn phase6n_optional_inputs_prune_and_rebind_like_python() {
    let omitted = normalized_planning_result_value(planner_input(
        "planner_phase6n_optional_inputs",
        &["planner.phase6n.optional_inputs"],
    ));
    assert_eq!(
        omitted,
        read_golden("phase6n_planner_optional_inputs_omitted.json")
    );
    assert_eq!(omitted["execution_plan"]["inputs"], json!([]));
    assert_eq!(
        omitted["execution_plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["planner.phase6n.optional_inputs/prepare"]
    );

    let mut bound = planner_input(
        "planner_phase6n_optional_inputs",
        &["planner.phase6n.optional_inputs"],
    );
    bound.input_bindings.insert(
        "planner.phase6n.optional_inputs/optional_cfg".to_string(),
        json!("relative/optional.cfg"),
    );
    let actual_bound = normalized_planning_result_value(bound);
    assert_eq!(
        actual_bound,
        read_golden("phase6n_planner_optional_inputs_bound.json")
    );
    assert_eq!(
        actual_bound["execution_plan"]["inputs"][0]["value"],
        json!({"type": "file_path", "value": "$REPO_ROOT/relative/optional.cfg", "location": "host"})
    );
}

#[test]
fn phase6n_input_defaults_and_multiple_values_match_python() {
    let actual = normalized_planning_result_value(planner_input(
        "planner_phase6n_input_defaults_multiple",
        &["planner.phase6n.input_defaults_multiple"],
    ));
    let expected = read_golden("phase6n_planner_input_defaults_multiple.json");

    assert_eq!(actual, expected);
    assert_eq!(
        actual["execution_plan"]["inputs"],
        json!([
            {
                "id": "planner.phase6n.input_defaults_multiple/default_cfg",
                "value": {"type": "file_path", "value": "$REPO_ROOT/relative/default.cfg", "location": "host"}
            },
            {
                "id": "planner.phase6n.input_defaults_multiple/default_dir",
                "value": {"type": "directory_path", "value": "$REPO_ROOT/relative/data", "location": "host"}
            },
            {
                "id": "planner.phase6n.input_defaults_multiple/multi_cfgs",
                "value": {
                    "type": "path_list",
                    "value": ["$REPO_ROOT/relative/a.cfg", "$REPO_ROOT/relative/b.cfg"],
                    "location": "host"
                }
            }
        ])
    );
}

#[test]
fn phase6n_dependency_expansion_and_namespacing_match_python() {
    let actual = normalized_planning_result_value(planner_input(
        "planner_phase6n_dependency_graph",
        &["planner.phase6n.dependency_graph"],
    ));
    let expected = read_golden("phase6n_planner_dependency_graph.json");

    assert_eq!(actual, expected);
    assert_eq!(
        actual["execution_plan"]["source"]["expanded_recipe_refs"],
        json!([
            "planner.phase6n.dep_a",
            "planner.phase6n.dep_b",
            "planner.phase6n.dependency_graph"
        ])
    );
    assert_eq!(
        actual["execution_plan"]["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|artifact| artifact["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "planner.phase6n.dep_a/shared_asset",
            "planner.phase6n.dep_b/shared_asset",
            "planner.phase6n.dependency_graph/shared_asset",
        ]
    );
}

#[test]
fn phase6n_step_data_refs_conditions_and_constraints_match_python() {
    let actual = normalized_planning_result_value(planner_input(
        "planner_phase6n_step_data",
        &["planner.phase6n.step_data"],
    ));
    let expected = read_golden("phase6n_planner_step_data.json");

    assert_eq!(actual, expected);
    let copy = actual["execution_plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == "planner.phase6n.step_data/copy")
        .unwrap()
        .clone();
    assert_eq!(
        copy["constraints"]["conflicts_with"],
        json!(["planner.phase6n.step_data/cleanup"])
    );
    assert_eq!(
        copy["params"]["source"],
        json!({"ref": "steps.planner.phase6n.step_data/extract.outputs.extracted_paths"})
    );
    assert_eq!(
        copy["skip_if"][0]["params"]["nested"],
        json!({"ref": "nested.skip.literal"})
    );
    assert_eq!(
        copy["verify"][0]["params"]["nested"],
        json!({"ref": "nested.verify.literal"})
    );
}

#[test]
fn artifact_selection_explicit_and_groups_preserve_normalized_order() {
    let actual = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        Some(vec!["archive_zip"]),
        Some(vec!["install_group", "extras_group"]),
        vec![
            ("app_apk", "https://example.com/app.apk"),
            ("archive_zip", "https://example.com/archive.zip"),
            ("shader_zip", "https://example.com/shader.zip"),
            ("core_zip", "https://example.com/core.zip"),
        ],
        vec![
            ("install_group", vec!["app_apk"]),
            ("extras_group", vec!["shader_zip", "core_zip"]),
        ],
    ));

    assert_normalized_artifacts(
        &actual,
        "resolve",
        &[
            "planner.artifact_selection/archive_zip",
            "planner.artifact_selection/app_apk",
            "planner.artifact_selection/shader_zip",
            "planner.artifact_selection/core_zip",
        ],
    );
}

#[test]
fn artifact_selection_explicit_only_and_group_only_are_supported() {
    let explicit_only = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        Some(vec!["app_apk", "archive_zip"]),
        None,
        artifact_selection_artifacts(),
        vec![],
    ));
    assert_normalized_artifacts(
        &explicit_only,
        "resolve",
        &[
            "planner.artifact_selection/app_apk",
            "planner.artifact_selection/archive_zip",
        ],
    );

    let group_only = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        None,
        Some(vec!["install_group"]),
        artifact_selection_artifacts(),
        vec![("install_group", vec!["app_apk", "archive_zip"])],
    ));
    assert_normalized_artifacts(
        &group_only,
        "resolve",
        &[
            "planner.artifact_selection/app_apk",
            "planner.artifact_selection/archive_zip",
        ],
    );
}

#[test]
fn artifact_selection_extract_artifacts_success_and_invalid_selection_are_validated() {
    let success = planning_result_value(artifact_selection_input(
        "extract_artifacts",
        None,
        Some(vec!["install_group"]),
        artifact_selection_artifacts(),
        vec![("install_group", vec!["archive_zip"])],
    ));
    assert_normalized_artifacts(
        &success,
        "extract",
        &["planner.artifact_selection/archive_zip"],
    );
    assert_eq!(
        artifact_selection_step(&success, "extract")["params"]["extract_on"],
        json!({"value": "host"})
    );

    let invalid = planning_result_value(artifact_selection_input(
        "extract_artifacts",
        Some(vec!["missing_zip"]),
        None,
        artifact_selection_artifacts(),
        vec![],
    ));
    assert_planner_error(
        &invalid,
        "unknown_artifact_ref",
        "extract",
        "artifacts",
        "missing_zip",
    );
}

#[test]
fn artifact_selection_reports_unknown_explicit_artifact_group_and_group_member() {
    let unknown_artifact = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        Some(vec!["missing_zip"]),
        None,
        artifact_selection_artifacts(),
        vec![],
    ));
    assert_planner_error(
        &unknown_artifact,
        "unknown_artifact_ref",
        "resolve",
        "artifacts",
        "missing_zip",
    );

    let unknown_group = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        None,
        Some(vec!["missing_group"]),
        artifact_selection_artifacts(),
        vec![],
    ));
    assert_planner_error(
        &unknown_group,
        "unknown_artifact_group_ref",
        "resolve",
        "artifact_groups",
        "missing_group",
    );

    let unknown_group_member = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        None,
        Some(vec!["broken_group"]),
        artifact_selection_artifacts(),
        vec![("broken_group", vec!["missing_member"])],
    ));
    assert_planner_error(
        &unknown_group_member,
        "unknown_artifact_ref",
        "resolve",
        "artifact_groups",
        "missing_member",
    );
}

#[test]
fn artifact_selection_reports_duplicate_explicit_group_and_mixed_expansion() {
    let duplicate_explicit = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        Some(vec!["app_apk", "app_apk"]),
        None,
        artifact_selection_artifacts(),
        vec![],
    ));
    assert_planner_error(
        &duplicate_explicit,
        "duplicate_artifact_selection",
        "resolve",
        "artifacts",
        "app_apk",
    );

    let duplicate_groups = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        None,
        Some(vec!["install_group", "extras_group"]),
        artifact_selection_artifacts(),
        vec![
            ("install_group", vec!["app_apk"]),
            ("extras_group", vec!["app_apk"]),
        ],
    ));
    assert_planner_error(
        &duplicate_groups,
        "duplicate_artifact_selection",
        "resolve",
        "artifact_groups",
        "app_apk",
    );

    let duplicate_mixed = planning_result_value(artifact_selection_input(
        "resolve_artifacts",
        Some(vec!["archive_zip"]),
        Some(vec!["install_group"]),
        artifact_selection_artifacts(),
        vec![("install_group", vec!["archive_zip"])],
    ));
    assert_planner_error(
        &duplicate_mixed,
        "duplicate_artifact_selection",
        "resolve",
        "artifact_groups",
        "archive_zip",
    );
}

#[test]
fn step_ref_validation_preserves_explicit_refs_and_rewrites_shorthand_refs() {
    let explicit = planning_result_value(ref_validation_input(vec![
        ref_validation_step("extract", "extract_artifacts", vec![], OrderedMap::new()),
        ref_validation_step(
            "copy",
            "copy_files",
            vec!["extract"],
            ref_params(vec![(
                "source",
                ParamValue::Ref("steps.extract.outputs.extracted_paths".to_string()),
            )]),
        ),
    ]));
    assert_eq!(explicit["status"], "success", "{explicit:#}");
    assert_eq!(
        *execution_step_param(&explicit, "copy", "source"),
        json!({"ref": "steps.planner.ref_validation/extract.outputs.extracted_paths"})
    );

    let shorthand = planning_result_value(ref_validation_input(vec![
        ref_validation_step(
            "copy",
            "copy_files",
            vec![],
            ref_params(vec![(
                "source",
                ParamValue::Ref("steps.extract".to_string()),
            )]),
        ),
        ref_validation_step("extract", "extract_artifacts", vec![], OrderedMap::new()),
    ]));
    assert_eq!(shorthand["status"], "success", "{shorthand:#}");
    assert_eq!(
        *execution_step_param(&shorthand, "copy", "source"),
        json!({"ref": "steps.planner.ref_validation/extract.outputs.extracted_paths"})
    );
}

#[test]
fn step_ref_validation_reports_invalid_targets_outputs_and_formats() {
    let unknown_step = planning_result_value(ref_validation_input(vec![ref_validation_step(
        "copy",
        "copy_files",
        vec![],
        ref_params(vec![(
            "source",
            ParamValue::Ref("steps.missing".to_string()),
        )]),
    )]));
    assert_step_ref_error(
        &unknown_step,
        "unknown_step_ref",
        "copy",
        "source",
        "steps.missing",
    );

    let unknown_output = planning_result_value(ref_validation_input(vec![
        ref_validation_step("extract", "extract_artifacts", vec![], OrderedMap::new()),
        ref_validation_step(
            "copy",
            "copy_files",
            vec![],
            ref_params(vec![(
                "source",
                ParamValue::Ref("steps.extract.outputs.missing".to_string()),
            )]),
        ),
    ]));
    assert_step_ref_error(
        &unknown_output,
        "unknown_step_output",
        "copy",
        "source",
        "steps.extract.outputs.missing",
    );

    let no_primary_output = planning_result_value(ref_validation_input(vec![
        ref_validation_step(
            "install",
            "install_apk",
            vec![],
            ref_params(vec![(
                "app",
                ParamValue::Ref("artifacts.app_apk.local_path".to_string()),
            )]),
        ),
        ref_validation_step(
            "copy",
            "copy_files",
            vec![],
            ref_params(vec![(
                "source",
                ParamValue::Ref("steps.install".to_string()),
            )]),
        ),
    ]));
    assert_step_ref_error(
        &no_primary_output,
        "step_ref_has_no_primary_output",
        "copy",
        "source",
        "steps.install",
    );

    let malformed = planning_result_value(ref_validation_input(vec![ref_validation_step(
        "copy",
        "copy_files",
        vec![],
        ref_params(vec![("source", ParamValue::Ref("steps.".to_string()))]),
    )]));
    assert_step_ref_error(&malformed, "invalid_ref_format", "copy", "source", "steps.");
}

#[test]
fn step_ref_validation_uses_emitted_selected_step_universe() {
    let actual = planning_result_value(ref_validation_input(vec![
        ref_validation_step(
            "unavailable_extract",
            "extract_artifacts",
            vec![],
            OrderedMap::new(),
        ),
        ref_validation_step(
            "copy",
            "copy_files",
            vec![],
            ref_params(vec![(
                "source",
                ParamValue::Ref("steps.unavailable_extract".to_string()),
            )]),
        ),
    ]));

    assert_step_ref_error(
        &actual,
        "unknown_step_ref",
        "copy",
        "source",
        "steps.unavailable_extract",
    );
}

#[test]
fn dependency_validation_orders_valid_forward_and_duplicate_dependencies() {
    let forward = planning_result_value(dependency_validation_input(vec![
        dependency_step("consumer", vec!["producer"]),
        dependency_step("producer", vec![]),
        dependency_step("independent", vec![]),
    ]));
    assert_eq!(forward["status"], "success", "{forward:#}");
    assert_eq!(
        dependency_step_ids(&forward),
        vec![
            "planner.dependency_validation/producer",
            "planner.dependency_validation/independent",
            "planner.dependency_validation/consumer",
        ]
    );

    let duplicate = planning_result_value(dependency_validation_input(vec![
        dependency_step("seed", vec![]),
        dependency_step("consumer", vec!["seed", "seed"]),
    ]));
    assert_eq!(duplicate["status"], "success", "{duplicate:#}");
    assert_eq!(
        *dependency_step_dependencies(&duplicate, "consumer"),
        json!([
            "planner.dependency_validation/seed",
            "planner.dependency_validation/seed"
        ])
    );
}

#[test]
fn dependency_validation_reports_unknown_self_and_non_emitted_dependencies() {
    let unknown = planning_result_value(dependency_validation_input(vec![dependency_step(
        "consumer",
        vec!["missing"],
    )]));
    assert_dependency_error(
        &unknown,
        "unknown_step_dependency",
        "consumer",
        Some("missing"),
    );

    let self_dependency =
        planning_result_value(dependency_validation_input(vec![dependency_step(
            "selfish",
            vec!["selfish"],
        )]));
    assert_dependency_error(
        &self_dependency,
        "self_step_dependency",
        "selfish",
        Some("selfish"),
    );

    let unavailable_target = planning_result_value(dependency_validation_input(vec![
        unavailable_dependency_step("unavailable"),
        dependency_step("consumer", vec!["unavailable"]),
    ]));
    assert_dependency_error(
        &unavailable_target,
        "unknown_step_dependency",
        "consumer",
        Some("unavailable"),
    );
}

#[test]
fn dependency_validation_reports_cycles_deterministically_without_cascading_unknowns() {
    let simple_cycle = planning_result_value(dependency_validation_input(vec![
        dependency_step("a", vec!["b"]),
        dependency_step("b", vec!["a"]),
    ]));
    assert_dependency_cycle(&simple_cycle, &["a", "b", "a"]);

    let multi_step_cycle = planning_result_value(dependency_validation_input(vec![
        dependency_step("a", vec!["b"]),
        dependency_step("b", vec!["c"]),
        dependency_step("c", vec!["a"]),
    ]));
    assert_dependency_cycle(&multi_step_cycle, &["a", "b", "c", "a"]);

    let unknown = planning_result_value(dependency_validation_input(vec![dependency_step(
        "consumer",
        vec!["missing"],
    )]));
    let errors = unknown["errors"].as_array().unwrap();
    assert!(
        !errors
            .iter()
            .any(|error| error["code"] == "dependency_cycle"),
        "unknown dependency should not cascade into cycle errors: {errors:#?}"
    );
}

#[test]
fn dependency_validation_ignores_invalid_dependencies_on_unavailable_steps() {
    let actual = planning_result_value(dependency_validation_input(vec![
        unavailable_dependency_step("unavailable"),
        dependency_step("available", vec![]),
    ]));

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(
        dependency_step_ids(&actual),
        vec!["planner.dependency_validation/available"]
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

#[test]
fn planner_parity_fixture_inventory_consumes_checked_in_evidence_only() {
    let inventory = planner_parity_fixture_inventory();
    let inventory_goldens = inventory
        .iter()
        .map(|entry| entry.golden.to_string())
        .collect::<BTreeSet<_>>();
    let discovered_goldens = discovered_planner_golden_names();

    assert_eq!(
        discovered_goldens, inventory_goldens,
        "Planner parity goldens must be intentionally classified. Add new phase6m/phase6n planner goldens to the P7A inventory, or explain why they are outside P7A planner parity scope.",
    );

    for entry in inventory {
        assert!(
            authored_root(entry.fixture).is_dir(),
            "planner parity authored fixture should exist: {}",
            entry.fixture
        );
        let parsed = read_golden(entry.golden);
        assert_planner_golden_shape(entry.golden, &parsed);
    }
}

fn artifact_selection_input(
    step_type: &str,
    artifacts: Option<Vec<&str>>,
    artifact_groups: Option<Vec<&str>>,
    artifact_defs: Vec<(&str, &str)>,
    group_defs: Vec<(&str, Vec<&str>)>,
) -> PlannerInput {
    let mut params = OrderedMap::new();
    if let Some(artifacts) = artifacts {
        params.insert(
            "artifacts".to_string(),
            ParamValue::Literal(json!(artifacts)),
        );
    }
    if let Some(artifact_groups) = artifact_groups {
        params.insert(
            "artifact_groups".to_string(),
            ParamValue::Literal(json!(artifact_groups)),
        );
    }

    PlannerInput {
        recipes: vec![Recipe {
            schema_version: 1,
            kind: "recipe".to_string(),
            id: "planner.artifact_selection".to_string(),
            name: "Planner Artifact Selection".to_string(),
            description: None,
            recipe_dependencies: Vec::new(),
            provides: RecipeProvides {
                features: Vec::new(),
            },
            inputs: OrderedMap::new(),
            artifacts: artifact_defs
                .into_iter()
                .map(|(id, url)| {
                    (
                        id.to_string(),
                        RemoteFileArtifact {
                            type_name: "remote_file".to_string(),
                            url: url.to_string(),
                            cache: "default".to_string(),
                        },
                    )
                })
                .collect(),
            artifact_groups: group_defs
                .into_iter()
                .map(|(id, members)| {
                    (
                        id.to_string(),
                        members.into_iter().map(ToString::to_string).collect(),
                    )
                })
                .collect(),
            steps: vec![Step {
                id: if step_type == "extract_artifacts" {
                    "extract".to_string()
                } else {
                    "resolve".to_string()
                },
                type_name: step_type.to_string(),
                name: "Artifact Selection".to_string(),
                description: None,
                user_toggleable: false,
                dependencies: Vec::new(),
                constraints: StepConstraints {
                    capabilities: Vec::new(),
                    conflicts_with: Vec::new(),
                },
                skip_if: Vec::<StepCondition>::new(),
                params,
                verify: Vec::<StepCondition>::new(),
            }],
        }],
        selected_recipe_refs: vec!["planner.artifact_selection".to_string()],
        input_bindings: OrderedMap::new(),
        plan_id: "plan.artifact_selection.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
    }
}

fn artifact_selection_artifacts() -> Vec<(&'static str, &'static str)> {
    vec![
        ("app_apk", "https://example.com/app.apk"),
        ("archive_zip", "https://example.com/archive.zip"),
    ]
}

fn ref_validation_input(steps: Vec<Step>) -> PlannerInput {
    PlannerInput {
        recipes: vec![Recipe {
            schema_version: 1,
            kind: "recipe".to_string(),
            id: "planner.ref_validation".to_string(),
            name: "Planner Ref Validation".to_string(),
            description: None,
            recipe_dependencies: Vec::new(),
            provides: RecipeProvides {
                features: Vec::new(),
            },
            inputs: OrderedMap::new(),
            artifacts: vec![(
                "app_apk".to_string(),
                RemoteFileArtifact {
                    type_name: "remote_file".to_string(),
                    url: "https://example.com/app.apk".to_string(),
                    cache: "default".to_string(),
                },
            )]
            .into_iter()
            .collect(),
            artifact_groups: OrderedMap::new(),
            steps,
        }],
        selected_recipe_refs: vec!["planner.ref_validation".to_string()],
        input_bindings: OrderedMap::new(),
        plan_id: "plan.ref_validation.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
    }
}

fn ref_validation_step(
    id: &str,
    step_type: &str,
    dependencies: Vec<&str>,
    params: OrderedMap<ParamValue>,
) -> Step {
    Step {
        id: id.to_string(),
        type_name: step_type.to_string(),
        name: id.to_string(),
        description: None,
        user_toggleable: false,
        dependencies: dependencies.into_iter().map(ToString::to_string).collect(),
        constraints: StepConstraints {
            capabilities: if id == "unavailable_extract" {
                vec!["unavailable_capability".to_string()]
            } else {
                Vec::new()
            },
            conflicts_with: Vec::new(),
        },
        skip_if: Vec::<StepCondition>::new(),
        params,
        verify: Vec::<StepCondition>::new(),
    }
}

fn ref_params(entries: Vec<(&str, ParamValue)>) -> OrderedMap<ParamValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn execution_step_param<'a>(actual: &'a Value, step_id: &str, param: &str) -> &'a Value {
    let execution_step_id = format!("planner.ref_validation/{step_id}");
    &actual["execution_plan"]["steps"]
        .as_array()
        .expect("execution plan should include steps")
        .iter()
        .find(|step| step["id"] == execution_step_id)
        .expect("ref validation step should exist")["params"][param]
}

fn dependency_validation_input(steps: Vec<Step>) -> PlannerInput {
    PlannerInput {
        recipes: vec![Recipe {
            schema_version: 1,
            kind: "recipe".to_string(),
            id: "planner.dependency_validation".to_string(),
            name: "Planner Dependency Validation".to_string(),
            description: None,
            recipe_dependencies: Vec::new(),
            provides: RecipeProvides {
                features: Vec::new(),
            },
            inputs: OrderedMap::new(),
            artifacts: OrderedMap::new(),
            artifact_groups: OrderedMap::new(),
            steps,
        }],
        selected_recipe_refs: vec!["planner.dependency_validation".to_string()],
        input_bindings: OrderedMap::new(),
        plan_id: "plan.dependency_validation.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
    }
}

fn dependency_step(id: &str, dependencies: Vec<&str>) -> Step {
    dependency_step_with_capabilities(id, dependencies, Vec::new())
}

fn unavailable_dependency_step(id: &str) -> Step {
    dependency_step_with_capabilities(id, vec!["missing"], vec!["unavailable_capability"])
}

fn dependency_step_with_capabilities(
    id: &str,
    dependencies: Vec<&str>,
    capabilities: Vec<&str>,
) -> Step {
    let mut params = OrderedMap::new();
    params.insert("duration_ms".to_string(), ParamValue::Literal(json!(1)));
    Step {
        id: id.to_string(),
        type_name: "wait".to_string(),
        name: id.to_string(),
        description: None,
        user_toggleable: false,
        dependencies: dependencies.into_iter().map(ToString::to_string).collect(),
        constraints: StepConstraints {
            capabilities: capabilities.into_iter().map(ToString::to_string).collect(),
            conflicts_with: Vec::new(),
        },
        skip_if: Vec::<StepCondition>::new(),
        params,
        verify: Vec::<StepCondition>::new(),
    }
}

fn dependency_step_ids(actual: &Value) -> Vec<&str> {
    actual["execution_plan"]["steps"]
        .as_array()
        .expect("execution plan should include steps")
        .iter()
        .map(|step| step["id"].as_str().unwrap())
        .collect()
}

fn dependency_step_dependencies<'a>(actual: &'a Value, step_id: &str) -> &'a Value {
    let execution_step_id = format!("planner.dependency_validation/{step_id}");
    &actual["execution_plan"]["steps"]
        .as_array()
        .expect("execution plan should include steps")
        .iter()
        .find(|step| step["id"] == execution_step_id)
        .expect("dependency validation step should exist")["dependencies"]
}

fn assert_normalized_artifacts(actual: &Value, step_id: &str, expected: &[&str]) {
    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(actual["errors"], json!([]));
    assert_eq!(
        artifact_selection_step(actual, step_id)["params"]["artifacts"],
        json!({"value": expected})
    );
}

fn artifact_selection_step<'a>(actual: &'a Value, step_id: &str) -> &'a Value {
    let execution_step_id = format!("planner.artifact_selection/{step_id}");
    actual["execution_plan"]["steps"]
        .as_array()
        .expect("execution plan should include steps")
        .iter()
        .find(|step| step["id"] == execution_step_id)
        .expect("artifact selection step should exist")
}

fn assert_step_ref_error(actual: &Value, code: &str, step_id: &str, param: &str, ref_value: &str) {
    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null);
    let errors = actual["errors"]
        .as_array()
        .expect("planner result should include errors");
    assert!(
        errors.iter().any(|error| {
            error["code"] == code
                && error["details"]["recipe_ref"] == "planner.ref_validation"
                && error["details"]["step_id"] == step_id
                && error["details"]["param"] == param
                && error["details"]["ref"] == ref_value
                && error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(ref_value))
        }),
        "expected step ref error code={code} step_id={step_id} param={param} ref={ref_value}; actual errors: {errors:#?}",
    );
}

fn assert_dependency_error(actual: &Value, code: &str, step_id: &str, dependency: Option<&str>) {
    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null);
    let errors = actual["errors"]
        .as_array()
        .expect("planner result should include errors");
    assert!(
        errors.iter().any(|error| {
            error["code"] == code
                && error["details"]["recipe_ref"] == "planner.dependency_validation"
                && error["details"]["step_id"] == step_id
                && dependency.is_none_or(|dependency| {
                    error["details"]["dependency"] == dependency
                        && error["message"]
                            .as_str()
                            .is_some_and(|message| message.contains(dependency))
                })
        }),
        "expected dependency error code={code} step_id={step_id} dependency={dependency:?}; actual errors: {errors:#?}",
    );
}

fn assert_dependency_cycle(actual: &Value, expected_cycle: &[&str]) {
    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null);
    let errors = actual["errors"]
        .as_array()
        .expect("planner result should include errors");
    assert!(
        errors.iter().any(|error| {
            error["code"] == "dependency_cycle"
                && error["details"]["recipe_ref"] == "planner.dependency_validation"
                && error["details"]["cycle"] == json!(expected_cycle)
        }),
        "expected dependency cycle {expected_cycle:?}; actual errors: {errors:#?}",
    );
}

fn assert_planner_error(
    actual: &Value,
    code: &str,
    step_id: &str,
    param: &str,
    offending_id: &str,
) {
    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null);
    let errors = actual["errors"]
        .as_array()
        .expect("planner result should include errors");
    assert!(
        errors.iter().any(|error| {
            error["code"] == code
                && error["details"]["step_id"] == step_id
                && error["details"]["param"] == param
                && (error["details"]["artifact_id"] == offending_id
                    || error["details"]["group_id"] == offending_id
                    || error["details"]["duplicate_artifact_id"] == offending_id)
                && error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(offending_id))
        }),
        "expected planner error code={code} step_id={step_id} param={param} offending_id={offending_id}; actual errors: {errors:#?}",
    );
}

struct PlannerParityFixtureEntry {
    fixture: &'static str,
    golden: &'static str,
}

fn planner_parity_fixture_inventory() -> &'static [PlannerParityFixtureEntry] {
    &[
        PlannerParityFixtureEntry {
            fixture: "planner_minimal",
            golden: "phase6m_planner_minimal.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_dependencies",
            golden: "phase6m_planner_dependencies.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_refs_artifacts",
            golden: "phase6m_planner_refs_artifacts.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_inputs",
            golden: "phase6m_planner_inputs_bound.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_inputs",
            golden: "phase6m_planner_inputs_missing.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_grant_permissions",
            golden: "phase6m_planner_grant_permissions.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_phase6n_builtins_all",
            golden: "phase6n_planner_builtins_all.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_phase6n_optional_inputs",
            golden: "phase6n_planner_optional_inputs_omitted.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_phase6n_optional_inputs",
            golden: "phase6n_planner_optional_inputs_bound.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_phase6n_input_defaults_multiple",
            golden: "phase6n_planner_input_defaults_multiple.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_phase6n_dependency_graph",
            golden: "phase6n_planner_dependency_graph.json",
        },
        PlannerParityFixtureEntry {
            fixture: "planner_phase6n_step_data",
            golden: "phase6n_planner_step_data.json",
        },
    ]
}

fn discovered_planner_golden_names() -> BTreeSet<String> {
    fs::read_dir(golden_dir())
        .expect("planner golden directory should be readable")
        .map(|entry| {
            entry
                .expect("planner golden directory entry should be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| {
            name.ends_with(".json")
                && (name.starts_with("phase6m_planner_") || name.starts_with("phase6n_planner_"))
        })
        .collect()
}

fn assert_planner_golden_shape(name: &str, parsed: &Value) {
    assert_eq!(
        parsed["schema_version"], 1,
        "{name} should use schema version 1"
    );
    assert_eq!(
        parsed["kind"], "planning_result",
        "{name} should be a planning result"
    );
    assert!(
        matches!(
            parsed["status"].as_str(),
            Some("success" | "warning" | "error")
        ),
        "{name} should have a valid planning status"
    );
    assert!(
        parsed["warnings"].as_array().is_some(),
        "{name} should have a warnings array"
    );
    let errors = parsed["errors"]
        .as_array()
        .expect("planner golden should have an errors array");

    match &parsed["execution_plan"] {
        Value::Object(plan) => assert_execution_plan_shape(name, plan),
        Value::Null => assert!(
            !errors.is_empty(),
            "{name} should include errors when execution_plan is null"
        ),
        _ => panic!("{name} should have an execution_plan object or null"),
    }
}

fn assert_execution_plan_shape(name: &str, plan: &serde_json::Map<String, Value>) {
    assert_non_empty_string(name, plan.get("id"), "execution_plan.id");
    assert_eq!(
        plan.get("schema_version"),
        Some(&json!(1)),
        "{name} execution_plan should use schema version 1"
    );
    assert_eq!(
        plan.get("kind"),
        Some(&json!("execution_plan")),
        "{name} execution_plan should have kind execution_plan"
    );
    let source = plan
        .get("source")
        .and_then(Value::as_object)
        .expect("planner execution_plan should include source object");
    assert_non_empty_string(
        name,
        source.get("device_profile_ref"),
        "execution_plan.source.device_profile_ref",
    );
    assert_non_empty_string(
        name,
        source.get("device_plan_ref"),
        "execution_plan.source.device_plan_ref",
    );
    assert!(
        source
            .get("selected_recipe_refs")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()),
        "{name} should include selected recipe refs"
    );
    assert!(
        source
            .get("expanded_recipe_refs")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()),
        "{name} should include expanded recipe refs"
    );

    let steps = plan
        .get("steps")
        .and_then(Value::as_array)
        .expect("planner execution_plan should include steps array");
    assert!(!steps.is_empty(), "{name} should include at least one step");
    for (index, step) in steps.iter().enumerate() {
        let step = step
            .as_object()
            .expect("planner execution_plan step should be an object");
        assert_non_empty_string(name, step.get("id"), &format!("steps[{index}].id"));
        assert_non_empty_string(
            name,
            step.get("recipe_ref"),
            &format!("steps[{index}].recipe_ref"),
        );
        assert_non_empty_string(name, step.get("type"), &format!("steps[{index}].type"));
        assert!(
            step.get("dependencies").and_then(Value::as_array).is_some(),
            "{name} steps[{index}].dependencies should be an array"
        );
        assert!(
            step.get("params").and_then(Value::as_object).is_some(),
            "{name} steps[{index}].params should be an object"
        );
    }
}

fn assert_non_empty_string(name: &str, value: Option<&Value>, field: &str) {
    assert!(
        value
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty()),
        "{name} should include non-empty {field}"
    );
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
