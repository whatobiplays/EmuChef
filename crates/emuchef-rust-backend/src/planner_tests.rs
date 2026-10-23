use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tempfile::TempDir;

use crate::authored_models::{
    load_app_definition, AppArtifactKind, AppArtifactSource, AppArtifactV1,
};
use crate::device_probe::{DetectedDeviceFacts, DeviceProbe, FakeDeviceProbe};
use crate::device_profile_match::build_detected_device_profile_mismatch_warning;
use crate::model::{
    AppArtifactInputBinding, AppArtifactReference, InputDeclaration, InputValidation, OrderedMap,
    ParamValue, Recipe, RecipeArtifact, RecipeProvides, RemoteFileArtifact, Step, StepCondition,
    StepConstraints,
};
use crate::planner::{
    normalized_plan_step_note, plan_execution_with_app_definitions, resolve_runtime_bindings,
    BindingSource, DeviceContext, PlannerInput, PlanningStatus, RuntimeCapabilities,
};
use crate::runtime_configuration::input_reuse_diagnostics;

#[test]
fn progress_note_fallback_order_is_deterministic() {
    assert_eq!(
        normalized_plan_step_note(
            Some("  Copying files  "),
            "Copy Files",
            "copy_files",
            "copy"
        ),
        "Copying files"
    );
    assert_eq!(
        normalized_plan_step_note(None, " Copy Files ", "copy_files", "copy"),
        "Copy Files"
    );
    assert_eq!(
        normalized_plan_step_note(None, "", "grant_permissions", "grant"),
        "Grant permissions"
    );
    assert_eq!(
        normalized_plan_step_note(None, "", "", "fallback-id"),
        "fallback-id"
    );
}
use crate::planner_device_plan::{
    discover_device_plan_inventory, discover_device_profile_inventory,
    load_device_plan_profile_match_criteria, plan_from_authored_device_plan_with_detected_facts,
    planner_input_from_authored_device_plan_with_detected_facts,
};

const RETROARCH_ONLY_REFS: &[&str] = &["app.retroarch.provision"];
const RETROARCH_BIOS_REFS: &[&str] = &["app.retroarch.provision", "feature.copy_bios"];
const RETROARCH_BIOS_XANITEOG_REFS: &[&str] = &[
    "app.retroarch.provision",
    "feature.copy_bios",
    "app.xaniteog.install",
];
const NO_REQUIRED_BINDINGS: &[RepoPlanE2eBinding] = &[];
const BIOS_BINDINGS: &[RepoPlanE2eBinding] = &[RepoPlanE2eBinding::BiosSourceDir];
const BIOS_XANITEOG_BINDINGS: &[RepoPlanE2eBinding] = &[
    RepoPlanE2eBinding::BiosSourceDir,
    RepoPlanE2eBinding::XaniteogApk,
];
const RETROARCH_CFG_BINDINGS: &[RepoPlanE2eBinding] = &[RepoPlanE2eBinding::RetroarchCfg];

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
        .join("compatibility_goldens_v1")
        .join(name)
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("compatibility_goldens_v1")
}

fn read_golden(name: &str) -> Value {
    let text =
        fs::read_to_string(golden_path(name)).expect("Compatibility fixture should be readable");
    serde_json::from_str(&text).expect("Compatibility fixture should be valid JSON")
}

fn fixture_device_context() -> DeviceContext {
    // Compatibility fixtures use synthetic context; no probing or profile
    // resolution happens in this helper.
    DeviceContext {
        manufacturer: "Example".to_string(),
        model: "Example".to_string(),
        android_version: 13,
        android_api_level: Some(33),
        device_tags: vec!["example_tag".to_string()],
    }
}

fn fixture_runtime_capabilities() -> RuntimeCapabilities {
    // Uses the capability defaults represented by the compatibility fixtures.
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
        materialize_required_test_host_binding(input_id, value);
        input
            .explicit_input_bindings
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
        materialize_required_test_host_binding(input_id, value);
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

fn repo_detected_context_planner_input_with_bindings(
    device_plan_ref: &str,
    input_bindings: &[(&str, Value)],
    detected_facts: &DetectedDeviceFacts,
) -> PlannerInput {
    let mut bindings = OrderedMap::new();
    for (input_id, value) in input_bindings {
        materialize_required_test_host_binding(input_id, value);
        bindings.insert((*input_id).to_string(), value.clone());
    }
    planner_input_from_authored_device_plan_with_detected_facts(
        repo_authored_root(),
        device_plan_ref,
        format!("plan.p8o.{device_plan_ref}.001"),
        bindings,
        detected_facts,
    )
    .unwrap_or_else(|error| {
        panic!("{device_plan_ref} should build detected-context PlannerInput: {error}")
    })
}

fn repo_detected_context_planning_result_with_bindings(
    device_plan_ref: &str,
    plan_id: &str,
    input_bindings: &[(&str, Value)],
    detected_facts: &DetectedDeviceFacts,
) -> Value {
    let mut bindings = OrderedMap::new();
    for (input_id, value) in input_bindings {
        materialize_required_test_host_binding(input_id, value);
        bindings.insert((*input_id).to_string(), value.clone());
    }
    let authored_root = repo_authored_root();
    let app_catalog = crate::catalog::load_app_definition_catalog(&authored_root);
    assert!(
        app_catalog.diagnostics.is_empty(),
        "repo App Definition catalog should be valid: {:?}",
        app_catalog.diagnostics
    );
    let app_definitions = app_catalog
        .entries
        .into_iter()
        .map(|(_, app)| app)
        .collect::<Vec<_>>();
    serde_json::to_value(
        plan_from_authored_device_plan_with_detected_facts(
            authored_root,
            device_plan_ref,
            plan_id.to_string(),
            bindings,
            &app_definitions,
            detected_facts,
        )
        .unwrap_or_else(|error| {
            panic!("{device_plan_ref} should build detected-context PlanningResult: {error}")
        }),
    )
    .expect("detected-context planning result should serialize")
}

fn repo_baseline_planning_result_with_bindings(
    device_plan_ref: &str,
    plan_id: &str,
    input_bindings: &[(&str, Value)],
) -> Value {
    let mut input = repo_device_plan_planner_input_with_bindings_and_plan_id(
        device_plan_ref,
        plan_id,
        input_bindings,
    );
    // Product planning fixtures that include RetroArch provide a known target
    // API level so bounded App Definition actions can be resolved.
    input.device_context.android_api_level = Some(35);
    planning_result_value(input)
}

fn repo_device_plan_planner_input_with_bindings_and_plan_id(
    device_plan_ref: &str,
    plan_id: &str,
    input_bindings: &[(&str, Value)],
) -> PlannerInput {
    let mut bindings = OrderedMap::new();
    for (input_id, value) in input_bindings {
        materialize_required_test_host_binding(input_id, value);
        bindings.insert((*input_id).to_string(), value.clone());
    }
    PlannerInput::from_authored_device_plan(
        repo_authored_root(),
        device_plan_ref,
        plan_id.to_string(),
        bindings,
    )
    .unwrap_or_else(|error| panic!("{device_plan_ref} should build PlannerInput: {error}"))
}

fn materialize_required_test_host_binding(input_id: &str, value: &Value) {
    let Some((recipe_id, local_input_id)) = input_id.split_once('/') else {
        return;
    };
    let Some(entry) = authored_corpus_recipe_inventory()
        .iter()
        .find(|entry| entry.recipe_id == recipe_id)
    else {
        return;
    };
    let recipe = crate::yaml::load_recipe_from_path(repo_root().join(entry.path))
        .expect("test binding recipe should load");
    let Some(declaration) = recipe.inputs.get(local_input_id) else {
        return;
    };
    if !declaration.validation.must_exist
        || !matches!(
            declaration.type_name.as_str(),
            "file" | "directory" | "path" | "path_list"
        )
    {
        return;
    }
    for item in declaration.binding_items(value).into_iter().flatten() {
        let Some(path) = item.as_str().map(Path::new) else {
            continue;
        };
        let expected_kind = declaration
            .validation
            .path_kind
            .as_deref()
            .unwrap_or(declaration.type_name.as_str());
        if expected_kind == "directory" {
            fs::create_dir_all(path).expect("test binding directory should be created");
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("test binding parent should be created");
            }
            fs::write(path, b"emuchef-test-binding").expect("test binding file should be created");
        }
    }
}

fn assert_planning_result_stable_non_context_fields_eq(baseline: &Value, actual: &Value) {
    assert_eq!(actual["errors"], baseline["errors"]);
    assert_eq!(actual["schema_version"], baseline["schema_version"]);
    assert_eq!(actual["kind"], baseline["kind"]);
    assert_eq!(
        actual["execution_plan"]["id"],
        baseline["execution_plan"]["id"]
    );
    assert_eq!(
        actual["execution_plan"]["source"],
        baseline["execution_plan"]["source"]
    );
    assert_eq!(
        actual["execution_plan"]["runtime_capabilities"],
        baseline["execution_plan"]["runtime_capabilities"]
    );
    assert_eq!(
        actual["execution_plan"]["inputs"],
        baseline["execution_plan"]["inputs"]
    );
    assert_eq!(
        actual["execution_plan"]["artifacts"],
        baseline["execution_plan"]["artifacts"]
    );
    assert_eq!(
        actual["execution_plan"]["steps"],
        baseline["execution_plan"]["steps"]
    );
}

fn assert_non_context_planner_input_fields_eq(baseline: &PlannerInput, actual: &PlannerInput) {
    assert_eq!(actual.recipes, baseline.recipes);
    assert_eq!(actual.selected_recipe_refs, baseline.selected_recipe_refs);
    assert_eq!(
        actual.explicit_input_bindings,
        baseline.explicit_input_bindings
    );
    assert_eq!(
        actual.user_configuration_bindings,
        baseline.user_configuration_bindings
    );
    assert_eq!(
        actual.device_plan_input_bindings,
        baseline.device_plan_input_bindings
    );
    assert_eq!(actual.plan_id, baseline.plan_id);
    assert_eq!(actual.device_plan_ref, baseline.device_plan_ref);
    assert_eq!(actual.device_profile_ref, baseline.device_profile_ref);
    assert_eq!(actual.runtime_capabilities, baseline.runtime_capabilities);
}

fn repo_plan_e2e_input(case: &RepoPlanE2eCase, temp: &TempDir) -> PlannerInput {
    repo_plan_e2e_input_for_device_plan(case.device_plan_ref, temp, case.required_bindings)
}

fn repo_plan_e2e_input_for_device_plan(
    device_plan_ref: &str,
    temp: &TempDir,
    binding_kinds: &[RepoPlanE2eBinding],
) -> PlannerInput {
    let mut input = PlannerInput::from_authored_device_plan(
        repo_authored_root(),
        device_plan_ref,
        format!("plan.p7l.{device_plan_ref}.001"),
        repo_plan_e2e_bindings(temp, binding_kinds),
    )
    .unwrap_or_else(|error| {
        panic!("{device_plan_ref} should build current repository PlannerInput: {error}")
    });
    // These deterministic repository qualification plans model Android API 35.
    input.device_context.android_api_level = Some(35);
    input
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
            fs::create_dir_all(&path)
                .expect("current repository BIOS source temp directory should be created");
            ("feature.copy_bios/bios_source_dir", path)
        }
        RepoPlanE2eBinding::XaniteogApk => {
            let path = temp.path().join("xaniteog.apk");
            fs::write(&path, [])
                .expect("current repository XaniteOG placeholder APK path should be created");
            ("app.xaniteog.install/xaniteog_apk", path)
        }
        RepoPlanE2eBinding::RetroarchCfg => {
            let path = temp.path().join("retroarch.cfg");
            fs::write(&path, [])
                .expect("current repository RetroArch placeholder config path should be created");
            ("app.retroarch.provision/retroarch_cfg", path)
        }
    }
}

fn repo_plan_e2e_cases() -> Vec<RepoPlanE2eCase> {
    vec![
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.konkr_pocket_fit.base",
            device_profile_ref: "ayaneo.konkr_pocket_fit",
            selected_recipe_refs: RETROARCH_BIOS_REFS,
            required_bindings: BIOS_BINDINGS,
        },
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.pocket_s_mini.base",
            device_profile_ref: "ayaneo.pocket_s_mini",
            selected_recipe_refs: RETROARCH_ONLY_REFS,
            required_bindings: NO_REQUIRED_BINDINGS,
        },
    ]
}

fn repo_plan_e2e_no_app_data_write_cases() -> Vec<RepoPlanE2eCase> {
    vec![
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.generic.base",
            device_profile_ref: "ayaneo.generic",
            selected_recipe_refs: RETROARCH_BIOS_REFS,
            required_bindings: BIOS_BINDINGS,
        },
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.pocket_air_mini.base",
            device_profile_ref: "ayaneo.pocket_air_mini",
            selected_recipe_refs: RETROARCH_BIOS_REFS,
            required_bindings: BIOS_BINDINGS,
        },
        RepoPlanE2eCase {
            device_plan_ref: "ayaneo.pocket_s2.base",
            device_profile_ref: "ayaneo.pocket_s2",
            selected_recipe_refs: RETROARCH_BIOS_XANITEOG_REFS,
            required_bindings: BIOS_XANITEOG_BINDINGS,
        },
    ]
}

fn planning_result_value(input: PlannerInput) -> Value {
    let has_app_context = input
        .recipes
        .iter()
        .any(|recipe| recipe.steps.iter().any(|step| step.app_ref.is_some()));
    let apps = if has_app_context {
        let catalog = crate::catalog::load_app_definition_catalog(&repo_authored_root());
        assert!(
            catalog.diagnostics.is_empty(),
            "repo App Definition catalog should be valid: {:?}",
            catalog.diagnostics
        );
        catalog.entries.into_iter().map(|(_, app)| app).collect()
    } else {
        Vec::new()
    };
    serde_json::to_value(plan_execution_with_app_definitions(input, &apps))
        .expect("planning result should serialize")
}

fn authored_corpus_planning_result_value(input: PlannerInput) -> Value {
    let catalog = crate::catalog::load_app_definition_catalog(&repo_authored_root());
    assert!(
        catalog.diagnostics.is_empty(),
        "repo App Definition catalog should be valid: {:?}",
        catalog.diagnostics
    );
    let apps = catalog
        .entries
        .into_iter()
        .map(|(_, app)| app)
        .collect::<Vec<_>>();
    serde_json::to_value(plan_execution_with_app_definitions(input, &apps))
        .expect("authored corpus planning result should serialize")
}

fn normalized_planning_result_value(input: PlannerInput) -> Value {
    normalize_paths(planning_result_value(input))
}

