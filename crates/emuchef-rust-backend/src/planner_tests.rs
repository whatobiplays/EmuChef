use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tempfile::TempDir;

use crate::model::{
    OrderedMap, ParamValue, Recipe, RecipeProvides, RemoteFileArtifact, Step, StepCondition,
    StepConstraints,
};
use crate::planner::{
    build_permission_intent, plan_execution, DeviceContext, PlannerInput, RuntimeCapabilities,
};
use crate::planner_device_plan::{
    discover_device_plan_inventory, discover_device_profile_inventory,
};

const P7L_RETROARCH_ONLY_REFS: &[&str] = &["app.retroarch.provision"];
const P7L_RETROARCH_BIOS_REFS: &[&str] = &["app.retroarch.provision", "feature.copy_bios"];
const P7L_RETROARCH_BIOS_XANITEOG_REFS: &[&str] = &[
    "app.retroarch.provision",
    "feature.copy_bios",
    "app.xaniteog.install",
];
const P7L_NO_REQUIRED_BINDINGS: &[RepoPlanE2eBinding] = &[];
const P7L_BIOS_BINDINGS: &[RepoPlanE2eBinding] = &[RepoPlanE2eBinding::BiosSourceDir];
const P7L_BIOS_XANITEOG_BINDINGS: &[RepoPlanE2eBinding] = &[
    RepoPlanE2eBinding::BiosSourceDir,
    RepoPlanE2eBinding::XaniteogApk,
];
const P7L_RETROARCH_CFG_BINDINGS: &[RepoPlanE2eBinding] = &[RepoPlanE2eBinding::RetroarchCfg];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RepoPlanE2eBinding {
    BiosSourceDir,
    XaniteogApk,
    RetroarchCfg,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RepoPlanE2eCase {
    device_plan_ref: &'static str,
    device_profile_ref: &'static str,
    selected_recipe_refs: &'static [&'static str],
    required_bindings: &'static [RepoPlanE2eBinding],
}

fn authored_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("authored_root")
        .join(name)
        .join("authored")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should resolve from crate manifest dir")
}

fn repo_authored_root() -> PathBuf {
    repo_root().join("authored")
}

fn repo_authored_recipes_dir() -> PathBuf {
    repo_authored_root().join("recipes")
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

fn authored_corpus_planner_input(selected_recipe_refs: &[&str]) -> PlannerInput {
    PlannerInput::from_authored_root(
        repo_authored_root(),
        selected_recipe_refs
            .iter()
            .map(|item| item.to_string())
            .collect(),
        "plan.p7h.authored_corpus.001".to_string(),
        "p7h.synthetic.device_plan".to_string(),
        "p7h.synthetic.device_profile".to_string(),
        fixture_device_context(),
        fixture_runtime_capabilities(),
    )
    .expect("repo authored corpus recipes should load through the Rust planner model")
}

fn authored_corpus_planner_input_with_bindings(
    selected_recipe_refs: &[&str],
    input_bindings: &[(&str, Value)],
) -> PlannerInput {
    let mut input = authored_corpus_planner_input(selected_recipe_refs);
    for (input_id, value) in input_bindings {
        input
            .input_bindings
            .insert((*input_id).to_string(), value.clone());
    }
    input
}

fn repo_device_plan_planner_input_with_bindings(
    device_plan_ref: &str,
    input_bindings: &[(&str, Value)],
) -> PlannerInput {
    let mut bindings = OrderedMap::new();
    for (input_id, value) in input_bindings {
        bindings.insert((*input_id).to_string(), value.clone());
    }
    PlannerInput::from_authored_device_plan(
        repo_authored_root(),
        device_plan_ref,
        format!("plan.p7i.{device_plan_ref}.001"),
        bindings,
    )
    .unwrap_or_else(|error| panic!("{device_plan_ref} should build PlannerInput: {error}"))
}

fn repo_plan_e2e_input(case: &RepoPlanE2eCase, temp: &TempDir) -> PlannerInput {
    repo_plan_e2e_input_for_device_plan(case.device_plan_ref, temp, case.required_bindings)
}

fn repo_plan_e2e_input_for_device_plan(
    device_plan_ref: &str,
    temp: &TempDir,
    binding_kinds: &[RepoPlanE2eBinding],
) -> PlannerInput {
    PlannerInput::from_authored_device_plan(
        repo_authored_root(),
        device_plan_ref,
        format!("plan.p7l.{device_plan_ref}.001"),
        repo_plan_e2e_bindings(temp, binding_kinds),
    )
    .unwrap_or_else(|error| panic!("{device_plan_ref} should build P7L PlannerInput: {error}"))
}

fn repo_plan_e2e_bindings(
    temp: &TempDir,
    binding_kinds: &[RepoPlanE2eBinding],
) -> OrderedMap<Value> {
    let mut bindings = OrderedMap::new();
    for binding_kind in binding_kinds {
        let (input_id, path) = repo_plan_e2e_binding_path(temp, *binding_kind);
        bindings.insert(
            input_id.to_string(),
            json!(path.to_string_lossy().to_string()),
        );
    }
    bindings
}

fn repo_plan_e2e_binding_path(
    temp: &TempDir,
    binding_kind: RepoPlanE2eBinding,
) -> (&'static str, PathBuf) {
    match binding_kind {
        RepoPlanE2eBinding::BiosSourceDir => {
            let path = temp.path().join("bios-source");
            fs::create_dir_all(&path).expect("P7L BIOS source temp directory should be created");
            ("feature.copy_bios/bios_source_dir", path)
        }
        RepoPlanE2eBinding::XaniteogApk => {
            let path = temp.path().join("xaniteog.apk");
            fs::write(&path, []).expect("P7L XaniteOG placeholder APK path should be created");
            ("app.xaniteog.install/xaniteog_apk", path)
        }
        RepoPlanE2eBinding::RetroarchCfg => {
            let path = temp.path().join("retroarch.cfg");
            fs::write(&path, []).expect("P7L RetroArch placeholder config path should be created");
            ("app.retroarch.provision/retroarch_cfg", path)
        }
    }
}

fn repo_plan_e2e_cases() -> Vec<RepoPlanE2eCase> {
    vec![
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.konkr_pocket_fit.base",
            device_profile_ref: "ayaneo.konkr_pocket_fit",
            selected_recipe_refs: P7L_RETROARCH_ONLY_REFS,
            required_bindings: P7L_NO_REQUIRED_BINDINGS,
        },
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.pocket_s_mini.base",
            device_profile_ref: "ayaneo.pocket_s_mini",
            selected_recipe_refs: P7L_RETROARCH_ONLY_REFS,
            required_bindings: P7L_NO_REQUIRED_BINDINGS,
        },
    ]
}

fn repo_plan_e2e_gap_cases() -> Vec<RepoPlanE2eCase> {
    vec![
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.generic.base",
            device_profile_ref: "ayaneo.generic",
            selected_recipe_refs: P7L_RETROARCH_BIOS_REFS,
            required_bindings: P7L_BIOS_BINDINGS,
        },
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.pocket_air_mini.base",
            device_profile_ref: "ayaneo.pocket_air_mini",
            selected_recipe_refs: P7L_RETROARCH_BIOS_REFS,
            required_bindings: P7L_BIOS_BINDINGS,
        },
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.pocket_s2.base",
            device_profile_ref: "ayaneo.pocket_s2",
            selected_recipe_refs: P7L_RETROARCH_BIOS_XANITEOG_REFS,
            required_bindings: P7L_BIOS_XANITEOG_BINDINGS,
        },
    ]
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
fn permission_intent_builds_structured_runtime_and_appop_actions_from_selected_grant_steps() {
    let steps = permission_intent_steps(vec![param_contract_step(
        "grant",
        "grant_permissions",
        vec![],
        ref_params(vec![
            (
                "runtime",
                ParamValue::Literal(json!([
                    {
                        "package_name": "com.example.app",
                        "name": "android.permission.POST_NOTIFICATIONS",
                        "required": false,
                        "when": {"android_api_min": 33}
                    }
                ])),
            ),
            (
                "appops",
                ParamValue::Literal(json!([
                    {
                        "package_name": "com.example.app",
                        "op": "MANAGE_EXTERNAL_STORAGE",
                        "mode": "allow",
                        "required": true,
                        "when": {"rooted": false}
                    }
                ])),
            ),
            (
                "policy",
                ParamValue::Literal(json!({"on_failure": "fail", "require_all": true})),
            ),
        ]),
    )]);

    let intent = serde_json::to_value(build_permission_intent(&steps))
        .expect("internal permission intent should serialize for tests");

    assert_eq!(
        intent,
        json!({
            "grants": [
                {
                    "recipe_ref": "planner.permission_intent",
                    "step_id": "grant",
                    "execution_step_id": "planner.permission_intent/grant",
                    "policy": {"on_failure": "fail", "require_all": true},
                    "actions": [
                        {
                            "kind": "runtime_permission",
                            "package_name": "com.example.app",
                            "permission": "android.permission.POST_NOTIFICATIONS",
                            "required": false,
                            "when": {"android_api_min": 33},
                            "source_section": "params.runtime[0]"
                        },
                        {
                            "kind": "appop",
                            "package_name": "com.example.app",
                            "op": "MANAGE_EXTERNAL_STORAGE",
                            "desired_mode": "allow",
                            "required": true,
                            "when": {"rooted": false},
                            "source_section": "params.appops[0]"
                        }
                    ]
                }
            ]
        })
    );
}

#[test]
fn permission_intent_defaults_policy_required_and_empty_grants_without_serialized_plan_field() {
    let grant_with_defaults = param_contract_step(
        "grant_defaults",
        "grant_permissions",
        vec![],
        ref_params(vec![(
            "runtime",
            ParamValue::Literal(json!([
                {
                    "package_name": "com.example.defaults",
                    "name": "android.permission.CAMERA"
                }
            ])),
        )]),
    );
    let empty_grant = param_contract_step(
        "grant_empty",
        "grant_permissions",
        vec![],
        OrderedMap::new(),
    );
    let steps = permission_intent_steps(vec![grant_with_defaults.clone(), empty_grant]);

    let first = serde_json::to_value(build_permission_intent(&steps))
        .expect("internal permission intent should serialize for tests");
    let second = serde_json::to_value(build_permission_intent(&steps))
        .expect("internal permission intent should serialize for tests");

    assert_eq!(first, second);
    assert_eq!(first["grants"].as_array().unwrap().len(), 1);
    assert_eq!(
        first["grants"][0]["policy"],
        json!({"on_failure": "warn", "require_all": false})
    );
    assert_eq!(first["grants"][0]["actions"][0]["required"], true);

    let no_actions = serde_json::to_value(build_permission_intent(&permission_intent_steps(vec![
        grant_empty_permission_step("grant_empty"),
    ])))
    .expect("internal permission intent should serialize for tests");
    assert_eq!(no_actions, json!({"grants": []}));

    let plan = planning_result_value(PlannerInput {
        recipes: vec![Recipe {
            schema_version: 1,
            kind: "recipe".to_string(),
            id: "planner.permission_intent".to_string(),
            name: "Planner Permission Intent".to_string(),
            description: None,
            recipe_dependencies: Vec::new(),
            provides: RecipeProvides {
                features: Vec::new(),
            },
            inputs: OrderedMap::new(),
            artifacts: OrderedMap::new(),
            artifact_groups: OrderedMap::new(),
            steps: vec![grant_with_defaults],
        }],
        selected_recipe_refs: vec!["planner.permission_intent".to_string()],
        input_bindings: OrderedMap::new(),
        plan_id: "plan.permission_intent.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
    });
    assert!(!plan["execution_plan"]
        .as_object()
        .unwrap()
        .contains_key("permission_plan"));
}

