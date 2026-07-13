use std::fs;
use std::path::{Path, PathBuf};

use emuchef_rust_backend::run_with_args_and_input;
use serde_json::{json, Value};
use tempfile::TempDir;

fn write_authored_root(temp: &TempDir) -> PathBuf {
    let root = temp.path().join("authored");
    for directory in ["recipes", "device_plans", "device_profiles", "apps"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    fs::write(
        root.join("recipes/feature.dep.yaml"),
        "schema_version: 1\nkind: recipe\nid: feature.dep\nname: Dependency\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs:\n  dep_value:\n    type: string\n    label: Dependency value\n    required: false\n    default: from-dependency\nartifacts: {}\nartifact_groups: {}\nsteps: []\n",
    )
    .unwrap();
    fs::write(
        root.join("recipes/feature.test.yaml"),
        "schema_version: 1\nkind: recipe\nid: feature.test\nname: Feature test\nrecipe_dependencies: [feature.dep]\nprovides:\n  features: []\ninputs:\n  value:\n    type: enum\n    role: mode\n    label: Runtime value\n    description: Select a runtime value.\n    required: true\n    sensitive: true\n    options:\n      - value: device\n        label: Device\n      - value: saved\n        label: Saved\n      - value: explicit\n        label: Explicit\n  required_missing:\n    type: string\n    label: Missing value\n    required: true\n  destination:\n    type: device_path\n    label: Destination\n    required: true\n    default: /sdcard/Default\n    validation:\n      allowed_prefixes: [/sdcard]\n  toggle:\n    type: boolean\n    label: Advanced toggle\n    advanced: true\n    required: false\n    default: false\nartifacts: {}\nartifact_groups: {}\nsteps:\n  - id: wait\n    type: wait\n    name: Wait\n    user_toggleable: false\n    dependencies: []\n    constraints:\n      capabilities: []\n      conflicts_with: []\n    skip_if: []\n    params:\n      duration_ms: 1\n    verify: []\n",
    )
    .unwrap();
    fs::write(
        root.join("device_profiles/test.profile.yaml"),
        "schema_version: 1\nkind: device_profile\nid: test.profile\nname: Test profile\nmatch: {}\ncapability_defaults:\n  adb_available: true\n  apk_install: true\n  shared_storage_write: true\n  app_launch: true\n  shell_command: true\n  package_remove_for_user: false\n  root_shell: false\n  app_data_write: false\ndevice_tags: []\n",
    )
    .unwrap();
    fs::write(
        root.join("device_plans/test.plan.yaml"),
        "schema_version: 1\nkind: device_plan\nid: test.plan\nname: Test plan\ndevice_profile_ref: test.profile\nrecipes:\n  - recipe_ref: feature.test\n    selected_by_default: true\n  - recipe_ref: feature.dep\n    selected_by_default: false\ndefaults: {}\noverrides:\n  feature.test/value: device\n",
    )
    .unwrap();
    root
}

fn write_configuration(temp: &TempDir, value: &str) -> PathBuf {
    let root = temp.path().join("configurations");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("saved.default.yaml"),
        format!(
            "schema_version: 1\nkind: user_configuration\nid: saved.default\nname: Saved default\ndevice_plan: test.plan\nselected_recipes: [feature.test]\nbindings:\n  feature.test/value:\n    value: {value}\n"
        ),
    )
    .unwrap();
    root
}

fn inline_configuration(value: &str) -> Value {
    json!({
        "schema_version": 1,
        "kind": "user_configuration",
        "id": "inline.default",
        "name": "Inline default",
        "device_plan": "test.plan",
        "selected_recipes": ["feature.test"],
        "bindings": {
            "feature.test/value": { "value": value },
            "feature.test/required_missing": { "value": "complete" },
        },
    })
}

fn describe(payload: Value) -> Value {
    runtime_request("describeConfiguration", payload)
}

fn runtime_request(operation: &str, payload: Value) -> Value {
    let request = json!({ "type": operation, "payload": payload });
    let output = run_with_args_and_input(&[request.to_string()], "");
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    serde_json::from_str(output.stdout.trim()).unwrap()
}

fn input_by_key<'a>(response: &'a Value, key: &str) -> &'a Value {
    response["result"]["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["key"] == key)
        .unwrap_or_else(|| panic!("missing input {key}: {response:#}"))
}