fn normalized_planning_result_for_fixture(fixture: &str, selected_recipe_refs: &[&str]) -> Value {
    let root = authored_root(fixture);
    let app_catalog = crate::catalog::load_app_definition_catalog(&root);
    assert!(
        app_catalog.diagnostics.is_empty(),
        "planner fixture App Definitions should be valid: {:?}",
        app_catalog.diagnostics
    );
    let app_definitions = app_catalog
        .entries
        .into_iter()
        .map(|(_, app)| app)
        .collect::<Vec<_>>();
    let result = plan_execution_with_app_definitions(
        planner_input(fixture, selected_recipe_refs),
        &app_definitions,
    );
    normalize_paths(serde_json::to_value(result).expect("fixture planning result should serialize"))
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
fn minimal_plan_matches_compatibility_planning_result_shape() {
    let actual = planning_result_value(planner_input("planner_minimal", &["planner.minimal"]));
    let expected = read_golden("phase6m_planner_minimal.json");

    assert_eq!(actual, expected);
    assert_eq!(
        actual["execution_plan"]["steps"][0]["params"]["duration_ms"],
        json!({"value": 1})
    );
}

#[test]
fn recipe_and_step_dependencies_match_compatibility_ordering() {
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
fn refs_artifacts_defaults_and_conditions_match_compatibility_execution_plan() {
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
fn required_input_bindings_match_compatibility_success_and_missing_error() {
    let mut bound = planner_input("planner_inputs", &["planner.inputs"]);
    bound.explicit_input_bindings.insert(
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
fn grant_permissions_uses_the_canonical_app_definition_plan() {
    let actual = normalized_planning_result_for_fixture(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    );

    assert_eq!(actual["status"], "success", "{actual:#}");
    let grant = planner_step(&actual, "planner.phase6n.builtins_all/grant");
    assert_eq!(grant["app_id"], "planner_app");
    assert_eq!(grant["params"], json!({}));
    assert_eq!(grant["permission_plan"]["permission_set"], "baseline");
    assert_eq!(grant["permission_plan"]["on_failure"], "fail");
    assert_eq!(
        grant["permission_plan"]["actions"][0]["permission"],
        "android.permission.POST_NOTIFICATIONS"
    );
    assert_eq!(
        grant["permission_plan"]["actions"][1]["op"],
        "MANAGE_EXTERNAL_STORAGE"
    );
}

#[test]
fn permission_plan_defaults_failure_policy_and_serializes_step_local_actions() {
    let default_policy = grant_permission_set_step("grant_defaults");
    let explicit_policy = param_contract_step(
        "grant_explicit",
        "grant_permissions",
        vec![],
        ref_params(vec![
            ("permission_set", ParamValue::Literal(json!("baseline"))),
            ("policy", ParamValue::Literal(json!({"on_failure": "fail"}))),
        ]),
    );
    let actual =
        param_contract_result_value(param_contract_input(vec![default_policy, explicit_policy]));

    assert_eq!(actual["status"], "success", "{actual:#}");
    let steps = actual["execution_plan"]["steps"].as_array().unwrap();
    let default = steps
        .iter()
        .find(|step| step["id"] == "planner.param_contract/grant_defaults")
        .unwrap();
    let explicit = steps
        .iter()
        .find(|step| step["id"] == "planner.param_contract/grant_explicit")
        .unwrap();

    assert_eq!(default["app_id"], "planner_app");
    assert_eq!(default["params"], json!({}));
    assert_eq!(default["permission_plan"]["permission_set"], "baseline");
    assert_eq!(default["permission_plan"]["on_failure"], "warn");
    assert_eq!(
        default["permission_plan"]["actions"][0]["kind"],
        "runtime_permission"
    );
    assert_eq!(
        default["permission_plan"]["actions"][0]["package_name"],
        "com.example.app"
    );
    assert_eq!(
        default["permission_plan"]["actions"][0]["permission"],
        "android.permission.CAMERA"
    );
    assert_eq!(explicit["permission_plan"]["on_failure"], "fail");
}

#[test]
fn permission_intent_validation_rejects_obvious_malformed_step_local_inputs() {
    let cases = vec![
        (
            {
                let mut step = grant_permission_set_step("manual");
                step.params
                    .insert("manual".to_string(), ParamValue::Literal(json!([])));
                step
            },
            "manual",
            "manual",
            "unknown_param",
            json!(["permission_set", "policy"]),
            json!([]),
        ),
        (
            {
                let mut step = grant_permission_set_step("bad_policy");
                step.params.insert(
                    "policy".to_string(),
                    ParamValue::Literal(json!({"on_failure": "explode"})),
                );
                step
            },
            "bad_policy",
            "policy.on_failure",
            "invalid_enum_value",
            json!(["warn", "fail"]),
            json!("explode"),
        ),
        (
            param_contract_step(
                "missing_policy",
                "grant_permissions",
                vec![],
                ref_params(vec![(
                    "permission_set",
                    ParamValue::Literal(json!("baseline")),
                )]),
            ),
            "missing_policy",
            "policy",
            "missing_required_param",
            json!({"on_failure": ["warn", "fail"]}),
            Value::Null,
        ),
        (
            {
                let mut step = grant_permission_set_step("retired_runtime");
                step.params.insert(
                    "runtime".to_string(),
                    ParamValue::Literal(json!([
                        {"package_name": "com.example.app", "required": "yes"}
                    ])),
                );
                step
            },
            "retired_runtime",
            "runtime",
            "unknown_param",
            json!(["permission_set", "policy"]),
            json!([{"package_name": "com.example.app", "required": "yes"}]),
        ),
        (
            {
                let mut step = grant_permission_set_step("retired_appops");
                step.params.insert(
                    "appops".to_string(),
                    ParamValue::Literal(json!([
                        {
                            "package_name": "com.example.app",
                            "op": "MANAGE_EXTERNAL_STORAGE",
                            "mode": "allow",
                            "when": {"android_api_min": 35, "android_api_max": 34}
                        }
                    ])),
                );
                step
            },
            "retired_appops",
            "appops",
            "unknown_param",
            json!(["permission_set", "policy"]),
            json!([{"package_name": "com.example.app", "op": "MANAGE_EXTERNAL_STORAGE", "mode": "allow", "when": {"android_api_min": 35, "android_api_max": 34}}]),
        ),
    ];

    for (step, step_id, param, code, expected, actual_value) in cases {
        let actual = param_contract_result_value(param_contract_input(vec![step]));
        assert_param_contract_error(&actual, code, step_id, param, expected, actual_value);
    }
}

#[test]
fn legacy_recipe_owned_permission_parameters_never_resolve_permission_actions() {
    let mut step = param_contract_step(
        "legacy",
        "grant_permissions",
        vec![],
        ref_params(vec![
            (
                "runtime",
                ParamValue::Literal(json!([
                    {
                        "package_name": "com.example.app",
                        "name": "android.permission.CAMERA"
                    }
                ])),
            ),
            (
                "appops",
                ParamValue::Literal(json!([
                    {
                        "package_name": "com.example.app",
                        "op": "MANAGE_EXTERNAL_STORAGE",
                        "mode": "allow"
                    }
                ])),
            ),
            (
                "policy",
                ParamValue::Literal(json!({"on_failure": "fail", "require_all": true})),
            ),
        ]),
    );
    step.app_ref = Some("planner_app".to_string());
    let actual = serde_json::to_value(plan_execution_with_app_definitions(
        param_contract_input(vec![step]),
        &[param_contract_app_definition()],
    ))
    .expect("planner result should serialize");

    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null);
    let errors = actual["errors"].as_array().expect("planner errors");
    for param in ["runtime", "appops"] {
        assert!(
            errors.iter().any(|error| {
                error["code"] == "unknown_param" && error["details"]["param"] == param
            }),
            "legacy {param} arrays must be rejected: {errors:#?}"
        );
    }
    assert!(
        errors.iter().any(|error| {
            error["code"] == "missing_required_param"
                && error["details"]["param"] == "permission_set"
        }),
        "legacy permission steps must require the canonical permission set: {errors:#?}"
    );
    assert!(
        errors.iter().any(|error| {
            error["code"] == "invalid_param_value"
                && error["details"]["param"] == "policy.require_all"
        }),
        "legacy require_all must be rejected: {errors:#?}"
    );
    assert!(!actual.to_string().contains("permission_plan"));
}

#[test]
fn canonical_permission_steps_reject_require_all_before_action_resolution() {
    let step = param_contract_step(
        "require_all",
        "grant_permissions",
        vec![],
        ref_params(vec![
            ("permission_set", ParamValue::Literal(json!("baseline"))),
            (
                "policy",
                ParamValue::Literal(json!({"on_failure": "warn", "require_all": true})),
            ),
        ]),
    );
    let actual = param_contract_result_value(param_contract_input(vec![step]));

    assert_eq!(actual["status"], "error", "{actual:#}");
    assert_eq!(actual["execution_plan"], Value::Null);
    let errors = actual["errors"].as_array().expect("planner errors");
    assert!(
        errors.iter().any(|error| {
            error["code"] == "invalid_param_value"
                && error["details"]["param"] == "policy.require_all"
        }),
        "canonical permission steps must reject legacy require_all: {errors:#?}"
    );
    assert!(!actual.to_string().contains("permission_plan"));
}

#[test]
fn builtin_planner_mappings_preserve_frozen_compatibility_surface_v1() {
    let actual = normalized_planning_result_for_fixture(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    );
    let expected = read_golden("phase6n_planner_builtins_all.json");

    assert_eq!(actual["status"], expected["status"]);
    assert_eq!(actual["warnings"], expected["warnings"]);
    assert_eq!(actual["errors"], expected["errors"]);
    for field in [
        "id",
        "source",
        "device_context",
        "runtime_capabilities",
        "inputs",
    ] {
        assert_eq!(
            actual["execution_plan"][field], expected["execution_plan"][field],
            "canonical plan should preserve frozen execution-plan field {field}"
        );
    }
    let actual_artifacts = actual["execution_plan"]["artifacts"].as_array().unwrap();
    let expected_artifacts = expected["execution_plan"]["artifacts"].as_array().unwrap();
    let actual_apk = actual_artifacts
        .iter()
        .find(|artifact| artifact["id"] == "planner.phase6n.builtins_all/app_apk")
        .unwrap();
    let expected_apk = expected_artifacts
        .iter()
        .find(|artifact| artifact["id"] == "planner.phase6n.builtins_all/app_apk")
        .unwrap();
    assert_eq!(actual_apk["id"], expected_apk["id"]);
    assert_eq!(actual_apk["type"], expected_apk["type"]);
    assert_eq!(
        actual_apk["source"],
        json!({
            "kind": "direct_url",
            "url": expected_apk["url"]
        })
    );
    assert_eq!(actual_apk["cache"], expected_apk["cache"]);
    assert_eq!(
        actual_apk["app_provenance"],
        json!({"app_id": "planner_app", "artifact_id": "apk", "kind": "apk"})
    );
    let actual_archive = actual_artifacts
        .iter()
        .find(|artifact| artifact["id"] == "planner.phase6n.builtins_all/archive_zip")
        .unwrap();
    let expected_archive = expected_artifacts
        .iter()
        .find(|artifact| artifact["id"] == "planner.phase6n.builtins_all/archive_zip")
        .unwrap();
    assert_eq!(actual_archive, expected_archive);
    let steps = actual["execution_plan"]["steps"].as_array().unwrap();
    let expected_steps = expected["execution_plan"]["steps"].as_array().unwrap();
    for step_id in [
        "planner.phase6n.builtins_all/resolve",
        "planner.phase6n.builtins_all/extract_artifacts",
        "planner.phase6n.builtins_all/extract_archive",
        "planner.phase6n.builtins_all/copy",
        "planner.phase6n.builtins_all/wait",
    ] {
        let step = steps
            .iter()
            .find(|step| step["id"] == step_id)
            .expect("canonical plan should retain the frozen generic step");
        let expected_step = expected_steps
            .iter()
            .find(|step| step["id"] == step_id)
            .expect("frozen golden should include the generic step");
        for field in [
            "id",
            "recipe_ref",
            "type",
            "name",
            "dependencies",
            "constraints",
            "params",
            "skip_if",
            "verify",
        ] {
            assert_eq!(
                step[field], expected_step[field],
                "canonical step {step_id} should preserve frozen field {field}"
            );
        }
    }
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
fn optional_inputs_prune_and_rebind_like_compatibility() {
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
    bound.explicit_input_bindings.insert(
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
fn input_defaults_and_multiple_values_match_compatibility() {
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
fn dependency_expansion_and_namespacing_match_compatibility() {
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
    let first = authored_corpus_planning_result_value(authored_corpus_planner_input_with_bindings(
        &selected_recipe_refs,
        &corpus_recipe_expansion_bindings(),
    ));
    let second =
        authored_corpus_planning_result_value(authored_corpus_planner_input_with_bindings(
            &selected_recipe_refs,
            &corpus_recipe_expansion_bindings(),
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
    for plan in discover_device_plan_inventory(repo_authored_root())
        .expect("repo device plan inventory should parse")
    {
        let selected_recipe_refs = plan
            .selected_recipe_refs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let selected_bindings = corpus_recipe_expansion_bindings()
            .into_iter()
            .filter(|(key, _)| {
                key.split_once('/')
                    .is_some_and(|(recipe_id, _)| selected_recipe_refs.contains(&recipe_id))
            })
            .collect::<Vec<_>>();
        let actual = planning_result_value(authored_corpus_planner_input_with_bindings(
            &selected_recipe_refs,
            &selected_bindings,
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
fn step_data_refs_conditions_and_constraints_match_compatibility() {
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
    let first = normalized_planning_result_for_fixture(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    );
    let second = normalized_planning_result_for_fixture(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    );

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
            "apps",
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
        plan["apps"],
        json!([{
            "id": "planner_app",
            "name": "Planner Test App",
            "package_id": "com.example.app"
        }])
    );

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
        let mut expected_step_fields = vec![
            "id",
            "recipe_ref",
            "type",
            "name",
            "dependencies",
            "constraints",
            "params",
            "skip_if",
            "verify",
        ];
        if step.contains_key("app_id") {
            expected_step_fields.push("app_id");
        }
        if step.contains_key("permission_plan") {
            expected_step_fields.push("permission_plan");
        }
        assert_object_keys(
            &format!("dto success steps[{index}]"),
            step,
            &expected_step_fields,
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
    let actual = normalized_planning_result_for_fixture(
        "planner_phase6n_builtins_all",
        &["planner.phase6n.builtins_all"],
    );
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
        &[],
    );
    assert_object_keys(
        "dto grant permission_plan",
        grant["permission_plan"].as_object().unwrap(),
        &["permission_set", "on_failure", "actions"],
    );
    assert_eq!(grant["permission_plan"]["permission_set"], "baseline");
    assert_eq!(grant["permission_plan"]["on_failure"], "fail");
    assert_eq!(
        grant["permission_plan"]["actions"],
        json!([
            {
                "kind": "runtime_permission",
                "package_name": "com.example.app",
                "permission": "android.permission.POST_NOTIFICATIONS",
                "applicability": {"status": "applicable"}
            },
            {
                "kind": "app_op",
                "package_name": "com.example.app",
                "op": "MANAGE_EXTERNAL_STORAGE",
                "mode": "allow",
                "applicability": {"status": "applicable"}
            }
        ])
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
                    "expected": ["input_ref", "artifact_ref", "step_output_ref"],
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
    let unknown_step =
        param_contract_result_value(ref_validation_input(vec![ref_validation_step(
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

    let unknown_output = param_contract_result_value(ref_validation_input(vec![
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

    let no_primary_output = param_contract_result_value(ref_validation_input(vec![
        ref_validation_step(
            "wait",
            "wait",
            vec![],
            ref_params(vec![("duration_ms", ParamValue::Literal(json!(1)))]),
        ),
        ref_validation_step(
            "copy",
            "copy_files",
            vec![],
            ref_params(vec![
                ("source", ParamValue::Ref("steps.wait".to_string())),
                ("dest", ParamValue::Literal(json!("/sdcard/Out"))),
            ]),
        ),
    ]));
    assert_step_ref_error(
        &no_primary_output,
        "step_ref_has_no_primary_output",
        "copy",
        "source",
        "steps.wait",
    );

    let malformed = param_contract_result_value(ref_validation_input(vec![ref_validation_step(
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
}

#[test]
fn selection_time_pruning_removes_dependents_of_capability_unavailable_steps() {
    let actual = planning_result_value(dependency_validation_input(vec![
        unavailable_dependency_step("unavailable"),
        dependency_step("consumer", vec!["unavailable"]),
        dependency_step("independent", vec![]),
    ]));

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(
        dependency_step_ids(&actual),
        vec!["planner.dependency_validation/independent"],
        "known but unavailable dependencies should prune dependents without weakening true missing-dependency diagnostics"
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
    let actual = param_contract_result_value(param_contract_input(vec![
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
            "planner.param_contract/resolve_install",
            "planner.param_contract/extract",
            "planner.param_contract/archive",
            "planner.param_contract/wait",
            "planner.param_contract/install",
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
    let actual = param_contract_result_value(param_contract_input(vec![
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
fn install_apk_rejects_retired_inline_package_and_checksum_inputs() {
    for (name, value) in [
        ("expected_package_name", json!("com.example.app")),
        ("expected_sha256", json!("0".repeat(64))),
    ] {
        let mut input = armsx1_recipe_planner_input();
        let install = input
            .recipes
            .iter_mut()
            .find(|recipe| recipe.id == "app.armsx1.install")
            .expect("ARMSX1 Recipe should be selected")
            .steps
            .iter_mut()
            .find(|step| step.type_name == "install_apk")
            .expect("ARMSX1 Recipe should include an APK install");
        install
            .params
            .insert(name.to_string(), ParamValue::Literal(value));

        let result = plan_execution_with_app_definitions(input, &[armsx1_app_definition()]);
        let error = result
            .errors
            .iter()
            .find(|error| error.code == "app_install_authority_override")
            .expect("retired inline App authority should be rejected");
        assert_eq!(error.details["fields"], json!([name]));
    }
}

#[test]
fn param_contract_reports_required_source_enum_bool_and_integer_violations() {
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
            json!(["input_ref", "artifact_ref", "step_output_ref"]),
            Value::Null,
        ),
        (
            param_contract_step(
                "copy",
                "copy_files",
                vec![],
                ref_params(vec![
                    ("source", ParamValue::Ref("steps.extract".to_string())),
                    (
                        "dest",
                        ParamValue::Ref("artifacts.app_apk.local_path".to_string()),
                    ),
                ]),
            ),
            "copy",
            "dest",
            "invalid_param_source",
            json!(["literal", "input_ref", "app_target"]),
            json!({"ref": "artifacts.app_apk.local_path"}),
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
        let actual = param_contract_result_value(param_contract_input(vec![step]));
        assert_param_contract_error(&actual, code, step_id, param, expected, actual_value);
    }

    let missing_app = param_contract_result_value(param_contract_input(vec![param_contract_step(
        "install",
        "install_apk",
        vec![],
        OrderedMap::new(),
    )]));
    assert_eq!(missing_app["status"], "error", "{missing_app:#}");
    assert!(
        missing_app["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["code"] == "app_install_artifact_provenance_invalid"),
        "app install without its declared artifact path must fail closed: {missing_app:#}"
    );
}

#[test]
fn param_contract_rejects_unknown_params_and_retired_lifecycle_forms() {
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
        ref_params(vec![(
            "package_name",
            ParamValue::Literal(json!("com.example.app")),
        )]),
    )]));
    assert!(
        non_focused["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["code"] == "app_context_required"),
        "lifecycle identity must come from App Definition context: {non_focused:#}"
    );

    let malformed_legacy = planning_result_value(param_contract_input(vec![param_contract_step(
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
    assert!(malformed_legacy["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error["code"] == "app_context_required"));
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
fn compatibility_fixture_inventory_consumes_checked_in_evidence_only() {
    let inventory = compatibility_fixture_inventory();
    let inventory_goldens = inventory
        .iter()
        .map(|entry| entry.golden.to_string())
        .collect::<BTreeSet<_>>();
    let discovered_goldens = discovered_planner_fixture_names();

    assert_eq!(
        discovered_goldens, inventory_goldens,
        "Compatibility fixtures must be intentionally classified in the compatibility inventory.",
    );

    for entry in inventory {
        assert!(
            authored_root(entry.fixture).is_dir(),
            "compatibility authored fixture should exist: {}",
            entry.fixture
        );
        let parsed = read_golden(entry.golden);
        assert_planner_fixture_shape(entry.golden, &parsed);
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

    let first = authored_corpus_planning_result_value(authored_corpus_planner_input_with_bindings(
        &selected_recipe_refs,
        &input_bindings,
    ));
    let second = authored_corpus_planning_result_value(
        authored_corpus_planner_input_with_bindings(&selected_recipe_refs, &input_bindings),
    );

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

    let actual = authored_corpus_planning_result_value(input);

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(snapshot_files(&repo_authored_recipes_dir()), recipes_before);
    assert_eq!(snapshot_files(&golden_dir()), goldens_before);
}

fn resolve_authored_bindings(
    selected_recipe_refs: &[&str],
    explicit: OrderedMap<Value>,
    user_configuration: OrderedMap<Value>,
    device_plan: OrderedMap<Value>,
) -> crate::planner::BindingResolution {
    let input = authored_corpus_planner_input(selected_recipe_refs);
    let recipes = input
        .recipes
        .into_iter()
        .map(|recipe| (recipe.id.clone(), recipe))
        .collect::<HashMap<_, _>>();
    resolve_runtime_bindings(
        &recipes,
        &selected_recipe_refs
            .iter()
            .map(|recipe| (*recipe).to_string())
            .collect::<Vec<_>>(),
        &explicit,
        &user_configuration,
        &device_plan,
    )
}

fn binding_map(values: &[(&str, Value)]) -> OrderedMap<Value> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

#[test]
fn binding_resolution_applies_precedence_and_preserves_provenance() {
    materialize_required_test_host_binding(
        "feature.copy_roms/source",
        &json!("/tmp/emuchef-binding-resolution-roms"),
    );
    let resolution = resolve_authored_bindings(
        &["feature.copy_roms"],
        binding_map(&[
            (
                "feature.copy_roms/source",
                json!("/tmp/emuchef-binding-resolution-roms"),
            ),
            ("feature.copy_roms/policy", json!("sync")),
        ]),
        binding_map(&[
            ("feature.copy_roms/destination", json!("/sdcard/User")),
            ("feature.copy_roms/policy", json!("invalid-shadowed")),
        ]),
        binding_map(&[("feature.copy_roms/destination", json!("/sdcard/DevicePlan"))]),
    );

    assert!(resolution.diagnostics.is_empty(), "{resolution:#?}");
    let by_key = resolution
        .resolved_inputs
        .iter()
        .map(|input| (input.key.as_str(), input))
        .collect::<HashMap<_, _>>();
    assert_eq!(
        by_key["feature.copy_roms/source"].source,
        Some(BindingSource::Explicit)
    );
    assert_eq!(
        by_key["feature.copy_roms/destination"].value,
        Some(json!("/sdcard/User"))
    );
    assert_eq!(
        by_key["feature.copy_roms/destination"].source,
        Some(BindingSource::UserConfiguration)
    );
    assert_eq!(
        serde_json::to_value(by_key["feature.copy_roms/destination"]).unwrap(),
        json!({
            "key": "feature.copy_roms/destination",
            "recipeId": "feature.copy_roms",
            "inputId": "destination",
            "type": "device_path",
            "value": "/sdcard/User",
            "source": "user_configuration",
        })
    );
    assert_eq!(
        by_key["feature.copy_roms/policy"].value,
        Some(json!("sync"))
    );
    assert_eq!(
        by_key["feature.copy_roms/policy"].source,
        Some(BindingSource::Explicit)
    );

    let default_only = resolve_authored_bindings(
        &["feature.copy_roms"],
        binding_map(&[(
            "feature.copy_roms/source",
            json!("/tmp/emuchef-binding-resolution-roms"),
        )]),
        OrderedMap::new(),
        OrderedMap::new(),
    );
    let destination = default_only
        .resolved_inputs
        .iter()
        .find(|input| input.key == "feature.copy_roms/destination")
        .unwrap();
    assert_eq!(destination.value, Some(json!("/sdcard/ROMs")));
    assert_eq!(destination.source, Some(BindingSource::RecipeDefault));
}

#[test]
fn binding_resolution_rejects_explicit_json_null_for_optional_string_input() {
    let recipe = review_snapshot_recipe();
    let recipes = HashMap::from([(recipe.id.clone(), recipe)]);
    let resolution = resolve_runtime_bindings(
        &recipes,
        &["feature.review".to_string()],
        &binding_map(&[("feature.review/credential", Value::Null)]),
        &OrderedMap::new(),
        &OrderedMap::new(),
    );

    assert!(resolution
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "binding_validation_failed"));
    let credential = resolution
        .resolved_inputs
        .iter()
        .find(|input| input.key == "feature.review/credential")
        .unwrap();
    assert_eq!(credential.value, Some(Value::Null));
    assert_eq!(credential.source, Some(BindingSource::Explicit));
}

#[test]
fn binding_resolution_validates_only_the_winner_and_redacts_sensitive_values() {
    materialize_required_test_host_binding(
        "feature.copy_roms/source",
        &json!("/tmp/emuchef-binding-resolution-roms"),
    );
    let mut input = authored_corpus_planner_input(&["feature.copy_roms"]);
    input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "feature.copy_roms")
        .unwrap()
        .inputs
        .get_mut("policy")
        .unwrap()
        .sensitive = true;
    let recipes = input
        .recipes
        .into_iter()
        .map(|recipe| (recipe.id.clone(), recipe))
        .collect::<HashMap<_, _>>();
    let resolution = resolve_runtime_bindings(
        &recipes,
        &["feature.copy_roms".to_string()],
        &binding_map(&[
            ("feature.copy_roms/source", json!(42)),
            ("feature.copy_roms/policy", json!("DO_NOT_LEAK")),
        ]),
        &binding_map(&[("feature.copy_roms/policy", json!("sync"))]),
        &binding_map(&[("feature.copy_roms/policy", json!("merge"))]),
    );

    assert_eq!(resolution.diagnostics.len(), 2, "{resolution:#?}");
    let serialized = serde_json::to_string(&resolution.diagnostics).unwrap();
    assert!(!serialized.contains("DO_NOT_LEAK"));
    assert!(resolution.diagnostics.iter().all(|diagnostic| {
        diagnostic.provenance == Some(BindingSource::Explicit)
            && diagnostic.details["provenance"] == "explicit"
    }));

    let invalid_user = resolve_runtime_bindings(
        &recipes,
        &["feature.copy_roms".to_string()],
        &binding_map(&[(
            "feature.copy_roms/source",
            json!("/tmp/emuchef-binding-resolution-roms"),
        )]),
        &binding_map(&[("feature.copy_roms/policy", json!("DO_NOT_LEAK"))]),
        &binding_map(&[("feature.copy_roms/policy", json!("sync"))]),
    );
    assert!(invalid_user.diagnostics.iter().any(|diagnostic| {
        diagnostic.key == "feature.copy_roms/policy"
            && diagnostic.provenance == Some(BindingSource::UserConfiguration)
    }));
}

#[test]
fn binding_resolution_reports_missing_unknown_unselected_and_optional_unbound_inputs() {
    let missing = resolve_authored_bindings(
        &["feature.copy_roms"],
        binding_map(&[
            ("unknown.recipe/value", json!("ignored")),
            ("app.retroarch.provision/retroarch_cfg", json!("ignored")),
        ]),
        OrderedMap::new(),
        OrderedMap::new(),
    );
    assert!(missing
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "binding_missing"));
    assert!(missing
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "unknown_binding"));
    assert!(missing
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "binding_recipe_not_selected"));

    let optional = resolve_authored_bindings(
        &["app.retroarch.provision"],
        OrderedMap::new(),
        OrderedMap::new(),
        OrderedMap::new(),
    );
    let retroarch = optional
        .resolved_inputs
        .iter()
        .find(|input| input.key == "app.retroarch.provision/retroarch_cfg")
        .unwrap();
    assert_eq!(retroarch.value, None);
    assert_eq!(retroarch.source, None);

    let first = resolve_authored_bindings(
        &["app.retroarch.provision"],
        binding_map(&[("z.unknown/value", json!(1)), ("a.unknown/value", json!(2))]),
        OrderedMap::new(),
        OrderedMap::new(),
    );
    let second = resolve_authored_bindings(
        &["app.retroarch.provision"],
        binding_map(&[("a.unknown/value", json!(2)), ("z.unknown/value", json!(1))]),
        OrderedMap::new(),
        OrderedMap::new(),
    );
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
}

#[test]
fn runtime_input_metadata_validates_enum_and_device_path_bindings() {
    let valid = planning_result_value(authored_corpus_planner_input_with_bindings(
        &["feature.copy_roms"],
        &[
            ("feature.copy_roms/source", json!("/tmp/ROMs")),
            ("feature.copy_roms/destination", json!("/sdcard/Games")),
            ("feature.copy_roms/policy", json!("sync")),
        ],
    ));
    assert_eq!(valid["status"], "success", "{valid:#}");
    let inputs = valid["execution_plan"]["inputs"]
        .as_array()
        .expect("execution inputs should be present");
    assert!(inputs.iter().any(|input| {
        input["id"] == "feature.copy_roms/destination"
            && input["value"]["type"] == "device_path"
            && input["value"]["location"] == "device"
    }));
    assert!(inputs.iter().any(|input| {
        input["id"] == "feature.copy_roms/policy"
            && input["value"]["type"] == "string"
            && input["value"]["value"] == "sync"
    }));

    let invalid_enum = planning_result_value(authored_corpus_planner_input_with_bindings(
        &["feature.copy_roms"],
        &[
            ("feature.copy_roms/source", json!("/tmp/ROMs")),
            ("feature.copy_roms/policy", json!("overwrite")),
        ],
    ));
    assert_eq!(invalid_enum["status"], "error");
    assert!(invalid_enum["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| {
            error["code"] == "binding_validation_failed"
                && error["details"]["input_id"] == "feature.copy_roms/policy"
        }));

    let invalid_prefix = planning_result_value(authored_corpus_planner_input_with_bindings(
        &["feature.copy_roms"],
        &[
            ("feature.copy_roms/source", json!("/tmp/ROMs")),
            ("feature.copy_roms/destination", json!("/data/local/ROMs")),
        ],
    ));
    assert_eq!(invalid_prefix["status"], "error");
    assert!(invalid_prefix["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| {
            error["code"] == "binding_validation_failed"
                && error["details"]["input_id"] == "feature.copy_roms/destination"
                && error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("allowed path prefixes"))
        }));

    let temp = TempDir::new().expect("missing-path test root should be created");
    let missing_path = temp.path().join("missing-roms");
    let mut missing_host_path = authored_corpus_planner_input(&["feature.copy_roms"]);
    missing_host_path.explicit_input_bindings.insert(
        "feature.copy_roms/source".to_string(),
        json!(missing_path.to_string_lossy()),
    );
    let missing_host_path = planning_result_value(missing_host_path);
    assert_eq!(missing_host_path["status"], "error");
    assert!(missing_host_path["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| {
            error["code"] == "binding_path_missing"
                && error["details"]["input_id"] == "feature.copy_roms/source"
                && error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("existing directory"))
        }));
}

#[test]
fn host_input_validation_reports_entry_specific_format_kind_and_duplicate_failures() {
    let temp = TempDir::new().expect("input validation root should be created");
    let config = temp.path().join("retroarch.cfg");
    let unsupported = temp.path().join("retroarch.txt");
    fs::write(&config, "config").unwrap();
    fs::write(&unsupported, "config").unwrap();

    let mut input = authored_corpus_planner_input(&["app.retroarch.provision"]);
    input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.retroarch.provision")
        .unwrap()
        .inputs
        .get_mut("retroarch_cfg")
        .unwrap()
        .multiple = true;
    input.explicit_input_bindings.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        json!([config, config]),
    );
    let duplicate = planning_result_value(input);
    assert!(duplicate["errors"].as_array().unwrap().iter().any(|error| {
        error["code"] == "binding_duplicate_path"
            && error["details"]["entry_index"] == 1
            && error["details"]["first_entry_index"] == 0
    }));

    let unsupported_result = planning_result_value(authored_corpus_planner_input_with_bindings(
        &["app.retroarch.provision"],
        &[("app.retroarch.provision/retroarch_cfg", json!(unsupported))],
    ));
    assert!(unsupported_result["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| {
            error["code"] == "binding_extension_unsupported" && error["details"]["entry_index"] == 0
        }));

    let mut kind_input = authored_corpus_planner_input(&["app.retroarch.provision"]);
    kind_input.explicit_input_bindings.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        json!(temp.path()),
    );
    let kind_result = planning_result_value(kind_input);
    assert!(kind_result["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| {
            error["code"] == "binding_path_kind_mismatch" && error["details"]["entry_index"] == 0
        }));
}

#[test]
fn canonical_file_reuse_is_a_non_blocking_label_based_warning() {
    let temp = TempDir::new().expect("reuse validation root should be created");
    let shared = temp.path().join("shared.bin");
    fs::write(&shared, "shared").unwrap();
    let mut input =
        authored_corpus_planner_input(&["app.retroarch.provision", "app.xaniteog.install"]);
    for recipe in &mut input.recipes {
        for declaration in recipe.inputs.values_mut() {
            if declaration.type_name == "file" {
                declaration.validation.allowed_extensions.clear();
            }
        }
    }
    input.explicit_input_bindings.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        json!(shared),
    );
    input.explicit_input_bindings.insert(
        "app.xaniteog.install/xaniteog_apk".to_string(),
        json!(shared),
    );
    let recipes = input
        .recipes
        .into_iter()
        .map(|recipe| (recipe.id.clone(), recipe))
        .collect::<HashMap<_, _>>();
    let selected = vec![
        "app.retroarch.provision".to_string(),
        "app.xaniteog.install".to_string(),
    ];
    let resolution = resolve_runtime_bindings(
        &recipes,
        &selected,
        &input.explicit_input_bindings,
        &OrderedMap::new(),
        &OrderedMap::new(),
    );
    let warnings = input_reuse_diagnostics(&recipes, &resolution.resolved_inputs);
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
    assert_eq!(warnings[0].severity, "warning");
    assert_eq!(warnings[0].code, "binding_path_reused");
    assert!(warnings[0].message.contains("RetroArch config"));
    assert!(warnings[0].message.contains("XaniteOG APK"));
    assert!(!warnings[0]
        .message
        .contains(temp.path().to_string_lossy().as_ref()));
}

#[test]
fn repo_device_profile_inventory_is_explicit_by_path_and_id() {
    let actual = discover_device_profile_inventory(repo_authored_root())
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
            (
                "authored/device_profiles/google.sdk_gphone64_arm64.yaml".to_string(),
                "google.sdk_gphone64_arm64".to_string(),
                true,
                false,
                vec![
                    "android_emulator".to_string(),
                    "arm64".to_string(),
                    "api_36".to_string(),
                ],
            ),
        ]
    );
}

#[test]
fn repo_device_plan_inventory_is_explicit_by_path_id_profile_and_selected_order() {
    let actual = discover_device_plan_inventory(repo_authored_root())
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
                vec!["feature.copy_bios".to_string()],
            ),
            (
                "authored/device_plans/ayaneo.konkr_pocket_fit.base.yaml".to_string(),
                "ayaneo.konkr_pocket_fit.base".to_string(),
                "ayaneo.konkr_pocket_fit".to_string(),
                vec![
                    "app.retroarch.provision".to_string(),
                    "feature.copy_bios".to_string(),
                ],
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
    for plan in discover_device_plan_inventory(repo_authored_root())
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
        vec![
            "app.retroarch.provision".to_string(),
            "feature.copy_bios".to_string(),
        ]
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
fn detected_context_planner_input_overrides_profile_scalars() {
    let input = repo_detected_context_planner_input_with_bindings(
        "ayaneo.konkr_pocket_fit.base",
        &[],
        &DetectedDeviceFacts {
            serial: None,
            manufacturer: Some("Detected Manufacturer".to_string()),
            brand: None,
            model: Some("Detected Model".to_string()),
            android_version: Some(15),
            android_api_level: Some(35),
            device_tags: Vec::new(),
            ..DetectedDeviceFacts::default()
        },
    );

    assert_eq!(input.device_context.manufacturer, "Detected Manufacturer");
    assert_eq!(input.device_context.model, "Detected Model");
    assert_eq!(input.device_context.android_version, 15);
    assert_eq!(input.device_context.android_api_level, Some(35));
    assert_eq!(
        input.device_context.device_tags,
        vec!["handheld_android".to_string(), "brand_ayaneo".to_string()]
    );
}

#[test]
fn detected_context_planner_input_preserves_absent_detected_fields() {
    let bindings = OrderedMap::new();
    let plan_id = "plan.p8o.detected.absent_fields.001".to_string();
    let baseline = PlannerInput::from_authored_device_plan(
        repo_authored_root(),
        "ayaneo.konkr_pocket_fit.base",
        plan_id.clone(),
        bindings.clone(),
    )
    .expect("baseline profile-derived PlannerInput should build");
    let actual = planner_input_from_authored_device_plan_with_detected_facts(
        repo_authored_root(),
        "ayaneo.konkr_pocket_fit.base",
        plan_id,
        bindings,
        &DetectedDeviceFacts::default(),
    )
    .expect("detected-context PlannerInput should build");

    assert_non_context_planner_input_fields_eq(&baseline, &actual);
    assert_eq!(actual.device_context, baseline.device_context);
}

#[test]
fn detected_context_planner_input_replaces_non_empty_detected_tags_in_order() {
    let input = repo_detected_context_planner_input_with_bindings(
        "ayaneo.konkr_pocket_fit.base",
        &[],
        &DetectedDeviceFacts {
            device_tags: vec![
                "detected_handheld".to_string(),
                "detected_vendor".to_string(),
            ],
            ..DetectedDeviceFacts::default()
        },
    );

    assert_eq!(
        input.device_context.device_tags,
        vec![
            "detected_handheld".to_string(),
            "detected_vendor".to_string()
        ]
    );
}

#[test]
fn detected_context_planner_input_preserves_profile_tags_when_detected_tags_empty() {
    let baseline =
        repo_device_plan_planner_input_with_bindings("ayaneo.konkr_pocket_fit.base", &[]);
    let actual = repo_detected_context_planner_input_with_bindings(
        "ayaneo.konkr_pocket_fit.base",
        &[],
        &DetectedDeviceFacts {
            manufacturer: Some("Detected Manufacturer".to_string()),
            device_tags: Vec::new(),
            ..DetectedDeviceFacts::default()
        },
    );

    assert_eq!(
        actual.device_context.device_tags,
        baseline.device_context.device_tags
    );
}

#[test]
fn detected_context_planner_input_preserves_selected_recipes_bindings_and_other_fields() {
    let root = temp_authored_root_with_device_plan_selection_yaml(
        r#"
schema_version: 1
kind: device_plan
id: detected.context.non_context
name: Detected Context Non Context
device_profile_ref: ayaneo.konkr_pocket_fit
recipes:
  - recipe_ref: app.retroarch.provision
    selected_by_default: true
  - recipe_ref: feature.copy_bios
    selected_by_default: true
  - recipe_ref: app.xaniteog.install
    selected_by_default: false
defaults: {}
overrides:
  feature.copy_bios/bios_source_dir: /tmp/override-bios
  app.xaniteog.install/xaniteog_apk: /tmp/override-xaniteog.apk
  config_variants:
    vendor_family: test
    screen_class: detected_context
metadata: {}
"#,
    );
    let authored_root = root.path().join("authored");
    let plan_id = "plan.p8o.detected.non_context.001".to_string();
    let mut bindings = OrderedMap::new();
    bindings.insert(
        "feature.copy_bios/bios_source_dir".to_string(),
        json!("/tmp/explicit-bios"),
    );
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        json!("/tmp/explicit-retroarch.cfg"),
    );
    let detected_facts = DetectedDeviceFacts {
        manufacturer: Some("Detected Manufacturer".to_string()),
        device_tags: vec!["detected_context".to_string()],
        ..DetectedDeviceFacts::default()
    };
    let baseline = PlannerInput::from_authored_device_plan(
        &authored_root,
        "detected.context.non_context",
        plan_id.clone(),
        bindings.clone(),
    )
    .expect("baseline PlannerInput should build");
    let actual = planner_input_from_authored_device_plan_with_detected_facts(
        &authored_root,
        "detected.context.non_context",
        plan_id,
        bindings,
        &detected_facts,
    )
    .expect("detected-context PlannerInput should build");
    let mut expected_context = baseline.device_context.clone();
    expected_context.manufacturer = "Detected Manufacturer".to_string();
    expected_context.device_tags = vec!["detected_context".to_string()];

    assert_non_context_planner_input_fields_eq(&baseline, &actual);
    assert_eq!(actual.device_context, expected_context);
}

#[test]
fn detected_context_planner_input_accepts_fake_probe_facts_without_live_probe() {
    let facts = DetectedDeviceFacts {
        serial: None,
        manufacturer: Some("Probe Manufacturer".to_string()),
        brand: None,
        model: Some("Probe Model".to_string()),
        android_version: Some(16),
        android_api_level: Some(36),
        device_tags: vec!["probe_detected".to_string()],
        ..DetectedDeviceFacts::default()
    };
    let probe = FakeDeviceProbe::new(Ok(facts.clone()));
    let detected_facts = probe
        .detect()
        .expect("fake probe should return configured facts");
    let input = repo_detected_context_planner_input_with_bindings(
        "ayaneo.konkr_pocket_fit.base",
        &[],
        &detected_facts,
    );

    assert_eq!(detected_facts, facts);
    assert_eq!(input.device_context.manufacturer, "Probe Manufacturer");
    assert_eq!(input.device_context.model, "Probe Model");
    assert_eq!(input.device_context.android_version, 16);
    assert_eq!(input.device_context.android_api_level, Some(36));
    assert_eq!(
        input.device_context.device_tags,
        vec!["probe_detected".to_string()]
    );
}

#[test]
fn detected_context_planner_input_does_not_emit_profile_mismatch_warning() {
    let mut bindings = OrderedMap::new();
    materialize_required_test_host_binding(
        "app.retroarch.provision/retroarch_cfg",
        &json!("/tmp/emuchef-p8o-retroarch.cfg"),
    );
    bindings.insert(
        "app.retroarch.provision/retroarch_cfg".to_string(),
        json!("/tmp/emuchef-p8o-retroarch.cfg"),
    );
    let input = planner_input_from_authored_device_plan_with_detected_facts(
        repo_authored_root(),
        "ayaneo.pocket_s_mini.base",
        "plan.p8o.detected.no_mismatch_warning.001".to_string(),
        bindings,
        &DetectedDeviceFacts {
            serial: None,
            manufacturer: Some("Not AYANEO".to_string()),
            brand: None,
            model: Some("Unmatched Device".to_string()),
            android_version: Some(9),
            android_api_level: Some(28),
            device_tags: vec!["unmatched".to_string()],
            ..DetectedDeviceFacts::default()
        },
    )
    .expect("detected-context PlannerInput should build");
    let actual = planning_result_value(input);

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(actual["warnings"], json!([]));
}

#[test]
fn detected_context_planning_result_matching_facts_keeps_success_without_mismatch_warning() {
    let plan_id = "plan.p8q.detected.matching.001";
    let bindings = [(
        "app.retroarch.provision/retroarch_cfg",
        json!("/tmp/emuchef-p8q-retroarch.cfg"),
    )];
    let baseline = repo_baseline_planning_result_with_bindings(
        "ayaneo.pocket_s_mini.base",
        plan_id,
        &bindings,
    );
    let actual = repo_detected_context_planning_result_with_bindings(
        "ayaneo.pocket_s_mini.base",
        plan_id,
        &bindings,
        &DetectedDeviceFacts {
            serial: Some("p8q-matching".to_string()),
            manufacturer: Some("AYANEO".to_string()),
            brand: Some("AYANEO".to_string()),
            model: Some("AYANEO Pocket S mini".to_string()),
            android_version: Some(13),
            android_api_level: Some(33),
            device_tags: vec!["detected_handheld".to_string()],
            ..DetectedDeviceFacts::default()
        },
    );

    assert_planning_result_stable_non_context_fields_eq(&baseline, &actual);
    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(actual["warnings"], json!([]));
    assert_eq!(
        actual["execution_plan"]["device_context"],
        json!({
            "manufacturer": "AYANEO",
            "model": "AYANEO Pocket S mini",
            "android_version": 13,
            "android_api_level": 33,
            "device_tags": ["detected_handheld"],
        })
    );
}

#[test]
fn detected_context_planning_result_appends_manufacturer_mismatch_warning() {
    let plan_id = "plan.p8q.detected.manufacturer_mismatch.001";
    let baseline =
        repo_baseline_planning_result_with_bindings("ayaneo.pocket_s_mini.base", plan_id, &[]);
    let actual = repo_detected_context_planning_result_with_bindings(
        "ayaneo.pocket_s_mini.base",
        plan_id,
        &[],
        &DetectedDeviceFacts {
            serial: Some("p8q-manufacturer".to_string()),
            manufacturer: Some("Valve".to_string()),
            brand: Some("AYANEO".to_string()),
            model: Some("AYANEO Pocket S mini".to_string()),
            android_version: Some(13),
            android_api_level: Some(35),
            ..DetectedDeviceFacts::default()
        },
    );
    let warnings = actual["warnings"]
        .as_array()
        .expect("warnings should be an array");

    assert_planning_result_stable_non_context_fields_eq(&baseline, &actual);
    assert_eq!(actual["status"], "warning", "{actual:#}");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], "device_profile_mismatch");
    assert_eq!(
        warnings[0]["details"]["reasons"],
        json!([
            "manufacturer 'Valve' did not contain any of: AYANEO",
            "brand matched one of: AYANEO",
            "model matched one of: Pocket S mini",
            "android version 13 met minimum 13"
        ])
    );
}

#[test]
fn detected_context_planning_result_appends_brand_and_model_mismatch_warning() {
    let actual = repo_detected_context_planning_result_with_bindings(
        "ayaneo.pocket_s_mini.base",
        "plan.p8q.detected.brand_model_mismatch.001",
        &[],
        &DetectedDeviceFacts {
            manufacturer: Some("AYANEO".to_string()),
            brand: Some("Valve".to_string()),
            model: Some("Steam Deck".to_string()),
            android_version: Some(13),
            android_api_level: Some(35),
            ..DetectedDeviceFacts::default()
        },
    );
    let warning = actual["warnings"][0].clone();

    assert_eq!(actual["status"], "warning", "{actual:#}");
    assert_eq!(warning["code"], "device_profile_mismatch");
    assert_eq!(
        warning["details"]["reasons"],
        json!([
            "manufacturer matched one of: AYANEO",
            "brand 'Valve' did not contain any of: AYANEO",
            "model 'Steam Deck' did not match any of: Pocket S mini",
            "android version 13 met minimum 13"
        ])
    );
}

#[test]
fn detected_context_planning_result_appends_android_minimum_mismatch_warning() {
    let actual = repo_detected_context_planning_result_with_bindings(
        "ayaneo.pocket_s_mini.base",
        "plan.p8q.detected.android_mismatch.001",
        &[],
        &DetectedDeviceFacts {
            manufacturer: Some("AYANEO".to_string()),
            brand: Some("AYANEO".to_string()),
            model: Some("AYANEO Pocket S mini".to_string()),
            android_version: Some(12),
            android_api_level: Some(35),
            ..DetectedDeviceFacts::default()
        },
    );
    let warning = actual["warnings"][0].clone();

    assert_eq!(actual["status"], "warning", "{actual:#}");
    assert_eq!(warning["code"], "device_profile_mismatch");
    assert_eq!(
        warning["details"]["reasons"],
        json!([
            "manufacturer matched one of: AYANEO",
            "brand matched one of: AYANEO",
            "model matched one of: Pocket S mini",
            "android version 12 was below minimum 13"
        ])
    );
}

#[test]
fn detected_context_planning_result_missing_identity_fields_do_not_warn_with_known_api() {
    let plan_id = "plan.p8q.detected.missing_fields.001";
    let baseline =
        repo_baseline_planning_result_with_bindings("ayaneo.pocket_s_mini.base", plan_id, &[]);
    let actual = repo_detected_context_planning_result_with_bindings(
        "ayaneo.pocket_s_mini.base",
        plan_id,
        &[],
        &DetectedDeviceFacts {
            android_api_level: Some(35),
            ..DetectedDeviceFacts::default()
        },
    );

    assert_planning_result_stable_non_context_fields_eq(&baseline, &actual);
    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(actual["warnings"], json!([]));
    assert_eq!(
        actual["execution_plan"]["device_context"],
        baseline["execution_plan"]["device_context"]
    );
}

#[test]
fn normal_authored_device_plan_planning_result_does_not_emit_detected_mismatch_warning() {
    let actual = repo_baseline_planning_result_with_bindings(
        "ayaneo.pocket_s_mini.base",
        "plan.p8q.normal.no_detected_warning.001",
        &[],
    );

    assert_eq!(actual["status"], "success", "{actual:#}");
    assert_eq!(actual["warnings"], json!([]));
}

#[test]
fn detected_context_planning_result_accepts_fake_probe_facts() {
    let probe = FakeDeviceProbe::new(Ok(DetectedDeviceFacts {
        serial: Some("p8q-fake-probe".to_string()),
        manufacturer: Some("Valve".to_string()),
        brand: Some("Valve".to_string()),
        model: Some("Steam Deck".to_string()),
        android_version: Some(12),
        android_api_level: Some(32),
        device_tags: vec!["fake_probe".to_string()],
        ..DetectedDeviceFacts::default()
    }));
    let facts = probe
        .detect()
        .expect("fake probe should return configured facts");
    let actual = repo_detected_context_planning_result_with_bindings(
        "ayaneo.pocket_s_mini.base",
        "plan.p8q.detected.fake_probe.001",
        &[],
        &facts,
    );

    assert_eq!(actual["status"], "warning", "{actual:#}");
    assert_eq!(actual["warnings"][0]["code"], "device_profile_mismatch");
    assert_eq!(
        actual["execution_plan"]["device_context"]["device_tags"],
        json!(["fake_probe"])
    );
}

#[test]
fn detected_context_planning_result_source_has_no_live_behavior() {
    let source = include_str!("planner_device_plan.rs");
    let code_without_line_comments = source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !(trimmed.starts_with("//") || trimmed.starts_with("///") || trimmed.starts_with("//!"))
        })
        .collect::<Vec<_>>()
        .join("\n");

    for forbidden in [
        "std::process",
        "Command::new",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "ureq",
        "hyper",
        "adb shell",
        "adb ",
    ] {
        assert!(
            !code_without_line_comments.contains(forbidden),
            "planner composition planner-device-plan composition must not contain live behavior marker {forbidden:?}"
        );
    }
}

#[test]
fn detected_profile_mismatch_matching_authored_criteria_returns_no_warning() {
    let selection =
        load_device_plan_profile_match_criteria(repo_authored_root(), "ayaneo.pocket_s_mini.base")
            .expect("repo device plan profile criteria should load");
    let facts = DetectedDeviceFacts {
        serial: Some("test-serial".to_string()),
        manufacturer: Some("AYANEO".to_string()),
        brand: Some("AYANEO".to_string()),
        model: Some("AYANEO Pocket S mini".to_string()),
        android_version: Some(13),
        ..DetectedDeviceFacts::default()
    };

    let warning = build_detected_device_profile_mismatch_warning(
        &selection.device_plan_ref,
        &selection.device_profile_ref,
        &facts,
        &selection.profile_match,
    );

    assert_eq!(warning, None);
}

#[test]
fn detected_profile_mismatch_warns_for_manufacturer_mismatch() {
    let selection =
        load_device_plan_profile_match_criteria(repo_authored_root(), "ayaneo.pocket_s_mini.base")
            .expect("repo device plan profile criteria should load");
    let facts = DetectedDeviceFacts {
        serial: Some("test-serial".to_string()),
        manufacturer: Some("Valve".to_string()),
        brand: Some("AYANEO".to_string()),
        model: Some("AYANEO Pocket S mini".to_string()),
        android_version: Some(13),
        ..DetectedDeviceFacts::default()
    };

    let warning = build_detected_device_profile_mismatch_warning(
        &selection.device_plan_ref,
        &selection.device_profile_ref,
        &facts,
        &selection.profile_match,
    )
    .expect("manufacturer mismatch should produce a warning");

    assert_eq!(warning.code, "device_profile_mismatch");
    assert_eq!(
        warning.message,
        "Selected device plan profile 'ayaneo.pocket_s_mini' does not match the detected device Valve AYANEO Pocket S mini."
    );
    assert_eq!(
        warning.details["device_plan_ref"],
        "ayaneo.pocket_s_mini.base"
    );
    assert_eq!(
        warning.details["device_profile_ref"],
        "ayaneo.pocket_s_mini"
    );
    assert_eq!(warning.details["serial"], "test-serial");
    assert_eq!(warning.details["manufacturer"], "Valve");
    assert_eq!(warning.details["brand"], "AYANEO");
    assert_eq!(warning.details["model"], "AYANEO Pocket S mini");
    assert_eq!(warning.details["android_version"], 13);
    assert_eq!(
        warning.details["reasons"],
        json!([
            "manufacturer 'Valve' did not contain any of: AYANEO",
            "brand matched one of: AYANEO",
            "model matched one of: Pocket S mini",
            "android version 13 met minimum 13"
        ])
    );
}

#[test]
fn detected_profile_mismatch_warns_for_brand_and_model_pattern_mismatches() {
    let selection =
        load_device_plan_profile_match_criteria(repo_authored_root(), "ayaneo.pocket_s_mini.base")
            .expect("repo device plan profile criteria should load");
    let facts = DetectedDeviceFacts {
        manufacturer: Some("AYANEO".to_string()),
        brand: Some("Valve".to_string()),
        model: Some("Steam Deck".to_string()),
        android_version: Some(13),
        ..DetectedDeviceFacts::default()
    };

    let warning = build_detected_device_profile_mismatch_warning(
        &selection.device_plan_ref,
        &selection.device_profile_ref,
        &facts,
        &selection.profile_match,
    )
    .expect("brand and model mismatches should produce a warning");

    assert_eq!(
        warning.details["reasons"],
        json!([
            "manufacturer matched one of: AYANEO",
            "brand 'Valve' did not contain any of: AYANEO",
            "model 'Steam Deck' did not match any of: Pocket S mini",
            "android version 13 met minimum 13"
        ])
    );
}

#[test]
fn detected_profile_mismatch_missing_detected_fields_do_not_warn() {
    let selection =
        load_device_plan_profile_match_criteria(repo_authored_root(), "ayaneo.pocket_s_mini.base")
            .expect("repo device plan profile criteria should load");

    let warning = build_detected_device_profile_mismatch_warning(
        &selection.device_plan_ref,
        &selection.device_profile_ref,
        &DetectedDeviceFacts::default(),
        &selection.profile_match,
    );

    assert_eq!(warning, None);
}

#[test]
fn detected_profile_mismatch_warns_for_android_version_below_minimum() {
    let selection =
        load_device_plan_profile_match_criteria(repo_authored_root(), "ayaneo.pocket_s_mini.base")
            .expect("repo device plan profile criteria should load");
    let facts = DetectedDeviceFacts {
        manufacturer: Some("AYANEO".to_string()),
        brand: Some("AYANEO".to_string()),
        model: Some("AYANEO Pocket S mini".to_string()),
        android_version: Some(12),
        ..DetectedDeviceFacts::default()
    };

    let warning = build_detected_device_profile_mismatch_warning(
        &selection.device_plan_ref,
        &selection.device_profile_ref,
        &facts,
        &selection.profile_match,
    )
    .expect("android version below minimum should produce a warning");

    assert_eq!(
        warning.details["reasons"],
        json!([
            "manufacturer matched one of: AYANEO",
            "brand matched one of: AYANEO",
            "model matched one of: Pocket S mini",
            "android version 12 was below minimum 13"
        ])
    );
}

#[test]
fn planner_device_plan_detected_profile_mismatch_loads_authored_brand_model_and_android_range_criteria(
) {
    let selection =
        load_device_plan_profile_match_criteria(repo_authored_root(), "ayaneo.pocket_s_mini.base")
            .expect("repo device plan profile criteria should load");
    let android_version = selection
        .profile_match
        .android_version
        .as_ref()
        .expect("repo profile should define android range criteria");

    assert_eq!(
        selection.profile_match.manufacturer_contains,
        vec!["AYANEO".to_string()]
    );
    assert_eq!(
        selection.profile_match.brand_contains,
        vec!["AYANEO".to_string()]
    );
    assert_eq!(
        selection.profile_match.model_patterns,
        vec!["Pocket S mini".to_string()]
    );
    assert_eq!(android_version.min, Some(13));
    assert_eq!(android_version.max, None);
}

#[test]
fn detected_profile_mismatch_android_max_is_parsed_but_not_evaluated_for_compatibility_contract() {
    let root = temp_authored_root_with_device_plan(
        "detected.max.profile",
        "ayaneo.konkr_pocket_fit",
        &[("app.retroarch.provision", true)],
    );
    let authored_root = root.path().join("authored");
    fs::write(
        authored_root.join("device_profiles/ayaneo.konkr_pocket_fit.yaml"),
        r#"
schema_version: 1
kind: device_profile
id: ayaneo.konkr_pocket_fit
name: AYANEO KONKR Pocket FIT
match:
  manufacturer_contains:
    - AYANEO
  brand_contains:
    - AYANEO
  model_patterns:
    - 'Pocket FIT'
  android_version:
    min: 13
    max: 13
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
  - handheld_android
"#
        .trim_start(),
    )
    .expect("temp profile with max should be written");
    let selection = load_device_plan_profile_match_criteria(&authored_root, "detected.max.profile")
        .expect("temp device plan profile criteria should load");
    let android_version = selection
        .profile_match
        .android_version
        .as_ref()
        .expect("temp profile should define android range criteria");
    let facts = DetectedDeviceFacts {
        manufacturer: Some("AYANEO".to_string()),
        brand: Some("AYANEO".to_string()),
        model: Some("KONKR Pocket FIT".to_string()),
        android_version: Some(14),
        ..DetectedDeviceFacts::default()
    };

    let warning = build_detected_device_profile_mismatch_warning(
        &selection.device_plan_ref,
        &selection.device_profile_ref,
        &facts,
        &selection.profile_match,
    );

    assert_eq!(android_version.max, Some(13));
    assert_eq!(warning, None);
}

#[test]
fn detected_profile_mismatch_accepts_fake_probe_facts_without_live_behavior() {
    let selection =
        load_device_plan_profile_match_criteria(repo_authored_root(), "ayaneo.pocket_s_mini.base")
            .expect("repo device plan profile criteria should load");
    let probe = FakeDeviceProbe::new(Ok(DetectedDeviceFacts {
        serial: Some("fake-probe-serial".to_string()),
        manufacturer: Some("Valve".to_string()),
        brand: Some("Valve".to_string()),
        model: Some("Steam Deck".to_string()),
        android_version: Some(12),
        ..DetectedDeviceFacts::default()
    }));
    let facts = probe
        .detect()
        .expect("fake probe should return configured facts");

    let warning = build_detected_device_profile_mismatch_warning(
        &selection.device_plan_ref,
        &selection.device_profile_ref,
        &facts,
        &selection.profile_match,
    )
    .expect("fake probe mismatch facts should produce a warning");

    assert_eq!(warning.code, "device_profile_mismatch");
    assert_eq!(warning.details["serial"], "fake-probe-serial");
}

#[test]
fn detected_profile_mismatch_helper_does_not_mutate_facts_or_criteria() {
    let selection =
        load_device_plan_profile_match_criteria(repo_authored_root(), "ayaneo.pocket_s_mini.base")
            .expect("repo device plan profile criteria should load");
    let original_criteria = selection.profile_match.clone();
    let facts = DetectedDeviceFacts {
        manufacturer: Some("Valve".to_string()),
        brand: Some("Valve".to_string()),
        model: Some("Steam Deck".to_string()),
        android_version: Some(12),
        ..DetectedDeviceFacts::default()
    };
    let original_facts = facts.clone();

    let _ = build_detected_device_profile_mismatch_warning(
        &selection.device_plan_ref,
        &selection.device_profile_ref,
        &facts,
        &selection.profile_match,
    );

    assert_eq!(facts, original_facts);
    assert_eq!(selection.profile_match, original_criteria);
}

#[test]
fn detected_profile_mismatch_source_has_no_live_behavior() {
    let source = include_str!("device_profile_match.rs");

    for forbidden in [
        "std::process",
        "Command::new",
        "std::env",
        "std::fs",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "ureq",
        "hyper",
    ] {
        assert!(
            !source.contains(forbidden),
            "profile mismatch helper must not contain live behavior marker {forbidden:?}"
        );
    }
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
            .explicit_input_bindings
            .get("feature.copy_bios/bios_source_dir"),
        Some(&json!("/tmp/emuchef-p7i-bios"))
    );
    assert_eq!(
        input
            .explicit_input_bindings
            .get("app.xaniteog.install/xaniteog_apk"),
        Some(&json!("/tmp/emuchef-p7i-xaniteog.apk"))
    );
    assert!(
        !input
            .device_plan_input_bindings
            .contains_key("config_variants"),
        "metadata-only device plan overrides must not become bindings"
    );
}

#[test]
fn repo_device_plan_defaults_and_config_variants_are_checked_in_metadata_only() {
    let mut actual = Vec::new();
    for plan in discover_device_plan_inventory(repo_authored_root())
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
            !input
                .device_plan_input_bindings
                .contains_key("show_advanced_steps"),
            "{} defaults.show_advanced_steps must not become a planner binding",
            plan.id
        );
        assert!(
            !input
                .device_plan_input_bindings
                .contains_key("config_variants"),
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
            .device_plan_input_bindings
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "feature.copy_bios/bios_source_dir",
            "app.xaniteog.install/xaniteog_apk",
        ]
    );
    assert_eq!(
        input
            .explicit_input_bindings
            .get("feature.copy_bios/bios_source_dir"),
        Some(&json!("/tmp/explicit-bios"))
    );
    assert_eq!(
        input
            .device_plan_input_bindings
            .get("app.xaniteog.install/xaniteog_apk"),
        Some(&json!("/tmp/override-xaniteog.apk"))
    );
    assert_eq!(
        input
            .explicit_input_bindings
            .get("app.retroarch.provision/retroarch_cfg"),
        Some(&json!("/tmp/explicit-retroarch.cfg"))
    );
    assert!(
        !input
            .device_plan_input_bindings
            .contains_key("config_variants"),
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
    .expect("ref-shaped defaults should stay inactive in current contract");

    assert!(
        input.device_plan_input_bindings.is_empty(),
        "device_plan.defaults must not populate Rust planner bindings in current contract"
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
fn repo_device_plan_profile_context_can_plan_successfully_without_compatibility_or_devices() {
    let mut first_input = repo_device_plan_planner_input_with_bindings(
        "ayaneo.pocket_s_mini.base",
        &[(
            "app.retroarch.provision/retroarch_cfg",
            json!("/tmp/emuchef-p7i-retroarch.cfg"),
        )],
    );
    let mut second_input = repo_device_plan_planner_input_with_bindings(
        "ayaneo.pocket_s_mini.base",
        &[(
            "app.retroarch.provision/retroarch_cfg",
            json!("/tmp/emuchef-p7i-retroarch.cfg"),
        )],
    );
    first_input.device_context.android_api_level = Some(35);
    second_input.device_context.android_api_level = Some(35);
    let first = planning_result_value(first_input);
    let second = planning_result_value(second_input);

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
    assert!(!first["execution_plan"]
        .as_object()
        .unwrap()
        .contains_key("permission_plan"));
    let baseline = planner_step(&first, "app.retroarch.provision/grant_retroarch_baseline");
    assert_eq!(baseline["params"], json!({}));
    assert_eq!(baseline["permission_plan"]["permission_set"], "baseline");
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
fn repo_plan_e2e_normalized_steps_match_runtime_contract() {
    let temp = TempDir::new().expect("repo plan e2e temp root should be created");
    let actual = planning_result_value(repo_plan_e2e_input_for_device_plan(
        "ayaneo.pocket_s_mini.base",
        &temp,
        NO_REQUIRED_BINDINGS,
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
        RETROARCH_CFG_BINDINGS,
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
        let temp = TempDir::new().expect("repo plan e2e temp root should be created");
        let actual = planning_result_value(repo_plan_e2e_input(&case, &temp));

        assert_eq!(
            actual["status"], "success",
            "{}: {actual:#}",
            case.device_plan_ref
        );
        assert_eq!(
            execution_input_ids(&actual),
            case.required_bindings
                .iter()
                .map(|binding| match binding {
                    RepoPlanE2eBinding::BiosSourceDir => "feature.copy_bios/bios_source_dir",
                    RepoPlanE2eBinding::XaniteogApk => "app.xaniteog.install/xaniteog_apk",
                    RepoPlanE2eBinding::RetroarchCfg => "app.retroarch.provision/retroarch_cfg",
                })
                .collect::<Vec<_>>(),
            "{} should expose only its explicitly supplied external bindings",
            case.device_plan_ref
        );
    }
}

#[test]
fn repo_plan_e2e_no_app_data_write_contexts_prune_app_data_dependents() {
    for case in repo_plan_e2e_no_app_data_write_cases() {
        let temp = TempDir::new().expect("repo plan e2e temp root should be created");
        let actual = planning_result_value(repo_plan_e2e_input(&case, &temp));

        assert_eq!(
            actual["status"], "success",
            "{}: {actual:#}",
            case.device_plan_ref
        );
        assert!(
            actual["errors"].as_array().is_some_and(Vec::is_empty),
            "{} should not report stale unknown dependency errors after app-data pruning: {actual:#}",
            case.device_plan_ref
        );
        assert_planning_result_shape(case.device_plan_ref, &actual);
        assert!(
            !execution_step_ids(&actual)
                .contains(&"app.retroarch.provision/launch_retroarch"),
            "{} should prune launch_retroarch when app-data copy dependencies are unavailable: {actual:#}",
            case.device_plan_ref
        );
    }
}

#[test]
fn invalid_app_target_is_reported_before_capability_pruning() {
    let case = repo_plan_e2e_no_app_data_write_cases().remove(0);
    let temp = TempDir::new().unwrap();
    let mut input = repo_plan_e2e_input(&case, &temp);
    input
        .selected_recipe_refs
        .push("app.retroarch.provision".to_string());
    input.runtime_capabilities.app_data_write = false;
    let recipe = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.retroarch.provision")
        .unwrap();
    let step = recipe
        .steps
        .iter_mut()
        .find(|step| step.id == "copy_assets")
        .unwrap();
    step.params.insert(
        "dest".to_string(),
        ParamValue::AppTarget("missing_target".to_string()),
    );
    let result = planning_result_value(input);
    assert_eq!(result["status"], "error", "{result:#}");
    assert!(
        result["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["code"] == "app_target_unknown"
                && error["details"]["step_id"] == "copy_assets"),
        "{result:#}"
    );
}

#[test]
fn derived_target_capability_matches_selected_plan_constraint() {
    let case = repo_plan_e2e_cases().remove(0);
    let temp = TempDir::new().unwrap();
    let result = planning_result_value(repo_plan_e2e_input(&case, &temp));
    assert_eq!(result["status"], "success", "{result:#}");
    let copy_cheats = planner_step(&result, "app.retroarch.provision/copy_cheats");
    assert_eq!(
        copy_cheats["constraints"]["capabilities"],
        json!(["shared_storage_write"])
    );
    assert_eq!(
        copy_cheats["params"]["dest"]["value"],
        "/storage/emulated/0/RetroArch/cheats"
    );
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
    .expect_err("selected_by_default is required by the authored device-plan contract");
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
        device_plan_presentation: None,
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
                        RecipeArtifact::RemoteFile(RemoteFileArtifact {
                            url: url.to_string(),
                            cache: "default".to_string(),
                        }),
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
                progress_note: None,
                user_toggleable: false,
                app_ref: None,
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
        explicit_input_bindings: OrderedMap::new(),
        user_configuration_bindings: OrderedMap::new(),
        device_plan_input_bindings: OrderedMap::new(),
        plan_id: "plan.artifact_selection.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
        catalog_identity: None,
        target_device: None,
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
        device_plan_presentation: None,
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
                RecipeArtifact::RemoteFile(RemoteFileArtifact {
                    url: "https://example.com/app.apk".to_string(),
                    cache: "default".to_string(),
                }),
            )]
            .into_iter()
            .collect(),
            artifact_groups: OrderedMap::new(),
            steps,
        }],
        selected_recipe_refs: vec!["planner.ref_validation".to_string()],
        explicit_input_bindings: OrderedMap::new(),
        user_configuration_bindings: OrderedMap::new(),
        device_plan_input_bindings: OrderedMap::new(),
        plan_id: "plan.ref_validation.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
        catalog_identity: None,
        target_device: None,
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
        progress_note: None,
        user_toggleable: false,
        app_ref: None,
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
        device_plan_presentation: None,
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
        explicit_input_bindings: OrderedMap::new(),
        user_configuration_bindings: OrderedMap::new(),
        device_plan_input_bindings: OrderedMap::new(),
        plan_id: "plan.dependency_validation.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
        catalog_identity: None,
        target_device: None,
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
        progress_note: None,
        user_toggleable: false,
        app_ref: None,
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
        device_plan_presentation: None,
        recipes,
        selected_recipe_refs: selected_recipe_refs
            .into_iter()
            .map(ToString::to_string)
            .collect(),
        explicit_input_bindings: OrderedMap::new(),
        user_configuration_bindings: OrderedMap::new(),
        device_plan_input_bindings: OrderedMap::new(),
        plan_id: "plan.recipe_expansion.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
        catalog_identity: None,
        target_device: None,
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
        progress_note: None,
        user_toggleable: false,
        app_ref: None,
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

fn corpus_recipe_expansion_bindings() -> Vec<(&'static str, Value)> {
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
        device_plan_presentation: None,
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
                    RecipeArtifact::RemoteFile(RemoteFileArtifact {
                        url: "https://example.com/app.apk".to_string(),
                        cache: "default".to_string(),
                    }),
                ),
                (
                    "archive_zip".to_string(),
                    RecipeArtifact::RemoteFile(RemoteFileArtifact {
                        url: "https://example.com/archive.zip".to_string(),
                        cache: "default".to_string(),
                    }),
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
        explicit_input_bindings: OrderedMap::new(),
        user_configuration_bindings: OrderedMap::new(),
        device_plan_input_bindings: OrderedMap::new(),
        plan_id: "plan.param_contract.001".to_string(),
        device_plan_ref: "example.device_plan".to_string(),
        device_profile_ref: "example.device_profile".to_string(),
        device_context: fixture_device_context(),
        runtime_capabilities: fixture_runtime_capabilities(),
        catalog_identity: None,
        target_device: None,
    }
}

fn param_contract_app_definition() -> crate::authored_models::AppDefinitionV1 {
    crate::authored_models::AppDefinitionV1 {
        schema_version: 1,
        kind: "app_definition".to_string(),
        id: "planner_app".to_string(),
        name: "Planner Test App".to_string(),
        description: None,
        category: None,
        package_id: "com.example.app".to_string(),
        artifacts: vec![(
            "apk".to_string(),
            AppArtifactV1 {
                kind: AppArtifactKind::Apk,
                name: Some("Planner Test APK".to_string()),
                description: None,
                source: AppArtifactSource::DirectUrl {
                    url: "https://example.com/app.apk".to_string(),
                    sha256: None,
                },
            },
        )]
        .into_iter()
        .collect(),
        permission_sets: Some(crate::authored_models::AppPermissionSets {
            baseline: Some(crate::authored_models::AppPermissionSet {
                runtime: vec![crate::authored_models::AppRuntimePermissionAction {
                    permission: "android.permission.CAMERA".to_string(),
                    android_api_min: None,
                    android_api_max: None,
                }],
                app_ops: Vec::new(),
            }),
            elevated: None,
        }),
        targets: Default::default(),
        launcher_activity: None,
        metadata: Default::default(),
    }
}

fn param_contract_result_value(mut input: PlannerInput) -> Value {
    let app = param_contract_app_definition();
    let app_ref = app.id.clone();

    for recipe in &mut input.recipes {
        let has_install = recipe
            .steps
            .iter()
            .any(|step| step.type_name == "install_apk");
        let mut resolver_steps = Vec::new();
        if has_install {
            recipe.artifacts.insert(
                "app_apk".to_string(),
                RecipeArtifact::AppArtifact(AppArtifactReference {
                    app_ref: app_ref.clone(),
                    artifact: "apk".to_string(),
                    input: None,
                    cache: "default".to_string(),
                }),
            );
        }

        for step in &mut recipe.steps {
            if step.type_name == "install_apk" {
                step.app_ref = Some(app_ref.clone());
                let resolver_id = format!("resolve_{}", step.id);
                step.dependencies.push(resolver_id.clone());
                resolver_steps.push(param_contract_step(
                    &resolver_id,
                    "resolve_artifacts",
                    vec![],
                    ref_params(vec![("artifacts", ParamValue::Literal(json!(["app_apk"])))]),
                ));
            } else if step.type_name == "grant_permissions" {
                step.app_ref = Some(app_ref.clone());
            }
        }
        if !resolver_steps.is_empty() {
            recipe.steps.splice(0..0, resolver_steps);
        }
    }

    serde_json::to_value(plan_execution_with_app_definitions(input, &[app]))
        .expect("planner result should serialize")
}

fn grant_permission_set_step(id: &str) -> Step {
    param_contract_step(
        id,
        "grant_permissions",
        vec![],
        ref_params(vec![
            ("permission_set", ParamValue::Literal(json!("baseline"))),
            ("policy", ParamValue::Literal(json!({"on_failure": "warn"}))),
        ]),
    )
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
        progress_note: None,
        user_toggleable: false,
        app_ref: None,
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

struct CompatibilityFixtureEntry {
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
            path: "authored/recipes/app.armsx1.install.yaml",
            recipe_id: "app.armsx1.install",
        },
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
        AuthoredCorpusRecipeEntry {
            path: "authored/recipes/feature.copy_roms.yaml",
            recipe_id: "feature.copy_roms",
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

fn compatibility_fixture_inventory() -> &'static [CompatibilityFixtureEntry] {
    &[
        CompatibilityFixtureEntry {
            fixture: "planner_minimal",
            golden: "phase6m_planner_minimal.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_dependencies",
            golden: "phase6m_planner_dependencies.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_refs_artifacts",
            golden: "phase6m_planner_refs_artifacts.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_inputs",
            golden: "phase6m_planner_inputs_bound.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_inputs",
            golden: "phase6m_planner_inputs_missing.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_grant_permissions",
            golden: "phase6m_planner_grant_permissions.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_phase6n_builtins_all",
            golden: "phase6n_planner_builtins_all.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_phase6n_optional_inputs",
            golden: "phase6n_planner_optional_inputs_omitted.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_phase6n_optional_inputs",
            golden: "phase6n_planner_optional_inputs_bound.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_phase6n_input_defaults_multiple",
            golden: "phase6n_planner_input_defaults_multiple.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_phase6n_dependency_graph",
            golden: "phase6n_planner_dependency_graph.json",
        },
        CompatibilityFixtureEntry {
            fixture: "planner_phase6n_step_data",
            golden: "phase6n_planner_step_data.json",
        },
    ]
}

fn discovered_planner_fixture_names() -> BTreeSet<String> {
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

fn assert_planner_fixture_shape(name: &str, parsed: &Value) {
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
    let mut expected_plan_fields = vec![
        "id",
        "source",
        "device_context",
        "runtime_capabilities",
        "inputs",
        "artifacts",
        "steps",
        "schema_version",
        "kind",
    ];
    if plan.contains_key("apps") {
        expected_plan_fields.push("apps");
    }
    if plan.contains_key("device_plan") {
        expected_plan_fields.push("device_plan");
    }
    if plan.contains_key("recipes") {
        expected_plan_fields.push("recipes");
    }
    if plan.contains_key("resolved_inputs") {
        expected_plan_fields.push("resolved_inputs");
    }
    assert_object_keys(
        &format!("{name}.execution_plan"),
        plan,
        &expected_plan_fields,
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
    assert!(
        plan.get("apps")
            .is_none_or(|apps| apps.as_array().is_some()),
        "{name} execution_plan.apps should be an array when present"
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
        let mut expected_step_fields = vec![
            "id",
            "recipe_ref",
            "type",
            "name",
            "dependencies",
            "constraints",
            "params",
            "skip_if",
            "verify",
        ];
        if step.contains_key("app_id") {
            expected_step_fields.push("app_id");
        }
        let is_app_permission_step = step.get("type").and_then(Value::as_str)
            == Some("grant_permissions")
            && step.contains_key("app_id");
        if is_app_permission_step {
            expected_step_fields.push("permission_plan");
        }
        assert_object_keys(
            &format!("{name}.execution_plan.steps[{index}]"),
            step,
            &expected_step_fields,
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
            step.get("app_id")
                .is_none_or(|app_id| app_id.as_str().is_some()),
            "{name} steps[{index}].app_id should be a string when present"
        );
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
        if is_app_permission_step {
            let permission_plan = step
                .get("permission_plan")
                .and_then(Value::as_object)
                .expect("App Definition permission steps should carry their typed plan");
            assert!(
                matches!(
                    permission_plan
                        .get("permission_set")
                        .and_then(Value::as_str),
                    Some("baseline" | "elevated")
                ),
                "{name} steps[{index}] should select a known permission set"
            );
            assert!(permission_plan
                .get("actions")
                .and_then(Value::as_array)
                .is_some_and(|actions| !actions.is_empty()));
            assert_eq!(
                step.get("params")
                    .and_then(Value::as_object)
                    .map(serde_json::Map::len),
                Some(0),
                "{name} permission action facts must not be duplicated in params"
            );
        }
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

fn armsx1_app_definition() -> crate::authored_models::AppDefinitionV1 {
    load_app_definition(repo_authored_root().join("apps/armsx1.yaml"))
        .expect("ARMSX1 App Definition should load")
}

fn armsx1_recipe_planner_input() -> PlannerInput {
    authored_corpus_planner_input(&["app.armsx1.install"])
}

fn assert_app_planning_error(
    input: PlannerInput,
    apps: &[crate::authored_models::AppDefinitionV1],
    expected_code: &str,
) {
    let result = plan_execution_with_app_definitions(input, apps);
    assert_eq!(result.status, PlanningStatus::Error);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.code == expected_code),
        "expected planner error {expected_code}, got {:?}",
        result.errors
    );
}

fn user_provided_artifact_planner_case(
    binding: Option<&str>,
    input_type: Option<&str>,
    multiple: bool,
    selected: bool,
) -> (PlannerInput, crate::authored_models::AppDefinitionV1) {
    let mut params = OrderedMap::new();
    params.insert(
        "artifacts".to_string(),
        ParamValue::Literal(json!(if selected {
            ["user_config"]
        } else {
            ["archive_zip"]
        })),
    );
    let mut input = param_contract_input(vec![param_contract_step(
        "resolve",
        "resolve_artifacts",
        Vec::new(),
        params,
    )]);
    let recipe = &mut input.recipes[0];
    if let Some(input_type) = input_type {
        recipe.inputs.insert(
            "user_file".to_string(),
            InputDeclaration {
                type_name: input_type.to_string(),
                role: "source".to_string(),
                label: "User file".to_string(),
                description: None,
                required: true,
                multiple,
                validation: InputValidation {
                    must_exist: false,
                    allowed_extensions: Vec::new(),
                    path_kind: None,
                    allowed_prefixes: Vec::new(),
                },
                default: Value::Null,
                options: Vec::new(),
                sensitive: false,
                advanced: false,
                metadata: OrderedMap::new(),
            },
        );
        input.explicit_input_bindings.insert(
            "planner.param_contract/user_file".to_string(),
            if multiple {
                json!(["/tmp/user.cfg"])
            } else {
                json!("/tmp/user.cfg")
            },
        );
    }
    recipe.artifacts.insert(
        "user_config".to_string(),
        RecipeArtifact::AppArtifact(AppArtifactReference {
            app_ref: "armsx1".to_string(),
            artifact: "user_config".to_string(),
            input: binding.map(|input_id| AppArtifactInputBinding {
                input_id: input_id.to_string(),
            }),
            cache: "default".to_string(),
        }),
    );
    let mut app = armsx1_app_definition();
    app.artifacts.insert(
        "user_config".to_string(),
        AppArtifactV1 {
            kind: AppArtifactKind::File,
            name: None,
            description: None,
            source: AppArtifactSource::UserProvided,
        },
    );
    (input, app)
}

#[test]
fn selected_user_provided_artifact_emits_its_recipe_input_in_the_immutable_plan() {
    let (input, app) =
        user_provided_artifact_planner_case(Some("user_file"), Some("file"), false, true);
    let result = plan_execution_with_app_definitions(input, &[app]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.expect("user input plan should emit");
    assert!(plan
        .inputs
        .iter()
        .any(|input| input.id == "planner.param_contract/user_file"));
    let artifact = plan
        .artifacts
        .iter()
        .find(|artifact| artifact.id == "planner.param_contract/user_config")
        .expect("selected user-provided artifact should emit");
    assert_eq!(
        artifact.source,
        crate::planner::ExecutionArtifactSource::LocalInput {
            input_id: "planner.param_contract/user_file".to_string(),
        }
    );
}

#[test]
fn selected_user_provided_apk_artifact_emits_its_recipe_input_in_the_immutable_plan() {
    let (input, mut app) =
        user_provided_artifact_planner_case(Some("user_file"), Some("file"), false, true);
    app.artifacts.get_mut("user_config").unwrap().kind =
        crate::authored_models::AppArtifactKind::Apk;

    let result = plan_execution_with_app_definitions(input, &[app]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result
        .execution_plan
        .expect("user-provided APK plan should emit");
    assert!(plan
        .inputs
        .iter()
        .any(|input| input.id == "planner.param_contract/user_file"));
    let artifact = plan
        .artifacts
        .iter()
        .find(|artifact| artifact.id == "planner.param_contract/user_config")
        .expect("selected user-provided APK should emit");
    assert!(matches!(
        artifact.source,
        crate::planner::ExecutionArtifactSource::LocalInput { .. }
    ));
    assert_eq!(
        artifact
            .app_provenance
            .as_ref()
            .map(|provenance| provenance.kind.as_str()),
        Some("apk")
    );
}

#[test]
fn selected_user_provided_artifact_rejects_missing_or_non_single_file_bindings() {
    let (missing, app) = user_provided_artifact_planner_case(None, None, false, true);
    assert_app_planning_error(missing, &[app], "app_artifact_input_binding_required");

    let (directory, app) =
        user_provided_artifact_planner_case(Some("user_file"), Some("directory"), false, true);
    assert_app_planning_error(directory, &[app], "app_artifact_input_binding_invalid");

    let (multiple, app) =
        user_provided_artifact_planner_case(Some("user_file"), Some("file"), true, true);
    assert_app_planning_error(multiple, &[app], "app_artifact_input_binding_invalid");
}

#[test]
fn unused_optional_user_provided_artifact_needs_no_input_binding() {
    let (input, app) = user_provided_artifact_planner_case(None, None, false, false);
    let result = plan_execution_with_app_definitions(input, &[app]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result
        .execution_plan
        .expect("unused optional artifact should be harmless");
    assert!(plan.inputs.is_empty());
    assert!(!plan
        .artifacts
        .iter()
        .any(|artifact| { artifact.id == "planner.param_contract/user_config" }));
}

#[test]
fn direct_url_file_app_artifact_is_planned_with_file_provenance() {
    let (input, mut app) = user_provided_artifact_planner_case(None, None, false, true);
    app.artifacts.get_mut("user_config").unwrap().source = AppArtifactSource::DirectUrl {
        url: "https://downloads.example.com/retroarch.cfg".to_string(),
        sha256: None,
    };
    let result = plan_execution_with_app_definitions(input, &[app]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.expect("direct file plan should emit");
    let artifact = plan
        .artifacts
        .iter()
        .find(|artifact| artifact.id == "planner.param_contract/user_config")
        .expect("selected direct URL file should emit");
    assert_eq!(
        artifact.source,
        crate::planner::ExecutionArtifactSource::DirectUrl {
            url: "https://downloads.example.com/retroarch.cfg".to_string(),
            sha256: None,
        }
    );
    assert_eq!(
        artifact
            .app_provenance
            .as_ref()
            .map(|provenance| provenance.kind.as_str()),
        Some("file")
    );
}

#[test]
fn latest_release_file_app_artifact_is_planned_and_resolved_offline() {
    let (input, mut app) = user_provided_artifact_planner_case(None, None, false, true);
    let selected_url = "https://github.com/owner/repo/releases/download/v1.2.3/configuration.zip";
    let bytes = b"an app-owned archive, not an APK";
    app.artifacts.get_mut("user_config").unwrap().source = AppArtifactSource::LatestRelease {
        provider: crate::authored_models::ReleaseProvider::Github,
        base_url: "https://api.github.com".to_string(),
        repository: "owner/repo".to_string(),
        asset_pattern: Some("^configuration\\.zip$".to_string()),
        invert_asset_pattern: None,
        prerelease: false,
    };

    let planning = plan_execution_with_app_definitions(input, &[app]);
    assert_eq!(
        planning.status,
        PlanningStatus::Success,
        "{:?}",
        planning.errors
    );
    let execution_plan = planning
        .execution_plan
        .expect("latest-release file plan should be emitted");
    let artifact = execution_plan
        .artifacts
        .iter()
        .find(|artifact| artifact.id == "planner.param_contract/user_config")
        .expect("selected file artifact should be in the execution plan");
    assert!(matches!(
        &artifact.source,
        crate::planner::ExecutionArtifactSource::RemoteRelease {
            provider,
            service_origin,
            repository,
            asset_pattern: Some(pattern),
            ..
        } if provider == "github"
            && service_origin == "https://api.github.com"
            && repository == "owner/repo"
            && pattern == "^configuration\\.zip$"
    ));
    assert_eq!(
        artifact
            .app_provenance
            .as_ref()
            .map(|provenance| provenance.kind.as_str()),
        Some("file")
    );

    let workspace = tempfile::tempdir().expect("executor temp root should be created");
    let cache_root = workspace.path().join("cache");
    fs::create_dir_all(&cache_root).expect("cache root should be created");
    let artifact_cache_path = cache_root.join(crate::artifact_resolver::artifact_local_filename(
        "planner.param_contract/user_config",
        selected_url,
        "default",
    ));
    fs::write(&artifact_cache_path, bytes)
        .expect("selected release cache fixture should be written");

    let mut runner = crate::executor::ExecutorRunner::new(
        crate::executor::DryRunExecutorAdapters::with_sandbox_roots(
            workspace.path().join("runtime"),
            cache_root,
            workspace.path().join("device"),
            Vec::new(),
        ),
    );
    runner.set_github_release_metadata_fixtures(vec![json!([{
        "tag_name": "v1.2.3",
        "prerelease": false,
        "published_at": "2026-02-01T00:00:00Z",
        "assets": [
            {
                "name": "configuration.zip",
                "browser_download_url": selected_url
            },
            {
                "name": "application.apk",
                "browser_download_url": "https://github.com/owner/repo/releases/download/v1.2.3/application.apk"
            }
        ]
    }])]);

    let execution = serde_json::to_value(runner.run(&execution_plan))
        .expect("execution result should serialize");
    assert_eq!(execution["success"], true, "{execution:#}");
    let release = &execution["resolved_releases"][0];
    assert_eq!(
        release["recipeArtifactId"],
        "planner.param_contract/user_config"
    );
    assert_eq!(release["appId"], "armsx1");
    assert_eq!(release["artifactId"], "user_config");
    assert_eq!(release["kind"], "file");
    assert_eq!(release["releaseTag"], "v1.2.3");
    assert_eq!(release["assetName"], "configuration.zip");
    assert_eq!(release["cacheHit"], true);
    assert_eq!(release["calculatedSha256"].as_str().unwrap().len(), 64);
}

fn lifecycle_planner_input(
    step_type: &str,
    app_ref: Option<&str>,
    params: OrderedMap<ParamValue>,
) -> PlannerInput {
    let mut input = armsx1_recipe_planner_input();
    let recipe = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .expect("ARMSX1 recipe");
    let mut lifecycle = recipe
        .steps
        .iter()
        .find(|step| step.type_name == "install_apk")
        .expect("ARMSX1 install step")
        .clone();
    lifecycle.id = "lifecycle".to_string();
    lifecycle.type_name = step_type.to_string();
    lifecycle.name = "App lifecycle".to_string();
    lifecycle.app_ref = app_ref.map(str::to_string);
    lifecycle.dependencies.clear();
    lifecycle.constraints.capabilities.clear();
    lifecycle.skip_if.clear();
    lifecycle.verify.clear();
    lifecycle.params = params;
    recipe.steps.push(lifecycle);
    input
}

#[test]
fn app_lifecycle_plan_contains_concrete_identity_and_keeps_its_snapshot() {
    let mut app = armsx1_app_definition();
    app.launcher_activity = Some("com.nanodata.armsx.MainActivity".to_string());
    let input = lifecycle_planner_input("launch_app", Some("armsx1"), OrderedMap::new());
    let result = plan_execution_with_app_definitions(input, &[app.clone()]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.expect("valid app-aware launch plan");
    let launch = plan
        .steps
        .iter()
        .find(|step| step.id.ends_with("/lifecycle"))
        .unwrap();
    assert_eq!(launch.app_id.as_deref(), Some("armsx1"));
    assert_eq!(launch.params.len(), 2);
    assert_eq!(
        launch.params["package_name"],
        crate::planner::ExecutionParamValue::Literal {
            value: json!("com.nanodata.armsx"),
        }
    );
    assert_eq!(
        launch.params["activity"],
        crate::planner::ExecutionParamValue::Literal {
            value: json!("com.nanodata.armsx.MainActivity"),
        }
    );
    let digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
    app.package_id = "com.example.changed".to_string();
    app.launcher_activity = Some("com.example.changed.OtherActivity".to_string());
    assert_eq!(
        crate::plan_digest::execution_plan_digest(&plan).unwrap(),
        digest
    );
    let parsed =
        crate::cli::parse_execution_plan_json(&serde_json::to_value(&plan).unwrap()).unwrap();
    assert_eq!(parsed.apps, plan.apps);
    assert_eq!(parsed.steps[1].app_id, launch.app_id);
    assert_eq!(parsed.steps[1].params, launch.params);

    let stop = plan_execution_with_app_definitions(
        lifecycle_planner_input("force_stop_app", Some("armsx1"), OrderedMap::new()),
        &[armsx1_app_definition()],
    );
    assert_eq!(stop.status, PlanningStatus::Success, "{:?}", stop.errors);
    let stop_plan = stop.execution_plan.unwrap();
    let stop_step = stop_plan
        .steps
        .iter()
        .find(|step| step.id.ends_with("/lifecycle"))
        .unwrap();
    assert_eq!(stop_step.app_id.as_deref(), Some("armsx1"));
    assert_eq!(stop_step.params.len(), 1);
    assert_eq!(
        stop_step.params["package_name"],
        crate::planner::ExecutionParamValue::Literal {
            value: json!("com.nanodata.armsx"),
        }
    );
}

#[test]
fn app_lifecycle_launch_rejects_missing_or_invalid_launcher() {
    let input = lifecycle_planner_input("launch_app", Some("armsx1"), OrderedMap::new());
    assert_app_planning_error(
        input.clone(),
        &[armsx1_app_definition()],
        "app_launcher_activity_required",
    );
    assert_app_planning_error(
        lifecycle_planner_input("launch_app", Some("unknown-app"), OrderedMap::new()),
        &[armsx1_app_definition()],
        "app_definition_unknown",
    );
    assert_app_planning_error(
        lifecycle_planner_input("launch_app", Some(""), OrderedMap::new()),
        &[armsx1_app_definition()],
        "app_reference_invalid",
    );
    let mut malformed = armsx1_app_definition();
    malformed.launcher_activity = Some("com.nanodata.armsx/.MainActivity".to_string());
    assert_app_planning_error(input, &[malformed], "app_definition_invalid");
}

#[test]
fn lifecycle_form_diagnostics_reject_missing_context_without_catalog() {
    let input = lifecycle_planner_input("launch_app", None, OrderedMap::new());
    let recipe = input
        .recipes
        .iter()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap();
    let diagnostics = crate::validation::validate_loaded_recipe_result(
        recipe,
        std::path::Path::new("app.armsx1.install.yaml"),
        None,
    );
    assert!(
        diagnostics["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["code"] == "app_context_required"
                    && entry["field"]
                        .as_str()
                        .is_some_and(|field| field.ends_with(".app_ref"))
            }),
        "{diagnostics:#}"
    );
}

#[test]
fn app_lifecycle_rejects_overrides_and_retired_literal_forms() {
    let mut app = armsx1_app_definition();
    app.launcher_activity = Some("com.nanodata.armsx.MainActivity".to_string());
    for step_type in ["launch_app", "force_stop_app"] {
        let mut params = OrderedMap::new();
        params.insert(
            "package_name".to_string(),
            ParamValue::Literal(json!("com.example.other")),
        );
        assert_app_planning_error(
            lifecycle_planner_input(step_type, Some("armsx1"), params.clone()),
            &[app.clone()],
            "app_lifecycle_identity_override_forbidden",
        );
        params.insert("package_name".to_string(), ParamValue::Literal(json!(" ")));
        assert_app_planning_error(
            lifecycle_planner_input(step_type, None, params),
            &[app.clone()],
            "app_context_required",
        );
        assert_app_planning_error(
            lifecycle_planner_input(step_type, None, OrderedMap::new()),
            &[app.clone()],
            "app_context_required",
        );
    }
    let mut malformed_activity = OrderedMap::new();
    malformed_activity.insert(
        "package_name".to_string(),
        ParamValue::Literal(json!("com.example.legacy")),
    );
    malformed_activity.insert(
        "activity".to_string(),
        ParamValue::Literal(json!("bad activity")),
    );
    assert_app_planning_error(
        lifecycle_planner_input("launch_app", None, malformed_activity),
        &[app.clone()],
        "app_context_required",
    );

    for activity in [
        ".MainActivity",
        "com.example.legacy/.MainActivity",
        "com.example.legacy/com.example.legacy.MainActivity",
        "com.example.legacy.MainActivity",
    ] {
        let mut legacy = OrderedMap::new();
        legacy.insert(
            "package_name".to_string(),
            ParamValue::Literal(json!("com.example.legacy")),
        );
        legacy.insert("activity".to_string(), ParamValue::Literal(json!(activity)));
        assert_app_planning_error(
            lifecycle_planner_input("launch_app", None, legacy),
            &[app.clone()],
            "app_context_required",
        );
    }
}

#[test]
fn armsx1_plan_contains_app_identity_and_late_bound_release_policy() {
    let mut input = armsx1_recipe_planner_input();
    input.target_device = Some(crate::planner::TargetDeviceBinding {
        serial: "device-1".to_string(),
        manufacturer: Some("Example".to_string()),
        model: Some("Example".to_string()),
        android_api_level: Some(33),
    });
    let result = plan_execution_with_app_definitions(input, &[armsx1_app_definition()]);

    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result
        .execution_plan
        .expect("ARMSX1 plan should be emitted");
    assert_eq!(plan.apps.len(), 1);
    assert_eq!(plan.apps[0].id, "armsx1");
    assert_eq!(plan.apps[0].name, "ARMSX1");
    assert_eq!(plan.apps[0].category.as_deref(), Some("emulator"));
    assert_eq!(plan.apps[0].package_id, "com.nanodata.armsx");
    assert_eq!(plan.artifacts.len(), 1);
    assert_eq!(
        plan.artifacts[0].app_provenance.as_ref().unwrap().app_id,
        "armsx1"
    );
    assert_eq!(
        plan.artifacts[0].app_provenance.as_ref().unwrap().kind,
        "apk"
    );
    assert!(matches!(
        &plan.artifacts[0].source,
        crate::planner::ExecutionArtifactSource::RemoteRelease {
            provider,
            service_origin,
            repository,
            include_prereleases: false,
            asset_pattern,
            invert_asset_pattern: false,
            ..
        } if provider == "github"
            && service_origin == "https://github.com"
            && repository == "ARMSX2/ARMSX1"
            && asset_pattern.as_deref()
                == Some(
                    r"^ARMSX1-(?:release-[0-9]{8}-arm64-v8a|[0-9]+\.[0-9]+\.[0-9]+)\.apk$",
                )
    ));
    let install = plan
        .steps
        .iter()
        .find(|step| step.type_name == "install_apk")
        .expect("ARMSX1 install step should be planned");
    assert_eq!(install.app_id.as_deref(), Some("armsx1"));
    assert_eq!(install.verify[0].app_id.as_deref(), Some("armsx1"));
    assert!(install.verify[0].params.is_empty());

    let serialized = serde_json::to_value(&plan).expect("execution plan should serialize");
    assert_eq!(serialized["apps"][0]["category"], "emulator");
    assert_eq!(serialized["apps"][0]["package_id"], "com.nanodata.armsx");
    assert_eq!(
        serialized["artifacts"][0]["source"]["kind"],
        "remote_release"
    );
    assert!(serialized["artifacts"][0].get("url").is_none());
    let parsed = crate::cli::parse_execution_plan_json(&serialized)
        .expect("serialized app-authority plan should parse for execution");
    assert_eq!(parsed, plan);
    assert_eq!(
        crate::plan_digest::execution_plan_digest(&parsed).unwrap(),
        crate::plan_digest::execution_plan_digest(&plan).unwrap()
    );
    assert_eq!(serialized["schema_version"], 1);
    assert!(serialized["artifacts"][0]["source"]
        .get("invert_asset_pattern")
        .is_none());
}

#[test]
fn latest_release_plan_accepts_an_omitted_filename_pattern_without_changing_v1_shape() {
    let mut app = armsx1_app_definition();
    let Some(crate::authored_models::AppArtifactSource::LatestRelease {
        asset_pattern,
        invert_asset_pattern,
        ..
    }) = app
        .artifacts
        .get_mut("apk")
        .map(|artifact| &mut artifact.source)
    else {
        panic!("ARMSX1 must use a latest-release artifact");
    };
    *asset_pattern = None;
    *invert_asset_pattern = None;

    let mut input = armsx1_recipe_planner_input();
    input.target_device = Some(crate::planner::TargetDeviceBinding {
        serial: "device-1".to_string(),
        manufacturer: Some("Example".to_string()),
        model: Some("Example".to_string()),
        android_api_level: Some(33),
    });
    let result = plan_execution_with_app_definitions(input, &[app]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.unwrap();
    let serialized = serde_json::to_value(&plan).unwrap();
    let source = &serialized["artifacts"][0]["source"];
    assert_eq!(serialized["schema_version"], 1);
    assert!(source.get("asset_pattern").is_none());
    assert!(source.get("invert_asset_pattern").is_none());
    let parsed = crate::cli::parse_execution_plan_json(&serialized).unwrap();
    assert_eq!(parsed, plan);
    assert_eq!(
        crate::plan_digest::execution_plan_digest(&parsed).unwrap(),
        crate::plan_digest::execution_plan_digest(&plan).unwrap()
    );
}

#[test]
fn app_authority_requires_context_for_a_recipe_app_artifact_install() {
    let mut input = armsx1_recipe_planner_input();
    let install = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap()
        .steps
        .iter_mut()
        .find(|step| step.type_name == "install_apk")
        .unwrap();
    install.app_ref = None;

    assert_app_planning_error(input, &[armsx1_app_definition()], "app_context_required");
}

#[test]
fn app_authority_rejects_unknown_step_app_reference() {
    let mut input = armsx1_recipe_planner_input();
    let install = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap()
        .steps
        .iter_mut()
        .find(|step| step.type_name == "install_apk")
        .unwrap();
    install.app_ref = Some("missing-app".to_string());

    assert_app_planning_error(input, &[armsx1_app_definition()], "app_definition_unknown");

    let mut input = armsx1_recipe_planner_input();
    let artifact = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap()
        .artifacts
        .get_mut("installer")
        .unwrap();
    let RecipeArtifact::AppArtifact(reference) = artifact else {
        panic!("ARMSX1 Recipe should refer to an App Definition artifact");
    };
    reference.app_ref = "missing-app".to_string();
    assert_app_planning_error(input, &[armsx1_app_definition()], "app_definition_unknown");
}

fn issue19_permission_planner_case(
    permission_set: &str,
    actions: Vec<crate::authored_models::AppRuntimePermissionAction>,
    context_api_level: Option<i64>,
    target_api_level: Option<i64>,
    include_dependent: bool,
    authored_root_requirement: bool,
) -> (PlannerInput, crate::authored_models::AppDefinitionV1) {
    let mut recipe = recipe_expansion_recipe("issue19.permission", Vec::new());
    let params = ref_params(vec![
        ("permission_set", ParamValue::Literal(json!(permission_set))),
        (
            "policy",
            ParamValue::Literal(json!({ "on_failure": "warn" })),
        ),
    ]);
    let mut permission = param_contract_step("permission", "grant_permissions", Vec::new(), params);
    permission.app_ref = Some("armsx1".to_string());
    if authored_root_requirement {
        permission
            .constraints
            .capabilities
            .push("root_shell".to_string());
    }
    recipe.steps = vec![permission];
    if include_dependent {
        let mut dependent = recipe_expansion_wait_step();
        dependent.id = "dependent".to_string();
        dependent.dependencies.push("permission".to_string());
        recipe.steps.push(dependent);
    }

    let mut input = recipe_expansion_input(vec![recipe], vec!["issue19.permission"]);
    input.device_context.android_api_level = context_api_level;
    input.target_device =
        target_api_level.map(|android_api_level| crate::planner::TargetDeviceBinding {
            serial: "issue19-target".to_string(),
            manufacturer: None,
            model: None,
            android_api_level: Some(android_api_level),
        });

    let selected_permission_set = crate::authored_models::AppPermissionSet {
        runtime: actions,
        app_ops: Vec::new(),
    };
    let mut app = armsx1_app_definition();
    app.permission_sets = Some(if permission_set == "baseline" {
        crate::authored_models::AppPermissionSets {
            baseline: Some(selected_permission_set),
            elevated: None,
        }
    } else {
        crate::authored_models::AppPermissionSets {
            baseline: None,
            elevated: Some(selected_permission_set),
        }
    });

    (input, app)
}

fn issue19_runtime_permission_action(
    permission: &str,
    android_api_min: Option<i64>,
    android_api_max: Option<i64>,
) -> crate::authored_models::AppRuntimePermissionAction {
    crate::authored_models::AppRuntimePermissionAction {
        permission: permission.to_string(),
        android_api_min,
        android_api_max,
    }
}

#[test]
fn issue19_all_inapplicable_elevated_step_stays_selected_without_shell_or_root() {
    let action =
        issue19_runtime_permission_action("android.permission.POST_NOTIFICATIONS", Some(33), None);
    let (mut input, app) =
        issue19_permission_planner_case("elevated", vec![action], Some(35), Some(30), true, false);

    for (root_shell, shell_command) in [(false, true), (true, false)] {
        input.runtime_capabilities.root_shell = root_shell;
        input.runtime_capabilities.shell_command = shell_command;

        let result = plan_execution_with_app_definitions(input.clone(), std::slice::from_ref(&app));

        assert_eq!(
            result.status,
            PlanningStatus::Success,
            "permission action is outside the target API range: {:?}",
            result.errors
        );
        let plan = result.execution_plan.expect("inapplicable permission plan");
        assert_eq!(plan.device_context.android_api_level, Some(30));
        let permission_step = plan
            .steps
            .iter()
            .find(|step| step.id == "issue19.permission/permission")
            .expect("all-inapplicable permission step remains selected");
        let action = &permission_step.permission_plan.as_ref().unwrap().actions[0];
        assert!(matches!(
            action.applicability(),
            crate::planner::PermissionActionApplicability::NotApplicable { .. }
        ));
        assert!(plan
            .steps
            .iter()
            .any(|step| step.id == "issue19.permission/dependent"));
        assert!(permission_step.params.is_empty());

        let plan_json = serde_json::to_value(&plan).unwrap();
        assert!(!crate::executor::root_requirements::reviewed_plan_requires_root_json(&plan_json));
        let mut runner = crate::executor::ExecutorRunner::new(
            crate::executor::DryRunExecutorAdapters::default(),
        );
        let execution = runner.run(&plan);
        assert!(execution.success);
        assert_eq!(execution.steps[0].step_id, "issue19.permission/permission");
        assert_eq!(
            execution.steps[0].status,
            crate::executor::StepRunStatus::Skipped
        );
        assert_eq!(runner.adapters().device().root_preflight_calls(), 0);
        assert!(runner.adapters().device().commands().is_empty());
    }
}

#[test]
fn issue19_mixed_permission_actions_keep_existing_shell_and_root_gates() {
    let actions = vec![
        issue19_runtime_permission_action("android.permission.CAMERA", Some(30), None),
        issue19_runtime_permission_action("android.permission.POST_NOTIFICATIONS", Some(33), None),
    ];
    let (mut input, app) =
        issue19_permission_planner_case("elevated", actions, Some(30), None, false, false);
    input.runtime_capabilities.root_shell = false;
    input.runtime_capabilities.shell_command = true;

    let result = plan_execution_with_app_definitions(input.clone(), std::slice::from_ref(&app));
    assert_eq!(result.status, PlanningStatus::Error);
    assert!(result
        .errors
        .iter()
        .any(|error| error.code == "empty_execution_plan"));

    input.runtime_capabilities.root_shell = true;
    input.runtime_capabilities.shell_command = false;
    let result = plan_execution_with_app_definitions(input.clone(), std::slice::from_ref(&app));
    assert_eq!(result.status, PlanningStatus::Error);
    assert!(result
        .errors
        .iter()
        .any(|error| error.code == "empty_execution_plan"));

    input.runtime_capabilities.root_shell = true;
    input.runtime_capabilities.shell_command = true;
    let result = plan_execution_with_app_definitions(input, &[app]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.unwrap();
    assert!(
        crate::executor::root_requirements::reviewed_plan_requires_root_json(
            &serde_json::to_value(&plan).unwrap()
        )
    );
    let actions = &plan.steps[0].permission_plan.as_ref().unwrap().actions;
    assert!(matches!(
        actions[0].applicability(),
        crate::planner::PermissionActionApplicability::Applicable
    ));
    assert!(matches!(
        actions[1].applicability(),
        crate::planner::PermissionActionApplicability::NotApplicable { .. }
    ));
}

#[test]
fn issue19_all_inapplicable_actions_preserve_authored_root_requirement() {
    let action =
        issue19_runtime_permission_action("android.permission.POST_NOTIFICATIONS", Some(33), None);
    let (mut input, app) =
        issue19_permission_planner_case("elevated", vec![action], Some(35), Some(30), false, true);
    input.runtime_capabilities.root_shell = false;
    input.runtime_capabilities.shell_command = false;

    let result = plan_execution_with_app_definitions(input, &[app]);

    assert_eq!(result.status, PlanningStatus::Error);
    assert!(result
        .errors
        .iter()
        .any(|error| error.code == "empty_execution_plan"));
}

#[test]
fn issue19_unknown_android_api_for_bounded_action_remains_fail_closed() {
    let action =
        issue19_runtime_permission_action("android.permission.POST_NOTIFICATIONS", Some(33), None);
    let (mut input, app) =
        issue19_permission_planner_case("elevated", vec![action], None, None, false, false);
    input.runtime_capabilities.root_shell = false;
    input.runtime_capabilities.shell_command = false;

    assert_app_planning_error(input, &[app], "permission_api_level_unknown");
}

#[test]
fn issue19_pre_change_applicable_and_unrelated_plan_fingerprints() {
    let action = issue19_runtime_permission_action("android.permission.CAMERA", Some(30), None);
    let (input, app) =
        issue19_permission_planner_case("elevated", vec![action], Some(35), None, true, false);
    let applicable = plan_execution_with_app_definitions(input, &[app]);
    assert_eq!(
        applicable.status,
        PlanningStatus::Success,
        "{:?}",
        applicable.errors
    );
    let applicable = applicable.execution_plan.unwrap();
    let applicable_steps = applicable
        .steps
        .iter()
        .map(|step| format!("{}:{}", step.id, step.type_name))
        .collect::<Vec<_>>();
    let applicable_digest = crate::plan_digest::execution_plan_digest(&applicable).unwrap();

    assert_eq!(
        applicable_steps,
        vec![
            "issue19.permission/permission:grant_permissions",
            "issue19.permission/dependent:wait",
        ]
    );
    assert_eq!(
        applicable_digest,
        "a1597e4c3d5ad912b837ea4fa0518e0cc6aef27077c2ae2a296778cd935b0f78"
    );

    let unrelated = crate::planner::plan_execution(recipe_expansion_input(
        vec![recipe_expansion_recipe("issue19.unrelated", Vec::new())],
        vec!["issue19.unrelated"],
    ));
    assert_eq!(
        unrelated.status,
        PlanningStatus::Success,
        "{:?}",
        unrelated.errors
    );
    let unrelated = unrelated.execution_plan.unwrap();
    let unrelated_steps = unrelated
        .steps
        .iter()
        .map(|step| format!("{}:{}", step.id, step.type_name))
        .collect::<Vec<_>>();
    let unrelated_digest = crate::plan_digest::execution_plan_digest(&unrelated).unwrap();

    assert_eq!(unrelated_steps, vec!["issue19.unrelated/wait:wait"]);
    assert_eq!(
        unrelated_digest,
        "dd076985ceece7c47eda8b2c1b79fb0271124f51d8f79bf4f6462871e611ebbe"
    );
}

#[test]
fn unbounded_app_permission_actions_resolve_without_a_known_android_api_level() {
    let mut app = armsx1_app_definition();
    app.permission_sets = Some(crate::authored_models::AppPermissionSets {
        baseline: Some(crate::authored_models::AppPermissionSet {
            runtime: vec![crate::authored_models::AppRuntimePermissionAction {
                permission: "android.permission.CAMERA".to_string(),
                android_api_min: None,
                android_api_max: None,
            }],
            app_ops: Vec::new(),
        }),
        elevated: None,
    });
    let params = ref_params(vec![
        ("permission_set", ParamValue::Literal(json!("baseline"))),
        ("policy", ParamValue::Literal(json!({"on_failure": "warn"}))),
    ]);
    let mut input = lifecycle_planner_input("grant_permissions", Some("armsx1"), params);
    input.device_context.android_api_level = None;

    let result = plan_execution_with_app_definitions(input, &[app]);

    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let step = result
        .execution_plan
        .unwrap()
        .steps
        .into_iter()
        .find(|step| step.type_name == "grant_permissions")
        .unwrap();
    assert!(step.params.is_empty());
    assert!(matches!(
        step.permission_plan.unwrap().actions[0].applicability(),
        crate::planner::PermissionActionApplicability::Applicable
    ));
}

#[test]
fn bounded_elevated_permission_requires_api_level_before_root_capability_pruning() {
    let mut app = armsx1_app_definition();
    app.permission_sets = Some(crate::authored_models::AppPermissionSets {
        baseline: None,
        elevated: Some(crate::authored_models::AppPermissionSet {
            runtime: vec![crate::authored_models::AppRuntimePermissionAction {
                permission: "android.permission.POST_NOTIFICATIONS".to_string(),
                android_api_min: Some(33),
                android_api_max: None,
            }],
            app_ops: Vec::new(),
        }),
    });
    let params = ref_params(vec![
        ("permission_set", ParamValue::Literal(json!("elevated"))),
        ("policy", ParamValue::Literal(json!({"on_failure": "warn"}))),
    ]);
    let mut input = lifecycle_planner_input("grant_permissions", Some("armsx1"), params);
    input.device_context.android_api_level = None;
    input.runtime_capabilities.root_shell = false;

    assert_app_planning_error(input, &[app], "permission_api_level_unknown");
}

#[test]
fn confirmed_out_of_range_app_permission_is_retained_with_its_reason() {
    let mut app = armsx1_app_definition();
    app.permission_sets = Some(crate::authored_models::AppPermissionSets {
        baseline: Some(crate::authored_models::AppPermissionSet {
            runtime: vec![crate::authored_models::AppRuntimePermissionAction {
                permission: "android.permission.POST_NOTIFICATIONS".to_string(),
                android_api_min: Some(33),
                android_api_max: None,
            }],
            app_ops: Vec::new(),
        }),
        elevated: None,
    });
    let params = ref_params(vec![
        ("permission_set", ParamValue::Literal(json!("baseline"))),
        ("policy", ParamValue::Literal(json!({"on_failure": "warn"}))),
    ]);
    let mut input = lifecycle_planner_input("grant_permissions", Some("armsx1"), params);
    input.device_context.android_api_level = Some(35);
    input.target_device = Some(crate::planner::TargetDeviceBinding {
        serial: "target-api-30".to_string(),
        manufacturer: None,
        model: None,
        android_api_level: Some(30),
    });

    let result = plan_execution_with_app_definitions(input, &[app]);

    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.unwrap();
    assert_eq!(plan.device_context.android_api_level, Some(30));
    let step = plan
        .steps
        .into_iter()
        .find(|step| step.type_name == "grant_permissions")
        .unwrap();
    assert_eq!(
        step.permission_plan.unwrap().actions[0].applicability(),
        &crate::planner::PermissionActionApplicability::NotApplicable {
            reason: crate::planner::PermissionNotApplicableReason::AndroidApiBelowMinimum {
                actual: 30,
                minimum: 33,
            },
        }
    );
}

#[test]
fn new_form_permission_reference_is_validated_before_capability_pruning() {
    let params = ref_params(vec![
        ("permission_set", ParamValue::Literal(json!("baseline"))),
        ("policy", ParamValue::Literal(json!({"on_failure": "warn"}))),
    ]);
    let mut input = lifecycle_planner_input("grant_permissions", Some("missing-app"), params);
    input.runtime_capabilities.shell_command = false;

    assert_app_planning_error(input, &[armsx1_app_definition()], "app_definition_unknown");
}

#[test]
fn app_authority_rejects_an_apk_artifact_owned_by_a_different_app() {
    let mut input = armsx1_recipe_planner_input();
    let install = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap()
        .steps
        .iter_mut()
        .find(|step| step.type_name == "install_apk")
        .unwrap();
    install.app_ref = Some("other-app".to_string());
    let mut other_app = armsx1_app_definition();
    other_app.id = "other-app".to_string();
    other_app.name = "Other App".to_string();
    other_app.package_id = "com.example.other".to_string();

    assert_app_planning_error(
        input,
        &[armsx1_app_definition(), other_app],
        "app_install_artifact_provenance_invalid",
    );
}

#[test]
fn app_install_accepts_any_dependency_ancestor_resolver_match() {
    let mut input = armsx1_recipe_planner_input();
    let recipe = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap();
    recipe
        .artifact_groups
        .insert("installer_group".to_string(), vec!["installer".to_string()]);
    let resolver_index = recipe
        .steps
        .iter()
        .position(|step| step.type_name == "resolve_artifacts")
        .expect("ARMSX1 Recipe should have a resolver");
    let mut earlier_resolver = recipe.steps[resolver_index].clone();
    earlier_resolver.id = "earlier_unrelated_resolver".to_string();
    earlier_resolver.dependencies.clear();
    earlier_resolver.params.shift_remove("artifact_groups");
    earlier_resolver.params.insert(
        "artifacts".to_string(),
        crate::model::ParamValue::Literal(json!(["installer"])),
    );

    let dependency_resolver = &mut recipe.steps[resolver_index];
    dependency_resolver.params.shift_remove("artifacts");
    dependency_resolver.params.insert(
        "artifact_groups".to_string(),
        crate::model::ParamValue::Literal(json!(["installer_group"])),
    );
    recipe.steps.insert(0, earlier_resolver);

    let result = plan_execution_with_app_definitions(input, &[armsx1_app_definition()]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "a later matching resolver is a valid dependency ancestor: {:?}",
        result.errors
    );
}

#[test]
fn planner_validates_referenced_app_definition_and_named_artifact() {
    let mut invalid_app = armsx1_app_definition();
    invalid_app.package_id = "not a package".to_string();
    assert_app_planning_error(
        armsx1_recipe_planner_input(),
        &[invalid_app],
        "app_definition_invalid",
    );

    let mut unknown_artifact = armsx1_recipe_planner_input();
    let recipe_artifact = unknown_artifact
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap()
        .artifacts
        .get_mut("installer")
        .unwrap();
    let RecipeArtifact::AppArtifact(reference) = recipe_artifact else {
        panic!("ARMSX1 Recipe should refer to an App Definition artifact");
    };
    reference.artifact = "missing-apk".to_string();
    assert_app_planning_error(
        unknown_artifact,
        &[armsx1_app_definition()],
        "app_artifact_unknown",
    );

    let mut direct_url_app = armsx1_app_definition();
    let expected_sha256 = "a".repeat(64);
    direct_url_app.artifacts.get_mut("apk").unwrap().source = AppArtifactSource::DirectUrl {
        url: "https://example.com/app.apk?channel=stable".to_string(),
        sha256: Some(expected_sha256.clone()),
    };
    let result = plan_execution_with_app_definitions(
        armsx1_recipe_planner_input(),
        &[direct_url_app.clone()],
    );
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result
        .execution_plan
        .expect("direct-URL App Definition should produce an execution plan");
    let artifact = plan
        .artifacts
        .first()
        .expect("the referenced app artifact should be in the plan");
    assert_eq!(artifact.app_provenance.as_ref().unwrap().app_id, "armsx1");
    let serialized = serde_json::to_value(&plan).unwrap();
    assert_eq!(serialized["artifacts"][0]["source"]["kind"], "direct_url");
    assert_eq!(
        serialized["artifacts"][0]["source"]["url"],
        "https://example.com/app.apk?channel=stable"
    );
    assert_eq!(
        serialized["artifacts"][0]["source"]["sha256"],
        expected_sha256
    );
    let parsed = crate::cli::parse_execution_plan_json(&serialized)
        .expect("serialized direct-URL plan should parse for execution");
    assert!(matches!(
        &parsed.artifacts[0].source,
        crate::planner::ExecutionArtifactSource::DirectUrl { url, sha256 }
            if url == "https://example.com/app.apk?channel=stable"
                && sha256.as_deref() == Some(expected_sha256.as_str())
    ));
    assert_eq!(
        parsed.artifacts[0]
            .app_provenance
            .as_ref()
            .map(|provenance| provenance.app_id.as_str()),
        Some("armsx1")
    );

    let mut no_cache_input = armsx1_recipe_planner_input();
    let recipe_artifact = no_cache_input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap()
        .artifacts
        .get_mut("installer")
        .unwrap();
    let RecipeArtifact::AppArtifact(reference) = recipe_artifact else {
        panic!("ARMSX1 Recipe should refer to an App Definition artifact");
    };
    reference.cache = "none".to_string();
    let no_cache_result = plan_execution_with_app_definitions(no_cache_input, &[direct_url_app]);
    assert_eq!(no_cache_result.status, PlanningStatus::Success);
    assert_eq!(
        no_cache_result.execution_plan.unwrap().artifacts[0].cache,
        "none"
    );
}

#[test]
fn app_authority_rejects_non_apk_artifacts_and_missing_resolution_dependencies() {
    let mut input = armsx1_recipe_planner_input();
    let mut app = armsx1_app_definition();
    app.artifacts.get_mut("apk").unwrap().kind = AppArtifactKind::File;
    assert_app_planning_error(input, &[app], "app_install_artifact_provenance_invalid");

    input = armsx1_recipe_planner_input();
    let install = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap()
        .steps
        .iter_mut()
        .find(|step| step.type_name == "install_apk")
        .unwrap();
    install.dependencies.clear();
    assert_app_planning_error(
        input,
        &[armsx1_app_definition()],
        "app_install_resolution_dependency_missing",
    );
}

#[test]
fn app_authority_accepts_transitive_resolver_ancestor_selected_by_artifact_group() {
    let mut input = armsx1_recipe_planner_input();
    let recipe = input
        .recipes
        .iter_mut()
        .find(|recipe| recipe.id == "app.armsx1.install")
        .unwrap();
    recipe
        .artifact_groups
        .insert("apk_assets".to_string(), vec!["installer".to_string()]);

    let resolver_index = recipe
        .steps
        .iter()
        .position(|step| step.type_name == "resolve_artifacts")
        .unwrap();
    let resolver_id = recipe.steps[resolver_index].id.clone();
    let resolver = &mut recipe.steps[resolver_index];
    resolver.params.shift_remove("artifacts");
    resolver.params.insert(
        "artifact_groups".to_string(),
        ParamValue::Literal(json!(["apk_assets"])),
    );

    let install_index = recipe
        .steps
        .iter()
        .position(|step| step.type_name == "install_apk")
        .unwrap();
    let intermediary_id = "prepare_armsx1_install".to_string();
    recipe.steps[install_index].dependencies = vec![intermediary_id.clone()];
    recipe.steps.insert(
        install_index,
        Step {
            id: intermediary_id,
            type_name: "force_stop_app".to_string(),
            name: "Prepare ARMSX1 installation".to_string(),
            description: None,
            progress_note: None,
            user_toggleable: false,
            app_ref: Some("armsx1".to_string()),
            dependencies: vec![resolver_id],
            constraints: StepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            skip_if: Vec::new(),
            params: OrderedMap::new(),
            verify: Vec::new(),
        },
    );

    let result = plan_execution_with_app_definitions(input, &[armsx1_app_definition()]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "transitive resolver ancestry should be accepted: {:?}",
        result.errors
    );
    let plan = result
        .execution_plan
        .expect("ARMSX1 plan should be emitted");
    assert_eq!(plan.artifacts.len(), 1);
    assert_eq!(
        plan.artifacts[0].app_provenance.as_ref().unwrap().app_id,
        "armsx1"
    );
}

#[test]
fn retroarch_package_condition_uses_app_context_without_duplicate_identity() {
    let app = load_app_definition(repo_authored_root().join("apps/retroarch.yaml"))
        .expect("RetroArch App Definition should load");
    let result = plan_execution_with_app_definitions(
        authored_corpus_planner_input(&["app.retroarch.provision"]),
        &[app],
    );

    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.unwrap();
    let install = plan
        .steps
        .iter()
        .find(|step| step.type_name == "install_apk")
        .unwrap();
    assert_eq!(install.app_id.as_deref(), Some("retroarch"));
    assert_eq!(install.skip_if[0].app_id.as_deref(), Some("retroarch"));
    assert!(install.skip_if[0].params.is_empty());
}

fn review_snapshot_recipe() -> Recipe {
    let mode = InputDeclaration {
        type_name: "enum".to_string(),
        role: "mode".to_string(),
        label: "Runtime value".to_string(),
        description: Some("Select runtime value.".to_string()),
        required: true,
        multiple: false,
        validation: InputValidation {
            must_exist: false,
            allowed_extensions: Vec::new(),
            path_kind: None,
            allowed_prefixes: Vec::new(),
        },
        default: Value::Null,
        options: vec![
            crate::model::InputOption {
                value: json!("device"),
                label: "Device".to_string(),
            },
            crate::model::InputOption {
                value: json!("explicit"),
                label: "Explicit".to_string(),
            },
        ],
        sensitive: false,
        advanced: false,
        metadata: OrderedMap::new(),
    };
    let mut credential = mode.clone();
    credential.type_name = "string".to_string();
    credential.role = "credential".to_string();
    credential.label = "Credential".to_string();
    credential.description = None;
    credential.required = false;
    credential.options = Vec::new();
    credential.sensitive = true;
    let mut unused_default = credential.clone();
    unused_default.role = "mode".to_string();
    unused_default.label = "Unused default".to_string();
    unused_default.sensitive = false;
    unused_default.advanced = true;
    unused_default.default = json!("from-default");
    let mut inputs = OrderedMap::new();
    inputs.insert("mode".to_string(), mode);
    inputs.insert("credential".to_string(), credential);
    inputs.insert("unused_default".to_string(), unused_default);
    Recipe {
        schema_version: 1,
        kind: "recipe".to_string(),
        id: "feature.review".to_string(),
        name: "Review Feature".to_string(),
        description: Some("Feature description".to_string()),
        recipe_dependencies: Vec::new(),
        provides: RecipeProvides {
            features: Vec::new(),
        },
        inputs,
        artifacts: OrderedMap::new(),
        artifact_groups: OrderedMap::new(),
        steps: vec![recipe_expansion_wait_step()],
    }
}

fn review_snapshot_input() -> PlannerInput {
    let mut input = recipe_expansion_input(vec![review_snapshot_recipe()], vec!["feature.review"]);
    input
        .explicit_input_bindings
        .insert("feature.review/mode".to_string(), json!("explicit"));
    input
}

fn review_catalog_device_plan_yaml(name: &str, description: &str) -> String {
    format!(
        r#"
schema_version: 1
kind: device_plan
id: review.issue9.catalog
name: "{name}"
description: "{description}"
device_profile_ref: ayaneo.konkr_pocket_fit
recipes:
  - recipe_ref: feature.issue9.review
    selected_by_default: true
defaults: {{}}
overrides: {{}}
metadata: {{}}
"#
    )
}

fn write_review_catalog_recipe(
    path: &Path,
    id: &str,
    name: &str,
    description: &str,
    dependencies: &[&str],
    selected_enum_label: Option<&str>,
) {
    let mut recipe = review_snapshot_recipe();
    recipe.id = id.to_string();
    recipe.name = name.to_string();
    recipe.description = Some(description.to_string());
    recipe.recipe_dependencies = dependencies
        .iter()
        .map(|dependency| dependency.to_string())
        .collect();
    if let Some(label) = selected_enum_label {
        let mode = recipe
            .inputs
            .get_mut("mode")
            .expect("review fixture should include a mode input");
        mode.options
            .iter_mut()
            .find(|option| option.value == json!("explicit"))
            .expect("review fixture should include an explicit enum option")
            .label = label.to_string();
    }
    let yaml = crate::yaml::emit_recipe_yaml(&recipe)
        .expect("review fixture recipe should serialize as authored YAML");
    fs::write(path, yaml).expect("review fixture recipe should be written");
}

#[test]
fn plan_emits_device_plan_recipe_and_resolved_input_review_snapshots() {
    let mut input = review_snapshot_input();
    input
        .user_configuration_bindings
        .insert("feature.review/mode".to_string(), json!("device"));
    input
        .device_plan_input_bindings
        .insert("feature.review/mode".to_string(), json!("device"));
    input.device_plan_presentation = Some(crate::planner::ExecutionDevicePlanSnapshot {
        id: "example.device_plan".to_string(),
        name: Some("Planned Setup".to_string()),
        description: Some("Frozen plan description".to_string()),
    });
    let result = plan_execution_with_app_definitions(input, &[]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.expect("review snapshot plan");
    assert_eq!(
        plan.device_plan,
        Some(crate::planner::ExecutionDevicePlanSnapshot {
            id: "example.device_plan".to_string(),
            name: Some("Planned Setup".to_string()),
            description: Some("Frozen plan description".to_string()),
        })
    );
    assert_eq!(
        plan.recipes,
        vec![crate::planner::ExecutionRecipeSnapshot {
            id: "feature.review".to_string(),
            name: "Review Feature".to_string(),
            description: Some("Feature description".to_string()),
        }]
    );
    assert_eq!(
        plan.resolved_inputs,
        vec![
            crate::planner::ExecutionResolvedInputSnapshot {
                key: "feature.review/mode".to_string(),
                recipe_id: "feature.review".to_string(),
                input_id: "mode".to_string(),
                type_name: "enum".to_string(),
                value: Some(json!("explicit")),
                source: Some(BindingSource::Explicit),
                label: "Runtime value".to_string(),
                description: Some("Select runtime value.".to_string()),
                required: true,
                sensitive: false,
                selected_enum_label: Some("Explicit".to_string()),
            },
            crate::planner::ExecutionResolvedInputSnapshot {
                key: "feature.review/credential".to_string(),
                recipe_id: "feature.review".to_string(),
                input_id: "credential".to_string(),
                type_name: "string".to_string(),
                value: None,
                source: None,
                label: "Credential".to_string(),
                description: None,
                required: false,
                sensitive: true,
                selected_enum_label: None,
            },
            crate::planner::ExecutionResolvedInputSnapshot {
                key: "feature.review/unused_default".to_string(),
                recipe_id: "feature.review".to_string(),
                input_id: "unused_default".to_string(),
                type_name: "string".to_string(),
                value: Some(json!("from-default")),
                source: Some(BindingSource::RecipeDefault),
                label: "Unused default".to_string(),
                description: None,
                required: false,
                sensitive: false,
                selected_enum_label: None,
            },
        ]
    );
    assert!(plan.inputs.is_empty());

    let mut explicit_null = plan.clone();
    explicit_null.resolved_inputs[1].value = Some(Value::Null);
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&explicit_null).unwrap(),
        crate::plan_digest::execution_plan_digest(&plan).unwrap()
    );
    let absent_value = serde_json::to_value(&plan).unwrap();
    let explicit_null_value = serde_json::to_value(&explicit_null).unwrap();
    assert!(absent_value["resolved_inputs"][1].get("value").is_none());
    assert_eq!(
        explicit_null_value["resolved_inputs"][1]["value"],
        Value::Null
    );
}

#[test]
fn catalog_edits_preserve_existing_plan_review_and_refresh_fresh_plan() {
    let temp = temp_authored_root_with_device_plan_selection_yaml(
        &review_catalog_device_plan_yaml("Original Setup", "Original setup description"),
    );
    let authored_root = temp.path().join("authored");
    let device_plan_path = authored_root.join("device_plans/test_device_plan.yaml");
    let selected_recipe_path = authored_root.join("recipes/feature.issue9.review.yaml");
    let dependency_recipe_path =
        authored_root.join("recipes/feature.issue9.review_dependency.yaml");

    write_review_catalog_recipe(
        &selected_recipe_path,
        "feature.issue9.review",
        "Review Feature",
        "Original feature description",
        &["feature.issue9.review_dependency"],
        Some("Explicit"),
    );
    write_review_catalog_recipe(
        &dependency_recipe_path,
        "feature.issue9.review_dependency",
        "Original Dependency",
        "Original dependency description",
        &[],
        None,
    );

    let selected_input_key = "feature.issue9.review/mode";
    let plan_from_catalog = || {
        let mut input = PlannerInput::from_authored_device_plan(
            authored_root.clone(),
            "review.issue9.catalog",
            "plan.issue9.review-catalog".to_string(),
            OrderedMap::new(),
        )
        .unwrap_or_else(|error| panic!("catalog plan input should load: {error}"));
        input
            .explicit_input_bindings
            .insert(selected_input_key.to_string(), json!("explicit"));
        input.explicit_input_bindings.insert(
            "feature.issue9.review_dependency/mode".to_string(),
            json!("device"),
        );
        plan_execution_with_app_definitions(input, &[])
    };

    let original_result = plan_from_catalog();
    assert_eq!(
        original_result.status,
        PlanningStatus::Success,
        "{:?}",
        original_result.errors
    );
    let original_plan = original_result
        .execution_plan
        .expect("original catalog plan");
    assert_eq!(
        original_plan.device_plan.as_ref().unwrap().name.as_deref(),
        Some("Original Setup")
    );
    assert!(original_plan
        .source
        .expanded_recipe_refs
        .contains(&"feature.issue9.review_dependency".to_string()));
    assert_eq!(
        original_plan
            .recipes
            .iter()
            .find(|recipe| recipe.id == "feature.issue9.review")
            .unwrap()
            .name,
        "Review Feature"
    );
    assert_eq!(
        original_plan
            .recipes
            .iter()
            .find(|recipe| recipe.id == "feature.issue9.review_dependency")
            .unwrap()
            .name,
        "Original Dependency"
    );
    assert_eq!(
        original_plan
            .resolved_inputs
            .iter()
            .find(|input| input.key == selected_input_key)
            .unwrap()
            .selected_enum_label
            .as_deref(),
        Some("Explicit")
    );
    let original_digest = crate::plan_digest::execution_plan_digest(&original_plan).unwrap();
    let original_review = crate::review_projection::project_review(&original_plan, &[]);
    let original_review_bytes = serde_json::to_vec(&original_review).unwrap();

    fs::write(
        &device_plan_path,
        review_catalog_device_plan_yaml("Updated Setup", "Updated setup description"),
    )
    .unwrap();
    write_review_catalog_recipe(
        &selected_recipe_path,
        "feature.issue9.review",
        "Updated Review Feature",
        "Updated feature description",
        &["feature.issue9.review_dependency"],
        Some("Updated explicit value"),
    );
    write_review_catalog_recipe(
        &dependency_recipe_path,
        "feature.issue9.review_dependency",
        "Updated Dependency",
        "Updated dependency description",
        &[],
        None,
    );

    assert_eq!(
        serde_json::to_vec(&crate::review_projection::project_review(
            &original_plan,
            &[]
        ))
        .unwrap(),
        original_review_bytes,
        "catalog edits must not change review projected from an existing plan"
    );

    let updated_result = plan_from_catalog();
    assert_eq!(
        updated_result.status,
        PlanningStatus::Success,
        "{:?}",
        updated_result.errors
    );
    let updated_plan = updated_result.execution_plan.expect("updated catalog plan");
    assert_eq!(
        updated_plan.device_plan.as_ref().unwrap().name.as_deref(),
        Some("Updated Setup")
    );
    assert_eq!(
        updated_plan
            .recipes
            .iter()
            .find(|recipe| recipe.id == "feature.issue9.review")
            .unwrap()
            .name,
        "Updated Review Feature"
    );
    assert_eq!(
        updated_plan
            .recipes
            .iter()
            .find(|recipe| recipe.id == "feature.issue9.review_dependency")
            .unwrap()
            .name,
        "Updated Dependency"
    );
    assert_eq!(
        updated_plan
            .resolved_inputs
            .iter()
            .find(|input| input.key == selected_input_key)
            .unwrap()
            .selected_enum_label
            .as_deref(),
        Some("Updated explicit value")
    );
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&updated_plan).unwrap(),
        original_digest
    );
    assert_ne!(
        serde_json::to_vec(&crate::review_projection::project_review(
            &updated_plan,
            &[]
        ))
        .unwrap(),
        original_review_bytes
    );
}

#[test]
fn catalog_deletions_preserve_existing_plan_review_projection() {
    let temp = temp_authored_root_with_device_plan_selection_yaml(
        &review_catalog_device_plan_yaml("Deletion Setup", "Setup retained for review"),
    );
    let authored_root = temp.path().join("authored");
    let device_plan_path = authored_root.join("device_plans/test_device_plan.yaml");
    let selected_recipe_path = authored_root.join("recipes/feature.issue9.review.yaml");
    let dependency_recipe_path =
        authored_root.join("recipes/feature.issue9.review_dependency.yaml");

    write_review_catalog_recipe(
        &selected_recipe_path,
        "feature.issue9.review",
        "Review Feature",
        "Feature retained for review",
        &["feature.issue9.review_dependency"],
        Some("Explicit"),
    );
    write_review_catalog_recipe(
        &dependency_recipe_path,
        "feature.issue9.review_dependency",
        "Review Dependency",
        "Dependency retained for review",
        &[],
        None,
    );

    let mut input = PlannerInput::from_authored_device_plan(
        authored_root,
        "review.issue9.catalog",
        "plan.issue9.review-catalog".to_string(),
        OrderedMap::new(),
    )
    .unwrap_or_else(|error| panic!("catalog plan input should load: {error}"));
    input
        .explicit_input_bindings
        .insert("feature.issue9.review/mode".to_string(), json!("explicit"));
    input.explicit_input_bindings.insert(
        "feature.issue9.review_dependency/mode".to_string(),
        json!("device"),
    );

    let result = plan_execution_with_app_definitions(input, &[]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let retained_plan = result
        .execution_plan
        .expect("catalog plan should be emitted");
    assert_eq!(
        retained_plan.device_plan.as_ref().unwrap().name.as_deref(),
        Some("Deletion Setup")
    );
    assert!(retained_plan
        .recipes
        .iter()
        .any(|recipe| recipe.id == "feature.issue9.review"));
    assert!(retained_plan
        .recipes
        .iter()
        .any(|recipe| recipe.id == "feature.issue9.review_dependency"));
    assert_eq!(
        retained_plan
            .resolved_inputs
            .iter()
            .find(|input| input.key == "feature.issue9.review/mode")
            .unwrap()
            .selected_enum_label
            .as_deref(),
        Some("Explicit")
    );

    let original_review_bytes = serde_json::to_vec(&crate::review_projection::project_review(
        &retained_plan,
        &[],
    ))
    .unwrap();
    fs::remove_file(&device_plan_path).expect("source Device Plan should be removed");
    fs::remove_file(&selected_recipe_path).expect("selected Recipe should be removed");
    fs::remove_file(&dependency_recipe_path).expect("dependency Recipe should be removed");
    assert!(!device_plan_path.exists());
    assert!(!selected_recipe_path.exists());
    assert!(!dependency_recipe_path.exists());

    let retained_review_bytes = serde_json::to_vec(&crate::review_projection::project_review(
        &retained_plan,
        &[],
    ))
    .unwrap();
    assert_eq!(
        retained_review_bytes, original_review_bytes,
        "catalog deletions must not change review projected from a retained plan"
    );
}

#[test]
fn plan_digest_changes_when_review_presentation_snapshots_change() {
    let mut input = review_snapshot_input();
    input.device_plan_presentation = Some(crate::planner::ExecutionDevicePlanSnapshot {
        id: "example.device_plan".to_string(),
        name: Some("Planned Setup".to_string()),
        description: None,
    });
    let plan = plan_execution_with_app_definitions(input, &[])
        .execution_plan
        .expect("review snapshot plan");
    let digest = crate::plan_digest::execution_plan_digest(&plan).unwrap();
    assert_eq!(
        crate::plan_digest::execution_plan_digest(&plan.clone()).unwrap(),
        digest
    );
    let mut renamed = plan.clone();
    renamed.device_plan.as_mut().unwrap().name = Some("Renamed Setup".to_string());
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&renamed).unwrap(),
        digest
    );
    let mut relabeled = plan.clone();
    relabeled.resolved_inputs[0].label = "Renamed label".to_string();
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&relabeled).unwrap(),
        digest
    );
    let mut changed_selected_enum_label = plan.clone();
    changed_selected_enum_label.resolved_inputs[0].selected_enum_label =
        Some("Updated enum choice".to_string());
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&changed_selected_enum_label).unwrap(),
        digest
    );
    let mut redescribed = plan.clone();
    redescribed.device_plan.as_mut().unwrap().description =
        Some("Updated plan description".to_string());
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&redescribed).unwrap(),
        digest
    );
    let mut changed_required = plan.clone();
    changed_required.resolved_inputs[0].required = !changed_required.resolved_inputs[0].required;
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&changed_required).unwrap(),
        digest
    );
    let mut changed_sensitive = plan.clone();
    changed_sensitive.resolved_inputs[0].sensitive =
        !changed_sensitive.resolved_inputs[0].sensitive;
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&changed_sensitive).unwrap(),
        digest
    );

    let mut refreshed_input = review_snapshot_input();
    refreshed_input.device_plan_presentation = Some(crate::planner::ExecutionDevicePlanSnapshot {
        id: "example.device_plan".to_string(),
        name: Some("Planned Setup".to_string()),
        description: None,
    });
    refreshed_input.recipes[0].name = "Updated Review Feature".to_string();
    refreshed_input.recipes[0]
        .inputs
        .get_mut("mode")
        .unwrap()
        .options[1]
        .label = "Updated explicit value".to_string();
    let refreshed_plan = plan_execution_with_app_definitions(refreshed_input, &[])
        .execution_plan
        .expect("refreshed review snapshot plan");
    assert_ne!(
        crate::plan_digest::execution_plan_digest(&refreshed_plan).unwrap(),
        digest
    );
    assert_eq!(
        refreshed_plan.resolved_inputs[0]
            .selected_enum_label
            .as_deref(),
        Some("Updated explicit value")
    );
}

#[test]
fn planner_inputs_without_presentation_keep_review_snapshot_fields_absent() {
    let result = plan_execution_with_app_definitions(review_snapshot_input(), &[]);
    assert_eq!(
        result.status,
        PlanningStatus::Success,
        "{:?}",
        result.errors
    );
    let plan = result.execution_plan.expect("generic plan");
    assert_eq!(plan.device_plan, None);
    assert!(plan.recipes.is_empty());
    assert!(plan.resolved_inputs.is_empty());
    let serialized = serde_json::to_value(&plan).unwrap();
    assert!(serialized.get("device_plan").is_none());
    assert!(serialized.get("resolved_inputs").is_none());
}