#[test]
fn permission_intent_serialized_for_tests_contains_no_shell_or_adb_command_strings() {
    let steps = permission_intent_steps(vec![param_contract_step(
        "grant",
        "grant_permissions",
        vec![],
        ref_params(vec![
            (
                "runtime",
                ParamValue::Literal(json!([
                    {"package_name": "com.example.app", "name": "android.permission.CAMERA"}
                ])),
            ),
            (
                "appops",
                ParamValue::Literal(json!([
                    {"package_name": "com.example.app", "op": "MANAGE_EXTERNAL_STORAGE", "mode": "allow"}
                ])),
            ),
        ]),
    )]);

    let serialized = serde_json::to_string(&build_permission_intent(&steps))
        .expect("internal permission intent should serialize for tests");

    for forbidden in ["adb", "shell", "pm grant", "appops set", "run_plan_command"] {
        assert!(
            !serialized.contains(forbidden),
            "internal planner intent must not contain executable command text {forbidden:?}: {serialized}"
        );
    }
}

#[test]
fn permission_intent_validation_rejects_obvious_malformed_step_local_inputs() {
    let cases = vec![
        (
            param_contract_step(
                "manual",
                "grant_permissions",
                vec![],
                ref_params(vec![("manual", ParamValue::Literal(json!([])))]),
            ),
            "manual",
            "manual",
            "unknown_param",
            json!(["appops", "policy", "runtime"]),
            json!([]),
        ),
        (
            param_contract_step(
                "bad_policy",
                "grant_permissions",
                vec![],
                ref_params(vec![(
                    "policy",
                    ParamValue::Literal(json!({"on_failure": "explode"})),
                )]),
            ),
            "bad_policy",
            "policy.on_failure",
            "invalid_enum_value",
            json!(["warn", "fail"]),
            json!("explode"),
        ),
        (
            param_contract_step(
                "bad_runtime",
                "grant_permissions",
                vec![],
                ref_params(vec![(
                    "runtime",
                    ParamValue::Literal(json!([
                        {"package_name": "com.example.app", "required": "yes"}
                    ])),
                )]),
            ),
            "bad_runtime",
            "runtime[0].name",
            "missing_required_param",
            json!("non-empty string"),
            Value::Null,
        ),
        (
            param_contract_step(
                "bad_appop",
                "grant_permissions",
                vec![],
                ref_params(vec![(
                    "appops",
                    ParamValue::Literal(json!([
                        {
                            "package_name": "com.example.app",
                            "op": "MANAGE_EXTERNAL_STORAGE",
                            "mode": "allow",
                            "when": {"android_api_min": 35, "android_api_max": 34}
                        }
                    ])),
                )]),
            ),
            "bad_appop",
            "appops[0].when",
            "invalid_param_value",
            json!("android_api_min <= android_api_max"),
            json!({"android_api_max": 34, "android_api_min": 35}),
        ),
    ];

    for (step, step_id, param, code, expected, actual_value) in cases {
        let actual = planning_result_value(param_contract_input(vec![step]));
        assert_param_contract_error(&actual, code, step_id, param, expected, actual_value);
    }
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
fn recipe_expansion_explicit_selected_refs_preserve_order_without_dependencies() {
    let actual = planning_result_value(recipe_expansion_input(
        vec![
            recipe_expansion_recipe("planner.recipe_expansion.alpha", vec![]),
            recipe_expansion_recipe("planner.recipe_expansion.beta", vec![]),
            recipe_expansion_recipe("planner.recipe_expansion.gamma", vec![]),
        ],
        vec![
            "planner.recipe_expansion.gamma",
            "planner.recipe_expansion.alpha",
            "planner.recipe_expansion.beta",
        ],
    ));

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(
        actual["execution_plan"]["source"]["selected_recipe_refs"],
        json!([
            "planner.recipe_expansion.gamma",
            "planner.recipe_expansion.alpha",
            "planner.recipe_expansion.beta"
        ])
    );
    assert_eq!(
        actual["execution_plan"]["source"]["expanded_recipe_refs"],
        json!([
            "planner.recipe_expansion.gamma",
            "planner.recipe_expansion.alpha",
            "planner.recipe_expansion.beta"
        ])
    );
}

#[test]
fn recipe_expansion_order_is_dependency_first_authored_and_first_occurrence_stable() {
    let actual = planning_result_value(recipe_expansion_input(
        vec![
            recipe_expansion_recipe("planner.recipe_expansion.dep_shared", vec![]),
            recipe_expansion_recipe(
                "planner.recipe_expansion.dep_b",
                vec!["planner.recipe_expansion.dep_shared"],
            ),
            recipe_expansion_recipe("planner.recipe_expansion.dep_a", vec![]),
            recipe_expansion_recipe(
                "planner.recipe_expansion.selected_a",
                vec![
                    "planner.recipe_expansion.dep_b",
                    "planner.recipe_expansion.dep_a",
                ],
            ),
            recipe_expansion_recipe("planner.recipe_expansion.dep_c", vec![]),
            recipe_expansion_recipe(
                "planner.recipe_expansion.selected_b",
                vec![
                    "planner.recipe_expansion.dep_a",
                    "planner.recipe_expansion.dep_c",
                ],
            ),
        ],
        vec![
            "planner.recipe_expansion.selected_a",
            "planner.recipe_expansion.selected_b",
        ],
    ));

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(
        actual["execution_plan"]["source"]["selected_recipe_refs"],
        json!([
            "planner.recipe_expansion.selected_a",
            "planner.recipe_expansion.selected_b"
        ])
    );
    assert_eq!(
        actual["execution_plan"]["source"]["expanded_recipe_refs"],
        json!([
            "planner.recipe_expansion.dep_shared",
            "planner.recipe_expansion.dep_b",
            "planner.recipe_expansion.dep_a",
            "planner.recipe_expansion.selected_a",
            "planner.recipe_expansion.dep_c",
            "planner.recipe_expansion.selected_b"
        ])
    );
}

#[test]
fn recipe_expansion_unknown_selected_ref_error_shape_has_selected_context() {
    let actual = planning_result_value(recipe_expansion_input(
        vec![recipe_expansion_recipe(
            "planner.recipe_expansion.known",
            vec![],
        )],
        vec!["planner.recipe_expansion.missing_selected"],
    ));

    let error = assert_recipe_expansion_error(&actual, "recipe_not_found");
    assert_eq!(
        error["details"],
        json!({
            "recipe_ref": "planner.recipe_expansion.missing_selected",
            "selected_recipe_ref": "planner.recipe_expansion.missing_selected",
            "source": "selected_recipe_refs"
        })
    );
}

#[test]
fn recipe_expansion_unknown_dependency_ref_error_shape_has_dependency_context() {
    let actual = planning_result_value(recipe_expansion_input(
        vec![recipe_expansion_recipe(
            "planner.recipe_expansion.selected",
            vec!["planner.recipe_expansion.missing_dependency"],
        )],
        vec!["planner.recipe_expansion.selected"],
    ));

    let error = assert_recipe_expansion_error(&actual, "recipe_not_found");
    assert_eq!(
        error["details"],
        json!({
            "recipe_ref": "planner.recipe_expansion.missing_dependency",
            "dependency_ref": "planner.recipe_expansion.missing_dependency",
            "dependent_recipe_ref": "planner.recipe_expansion.selected",
            "source": "recipe_dependencies"
        })
    );
}

#[test]
fn recipe_expansion_dependency_cycle_error_shape_has_cycle_context() {
    let actual = planning_result_value(recipe_expansion_input(
        vec![
            recipe_expansion_recipe(
                "planner.recipe_expansion.cycle_a",
                vec!["planner.recipe_expansion.cycle_b"],
            ),
            recipe_expansion_recipe(
                "planner.recipe_expansion.cycle_b",
                vec!["planner.recipe_expansion.cycle_a"],
            ),
        ],
        vec!["planner.recipe_expansion.cycle_a"],
    ));

    let error = assert_recipe_expansion_error(&actual, "dependency_cycle");
    assert_eq!(
        error["details"]["cycle"],
        json!([
            "planner.recipe_expansion.cycle_a",
            "planner.recipe_expansion.cycle_b",
            "planner.recipe_expansion.cycle_a"
        ])
    );
}

#[test]
fn recipe_expansion_current_authored_corpus_has_no_non_empty_recipe_dependencies() {
    for entry in authored_corpus_recipe_inventory() {
        let path = repo_root().join(entry.path);
        let recipe = crate::yaml::load_recipe_from_path(&path)
            .unwrap_or_else(|error| panic!("{} should parse: {error}", entry.path));

        assert!(
            recipe.recipe_dependencies.is_empty(),
            "{} currently has no non-empty recipe_dependencies metadata",
            entry.path
        );
    }
}

#[test]
fn recipe_expansion_current_corpus_selected_and_expanded_refs_match() {
    let selected_recipe_refs = [
        "app.obtainium.install",
        "app.retroarch.provision",
        "app.xaniteog.install",
        "feature.copy_bios",
    ];
    let first = planning_result_value(authored_corpus_planner_input_with_bindings(
        &selected_recipe_refs,
        &p7k_corpus_recipe_expansion_bindings(),
    ));
    let second = planning_result_value(authored_corpus_planner_input_with_bindings(
        &selected_recipe_refs,
        &p7k_corpus_recipe_expansion_bindings(),
    ));

    assert_eq!(first, second);
    assert_eq!(first["status"], "success", "{first:#}");
    assert_eq!(
        first["execution_plan"]["source"]["selected_recipe_refs"],
        json!(selected_recipe_refs)
    );
    assert_eq!(
        first["execution_plan"]["source"]["expanded_recipe_refs"],
        json!(selected_recipe_refs)
    );
}

#[test]
fn recipe_expansion_checked_in_device_plan_selected_sets_match_expanded_refs_for_current_corpus() {
    for plan in discover_device_plan_inventory(&repo_authored_root())
        .expect("repo device plan inventory should parse")
    {
        let selected_recipe_refs = plan
            .selected_recipe_refs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let actual = planning_result_value(authored_corpus_planner_input_with_bindings(
            &selected_recipe_refs,
            &p7k_corpus_recipe_expansion_bindings(),
        ));

        assert_eq!(actual["status"], "success", "{}: {actual:#}", plan.id);
        assert_eq!(
            actual["execution_plan"]["source"]["selected_recipe_refs"],
            json!(plan.selected_recipe_refs),
            "{}",
            plan.id
        );
        assert_eq!(
            actual["execution_plan"]["source"]["expanded_recipe_refs"],
            json!(plan.selected_recipe_refs),
            "{}",
            plan.id
        );
    }
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
fn dto_success_result_shape_is_stable_for_supported_fixture_surface() {
    let first = normalized_planning_result_value(planner_input(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    ));
    let second = normalized_planning_result_value(planner_input(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    ));

    assert_eq!(first, second);
    assert_planning_result_shape("dto success result", &first);
    assert_eq!(first["status"], "success");
    assert_eq!(first["warnings"], json!([]));
    assert_eq!(first["errors"], json!([]));

    let plan = first["execution_plan"]
        .as_object()
        .expect("dto success result should include an execution plan");
    assert_object_keys(
        "dto success execution_plan",
        plan,
        &[
            "id",
            "source",
            "device_context",
            "runtime_capabilities",
            "inputs",
            "artifacts",
            "steps",
            "schema_version",
            "kind",
        ],
    );
    assert!(!plan.contains_key("permission_plan"));
    assert_eq!(plan["id"], "plan.example.device_plan.001");
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["kind"], "execution_plan");

    assert_eq!(
        plan["source"],
        json!({
            "device_profile_ref": "example.device_profile",
            "device_plan_ref": "example.device_plan",
            "selected_recipe_refs": ["planner.phase6n.builtins_all"],
            "expanded_recipe_refs": ["planner.phase6n.builtins_all"]
        })
    );
    assert_eq!(
        plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
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

    let steps = plan["steps"].as_array().unwrap();
    for (index, step) in steps.iter().enumerate() {
        let step = step
            .as_object()
            .expect("dto execution plan step should be an object");
        assert_object_keys(
            &format!("dto success steps[{index}]"),
            step,
            &[
                "id",
                "recipe_ref",
                "type",
                "name",
                "dependencies",
                "constraints",
                "params",
                "skip_if",
                "verify",
            ],
        );
        assert_object_keys(
            &format!("dto success steps[{index}].constraints"),
            step["constraints"]
                .as_object()
                .expect("step constraints should be an object"),
            &["capabilities", "conflicts_with"],
        );
        assert!(step["dependencies"].as_array().is_some());
        assert!(step["params"].as_object().is_some());
        assert!(step["skip_if"].as_array().is_some());
        assert!(step["verify"].as_array().is_some());
    }
}

#[test]
fn dto_normalized_params_ref_defaults_and_permission_surface_are_stable() {
    let actual = normalized_planning_result_value(planner_input(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    ));
    let plan = actual["execution_plan"].as_object().unwrap();
    assert!(!plan.contains_key("permission_plan"));

    let resolve = planner_step(&actual, "planner.phase6n.builtins_all/resolve");
    assert_object_keys(
        "dto resolve params",
        resolve["params"].as_object().unwrap(),
        &["artifacts"],
    );
    assert_eq!(
        resolve["params"]["artifacts"],
        json!({"value": [
            "planner.phase6n.builtins_all/archive_zip",
            "planner.phase6n.builtins_all/app_apk"
        ]})
    );

    let extract = planner_step(&actual, "planner.phase6n.builtins_all/extract_artifacts");
    assert_eq!(extract["params"]["extract_on"], json!({"value": "host"}));
    assert_eq!(
        extract["params"]["artifacts"],
        json!({"value": ["planner.phase6n.builtins_all/archive_zip"]})
    );

    let archive = planner_step(&actual, "planner.phase6n.builtins_all/extract_archive");
    assert_eq!(archive["params"]["cleanup"], json!({"value": true}));
    assert_eq!(
        archive["params"]["archive"],
        json!({"ref": "artifacts.planner.phase6n.builtins_all/archive_zip.local_path"})
    );

    let install = planner_step(&actual, "planner.phase6n.builtins_all/install");
    assert_eq!(
        install["params"]["replace_existing"],
        json!({"value": false})
    );

    let copy = planner_step(&actual, "planner.phase6n.builtins_all/copy");
    assert_eq!(
        copy["dependencies"],
        json!(["planner.phase6n.builtins_all/extract_artifacts"])
    );
    assert_eq!(
        copy["params"]["source"],
        json!({"ref": "steps.planner.phase6n.builtins_all/extract_artifacts.outputs.extracted_paths"})
    );
    assert_eq!(copy["params"]["copy_policy"], json!({"value": "merge"}));

    let grant = planner_step(&actual, "planner.phase6n.builtins_all/grant");
    assert_object_keys(
        "dto grant params",
        grant["params"].as_object().unwrap(),
        &["runtime", "appops", "policy"],
    );
    assert_eq!(
        grant["params"]["runtime"]["value"][0]["package_name"],
        "com.example.app"
    );
    assert_eq!(
        grant["params"]["appops"]["value"][0]["op"],
        "MANAGE_EXTERNAL_STORAGE"
    );
    assert_eq!(
        grant["params"]["policy"],
        json!({"value": {"on_failure": "fail", "require_all": true}})
    );
}

#[test]
fn dto_error_result_shape_uses_current_focused_step_param_diagnostic() {
    let first = planning_result_value(param_contract_input(vec![param_contract_step(
        "copy",
        "copy_files",
        vec![],
        ref_params(vec![("dest", ParamValue::Literal(json!("/sdcard/Out")))]),
    )]));
    let second = planning_result_value(param_contract_input(vec![param_contract_step(
        "copy",
        "copy_files",
        vec![],
        ref_params(vec![("dest", ParamValue::Literal(json!("/sdcard/Out")))]),
    )]));

    assert_eq!(first, second);
    assert_planning_result_shape("dto error result", &first);
    assert_eq!(first["status"], "error");
    assert_eq!(first["warnings"], json!([]));
    assert_eq!(first["execution_plan"], Value::Null);
    assert_eq!(
        first["errors"],
        json!([
            {
                "code": "missing_required_param",
                "message": "Param 'source' is required for step type 'copy_files'.",
                "details": {
                    "recipe_ref": "planner.param_contract",
                    "step_id": "copy",
                    "step_type": "copy_files",
                    "param": "source",
                    "expected": "ref",
                    "actual": null
                }
            }
        ])
    );
}

#[test]
fn dto_multiple_error_order_is_deterministic_for_existing_unknown_param_slice() {
    let actual = planning_result_value(param_contract_input(vec![param_contract_step(
        "copy",
        "copy_files",
        vec![],
        ref_params(vec![
            ("source", ParamValue::Ref("steps.extract".to_string())),
            ("dest", ParamValue::Literal(json!("/sdcard/Out"))),
            ("z_extra", ParamValue::Literal(json!(true))),
            ("a_extra", ParamValue::Literal(json!(false))),
        ]),
    )]));

    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null);
    let errors = actual["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2, "{errors:#?}");
    assert_eq!(
        errors
            .iter()
            .map(|error| (
                error["code"].as_str().unwrap(),
                error["details"]["param"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![("unknown_param", "a_extra"), ("unknown_param", "z_extra")]
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
fn param_contract_defaults_preserve_group_only_selection_order_and_shorthand_refs() {
    let actual = planning_result_value(param_contract_input(vec![
        param_contract_step(
            "extract",
            "extract_artifacts",
            vec![],
            ref_params(vec![(
                "artifact_groups",
                ParamValue::Literal(json!(["archives"])),
            )]),
        ),
        param_contract_step(
            "copy",
            "copy_files",
            vec!["extract"],
            ref_params(vec![
                ("source", ParamValue::Ref("steps.extract".to_string())),
                ("dest", ParamValue::Literal(json!("/sdcard/Extracted"))),
            ]),
        ),
        param_contract_step(
            "archive",
            "extract_archive",
            vec![],
            ref_params(vec![
                (
                    "archive",
                    ParamValue::Ref("artifacts.archive_zip.local_path".to_string()),
                ),
                ("extract_on", ParamValue::Literal(json!("device"))),
                ("dest", ParamValue::Literal(json!("/sdcard/Archive"))),
            ]),
        ),
        param_contract_step(
            "install",
            "install_apk",
            vec![],
            ref_params(vec![(
                "app",
                ParamValue::Ref("artifacts.app_apk.local_path".to_string()),
            )]),
        ),
        param_contract_step(
            "wait",
            "wait",
            vec![],
            ref_params(vec![("duration_ms", ParamValue::Literal(json!(1)))]),
        ),
    ]));

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(
        param_contract_step_ids(&actual),
        vec![
            "planner.param_contract/extract",
            "planner.param_contract/archive",
            "planner.param_contract/install",
            "planner.param_contract/wait",
            "planner.param_contract/copy",
        ]
    );
    assert_eq!(
        *param_contract_step_param(&actual, "extract", "artifacts"),
        json!({"value": ["planner.param_contract/archive_zip"]})
    );
    assert_eq!(
        *param_contract_step_param(&actual, "extract", "extract_on"),
        json!({"value": "host"})
    );
    assert_eq!(
        *param_contract_step_param(&actual, "copy", "source"),
        json!({"ref": "steps.planner.param_contract/extract.outputs.extracted_paths"})
    );
    assert_eq!(
        *param_contract_step_param(&actual, "copy", "copy_policy"),
        json!({"value": "merge"})
    );
    assert_eq!(
        *param_contract_step_param(&actual, "archive", "cleanup"),
        json!({"value": true})
    );
    assert_eq!(
        *param_contract_step_param(&actual, "install", "replace_existing"),
        json!({"value": false})
    );
}

#[test]
fn param_contract_accepts_valid_enum_and_bool_values() {
    let actual = planning_result_value(param_contract_input(vec![
        param_contract_step(
            "extract",
            "extract_artifacts",
            vec![],
            ref_params(vec![
                ("artifact_groups", ParamValue::Literal(json!(["archives"]))),
                ("extract_on", ParamValue::Literal(json!("device"))),
            ]),
        ),
        param_contract_step(
            "archive",
            "extract_archive",
            vec![],
            ref_params(vec![
                (
                    "archive",
                    ParamValue::Ref("artifacts.archive_zip.local_path".to_string()),
                ),
                ("extract_on", ParamValue::Literal(json!("device"))),
                ("dest", ParamValue::Literal(json!("/sdcard/Archive"))),
                (
                    "device_temp_path",
                    ParamValue::Literal(json!("/sdcard/tmp/archive.zip")),
                ),
                ("cleanup", ParamValue::Literal(json!(false))),
            ]),
        ),
        param_contract_step(
            "copy",
            "copy_files",
            vec![],
            ref_params(vec![
                (
                    "source",
                    ParamValue::Ref("artifacts.archive_zip.local_path".to_string()),
                ),
                ("dest", ParamValue::Literal(json!("/sdcard/Out"))),
                ("copy_policy", ParamValue::Literal(json!("sync"))),
            ]),
        ),
        param_contract_step(
            "install",
            "install_apk",
            vec![],
            ref_params(vec![
                (
                    "app",
                    ParamValue::Ref("artifacts.app_apk.local_path".to_string()),
                ),
                ("replace_existing", ParamValue::Literal(json!(true))),
            ]),
        ),
    ]));

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(
        *param_contract_step_param(&actual, "extract", "extract_on"),
        json!({"value": "device"})
    );
    assert_eq!(
        *param_contract_step_param(&actual, "archive", "cleanup"),
        json!({"value": false})
    );
    assert_eq!(
        *param_contract_step_param(&actual, "copy", "copy_policy"),
        json!({"value": "sync"})
    );
    assert_eq!(
        *param_contract_step_param(&actual, "install", "replace_existing"),
        json!({"value": true})
    );
}

#[test]
fn param_contract_reports_required_mode_enum_bool_and_integer_violations() {
    let cases = vec![
        (
            param_contract_step(
                "copy",
                "copy_files",
                vec![],
                ref_params(vec![("dest", ParamValue::Literal(json!("/sdcard/Out")))]),
            ),
            "copy",
            "source",
            "missing_required_param",
            json!("ref"),
            Value::Null,
        ),
        (
            param_contract_step(
                "copy",
                "copy_files",
                vec![],
                ref_params(vec![
                    ("source", ParamValue::Ref("steps.extract".to_string())),
                    ("dest", ParamValue::Ref("inputs.output_dir".to_string())),
                ]),
            ),
            "copy",
            "dest",
            "invalid_param_mode",
            json!("literal"),
            json!({"ref": "inputs.output_dir"}),
        ),
        (
            param_contract_step(
                "copy",
                "copy_files",
                vec![],
                ref_params(vec![
                    ("source", ParamValue::Ref("steps.extract".to_string())),
                    ("dest", ParamValue::Literal(json!("/sdcard/Out"))),
                    ("copy_policy", ParamValue::Literal(json!("overwrite"))),
                ]),
            ),
            "copy",
            "copy_policy",
            "invalid_enum_value",
            json!(["merge", "replace", "sync"]),
            json!("overwrite"),
        ),
        (
            param_contract_step("extract", "extract_artifacts", vec![], OrderedMap::new()),
            "extract",
            "artifacts",
            "missing_required_param",
            json!("literal list via artifacts or artifact_groups"),
            Value::Null,
        ),
        (
            param_contract_step(
                "extract",
                "extract_artifacts",
                vec![],
                ref_params(vec![(
                    "artifacts",
                    ParamValue::Literal(json!("archive_zip")),
                )]),
            ),
            "extract",
            "artifacts",
            "invalid_param_value",
            json!("literal list"),
            json!("archive_zip"),
        ),
        (
            param_contract_step(
                "extract",
                "extract_artifacts",
                vec![],
                ref_params(vec![
                    ("artifact_groups", ParamValue::Literal(json!(["archives"]))),
                    ("extract_on", ParamValue::Literal(json!("target"))),
                ]),
            ),
            "extract",
            "extract_on",
            "invalid_enum_value",
            json!(["host", "device"]),
            json!("target"),
        ),
        (
            param_contract_step("archive", "extract_archive", vec![], OrderedMap::new()),
            "archive",
            "archive",
            "missing_required_param",
            json!("ref"),
            Value::Null,
        ),
        (
            param_contract_step(
                "archive",
                "extract_archive",
                vec![],
                ref_params(vec![
                    (
                        "archive",
                        ParamValue::Ref("artifacts.archive_zip.local_path".to_string()),
                    ),
                    ("extract_on", ParamValue::Literal(json!("device"))),
                ]),
            ),
            "archive",
            "dest",
            "missing_required_param",
            json!("literal when extract_on is device"),
            Value::Null,
        ),
        (
            param_contract_step(
                "archive",
                "extract_archive",
                vec![],
                ref_params(vec![
                    (
                        "archive",
                        ParamValue::Ref("artifacts.archive_zip.local_path".to_string()),
                    ),
                    ("dest", ParamValue::Literal(json!("/sdcard/Out"))),
                ]),
            ),
            "archive",
            "dest",
            "invalid_param_value",
            json!("only valid when extract_on is device"),
            json!("/sdcard/Out"),
        ),
        (
            param_contract_step(
                "archive",
                "extract_archive",
                vec![],
                ref_params(vec![
                    (
                        "archive",
                        ParamValue::Ref("artifacts.archive_zip.local_path".to_string()),
                    ),
                    ("cleanup", ParamValue::Literal(json!("yes"))),
                ]),
            ),
            "archive",
            "cleanup",
            "invalid_param_value",
            json!("literal bool"),
            json!("yes"),
        ),
        (
            param_contract_step(
                "install",
                "install_apk",
                vec![],
                ref_params(vec![
                    (
                        "app",
                        ParamValue::Ref("artifacts.app_apk.local_path".to_string()),
                    ),
                    ("replace_existing", ParamValue::Literal(json!("false"))),
                ]),
            ),
            "install",
            "replace_existing",
            "invalid_param_value",
            json!("literal bool"),
            json!("false"),
        ),
        (
            param_contract_step("install", "install_apk", vec![], OrderedMap::new()),
            "install",
            "app",
            "missing_required_param",
            json!("ref"),
            Value::Null,
        ),
        (
            param_contract_step("wait", "wait", vec![], OrderedMap::new()),
            "wait",
            "duration_ms",
            "missing_required_param",
            json!("literal positive integer"),
            Value::Null,
        ),
        (
            param_contract_step(
                "wait",
                "wait",
                vec![],
                ref_params(vec![("duration_ms", ParamValue::Literal(json!(0)))]),
            ),
            "wait",
            "duration_ms",
            "invalid_param_value",
            json!("literal positive integer"),
            json!(0),
        ),
    ];

    for (step, step_id, param, code, expected, actual_value) in cases {
        let actual = planning_result_value(param_contract_input(vec![step]));
        assert_param_contract_error(&actual, code, step_id, param, expected, actual_value);
    }
}

#[test]
fn param_contract_reports_unknown_params_only_for_focused_step_types() {
    let focused = planning_result_value(param_contract_input(vec![param_contract_step(
        "copy",
        "copy_files",
        vec![],
        ref_params(vec![
            ("source", ParamValue::Ref("steps.extract".to_string())),
            ("dest", ParamValue::Literal(json!("/sdcard/Out"))),
            ("extra", ParamValue::Literal(json!(true))),
        ]),
    )]));
    assert_param_contract_error(
        &focused,
        "unknown_param",
        "copy",
        "extra",
        json!(["copy_policy", "dest", "source"]),
        json!(true),
    );

    let non_focused = planning_result_value(param_contract_input(vec![param_contract_step(
        "launch",
        "launch_app",
        vec![],
        ref_params(vec![
            (
                "package_name",
                ParamValue::Literal(json!("com.example.app")),
            ),
            ("extra", ParamValue::Literal(json!(true))),
        ]),
    )]));
    assert_eq!(non_focused["status"], "success", "{non_focused:#}");
}

#[test]
fn param_contract_wait_duration_rejects_negative_and_non_integer_values() {
    for (step_id, value) in [
        ("negative", json!(-1)),
        ("float", json!(1.5)),
        ("bool", json!(true)),
    ] {
        let actual = planning_result_value(param_contract_input(vec![param_contract_step(
            step_id,
            "wait",
            vec![],
            ref_params(vec![("duration_ms", ParamValue::Literal(value.clone()))]),
        )]));
        assert_param_contract_error(
            &actual,
            "invalid_param_value",
            step_id,
            "duration_ms",
            json!("literal positive integer"),
            value,
        );
    }
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

#[test]
fn authored_corpus_planner_recipe_inventory_is_explicit() {
    let expected = authored_corpus_recipe_inventory()
        .iter()
        .map(|entry| entry.path.to_string())
        .collect::<Vec<_>>();

    assert_eq!(discovered_authored_corpus_recipe_paths(), expected);
}

#[test]
fn authored_corpus_recipes_parse_through_rust_domain_model() {
    for entry in authored_corpus_recipe_inventory() {
        let path = repo_root().join(entry.path);
        let recipe = crate::yaml::load_recipe_from_path(&path)
            .unwrap_or_else(|error| panic!("{} should parse: {error}", entry.path));

        assert_eq!(recipe.schema_version, 1, "{}", entry.path);
        assert_eq!(recipe.kind, "recipe", "{}", entry.path);
        assert_eq!(recipe.id, entry.recipe_id, "{}", entry.path);
        assert!(!recipe.name.is_empty(), "{} should have a name", entry.path);
        assert!(!recipe.steps.is_empty(), "{} should have steps", entry.path);
        for step in recipe.steps {
            assert!(
                crate::step_specs::is_supported_step_type(&step.type_name),
                "{} step {} should use a Rust-known StepSpec type, got {}",
                entry.path,
                step.id,
                step.type_name
            );
        }
    }
}

#[test]
fn authored_corpus_unbound_required_inputs_emit_classified_errors() {
    for (recipe_ref, input_id) in [
        ("app.xaniteog.install", "app.xaniteog.install/xaniteog_apk"),
        ("feature.copy_bios", "feature.copy_bios/bios_source_dir"),
    ] {
        let first = planning_result_value(authored_corpus_planner_input(&[recipe_ref]));
        let second = planning_result_value(authored_corpus_planner_input(&[recipe_ref]));

        assert_eq!(
            first, second,
            "{recipe_ref} error output should be deterministic"
        );
        assert_eq!(first["status"], "error", "{recipe_ref}: {first:#}");
        assert_eq!(first["execution_plan"], Value::Null, "{recipe_ref}");

        let errors = first["errors"]
            .as_array()
            .expect("required-input planner result should include errors");
        assert_eq!(errors.len(), 1, "{recipe_ref}: {errors:#?}");
        assert_eq!(errors[0]["code"], "binding_missing", "{recipe_ref}");
        assert_eq!(errors[0]["details"]["input_id"], input_id, "{recipe_ref}");
        assert!(
            errors[0]["message"]
                .as_str()
                .is_some_and(|message| message.contains(input_id)),
            "{recipe_ref}: {errors:#?}"
        );
    }
}

#[test]
fn authored_corpus_retroarch_optional_cfg_omitted_and_bound_are_deterministic() {
    let omitted =
        planning_result_value(authored_corpus_planner_input(&["app.retroarch.provision"]));
    let omitted_repeat =
        planning_result_value(authored_corpus_planner_input(&["app.retroarch.provision"]));

    assert_eq!(omitted, omitted_repeat);
    assert_planning_result_shape("authored corpus retroarch omitted", &omitted);
    assert_eq!(omitted["status"], "success", "{omitted:#}");
    assert_eq!(omitted["execution_plan"]["inputs"], json!([]));
    let omitted_step_ids = execution_step_ids(&omitted);
    assert!(
        omitted_step_ids.contains(&"app.retroarch.provision/launch_retroarch"),
        "{omitted_step_ids:#?}"
    );
    assert!(
        !omitted_step_ids.contains(&"app.retroarch.provision/seed_retroarch_cfg"),
        "{omitted_step_ids:#?}"
    );
    let omitted_launch = planner_step(&omitted, "app.retroarch.provision/launch_retroarch");
    assert!(
        !omitted_launch["dependencies"]
            .as_array()
            .expect("launch dependencies should be an array")
            .iter()
            .any(|dependency| dependency == "app.retroarch.provision/seed_retroarch_cfg"),
        "{omitted_launch:#}"
    );

    let bound = planning_result_value(authored_corpus_planner_input_with_bindings(
        &["app.retroarch.provision"],
        &[(
            "app.retroarch.provision/retroarch_cfg",
            json!("/tmp/emuchef-p7h-retroarch.cfg"),
        )],
    ));
    let bound_repeat = planning_result_value(authored_corpus_planner_input_with_bindings(
        &["app.retroarch.provision"],
        &[(
            "app.retroarch.provision/retroarch_cfg",
            json!("/tmp/emuchef-p7h-retroarch.cfg"),
        )],
    ));

    assert_eq!(bound, bound_repeat);
    assert_planning_result_shape("authored corpus retroarch bound", &bound);
    assert_eq!(bound["status"], "success", "{bound:#}");
    assert_eq!(
        execution_input_ids(&bound),
        vec!["app.retroarch.provision/retroarch_cfg"]
    );
    let bound_step_ids = execution_step_ids(&bound);
    assert!(
        bound_step_ids.contains(&"app.retroarch.provision/seed_retroarch_cfg"),
        "{bound_step_ids:#?}"
    );
    let seed = planner_step(&bound, "app.retroarch.provision/seed_retroarch_cfg");
    assert_eq!(
        seed["params"]["source"],
        json!({"ref": "inputs.app.retroarch.provision/retroarch_cfg"})
    );
}

#[test]
fn authored_corpus_supported_synthetic_context_emits_execution_plan() {
    let selected_recipe_refs = [
        "app.obtainium.install",
        "app.retroarch.provision",
        "app.xaniteog.install",
        "feature.copy_bios",
    ];
    let input_bindings = [
        (
            "app.retroarch.provision/retroarch_cfg",
            json!("/tmp/emuchef-p7h-retroarch.cfg"),
        ),
        (
            "app.xaniteog.install/xaniteog_apk",
            json!("/tmp/emuchef-p7h-xaniteog.apk"),
        ),
        (
            "feature.copy_bios/bios_source_dir",
            json!("/tmp/emuchef-p7h-bios"),
        ),
    ];

    let first = planning_result_value(authored_corpus_planner_input_with_bindings(
        &selected_recipe_refs,
        &input_bindings,
    ));
    let second = planning_result_value(authored_corpus_planner_input_with_bindings(
        &selected_recipe_refs,
        &input_bindings,
    ));

    assert_eq!(first, second);
    assert_planning_result_shape("authored corpus supported synthetic context", &first);
    assert_eq!(first["status"], "success", "{first:#}");
    assert_eq!(
        first["execution_plan"]["source"]["device_plan_ref"],
        "p7h.synthetic.device_plan"
    );
    assert_eq!(
        first["execution_plan"]["source"]["device_profile_ref"],
        "p7h.synthetic.device_profile"
    );
    assert_eq!(
        first["execution_plan"]["source"]["selected_recipe_refs"],
        json!(selected_recipe_refs)
    );
    assert_eq!(
        first["execution_plan"]["source"]["expanded_recipe_refs"],
        json!(selected_recipe_refs)
    );
    assert_eq!(
        execution_input_ids(&first),
        vec![
            "app.retroarch.provision/retroarch_cfg",
            "app.xaniteog.install/xaniteog_apk",
            "feature.copy_bios/bios_source_dir",
        ]
    );

    let step_ids = execution_step_ids(&first);
    for expected_step_id in [
        "app.obtainium.install/resolve_artifacts",
        "app.obtainium.install/install_obtainium",
        "app.retroarch.provision/seed_retroarch_cfg",
        "app.retroarch.provision/launch_retroarch",
        "app.xaniteog.install/install_xaniteog",
        "feature.copy_bios/copy_bios_dir",
    ] {
        assert!(
            step_ids.contains(&expected_step_id),
            "missing {expected_step_id}; actual: {step_ids:#?}"
        );
    }
}

#[test]
fn authored_corpus_planner_uses_rust_inputs_and_preserves_checked_in_evidence() {
    let recipes_before = snapshot_files(&repo_authored_recipes_dir());
    let goldens_before = snapshot_files(&golden_dir());

    let input = authored_corpus_planner_input(&["app.obtainium.install"]);
    assert_eq!(
        input.recipes.len(),
        authored_corpus_recipe_inventory().len()
    );
    assert_eq!(input.selected_recipe_refs, vec!["app.obtainium.install"]);

    let actual = planning_result_value(input);

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(snapshot_files(&repo_authored_recipes_dir()), recipes_before);
    assert_eq!(snapshot_files(&golden_dir()), goldens_before);
}

#[test]
fn repo_device_profile_inventory_is_explicit_by_path_and_id() {
    let actual = discover_device_profile_inventory(&repo_authored_root())
        .expect("repo device profile inventory should parse")
        .into_iter()
        .map(|entry| {
            (
                repo_relative_path(&entry.path),
                entry.id,
                entry.runtime_capabilities.adb_available,
                entry.runtime_capabilities.app_data_write,
                entry.device_tags,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        vec![
            (
                "authored/device_profiles/ayaneo.generic.yaml".to_string(),
                "ayaneo.generic".to_string(),
                true,
                false,
                vec!["handheld_android".to_string(), "brand_ayaneo".to_string()],
            ),
            (
                "authored/device_profiles/ayaneo.konkr_pocket_fit.yaml".to_string(),
                "ayaneo.konkr_pocket_fit".to_string(),
                true,
                true,
                vec!["handheld_android".to_string(), "brand_ayaneo".to_string()],
            ),
            (
                "authored/device_profiles/ayaneo.pocket_air_mini.yaml".to_string(),
                "ayaneo.pocket_air_mini".to_string(),
                true,
                false,
                vec!["handheld_android".to_string(), "brand_ayaneo".to_string()],
            ),
            (
                "authored/device_profiles/ayaneo.pocket_s2.yaml".to_string(),
                "ayaneo.pocket_s2".to_string(),
                true,
                false,
                vec!["handheld_android".to_string(), "brand_ayaneo".to_string()],
            ),
            (
                "authored/device_profiles/ayaneo.pocket_s_mini.yaml".to_string(),
                "ayaneo.pocket_s_mini".to_string(),
                true,
                true,
                vec!["handheld_android".to_string(), "brand_ayaneo".to_string()],
            ),
        ]
    );
}

#[test]
fn repo_device_plan_inventory_is_explicit_by_path_id_profile_and_selected_order() {
    let actual = discover_device_plan_inventory(&repo_authored_root())
        .expect("repo device plan inventory should parse")
        .into_iter()
        .map(|entry| {
            (
                repo_relative_path(&entry.path),
                entry.id,
                entry.device_profile_ref,
                entry.selected_recipe_refs,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        vec![
            (
                "authored/device_plans/ayaneo.generic.base.yaml".to_string(),
                "ayaneo.generic.base".to_string(),
                "ayaneo.generic".to_string(),
                vec![
                    "app.retroarch.provision".to_string(),
                    "feature.copy_bios".to_string(),
                ],
            ),
            (
                "authored/device_plans/ayaneo.konkr_pocket_fit.base.yaml".to_string(),
                "ayaneo.konkr_pocket_fit.base".to_string(),
                "ayaneo.konkr_pocket_fit".to_string(),
                vec!["app.retroarch.provision".to_string()],
            ),
            (
                "authored/device_plans/ayaneo.pocket_air_mini.base.yaml".to_string(),
                "ayaneo.pocket_air_mini.base".to_string(),
                "ayaneo.pocket_air_mini".to_string(),
                vec![
                    "app.retroarch.provision".to_string(),
                    "feature.copy_bios".to_string(),
                ],
            ),
            (
                "authored/device_plans/ayaneo.pocket_s2.base.yaml".to_string(),
                "ayaneo.pocket_s2.base".to_string(),
                "ayaneo.pocket_s2".to_string(),
                vec![
                    "app.retroarch.provision".to_string(),
                    "feature.copy_bios".to_string(),
                    "app.xaniteog.install".to_string(),
                ],
            ),
            (
                "authored/device_plans/ayaneo.pocket_s_mini.base.yaml".to_string(),
                "ayaneo.pocket_s_mini.base".to_string(),
                "ayaneo.pocket_s_mini".to_string(),
                vec!["app.retroarch.provision".to_string()],
            ),
        ]
    );
}

#[test]
fn repo_device_plan_selected_refs_map_to_existing_authored_recipes() {
    let recipes = authored_corpus_recipe_inventory()
        .iter()
        .map(|entry| entry.recipe_id)
        .collect::<BTreeSet<_>>();
    for plan in discover_device_plan_inventory(&repo_authored_root())
        .expect("repo device plan inventory should parse")
    {
        for recipe_ref in &plan.selected_recipe_refs {
            assert!(
                recipes.contains(recipe_ref.as_str()),
                "{} selected unknown recipe {}",
                plan.id,
                recipe_ref
            );
        }
    }
}

#[test]
fn repo_device_plan_context_builds_planner_input_from_profile_data() {
    let input = repo_device_plan_planner_input_with_bindings("ayaneo.konkr_pocket_fit.base", &[]);

    assert_eq!(input.device_plan_ref, "ayaneo.konkr_pocket_fit.base");
    assert_eq!(input.device_profile_ref, "ayaneo.konkr_pocket_fit");
    assert_eq!(
        input.selected_recipe_refs,
        vec!["app.retroarch.provision".to_string()]
    );
    assert_eq!(input.device_context.manufacturer, "AYANEO");
    assert_eq!(input.device_context.model, "AYANEO KONKR Pocket FIT");
    assert_eq!(input.device_context.android_version, 14);
    assert_eq!(input.device_context.android_api_level, None);
    assert_eq!(
        input.device_context.device_tags,
        vec!["handheld_android".to_string(), "brand_ayaneo".to_string()]
    );
    assert!(input.runtime_capabilities.app_data_write);
    assert!(input.runtime_capabilities.root_shell);
}

#[test]
fn repo_device_plan_ingestion_accepts_supplied_bindings_without_applying_metadata_overrides() {
    let input = repo_device_plan_planner_input_with_bindings(
        "ayaneo.pocket_s2.base",
        &[
            (
                "feature.copy_bios/bios_source_dir",
                json!("/tmp/emuchef-p7i-bios"),
            ),
            (
                "app.xaniteog.install/xaniteog_apk",
                json!("/tmp/emuchef-p7i-xaniteog.apk"),
            ),
        ],
    );

    assert_eq!(
        input.selected_recipe_refs,
        vec![
            "app.retroarch.provision".to_string(),
            "feature.copy_bios".to_string(),
            "app.xaniteog.install".to_string(),
        ]
    );
    assert_eq!(
        input
            .input_bindings
            .get("feature.copy_bios/bios_source_dir"),
        Some(&json!("/tmp/emuchef-p7i-bios"))
    );
    assert_eq!(
        input
            .input_bindings
            .get("app.xaniteog.install/xaniteog_apk"),
        Some(&json!("/tmp/emuchef-p7i-xaniteog.apk"))
    );
    assert!(
        !input.input_bindings.contains_key("config_variants"),
        "metadata-only device plan overrides must not become bindings"
    );
}

#[test]
fn repo_device_plan_defaults_and_config_variants_are_checked_in_metadata_only() {
    let mut actual = Vec::new();
    for plan in discover_device_plan_inventory(&repo_authored_root())
        .expect("repo device plan inventory should parse")
    {
        let yaml =
            fs::read_to_string(&plan.path).expect("checked-in device plan should be readable");
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&yaml).expect("checked-in device plan should be valid YAML");

        actual.push((
            plan.id.clone(),
            parsed["defaults"]["show_advanced_steps"]
                .as_bool()
                .expect("checked-in default should be a bool"),
            parsed["overrides"]["config_variants"]["vendor_family"]
                .as_str()
                .expect("checked-in vendor family should be a string")
                .to_string(),
            parsed["overrides"]["config_variants"]["screen_class"]
                .as_str()
                .expect("checked-in screen class should be a string")
                .to_string(),
        ));

        let input = repo_device_plan_planner_input_with_bindings(&plan.id, &[]);
        assert!(
            !input.input_bindings.contains_key("show_advanced_steps"),
            "{} defaults.show_advanced_steps must not become a planner binding",
            plan.id
        );
        assert!(
            !input.input_bindings.contains_key("config_variants"),
            "{} overrides.config_variants must not become a planner binding",
            plan.id
        );
    }

    assert_eq!(
        actual,
        vec![
            (
                "ayaneo.generic.base".to_string(),
                false,
                "ayaneo".to_string(),
                "handheld_16_9".to_string(),
            ),
            (
                "ayaneo.konkr_pocket_fit.base".to_string(),
                false,
                "ayaneo".to_string(),
                "handheld_16_9".to_string(),
            ),
            (
                "ayaneo.pocket_air_mini.base".to_string(),
                false,
                "ayaneo".to_string(),
                "handheld_4_3".to_string(),
            ),
            (
                "ayaneo.pocket_s2.base".to_string(),
                false,
                "ayaneo".to_string(),
                "handheld_16_9".to_string(),
            ),
            (
                "ayaneo.pocket_s_mini.base".to_string(),
                false,
                "ayaneo".to_string(),
                "handheld_16_9".to_string(),
            ),
        ]
    );
}

#[test]
fn temp_device_plan_override_bindings_merge_before_explicit_bindings() {
    let root = temp_authored_root_with_device_plan_selection_yaml(
        r#"
schema_version: 1
kind: device_plan
id: override.binding_merge
name: Override Binding Merge
device_profile_ref: ayaneo.konkr_pocket_fit
recipes:
  - recipe_ref: app.retroarch.provision
    selected_by_default: true
  - recipe_ref: feature.copy_bios
    selected_by_default: true
  - recipe_ref: app.xaniteog.install
    selected_by_default: true
defaults:
  feature.copy_bios/bios_source_dir: /tmp/default-bios
overrides:
  feature.copy_bios/bios_source_dir: /tmp/override-bios
  config_variants:
    vendor_family: test
    screen_class: test_screen
  app.xaniteog.install/xaniteog_apk: /tmp/override-xaniteog.apk
metadata: {}
"#,
    );
    let mut explicit = OrderedMap::new();
    explicit.insert(
        "feature.copy_bios/bios_source_dir".to_string(),
        json!("/tmp/explicit-bios"),
    );
    explicit.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        json!("/tmp/explicit-retroarch.cfg"),
    );

    let input = PlannerInput::from_authored_device_plan(
        root.path().join("authored"),
        "override.binding_merge",
        "plan.p7j.binding_merge.001".to_string(),
        explicit,
    )
    .expect("ref-shaped override bindings should merge into private planner input");

    assert_eq!(
        input
            .input_bindings
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "feature.copy_bios/bios_source_dir",
            "app.xaniteog.install/xaniteog_apk",
            "app.retroarch.provision/retroarch_cfg",
        ]
    );
    assert_eq!(
        input
            .input_bindings
            .get("feature.copy_bios/bios_source_dir"),
        Some(&json!("/tmp/explicit-bios"))
    );
    assert_eq!(
        input
            .input_bindings
            .get("app.xaniteog.install/xaniteog_apk"),
        Some(&json!("/tmp/override-xaniteog.apk"))
    );
    assert_eq!(
        input
            .input_bindings
            .get("app.retroarch.provision/retroarch_cfg"),
        Some(&json!("/tmp/explicit-retroarch.cfg"))
    );
    assert!(
        !input.input_bindings.contains_key("config_variants"),
        "config variant metadata must not become a binding"
    );
}

#[test]
fn temp_device_plan_defaults_remain_inactive_even_when_ref_shaped() {
    let root = temp_authored_root_with_device_plan_selection_yaml(
        r#"
schema_version: 1
kind: device_plan
id: defaults.inactive
name: Defaults Inactive
device_profile_ref: ayaneo.konkr_pocket_fit
recipes:
  - recipe_ref: feature.copy_bios
    selected_by_default: true
defaults:
  feature.copy_bios/bios_source_dir: /tmp/default-bios
overrides: {}
metadata: {}
"#,
    );

    let input = PlannerInput::from_authored_device_plan(
        root.path().join("authored"),
        "defaults.inactive",
        "plan.p7j.defaults_inactive.001".to_string(),
        OrderedMap::new(),
    )
    .expect("ref-shaped defaults should stay inactive in P7J");

    assert!(
        input.input_bindings.is_empty(),
        "device_plan.defaults must not populate Rust planner bindings in P7J"
    );
}

#[test]
fn temp_device_plan_override_keys_are_strictly_classified() {
    let cases = [
        (
            "unsupported.metadata",
            "show_advanced_steps: false",
            "device_plan_override_unsupported",
            "show_advanced_steps",
        ),
        (
            "empty.recipe",
            "/bios_source_dir: /tmp/bios",
            "device_plan_override_malformed",
            "empty recipe",
        ),
        (
            "empty.input",
            "feature.copy_bios/: /tmp/bios",
            "device_plan_override_malformed",
            "empty input",
        ),
        (
            "too.many.slashes",
            "feature.copy_bios/bios/source_dir: /tmp/bios",
            "device_plan_override_malformed",
            "exactly one slash",
        ),
        (
            "unknown.recipe",
            "missing.recipe/bios_source_dir: /tmp/bios",
            "device_plan_override_unknown_binding",
            "missing.recipe",
        ),
        (
            "unknown.input",
            "feature.copy_bios/missing_input: /tmp/bios",
            "device_plan_override_unknown_binding",
            "missing_input",
        ),
    ];

    for (id_suffix, override_yaml, expected_code, expected_message) in cases {
        let plan_id = format!("override.{id_suffix}");
        let root = temp_authored_root_with_device_plan_selection_yaml(&format!(
            r#"
schema_version: 1
kind: device_plan
id: {plan_id}
name: Override Strict Classification
device_profile_ref: ayaneo.konkr_pocket_fit
recipes:
  - recipe_ref: feature.copy_bios
    selected_by_default: true
defaults: {{}}
overrides:
  {override_yaml}
metadata: {{}}
"#
        ));

        let err = match PlannerInput::from_authored_device_plan(
            root.path().join("authored"),
            &plan_id,
            "plan.p7j.strict_override.001".to_string(),
            OrderedMap::new(),
        ) {
            Ok(_) => panic!("{override_yaml} should be classified as an ingestion error"),
            Err(error) => error,
        };
        assert_eq!(err.code(), expected_code, "{override_yaml}");
        assert!(
            err.to_string().contains(expected_message),
            "{override_yaml}: {}",
            err
        );
    }
}

#[test]
fn repo_device_plan_profile_context_can_plan_successfully_without_python_or_devices() {
    let first = planning_result_value(repo_device_plan_planner_input_with_bindings(
        "ayaneo.pocket_s_mini.base",
        &[(
            "app.retroarch.provision/retroarch_cfg",
            json!("/tmp/emuchef-p7i-retroarch.cfg"),
        )],
    ));
    let second = planning_result_value(repo_device_plan_planner_input_with_bindings(
        "ayaneo.pocket_s_mini.base",
        &[(
            "app.retroarch.provision/retroarch_cfg",
            json!("/tmp/emuchef-p7i-retroarch.cfg"),
        )],
    ));

    assert_eq!(first, second);
    assert_planning_result_shape("repo device plan/profile context", &first);
    assert_eq!(first["status"], "success", "{first:#}");
    assert_eq!(
        first["execution_plan"]["source"]["device_plan_ref"],
        "ayaneo.pocket_s_mini.base"
    );
    assert_eq!(
        first["execution_plan"]["source"]["device_profile_ref"],
        "ayaneo.pocket_s_mini"
    );
    assert_eq!(
        first["execution_plan"]["source"]["selected_recipe_refs"],
        json!(["app.retroarch.provision"])
    );
    assert_eq!(
        execution_input_ids(&first),
        vec!["app.retroarch.provision/retroarch_cfg"]
    );
    let step_ids = execution_step_ids(&first);
    assert!(
        step_ids.contains(&"app.retroarch.provision/seed_retroarch_cfg"),
        "{step_ids:#?}"
    );
    assert!(!serde_json::to_string(&first)
        .expect("planning result should serialize")
        .contains("permission_plan"));
}

#[test]
fn repo_plan_e2e_from_checked_in_device_plan_succeeds() {
    for case in repo_plan_e2e_cases() {
        let temp = TempDir::new().expect("repo plan e2e temp root should be created");
        let actual = planning_result_value(repo_plan_e2e_input(&case, &temp));

        assert_eq!(
            actual["status"], "success",
            "{}: {actual:#}",
            case.device_plan_ref
        );
        assert_planning_result_shape(case.device_plan_ref, &actual);
        assert_eq!(
            actual["execution_plan"]["source"]["device_plan_ref"],
            case.device_plan_ref
        );
        assert_eq!(
            actual["execution_plan"]["source"]["device_profile_ref"],
            case.device_profile_ref
        );
        assert_eq!(
            actual["execution_plan"]["source"]["selected_recipe_refs"],
            json!(case.selected_recipe_refs)
        );
        assert_eq!(
            actual["execution_plan"]["source"]["expanded_recipe_refs"],
            json!(case.selected_recipe_refs),
            "{} currently has no non-empty checked-in recipe dependency closure",
            case.device_plan_ref
        );
    }
}

#[test]
fn repo_plan_e2e_selected_and_expanded_refs_are_deterministic() {
    for case in repo_plan_e2e_cases() {
        let temp = TempDir::new().expect("repo plan e2e temp root should be created");
        let first = planning_result_value(repo_plan_e2e_input(&case, &temp));
        let second = planning_result_value(repo_plan_e2e_input(&case, &temp));

        assert_eq!(
            first, second,
            "{} should emit the same private planner result for repeated runs with the same bindings",
            case.device_plan_ref
        );
        assert_eq!(
            first["execution_plan"]["source"]["selected_recipe_refs"],
            json!(case.selected_recipe_refs)
        );
        assert_eq!(
            first["execution_plan"]["source"]["expanded_recipe_refs"],
            json!(case.selected_recipe_refs)
        );
        assert_eq!(
            execution_step_ids(&first),
            execution_step_ids(&second),
            "{} execution step order should be deterministic",
            case.device_plan_ref
        );
    }
}

#[test]
fn repo_plan_e2e_normalized_steps_reflect_prior_p7_slices() {
    let temp = TempDir::new().expect("repo plan e2e temp root should be created");
    let actual = planning_result_value(repo_plan_e2e_input_for_device_plan(
        "ayaneo.pocket_s_mini.base",
        &temp,
        P7L_NO_REQUIRED_BINDINGS,
    ));

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert!(!actual["execution_plan"]
        .as_object()
        .expect("repo plan e2e should include execution_plan")
        .contains_key("permission_plan"));
    let step_ids = execution_step_ids(&actual);
    assert!(
        !step_ids.contains(&"app.retroarch.provision/seed_retroarch_cfg"),
        "optional RetroArch config step should be pruned when retroarch_cfg is unbound: {step_ids:#?}"
    );
    assert_step_before(
        &step_ids,
        "app.retroarch.provision/extract_assets",
        "app.retroarch.provision/copy_assets",
    );
    assert_step_before(
        &step_ids,
        "app.retroarch.provision/copy_assets",
        "app.retroarch.provision/launch_retroarch",
    );

    let resolve = planner_step(&actual, "app.retroarch.provision/resolve_artifacts");
    assert_object_keys(
        "repo plan e2e resolve params",
        resolve["params"].as_object().unwrap(),
        &["artifacts"],
    );
    let resolved_artifacts = string_array(
        &resolve["params"]["artifacts"]["value"],
        "repo plan e2e resolve artifacts",
    );
    assert_eq!(resolved_artifacts.len(), 24);
    assert_eq!(
        resolved_artifacts[0],
        "app.retroarch.provision/retroarch_apk"
    );
    for expected in [
        "app.retroarch.provision/asset_assets_zip",
        "app.retroarch.provision/core_ppsspp_zip",
        "app.retroarch.provision/core_files_ppsspp_zip",
    ] {
        assert!(
            resolved_artifacts.contains(&expected),
            "resolved artifacts should include {expected}: {resolved_artifacts:#?}"
        );
    }

    let extract_cores = planner_step(&actual, "app.retroarch.provision/extract_cores");
    assert_object_keys(
        "repo plan e2e extract_cores params",
        extract_cores["params"].as_object().unwrap(),
        &["artifacts", "extract_on"],
    );
    assert_eq!(
        extract_cores["params"]["extract_on"],
        json!({"value": "host"})
    );
    assert_eq!(
        string_array(
            &extract_cores["params"]["artifacts"]["value"],
            "repo plan e2e extracted core artifacts",
        )
        .len(),
        13
    );

    let extract_assets = planner_step(&actual, "app.retroarch.provision/extract_assets");
    assert_eq!(
        extract_assets["params"]["archive"],
        json!({"ref": "artifacts.app.retroarch.provision/asset_assets_zip.local_path"})
    );
    assert_eq!(extract_assets["params"]["cleanup"], json!({"value": true}));

    let copy_assets = planner_step(&actual, "app.retroarch.provision/copy_assets");
    assert_eq!(
        copy_assets["dependencies"],
        json!([
            "app.retroarch.provision/stop_retroarch_after_permissions",
            "app.retroarch.provision/extract_assets"
        ])
    );
    assert_eq!(
        copy_assets["params"]["source"],
        json!({"ref": "steps.app.retroarch.provision/extract_assets.outputs.extracted_path"})
    );
    assert_eq!(
        copy_assets["params"]["copy_policy"],
        json!({"value": "merge"})
    );

    let install = planner_step(&actual, "app.retroarch.provision/install_retroarch");
    assert_eq!(
        install["params"]["replace_existing"],
        json!({"value": false})
    );

    let config_temp = TempDir::new().expect("repo plan e2e config temp root should be created");
    let bound_config = planning_result_value(repo_plan_e2e_input_for_device_plan(
        "ayaneo.pocket_s_mini.base",
        &config_temp,
        P7L_RETROARCH_CFG_BINDINGS,
    ));
    assert_eq!(bound_config["status"], "success", "{bound_config:#}");
    assert_eq!(
        execution_input_ids(&bound_config),
        vec!["app.retroarch.provision/retroarch_cfg"]
    );
    let bound_step_ids = execution_step_ids(&bound_config);
    assert!(
        bound_step_ids.contains(&"app.retroarch.provision/seed_retroarch_cfg"),
        "bound retroarch_cfg should include the seed step: {bound_step_ids:#?}"
    );
    let seed = planner_step(&bound_config, "app.retroarch.provision/seed_retroarch_cfg");
    assert_eq!(
        seed["params"]["source"],
        json!({"ref": "inputs.app.retroarch.provision/retroarch_cfg"})
    );
    assert_eq!(seed["params"]["copy_policy"], json!({"value": "replace"}));
}

#[test]
fn repo_plan_e2e_requires_only_explicit_external_bindings_for_unbound_inputs() {
    for case in repo_plan_e2e_cases() {
        assert!(
            case.required_bindings.is_empty(),
            "{} should not need required external bindings in the current P7L success set",
            case.device_plan_ref
        );
        let temp = TempDir::new().expect("repo plan e2e temp root should be created");
        let actual = planning_result_value(repo_plan_e2e_input(&case, &temp));

        assert_eq!(
            actual["status"], "success",
            "{}: {actual:#}",
            case.device_plan_ref
        );
        assert_eq!(
            execution_input_ids(&actual),
            Vec::<&str>::new(),
            "{} should not synthesize optional input bindings",
            case.device_plan_ref
        );
    }
}

#[test]
fn repo_plan_e2e_checked_in_context_gaps_are_classified_without_expanding_scope() {
    for case in repo_plan_e2e_gap_cases() {
        let temp = TempDir::new().expect("repo plan e2e temp root should be created");
        let actual = planning_result_value(repo_plan_e2e_input(&case, &temp));

        assert_eq!(
            actual["status"], "error",
            "{}: {actual:#}",
            case.device_plan_ref
        );
        assert_eq!(actual["execution_plan"], Value::Null, "{actual:#}");
        let errors = actual["errors"]
            .as_array()
            .expect("gap result should include planner errors");
        assert!(
            errors.iter().all(|error| error["code"] == "unknown_step_dependency"),
            "{} should remain a current Rust planner dependency/capability gap, not a binding or executor path: {actual:#}",
            case.device_plan_ref
        );
        assert!(
            errors.iter().any(|error| {
                error["details"]["recipe_ref"] == "app.retroarch.provision"
                    && error["details"]["step_id"] == "launch_retroarch"
                    && error["details"]["dependency"] == "copy_assets"
            }),
            "{} should document the missing app-data copy dependency gap: {actual:#}",
            case.device_plan_ref
        );
    }
}

#[test]
fn repo_device_plan_ingestion_errors_are_classified_and_deterministic() {
    let missing_plan = PlannerInput::from_authored_device_plan(
        repo_authored_root(),
        "missing.device_plan",
        "plan.p7i.missing.001".to_string(),
        OrderedMap::new(),
    )
    .expect_err("missing device plan should be classified");
    assert_eq!(missing_plan.code(), "device_plan_not_found");
    assert!(missing_plan
        .to_string()
        .contains("Unknown device plan 'missing.device_plan'"));

    let missing_profile_root = temp_authored_root_with_device_plan(
        "broken.missing_profile",
        "missing.profile",
        &[("app.retroarch.provision", true)],
    );
    let first = PlannerInput::from_authored_device_plan(
        missing_profile_root.path().join("authored"),
        "broken.missing_profile",
        "plan.p7i.missing_profile.001".to_string(),
        OrderedMap::new(),
    )
    .expect_err("missing profile should be classified");
    let second = PlannerInput::from_authored_device_plan(
        missing_profile_root.path().join("authored"),
        "broken.missing_profile",
        "plan.p7i.missing_profile.001".to_string(),
        OrderedMap::new(),
    )
    .expect_err("missing profile should be deterministic");
    assert_eq!(first, second);
    assert_eq!(first.code(), "device_profile_not_found");

    let missing_recipe_root = temp_authored_root_with_device_plan(
        "broken.missing_recipe",
        "ayaneo.konkr_pocket_fit",
        &[("missing.recipe", true)],
    );
    let err = PlannerInput::from_authored_device_plan(
        missing_recipe_root.path().join("authored"),
        "broken.missing_recipe",
        "plan.p7i.missing_recipe.001".to_string(),
        OrderedMap::new(),
    )
    .expect_err("missing recipe should be classified");
    assert_eq!(err.code(), "recipe_not_found");
    assert!(err.to_string().contains("missing.recipe"));

    let missing_selected_flag_root = temp_authored_root_with_device_plan_selection_yaml(
        r#"
schema_version: 1
kind: device_plan
id: broken.missing_selected_flag
name: Broken Missing Selected Flag
device_profile_ref: ayaneo.konkr_pocket_fit
recipes:
  - recipe_ref: app.retroarch.provision
defaults: {}
overrides: {}
metadata: {}
"#,
    );
    let err = PlannerInput::from_authored_device_plan(
        missing_selected_flag_root.path().join("authored"),
        "broken.missing_selected_flag",
        "plan.p7i.missing_selected_flag.001".to_string(),
        OrderedMap::new(),
    )
    .expect_err("selected_by_default is required by current Python parser semantics");
    assert_eq!(err.code(), "authored_data_invalid");
    assert!(err.to_string().contains("selected_by_default"));
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
    let mut params = params;
    if step_type == "extract_artifacts"
        && !params.contains_key("artifacts")
        && !params.contains_key("artifact_groups")
    {
        params.insert(
            "artifacts".to_string(),
            ParamValue::Literal(json!(["app_apk"])),
        );
    }
    if step_type == "copy_files" && !params.contains_key("dest") {
        params.insert(
            "dest".to_string(),
            ParamValue::Literal(json!("/sdcard/Copy")),
        );
    }

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

fn recipe_expansion_input(recipes: Vec<Recipe>, selected_recipe_refs: Vec<&str>) -> PlannerInput {
    PlannerInput {
        recipes,
        selected_recipe_refs: selected_recipe_refs
            .into_iter()
            .map(ToString::to_string)
            .collect(),
        input_bindings: OrderedMap::new(),
        plan_id: "plan.recipe_expansion.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
    }
}

fn recipe_expansion_recipe(id: &str, recipe_dependencies: Vec<&str>) -> Recipe {
    Recipe {
        schema_version: 1,
        kind: "recipe".to_string(),
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        recipe_dependencies: recipe_dependencies
            .into_iter()
            .map(ToString::to_string)
            .collect(),
        provides: RecipeProvides {
            features: Vec::new(),
        },
        inputs: OrderedMap::new(),
        artifacts: OrderedMap::new(),
        artifact_groups: OrderedMap::new(),
        steps: vec![recipe_expansion_wait_step()],
    }
}

fn recipe_expansion_wait_step() -> Step {
    let mut params = OrderedMap::new();
    params.insert("duration_ms".to_string(), ParamValue::Literal(json!(1)));
    Step {
        id: "wait".to_string(),
        type_name: "wait".to_string(),
        name: "Wait".to_string(),
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
    }
}

fn assert_recipe_expansion_error<'a>(actual: &'a Value, expected_code: &str) -> &'a Value {
    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null, "{actual:#}");
    let errors = actual["errors"]
        .as_array()
        .expect("recipe expansion error result should include errors");
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_eq!(errors[0]["code"], expected_code, "{errors:#?}");
    &errors[0]
}

fn p7k_corpus_recipe_expansion_bindings() -> Vec<(&'static str, Value)> {
    vec![
        (
            "app.retroarch.provision/retroarch_cfg",
            json!("/tmp/emuchef-p7k-retroarch.cfg"),
        ),
        (
            "app.xaniteog.install/xaniteog_apk",
            json!("/tmp/emuchef-p7k-xaniteog.apk"),
        ),
        (
            "feature.copy_bios/bios_source_dir",
            json!("/tmp/emuchef-p7k-bios"),
        ),
    ]
}

fn param_contract_input(steps: Vec<Step>) -> PlannerInput {
    PlannerInput {
        recipes: vec![Recipe {
            schema_version: 1,
            kind: "recipe".to_string(),
            id: "planner.param_contract".to_string(),
            name: "Planner Param Contract".to_string(),
            description: None,
            recipe_dependencies: Vec::new(),
            provides: RecipeProvides {
                features: Vec::new(),
            },
            inputs: OrderedMap::new(),
            artifacts: vec![
                (
                    "app_apk".to_string(),
                    RemoteFileArtifact {
                        type_name: "remote_file".to_string(),
                        url: "https://example.com/app.apk".to_string(),
                        cache: "default".to_string(),
                    },
                ),
                (
                    "archive_zip".to_string(),
                    RemoteFileArtifact {
                        type_name: "remote_file".to_string(),
                        url: "https://example.com/archive.zip".to_string(),
                        cache: "default".to_string(),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            artifact_groups: vec![("archives".to_string(), vec!["archive_zip".to_string()])]
                .into_iter()
                .collect(),
            steps,
        }],
        selected_recipe_refs: vec!["planner.param_contract".to_string()],
        input_bindings: OrderedMap::new(),
        plan_id: "plan.param_contract.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
    }
}

fn param_contract_step(
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
            capabilities: Vec::new(),
            conflicts_with: Vec::new(),
        },
        skip_if: Vec::<StepCondition>::new(),
        params,
        verify: Vec::<StepCondition>::new(),
    }
}

fn permission_intent_steps(steps: Vec<Step>) -> Vec<(String, String, Step)> {
    steps
        .into_iter()
        .map(|step| {
            (
                format!("planner.permission_intent/{}", step.id),
                "planner.permission_intent".to_string(),
                step,
            )
        })
        .collect()
}

fn grant_empty_permission_step(id: &str) -> Step {
    param_contract_step(id, "grant_permissions", vec![], OrderedMap::new())
}

fn param_contract_step_ids(actual: &Value) -> Vec<&str> {
    actual["execution_plan"]["steps"]
        .as_array()
        .expect("execution plan should include steps")
        .iter()
        .map(|step| step["id"].as_str().unwrap())
        .collect()
}

fn execution_step_ids(actual: &Value) -> Vec<&str> {
    actual["execution_plan"]["steps"]
        .as_array()
        .expect("execution plan should include steps")
        .iter()
        .map(|step| step["id"].as_str().unwrap())
        .collect()
}

fn execution_input_ids(actual: &Value) -> Vec<&str> {
    actual["execution_plan"]["inputs"]
        .as_array()
        .expect("execution plan should include inputs")
        .iter()
        .map(|input| input["id"].as_str().unwrap())
        .collect()
}

fn string_array<'a>(actual: &'a Value, name: &str) -> Vec<&'a str> {
    actual
        .as_array()
        .unwrap_or_else(|| panic!("{name} should be an array"))
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{name} items should be strings"))
        })
        .collect()
}

fn assert_step_before(step_ids: &[&str], earlier: &str, later: &str) {
    let earlier_index = step_ids
        .iter()
        .position(|step_id| *step_id == earlier)
        .unwrap_or_else(|| panic!("{earlier} should be present in {step_ids:#?}"));
    let later_index = step_ids
        .iter()
        .position(|step_id| *step_id == later)
        .unwrap_or_else(|| panic!("{later} should be present in {step_ids:#?}"));
    assert!(
        earlier_index < later_index,
        "{earlier} should appear before {later}: {step_ids:#?}"
    );
}

fn param_contract_step_param<'a>(actual: &'a Value, step_id: &str, param: &str) -> &'a Value {
    let execution_step_id = format!("planner.param_contract/{step_id}");
    &actual["execution_plan"]["steps"]
        .as_array()
        .expect("execution plan should include steps")
        .iter()
        .find(|step| step["id"] == execution_step_id)
        .expect("param contract step should exist")["params"][param]
}

fn planner_step<'a>(actual: &'a Value, execution_step_id: &str) -> &'a Value {
    actual["execution_plan"]["steps"]
        .as_array()
        .expect("execution plan should include steps")
        .iter()
        .find(|step| step["id"] == execution_step_id)
        .expect("planner execution step should exist")
}

fn assert_param_contract_error(
    actual: &Value,
    code: &str,
    step_id: &str,
    param: &str,
    expected: Value,
    actual_value: Value,
) {
    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null);
    let errors = actual["errors"]
        .as_array()
        .expect("planner result should include errors");
    assert!(
        errors.iter().any(|error| {
            error["code"] == code
                && error["details"]["recipe_ref"] == "planner.param_contract"
                && error["details"]["step_id"] == step_id
                && error["details"]["step_type"].is_string()
                && error["details"]["param"] == param
                && error["details"]["expected"] == expected
                && error["details"]["actual"] == actual_value
                && error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(param))
        }),
        "expected param contract error code={code} step_id={step_id} param={param} expected={expected} actual={actual_value}; actual errors: {errors:#?}",
    );
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