#[test]
fn describes_defaults_dependencies_metadata_and_missing_values_without_side_effects() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let response = describe(json!({
        "authoredRoot": authored_root,
        "devicePlan": "test.plan",
    }));

    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["selectedRecipes"],
        json!(["feature.test"])
    );
    assert_eq!(
        response["result"]["expandedRecipes"],
        json!(["feature.dep", "feature.test"])
    );
    assert_eq!(
        input_by_key(&response, "feature.dep/dep_value")["valueSource"],
        "recipe_default"
    );
    let destination = input_by_key(&response, "feature.test/destination");
    assert_eq!(destination["value"], "/sdcard/Default");
    assert_eq!(destination["valueSource"], "recipe_default");
    assert_eq!(
        destination["validation"]["allowedPrefixes"],
        json!(["/sdcard"])
    );
    let value = input_by_key(&response, "feature.test/value");
    assert_eq!(value["value"], "device");
    assert_eq!(value["valueSource"], "device_plan");
    assert_eq!(value["sensitive"], true);
    assert_eq!(value["options"].as_array().unwrap().len(), 3);
    let missing = input_by_key(&response, "feature.test/required_missing");
    assert_eq!(missing["value"], Value::Null);
    assert_eq!(missing["valueSource"], Value::Null);
    assert!(missing["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "binding_missing"));
    assert_eq!(
        input_by_key(&response, "feature.test/toggle")["advanced"],
        true
    );
}

#[test]
fn explicit_values_shadow_invalid_saved_values_and_keep_provenance() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let configuration_root = write_configuration(&temp, "DO_NOT_LEAK");
    let response = describe(json!({
        "authoredRoot": authored_root,
        "configurationRoot": configuration_root,
        "userConfiguration": "saved.default",
        "bindings": { "feature.test/value": "explicit" },
        "deviceContext": {},
    }));

    assert_eq!(response["ok"], true);
    let value = input_by_key(&response, "feature.test/value");
    assert_eq!(value["value"], "explicit");
    assert_eq!(value["valueSource"], "explicit");
    let serialized = serde_json::to_string(&response["result"]["diagnostics"]).unwrap();
    assert!(!serialized.contains("DO_NOT_LEAK"));
    assert!(!response["result"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["key"] == "feature.test/value"));
}

#[test]
fn inline_configuration_is_shared_by_discovery_and_planning() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let missing_configuration_root = temp.path().join("must-not-be-read");
    let inline = inline_configuration("saved");

    let description = describe(json!({
        "authoredRoot": authored_root,
        "configurationRoot": missing_configuration_root,
        "userConfiguration": inline,
    }));
    assert_eq!(description["ok"], true, "{description:#}");
    assert_eq!(description["result"]["devicePlan"], "test.plan");
    let value = input_by_key(&description, "feature.test/value");
    assert_eq!(value["value"], "saved");
    assert_eq!(value["valueSource"], "user_configuration");
    assert!(!missing_configuration_root.exists());

    let planning = runtime_request(
        "planConfiguration",
        json!({
            "authoredRoot": authored_root,
            "userConfiguration": inline_configuration("saved"),
        }),
    );
    assert_eq!(planning["ok"], true, "{planning:#}");
    assert_eq!(planning["result"]["diagnostics"], json!([]));
    assert_eq!(planning["result"]["plan"]["id"], "plan.test.plan.001");
}

#[test]
fn inline_configuration_honors_explicit_device_plan_replacement() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let mut inline = inline_configuration("saved");
    inline["device_plan"] = json!("missing.saved.plan");

    let response = describe(json!({
        "authoredRoot": authored_root,
        "userConfiguration": inline,
        "devicePlan": "test.plan",
    }));
    assert_eq!(response["ok"], true, "{response:#}");
    assert_eq!(response["result"]["devicePlan"], "test.plan");
    assert!(!response["result"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "device_plan_not_found"));
}

#[test]
fn inline_configuration_honors_explicit_recipe_replacement() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let mut inline = inline_configuration("saved");
    inline["selected_recipes"] = json!(["missing.saved.recipe"]);

    let response = describe(json!({
        "authoredRoot": authored_root,
        "userConfiguration": inline,
        "selectedRecipes": ["feature.test"],
    }));
    assert_eq!(response["ok"], true, "{response:#}");
    assert_eq!(
        response["result"]["selectedRecipes"],
        json!(["feature.test"])
    );
    assert!(!response["result"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "unknown_recipe"));
}

#[test]
fn inline_configuration_rejects_invalid_structure_and_camel_case_aliases() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let mut inline = inline_configuration("saved");
    inline.as_object_mut().unwrap().remove("device_plan");
    inline["devicePlan"] = json!("test.plan");

    let response = describe(json!({
        "authoredRoot": authored_root,
        "userConfiguration": inline,
    }));
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "load_failed");
    assert_eq!(
        response["error"]["details"],
        json!({ "field": "userConfiguration" })
    );
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("device_plan"));
}

#[test]
fn inline_sensitive_invalid_saved_value_can_be_shadowed_without_leaking() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let response = describe(json!({
        "authoredRoot": authored_root,
        "userConfiguration": inline_configuration("DO_NOT_LEAK"),
        "bindings": { "feature.test/value": "explicit" },
    }));

    assert_eq!(response["ok"], true, "{response:#}");
    let value = input_by_key(&response, "feature.test/value");
    assert_eq!(value["value"], "explicit");
    assert_eq!(value["valueSource"], "explicit");
    let serialized = serde_json::to_string(&response).unwrap();
    assert!(!serialized.contains("DO_NOT_LEAK"));
}

#[test]
fn inline_semantic_errors_remain_diagnostics_in_a_success_envelope() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let response = describe(json!({
        "authoredRoot": authored_root,
        "userConfiguration": inline_configuration("not-an-option"),
    }));

    assert_eq!(response["ok"], true, "{response:#}");
    let diagnostic = response["result"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["key"] == "feature.test/value")
        .unwrap();
    assert_eq!(diagnostic["code"], "binding_validation_failed");
    assert_eq!(diagnostic["provenance"], "user_configuration");
    assert!(!serde_json::to_string(diagnostic)
        .unwrap()
        .contains("not-an-option"));
}

#[test]
fn request_device_plan_and_explicit_empty_selection_replace_saved_values() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let configuration_root = write_configuration(&temp, "saved");
    let response = describe(json!({
        "authoredRoot": authored_root,
        "configurationRoot": configuration_root,
        "userConfiguration": "saved.default",
        "devicePlan": "missing.plan",
        "selectedRecipes": [],
    }));

    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["devicePlan"], "missing.plan");
    assert_eq!(response["result"]["selectedRecipes"], json!([]));
    assert_eq!(response["result"]["inputs"], json!([]));
    let codes = response["result"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"device_plan_not_found"));
    assert!(codes.contains(&"binding_recipe_not_selected"));
}

#[test]
fn plan_configuration_returns_a_structured_plan_without_writing_a_plan_file() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let configuration_root = write_configuration(&temp, "DO_NOT_LEAK");
    let plan_path = temp.path().join("must-not-be-written.yaml");
    let response = runtime_request(
        "planConfiguration",
        json!({
            "authoredRoot": authored_root,
            "configurationRoot": configuration_root,
            "userConfiguration": "saved.default",
            "bindings": {
                "feature.test/value": "explicit",
                "feature.test/required_missing": "complete",
            },
            "deviceContext": {
                "manufacturer": "Explicit",
                "model": "Planning Device",
                "androidVersion": 14,
                "androidApiLevel": 34,
                "deviceTags": ["configured"],
            },
            "planPath": plan_path,
        }),
    );

    assert_eq!(response["ok"], true, "{response:#}");
    assert_eq!(response["result"]["diagnostics"], json!([]));
    assert_eq!(response["result"]["plan"]["id"], "plan.test.plan.001");
    assert_eq!(
        response["result"]["plan"]["device_context"],
        json!({
            "manufacturer": "Explicit",
            "model": "Planning Device",
            "android_version": 14,
            "android_api_level": 34,
            "device_tags": ["configured"],
        })
    );
    assert_eq!(
        response["result"]["plan"]["steps"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let value = response["result"]["resolvedInputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["key"] == "feature.test/value")
        .unwrap();
    assert_eq!(value["value"], "explicit");
    assert_eq!(value["source"], "explicit");
    assert!(!plan_path.exists());

    let incomplete = runtime_request(
        "planConfiguration",
        json!({
            "authoredRoot": authored_root,
            "devicePlan": "test.plan",
        }),
    );
    assert_eq!(incomplete["ok"], true);
    assert_eq!(incomplete["result"]["plan"], Value::Null);
    assert!(incomplete["result"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "binding_missing"));
}

#[test]
fn invalid_requests_and_path_forms_return_structured_errors_without_fallback() {
    let temp = TempDir::new().unwrap();
    let authored_root = write_authored_root(&temp);
    let missing_context = describe(json!({ "authoredRoot": authored_root }));
    assert_eq!(missing_context["ok"], false);
    assert_eq!(missing_context["error"]["code"], "invalid_request");

    let missing_path = temp.path().join("missing.yaml");
    let path_response = describe(json!({
        "authoredRoot": authored_root,
        "userConfiguration": missing_path,
    }));
    assert_eq!(path_response["ok"], false);
    assert_eq!(path_response["error"]["code"], "load_failed");
    assert!(!Path::new("missing.yaml").exists());
}