struct AuthoredCorpusRecipeEntry {
    path: &'static str,
    recipe_id: &'static str,
}

fn authored_corpus_recipe_inventory() -> &'static [AuthoredCorpusRecipeEntry] {
    &[
        AuthoredCorpusRecipeEntry {
            path: "authored/recipes/app.obtainium.install.yaml",
            recipe_id: "app.obtainium.install",
        },
        AuthoredCorpusRecipeEntry {
            path: "authored/recipes/app.retroarch.provision.yaml",
            recipe_id: "app.retroarch.provision",
        },
        AuthoredCorpusRecipeEntry {
            path: "authored/recipes/app.xaniteog.install.yaml",
            recipe_id: "app.xaniteog.install",
        },
        AuthoredCorpusRecipeEntry {
            path: "authored/recipes/feature.copy_bios.yaml",
            recipe_id: "feature.copy_bios",
        },
    ]
}

fn discovered_authored_corpus_recipe_paths() -> Vec<String> {
    let root = repo_root();
    let mut paths = fs::read_dir(repo_authored_recipes_dir())
        .expect("repo authored recipes directory should be readable")
        .map(|entry| {
            entry
                .expect("repo authored recipe entry should be readable")
                .path()
        })
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| matches!(extension, "yaml" | "yml"))
        })
        .map(|path| {
            path.strip_prefix(&root)
                .expect("authored corpus recipe should live under repo root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn repo_relative_path(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .expect("repo inventory path should live under repo root")
        .to_string_lossy()
        .replace('\\', "/")
}

fn temp_authored_root_with_device_plan(
    device_plan_id: &str,
    device_profile_ref: &str,
    selections: &[(&str, bool)],
) -> TempDir {
    let recipes = selections
        .iter()
        .map(|(recipe_ref, selected)| {
            format!("  - recipe_ref: {recipe_ref}\n    selected_by_default: {selected}\n")
        })
        .collect::<String>();
    temp_authored_root_with_device_plan_selection_yaml(&format!(
        r#"
schema_version: 1
kind: device_plan
id: {device_plan_id}
name: Test Device Plan
device_profile_ref: {device_profile_ref}
recipes:
{recipes}defaults: {{}}
overrides: {{}}
metadata: {{}}
"#
    ))
}

fn temp_authored_root_with_device_plan_selection_yaml(device_plan_yaml: &str) -> TempDir {
    let temp = TempDir::new().expect("temp authored root should be created");
    let authored_root = temp.path().join("authored");
    for directory in ["apps", "recipes", "device_profiles", "device_plans"] {
        fs::create_dir_all(authored_root.join(directory))
            .expect("temp authored directory should be created");
    }

    for entry in authored_corpus_recipe_inventory() {
        let source = repo_root().join(entry.path);
        let target = authored_root.join(entry.path.strip_prefix("authored/").unwrap());
        fs::copy(&source, &target).unwrap_or_else(|error| {
            panic!(
                "should copy {} to {}: {error}",
                source.display(),
                target.display()
            )
        });
    }
    fs::copy(
        repo_root().join("authored/device_profiles/ayaneo.konkr_pocket_fit.yaml"),
        authored_root.join("device_profiles/ayaneo.konkr_pocket_fit.yaml"),
    )
    .expect("profile fixture should copy");
    fs::write(
        authored_root.join("device_plans/test_device_plan.yaml"),
        device_plan_yaml.trim_start(),
    )
    .expect("temp device plan should be written");

    temp
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
    assert_planning_result_shape(name, parsed);
}

fn assert_planning_result_shape(name: &str, parsed: &Value) {
    let parsed_object = parsed
        .as_object()
        .expect("planner result should be a JSON object");
    assert_object_keys(
        name,
        parsed_object,
        &[
            "status",
            "warnings",
            "errors",
            "execution_plan",
            "schema_version",
            "kind",
        ],
    );
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
    assert_planner_message_array_shape(name, "warnings", &parsed["warnings"]);
    let errors = parsed["errors"]
        .as_array()
        .expect("planner golden should have an errors array");
    assert_planner_message_array_shape(name, "errors", &parsed["errors"]);

    match parsed["status"].as_str() {
        Some("success" | "warning") => match &parsed["execution_plan"] {
            Value::Object(plan) => assert_execution_plan_shape(name, plan),
            _ => panic!("{name} should include an execution_plan object for non-error status"),
        },
        Some("error") => {
            assert_eq!(
                parsed["execution_plan"],
                Value::Null,
                "{name} error results should not include an execution_plan"
            );
            assert!(
                !errors.is_empty(),
                "{name} should include errors when execution_plan is null"
            );
        }
        _ => panic!("{name} should have a valid planning status"),
    }
}

fn assert_execution_plan_shape(name: &str, plan: &serde_json::Map<String, Value>) {
    assert_object_keys(
        &format!("{name}.execution_plan"),
        plan,
        &[
            "id",
            "source",
            "device_context",
            "runtime_capabilities",
            "inputs",
            "artifacts",
            "steps",
            "schema_version",
            "kind",
        ],
    );
    assert!(
        !plan.contains_key("permission_plan"),
        "{name} execution_plan should not serialize permission_plan"
    );
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
    assert_object_keys(
        &format!("{name}.execution_plan.source"),
        source,
        &[
            "device_profile_ref",
            "device_plan_ref",
            "selected_recipe_refs",
            "expanded_recipe_refs",
        ],
    );
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
    assert!(
        plan.get("device_context")
            .and_then(Value::as_object)
            .is_some(),
        "{name} execution_plan.device_context should be an object"
    );
    assert!(
        plan.get("runtime_capabilities")
            .and_then(Value::as_object)
            .is_some(),
        "{name} execution_plan.runtime_capabilities should be an object"
    );
    assert!(
        plan.get("inputs").and_then(Value::as_array).is_some(),
        "{name} execution_plan.inputs should be an array"
    );
    assert!(
        plan.get("artifacts").and_then(Value::as_array).is_some(),
        "{name} execution_plan.artifacts should be an array"
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
        assert_object_keys(
            &format!("{name}.execution_plan.steps[{index}]"),
            step,
            &[
                "id",
                "recipe_ref",
                "type",
                "name",
                "dependencies",
                "constraints",
                "params",
                "skip_if",
                "verify",
            ],
        );
        assert_non_empty_string(name, step.get("id"), &format!("steps[{index}].id"));
        assert_non_empty_string(
            name,
            step.get("recipe_ref"),
            &format!("steps[{index}].recipe_ref"),
        );
        assert_non_empty_string(name, step.get("type"), &format!("steps[{index}].type"));
        assert_non_empty_string(name, step.get("name"), &format!("steps[{index}].name"));
        assert!(
            step.get("dependencies").and_then(Value::as_array).is_some(),
            "{name} steps[{index}].dependencies should be an array"
        );
        let constraints = step
            .get("constraints")
            .and_then(Value::as_object)
            .expect("planner execution_plan step constraints should be an object");
        assert_object_keys(
            &format!("{name}.execution_plan.steps[{index}].constraints"),
            constraints,
            &["capabilities", "conflicts_with"],
        );
        assert!(
            constraints
                .get("capabilities")
                .and_then(Value::as_array)
                .is_some(),
            "{name} steps[{index}].constraints.capabilities should be an array"
        );
        assert!(
            constraints
                .get("conflicts_with")
                .and_then(Value::as_array)
                .is_some(),
            "{name} steps[{index}].constraints.conflicts_with should be an array"
        );
        assert!(
            step.get("params").and_then(Value::as_object).is_some(),
            "{name} steps[{index}].params should be an object"
        );
        assert!(
            step.get("skip_if").and_then(Value::as_array).is_some(),
            "{name} steps[{index}].skip_if should be an array"
        );
        assert!(
            step.get("verify").and_then(Value::as_array).is_some(),
            "{name} steps[{index}].verify should be an array"
        );
    }
}

fn assert_planner_message_array_shape(name: &str, field: &str, messages: &Value) {
    let messages = messages
        .as_array()
        .expect("planner message collection should be an array");
    for (index, message) in messages.iter().enumerate() {
        let message = message
            .as_object()
            .expect("planner message should be an object");
        let context = format!("{name}.{field}[{index}]");
        assert_object_keys(&context, message, &["code", "message", "details"]);
        assert_non_empty_string(&context, message.get("code"), "code");
        assert_non_empty_string(&context, message.get("message"), "message");
        assert!(
            message.get("details").and_then(Value::as_object).is_some(),
            "{context}.details should be an object"
        );
    }
}

fn assert_object_keys(name: &str, object: &serde_json::Map<String, Value>, expected: &[&str]) {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "{name} should have expected key set");
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
