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
        "schema_version: 1\nkind: recipe\nid: feature.dep\nname: Dependency\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs:\n  dep_value:\n    type: string\n    role: bios\n    label: Dependency value\n    required: false\n    default: from-dependency\nartifacts:\n  dependency_asset:\n    type: remote_file\n    url: https://example.invalid/dependency.zip\n    cache: default\nartifact_groups: {}\nsteps: []\n",
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
    let options = response["result"]["recipeOptions"].as_array().unwrap();
    let selected = options
        .iter()
        .find(|recipe| recipe["id"] == "feature.test")
        .unwrap();
    assert_eq!(selected["selected"], true);
    assert_eq!(selected["recommended"], true);
    let dependency = options
        .iter()
        .find(|recipe| recipe["id"] == "feature.dep")
        .unwrap();
    assert_eq!(dependency["recommended"], false);
    assert_eq!(dependency["dependencyRequired"], true);
    assert_eq!(
        dependency["contentRequirements"],
        json!(["bios_files", "network_download"])
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
    assert_eq!(
        response["result"]["review"],
        json!({
            "setup": { "name": "Test plan" },
            "target": {
                "label": "Connected Android device",
                "manufacturer": "Explicit",
                "model": "Planning Device",
                "androidVersion": 14,
                "androidApiLevel": 34,
            },
            "features": [
                {
                    "name": "Dependency",
                    "automaticallyAdded": true,
                    "sections": [],
                },
                {
                    "name": "Feature test",
                    "automaticallyAdded": false,
                    "sections": [{
                        "kind": "preparation",
                        "label": "Prerequisites and preparation",
                        "actions": [{
                            "title": "Wait",
                            "requirement": "required",
                        }],
                    }],
                },
            ],
            "inputs": [
                { "label": "Dependency value", "summary": "Configured", "required": false },
                { "label": "Runtime value", "summary": "Provided", "required": true },
                { "label": "Missing value", "summary": "Configured", "required": true },
                { "label": "Destination", "summary": "/sdcard/Default", "required": true },
                { "label": "Advanced toggle", "summary": "No", "required": false },
            ],
            "notices": [],
            "work": { "actionCount": 1, "knownWaitSeconds": 1 },
            "canExecute": true,
        })
    );
    let public_review = serde_json::to_string(&response["result"]["review"]).unwrap();
    for forbidden in [
        "feature.test/value",
        "feature.test/required_missing",
        "plan.test.plan.001",
        "explicit",
    ] {
        assert!(
            !public_review.contains(forbidden),
            "review leaked {forbidden}"
        );
    }
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

fn write_target_availability_root(
    temp: &TempDir,
    shared_write: bool,
    app_data_write: bool,
) -> PathBuf {
    let root = temp.path().join("target-authored");
    let production = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
    for directory in ["recipes", "device_plans", "device_profiles", "apps"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    for recipe in [
        "app.retroarch.provision",
        "feature.copy_bios",
        "feature.copy_roms",
    ] {
        fs::copy(
            production.join(format!("recipes/{recipe}.yaml")),
            root.join(format!("recipes/{recipe}.yaml")),
        )
        .unwrap();
    }
    fs::copy(
        production.join("apps/retroarch.yaml"),
        root.join("apps/retroarch.yaml"),
    )
    .unwrap();
    fs::write(
        root.join("device_profiles/target.profile.yaml"),
        format!("schema_version: 1\nkind: device_profile\nid: target.profile\nname: Target profile\nmatch: {{}}\ncapability_defaults:\n  adb_available: true\n  apk_install: true\n  shared_storage_write: {shared_write}\n  app_launch: true\n  shell_command: true\n  package_remove_for_user: false\n  root_shell: true\n  app_data_write: {app_data_write}\ndevice_tags: []\n"),
    ).unwrap();
    fs::write(
        root.join("device_plans/target.plan.yaml"),
        "schema_version: 1\nkind: device_plan\nid: target.plan\nname: Target plan\ndevice_profile_ref: target.profile\nrecipes:\n  - recipe_ref: app.retroarch.provision\n    selected_by_default: true\n  - recipe_ref: feature.copy_bios\n    selected_by_default: true\n  - recipe_ref: feature.copy_roms\n    selected_by_default: true\ndefaults: {}\noverrides: {}\n",
    ).unwrap();
    root
}

fn issue19_wait_step(id: &str, capabilities: &[&str], dependencies: &[&str]) -> String {
    let capabilities = if capabilities.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", capabilities.join(", "))
    };
    let dependencies = if dependencies.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", dependencies.join(", "))
    };
    format!(
        "  - id: {id}\n    type: wait\n    name: {id}\n    user_toggleable: false\n    dependencies: {dependencies}\n    constraints:\n      capabilities: {capabilities}\n      conflicts_with: []\n    skip_if: []\n    params:\n      duration_ms: 1\n    verify: []\n"
    )
}

fn write_issue19_recipe(root: &Path, recipe_id: &str, steps: &[String]) {
    let mut recipe = format!(
        "schema_version: 1\nkind: recipe\nid: {recipe_id}\nname: {recipe_id}\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs: {{}}\nartifacts: {{}}\nartifact_groups: {{}}\n"
    );
    if steps.is_empty() {
        recipe.push_str("steps: []\n");
    } else {
        recipe.push_str("steps:\n");
        recipe.push_str(&steps.join(""));
    }
    fs::write(root.join(format!("recipes/{recipe_id}.yaml")), recipe).unwrap();

    let plan_path = root.join("device_plans/target.plan.yaml");
    let plan = fs::read_to_string(&plan_path).unwrap();
    let recipe_entry = format!("  - recipe_ref: {recipe_id}\n    selected_by_default: false\n");
    let updated = plan.replacen("defaults: {}", &format!("{recipe_entry}defaults: {{}}"), 1);
    assert_ne!(
        updated, plan,
        "device plan fixture must include {recipe_id}"
    );
    fs::write(plan_path, updated).unwrap();
}

fn write_issue19_availability_root(
    temp: &TempDir,
    root_shell: bool,
    shell_command: bool,
    shared_storage_write: bool,
    app_data_write: bool,
) -> PathBuf {
    let root = write_target_availability_root(temp, shared_storage_write, app_data_write);
    let profile_path = root.join("device_profiles/target.profile.yaml");
    let profile = fs::read_to_string(&profile_path).unwrap();
    let profile = profile
        .replace("  root_shell: true", &format!("  root_shell: {root_shell}"))
        .replace(
            "  shell_command: true",
            &format!("  shell_command: {shell_command}"),
        );
    fs::write(profile_path, profile).unwrap();

    let production = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored");
    fs::copy(
        production.join("apps/armsx1.yaml"),
        root.join("apps/armsx1.yaml"),
    )
    .unwrap();
    let app_path = root.join("apps/armsx1.yaml");
    let app = fs::read_to_string(&app_path).unwrap();
    fs::write(
        app_path,
        format!(
            "{app}\npermission_sets:\n  elevated:\n    runtime:\n      - permission: android.permission.POST_NOTIFICATIONS\n        android_api_min: 33\n    app_ops: []\n"
        ),
    )
    .unwrap();

    write_issue19_recipe(
        &root,
        "issue19.independent",
        &[
            issue19_wait_step("root", &["root_shell"], &[]),
            issue19_wait_step("shared", &["shared_storage_write"], &[]),
        ],
    );
    write_issue19_recipe(
        &root,
        "issue19.dependent",
        &[
            issue19_wait_step("root", &["root_shell"], &[]),
            issue19_wait_step("shared", &["shared_storage_write"], &["root"]),
        ],
    );
    write_issue19_recipe(
        &root,
        "issue19.conjunctive",
        &[issue19_wait_step(
            "both",
            &["root_shell", "shared_storage_write"],
            &[],
        )],
    );
    write_issue19_recipe(&root, "issue19.empty", &[]);
    write_issue19_recipe(
        &root,
        "issue19.permission",
        &["  - id: elevated\n    type: grant_permissions\n    name: Elevated permissions\n    user_toggleable: false\n    app_ref: armsx1\n    dependencies: []\n    constraints:\n      capabilities: []\n      conflicts_with: []\n    skip_if: []\n    params:\n      permission_set: elevated\n      policy:\n        on_failure: warn\n    verify: []\n".to_string()],
    );
    root
}

fn issue19_configuration_payload(root: &Path, recipe_id: &str) -> Value {
    json!({
        "authoredRoot": root,
        "devicePlan": "target.plan",
        "selectedRecipes": [recipe_id],
    })
}

fn target_recipe_option<'a>(response: &'a Value, recipe_id: &str) -> &'a Value {
    response["result"]["recipeOptions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["id"] == recipe_id)
        .unwrap_or_else(|| panic!("missing recipe option {recipe_id}: {response:#}"))
}

#[test]
fn issue19_independent_capability_paths_are_checked_against_planner_graphs() {
    for (root_shell, shared_storage_write, expected_available, expected_missing, expected_steps) in [
        (
            false,
            false,
            false,
            json!(["root_shell", "shared_storage_write"]),
            vec![],
        ),
        (
            true,
            false,
            true,
            json!([]),
            vec!["issue19.independent/root"],
        ),
        (
            false,
            true,
            true,
            json!([]),
            vec!["issue19.independent/shared"],
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let root =
            write_issue19_availability_root(&temp, root_shell, true, shared_storage_write, false);
        let payload = issue19_configuration_payload(&root, "issue19.independent");
        let description = describe(payload.clone());
        let plan = runtime_request("planConfiguration", payload);
        let option = target_recipe_option(&description, "issue19.independent");
        let emitted_steps = plan["result"]["plan"]["steps"]
            .as_array()
            .map(|steps| {
                steps
                    .iter()
                    .map(|step| step["id"].as_str().unwrap().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        assert_eq!(option["available"], expected_available, "{option:#}");
        assert_eq!(
            option["unavailableCapabilities"], expected_missing,
            "{option:#}"
        );
        assert!(option.get("requiredCapabilities").is_none(), "{option:#}");
        assert_eq!(
            option["available"],
            !emitted_steps.is_empty(),
            "Recipe availability must agree with the planner graph: {plan:#}"
        );
        assert_eq!(emitted_steps, expected_steps);
    }
}

#[test]
fn issue19_app_data_only_recipe_reports_its_real_missing_capability() {
    for (app_data_write, expected_available, expected_missing) in [
        (false, false, json!(["app_data_write"])),
        (true, true, json!([])),
    ] {
        let temp = TempDir::new().unwrap();
        let root = write_issue19_availability_root(&temp, true, true, true, app_data_write);
        write_issue19_recipe(
            &root,
            "issue19.app_data",
            &[issue19_wait_step("write", &["app_data_write"], &[])],
        );
        let payload = issue19_configuration_payload(&root, "issue19.app_data");
        let description = describe(payload.clone());
        let plan = runtime_request("planConfiguration", payload);
        let option = target_recipe_option(&description, "issue19.app_data");
        let planned_steps = plan["result"]["plan"]["steps"]
            .as_array()
            .map_or(0, Vec::len);

        assert_eq!(option["available"], expected_available, "{option:#}");
        assert_eq!(
            option["unavailableCapabilities"], expected_missing,
            "{option:#}"
        );
        assert_eq!(option["available"], planned_steps > 0, "{plan:#}");
    }
}

#[test]
fn issue19_dependency_pruning_reports_only_a_runnable_candidate_blocker() {
    for (root_shell, shared_storage_write, expected_available, expected_missing, expected_steps) in [
        (false, false, false, json!(["root_shell"]), vec![]),
        (false, true, false, json!(["root_shell"]), vec![]),
        (true, false, true, json!([]), vec!["issue19.dependent/root"]),
    ] {
        let temp = TempDir::new().unwrap();
        let root =
            write_issue19_availability_root(&temp, root_shell, true, shared_storage_write, false);
        let payload = issue19_configuration_payload(&root, "issue19.dependent");
        let description = describe(payload.clone());
        let plan = runtime_request("planConfiguration", payload);
        let option = target_recipe_option(&description, "issue19.dependent");
        let emitted_steps = plan["result"]["plan"]["steps"]
            .as_array()
            .map(|steps| {
                steps
                    .iter()
                    .map(|step| step["id"].as_str().unwrap().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        assert_eq!(option["available"], expected_available, "{option:#}");
        assert_eq!(
            option["unavailableCapabilities"], expected_missing,
            "{option:#}"
        );
        assert!(option.get("requiredCapabilities").is_none(), "{option:#}");
        assert_eq!(option["available"], !emitted_steps.is_empty());
        assert_eq!(emitted_steps, expected_steps);
    }
}

#[test]
fn issue19_conjunctive_step_reports_each_required_missing_capability() {
    let temp = TempDir::new().unwrap();
    let root = write_issue19_availability_root(&temp, false, true, false, false);
    let payload = issue19_configuration_payload(&root, "issue19.conjunctive");
    let description = describe(payload.clone());
    let plan = runtime_request("planConfiguration", payload);
    let option = target_recipe_option(&description, "issue19.conjunctive");

    assert_eq!(option["available"], false, "{option:#}");
    assert_eq!(
        option["unavailableCapabilities"],
        json!(["root_shell", "shared_storage_write"]),
        "{option:#}"
    );
    assert!(option.get("requiredCapabilities").is_none(), "{option:#}");
    assert_eq!(plan["result"]["plan"], Value::Null);
}

#[test]
fn issue19_empty_and_optional_input_only_graphs_keep_existing_availability() {
    let temp = TempDir::new().unwrap();
    let root = write_issue19_availability_root(&temp, false, true, true, false);
    let empty_payload = issue19_configuration_payload(&root, "issue19.empty");
    let empty_description = describe(empty_payload.clone());
    let empty_plan = runtime_request("planConfiguration", empty_payload);
    let empty = target_recipe_option(&empty_description, "issue19.empty");
    assert_eq!(empty["available"], true, "{empty:#}");
    assert_eq!(empty["unavailableCapabilities"], json!([]));
    assert!(empty.get("requiredCapabilities").is_none(), "{empty:#}");
    assert_eq!(empty_plan["result"]["plan"], Value::Null);

    let optional_path = root.join("recipes/feature.copy_bios.yaml");
    let optional_recipe = fs::read_to_string(&optional_path).unwrap();
    assert!(optional_recipe.contains("required: true"));
    fs::write(
        &optional_path,
        optional_recipe.replace("required: true", "required: false"),
    )
    .unwrap();
    let optional_payload = issue19_configuration_payload(&root, "feature.copy_bios");
    let optional_description = describe(optional_payload.clone());
    let optional_plan = runtime_request("planConfiguration", optional_payload);
    let optional = target_recipe_option(&optional_description, "feature.copy_bios");
    assert_eq!(optional["available"], true, "{optional:#}");
    assert_eq!(optional["unavailableCapabilities"], json!([]));
    assert!(
        optional.get("requiredCapabilities").is_none(),
        "{optional:#}"
    );
    assert_eq!(optional_plan["result"]["plan"], Value::Null);
}

#[test]
fn issue19_unsupported_capability_does_not_make_an_executable_recipe_look_empty() {
    let temp = TempDir::new().unwrap();
    let root = write_issue19_availability_root(&temp, true, true, true, false);
    write_issue19_recipe(
        &root,
        "issue19.unsupported",
        &[issue19_wait_step(
            "unsupported",
            &["future_device_privilege"],
            &[],
        )],
    );
    let payload = issue19_configuration_payload(&root, "issue19.unsupported");
    let description = describe(payload.clone());
    let plan = runtime_request("planConfiguration", payload);
    let option = target_recipe_option(&description, "issue19.unsupported");

    assert_eq!(option["available"], false, "{option:#}");
    assert_eq!(option["unavailableCapabilities"], json!([]));
    assert!(
        description["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["key"] == "issue19.unsupported"
                    && diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("future_device_privilege"))
            }),
        "{:#}",
        description["result"]["diagnostics"]
    );
    assert_eq!(plan["result"]["plan"], Value::Null);
}

#[test]
fn issue19_unsupported_capability_warning_preserves_other_retained_work() {
    let temp = TempDir::new().unwrap();
    let root = write_issue19_availability_root(&temp, true, true, true, false);
    write_issue19_recipe(
        &root,
        "issue19.mixed_unsupported",
        &[
            issue19_wait_step("unsupported", &["future_device_privilege"], &[]),
            issue19_wait_step("ordinary", &[], &[]),
        ],
    );
    let payload = issue19_configuration_payload(&root, "issue19.mixed_unsupported");
    let description = describe(payload.clone());
    let plan = runtime_request("planConfiguration", payload);
    let option = target_recipe_option(&description, "issue19.mixed_unsupported");
    let steps = plan["result"]["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["id"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(option["available"], true, "{option:#}");
    assert_eq!(option["unavailableCapabilities"], json!([]));
    assert_eq!(steps, vec!["issue19.mixed_unsupported/ordinary"]);
    assert!(
        description["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "unsupported_runtime_capability"
                    && diagnostic["key"] == "issue19.mixed_unsupported"
                    && diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("future_device_privilege"))
            }),
        "{:#}",
        description["result"]["diagnostics"]
    );
}

#[test]
fn issue19_unbound_user_provided_app_artifact_remains_selectable() {
    let temp = TempDir::new().unwrap();
    let root = write_issue19_availability_root(&temp, true, true, true, false);
    let app_path = root.join("apps/armsx1.yaml");
    let app = fs::read_to_string(&app_path).unwrap();
    let updated_app = app.replacen(
        "artifacts:\n",
        "artifacts:\n  user_config:\n    kind: file\n    source:\n      strategy: user_provided\n",
        1,
    );
    assert_ne!(app, updated_app);
    fs::write(&app_path, updated_app).unwrap();

    write_issue19_recipe(&root, "issue19.user_artifact", &[]);
    fs::write(
        root.join("recipes/issue19.user_artifact.yaml"),
        "schema_version: 1\nkind: recipe\nid: issue19.user_artifact\nname: User artifact\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs:\n  user_file:\n    type: file\n    role: source\n    label: User file\n    required: true\n    multiple: false\n    validation:\n      must_exist: false\n      allowed_extensions: []\n      path_kind: file\n    default: null\nartifacts:\n  user_config:\n    type: app_artifact\n    app_ref: armsx1\n    artifact: user_config\n    input:\n      ref: inputs.user_file\nsteps:\n  - id: resolve\n    type: resolve_artifacts\n    name: Resolve user configuration\n    user_toggleable: false\n    dependencies: []\n    constraints:\n      capabilities: []\n      conflicts_with: []\n    skip_if: []\n    params:\n      artifacts: [user_config]\n    verify: []\n  - id: ordinary\n    type: wait\n    name: Ordinary work\n    user_toggleable: false\n    dependencies: []\n    constraints:\n      capabilities: []\n      conflicts_with: []\n    skip_if: []\n    params:\n      duration_ms: 1\n    verify: []\n",
    )
    .unwrap();

    let payload = issue19_configuration_payload(&root, "issue19.user_artifact");
    let description = describe(payload.clone());
    let plan = runtime_request("planConfiguration", payload);
    let option = target_recipe_option(&description, "issue19.user_artifact");

    assert_eq!(option["available"], true, "{option:#}");
    assert_eq!(option["unavailableCapabilities"], json!([]));
    assert_eq!(plan["result"]["plan"], Value::Null, "{plan:#}");
    assert!(
        plan["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "binding_missing"),
        "{plan:#}"
    );
}

#[test]
fn issue19_applicable_elevated_permission_prunes_without_losing_independent_work() {
    for (root_shell, expected_steps) in [
        (false, vec!["issue19.permission/ordinary"]),
        (
            true,
            vec!["issue19.permission/elevated", "issue19.permission/ordinary"],
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let root = write_issue19_availability_root(&temp, root_shell, true, true, false);
        let recipe_path = root.join("recipes/issue19.permission.yaml");
        let recipe = fs::read_to_string(&recipe_path).unwrap();
        fs::write(
            recipe_path,
            format!("{recipe}{}", issue19_wait_step("ordinary", &[], &[])),
        )
        .unwrap();

        let mut payload = issue19_configuration_payload(&root, "issue19.permission");
        payload["deviceContext"] = json!({ "androidApiLevel": 35 });
        payload["targetDevice"] = json!({
            "serial": "issue19-target",
            "androidApiLevel": 35,
        });
        let description = describe(payload.clone());
        let plan = runtime_request("planConfiguration", payload);
        let option = target_recipe_option(&description, "issue19.permission");
        let emitted_steps = plan["result"]["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["id"].as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(option["available"], true, "{option:#}");
        assert_eq!(option["unavailableCapabilities"], json!([]));
        assert_eq!(emitted_steps, expected_steps);
        assert_eq!(option["available"], !emitted_steps.is_empty());
    }
}

#[test]
fn issue19_api_inapplicable_elevated_permission_is_available_without_shell_or_root() {
    let temp = TempDir::new().unwrap();
    let root = write_issue19_availability_root(&temp, false, false, false, false);
    let mut payload = issue19_configuration_payload(&root, "issue19.permission");
    payload["deviceContext"] = json!({ "androidApiLevel": 35 });
    payload["targetDevice"] = json!({
        "serial": "issue19-target",
        "androidApiLevel": 30,
    });
    let description = describe(payload.clone());
    let plan = runtime_request("planConfiguration", payload);
    let option = target_recipe_option(&description, "issue19.permission");

    assert_eq!(
        option["available"], true,
        "option={option:#}; diagnostics={:#}",
        description["result"]["diagnostics"]
    );
    assert_eq!(option["unavailableCapabilities"], json!([]));
    assert!(option.get("requiredCapabilities").is_none(), "{option:#}");
    let plan = plan["result"]["plan"].as_object().unwrap();
    assert_eq!(plan["steps"].as_array().unwrap().len(), 1);
    assert_eq!(plan["steps"][0]["id"], "issue19.permission/elevated");
    assert_eq!(plan["steps"][0]["constraints"]["capabilities"], json!([]));
}

#[test]
fn issue19_unknown_android_api_fails_closed_without_invented_capability_blockers() {
    let temp = TempDir::new().unwrap();
    let root = write_issue19_availability_root(&temp, false, false, false, false);
    let payload = issue19_configuration_payload(&root, "issue19.permission");
    let description = describe(payload.clone());
    let plan = runtime_request("planConfiguration", payload);
    let option = target_recipe_option(&description, "issue19.permission");

    assert_eq!(option["available"], false, "{option:#}");
    assert_eq!(option["unavailableCapabilities"], json!([]));
    assert!(option.get("requiredCapabilities").is_none(), "{option:#}");
    assert!(description["result"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| {
            diagnostic["code"] == "permission_api_level_unknown"
                && diagnostic["key"] == "issue19.permission"
        }));
    assert_eq!(plan["result"]["plan"], Value::Null);
}

#[test]
fn target_backed_recipe_options_include_derived_capabilities_and_preserve_generic_rom() {
    let temp = TempDir::new().unwrap();
    let root = write_target_availability_root(&temp, true, true);
    let response = describe(json!({
        "authoredRoot": root,
        "devicePlan": "target.plan",
        "deviceContext": {"androidApiLevel": 35},
    }));
    assert_eq!(response["ok"], true, "{response:#}");
    for recipe_id in [
        "app.retroarch.provision",
        "feature.copy_bios",
        "feature.copy_roms",
    ] {
        let option = target_recipe_option(&response, recipe_id);
        assert_eq!(
            option["available"], true,
            "{recipe_id}: {option:#}; diagnostics={:#}",
            response["result"]["diagnostics"]
        );
        assert_eq!(
            option["unavailableCapabilities"],
            json!([]),
            "{recipe_id}: {option:#}"
        );
        assert!(option.get("requiredCapabilities").is_none(), "{option:#}");
    }
}

#[test]
fn target_backed_recipe_options_report_missing_write_capabilities() {
    for (shared_write, app_data_write) in [(false, true), (true, false)] {
        let temp = TempDir::new().unwrap();
        let root = write_target_availability_root(&temp, shared_write, app_data_write);
        let response = describe(json!({
            "authoredRoot": root,
            "devicePlan": "target.plan",
            "deviceContext": {"androidApiLevel": 35},
        }));
        assert_eq!(response["ok"], true, "{response:#}");
        for recipe_id in [
            "app.retroarch.provision",
            "feature.copy_bios",
            "feature.copy_roms",
        ] {
            let option = target_recipe_option(&response, recipe_id);
            let blocked_by_shared_storage =
                !shared_write && matches!(recipe_id, "feature.copy_bios" | "feature.copy_roms");
            assert_eq!(
                option["available"], !blocked_by_shared_storage,
                "{recipe_id}: {option:#}"
            );
            assert_eq!(
                option["unavailableCapabilities"],
                if blocked_by_shared_storage {
                    json!(["shared_storage_write"])
                } else {
                    json!([])
                },
                "{recipe_id}: {option:#}"
            );
            assert!(option.get("requiredCapabilities").is_none(), "{option:#}");
        }
    }
}

#[test]
fn invalid_target_references_never_fall_back_to_authored_only_availability() {
    for (change, expected_code) in [
        ("unknown", "app_target_unknown"),
        ("wrong_kind", "app_target_kind_invalid"),
        ("cross_app_source", "app_source_provenance_mismatch"),
    ] {
        let temp = TempDir::new().unwrap();
        let root = write_target_availability_root(&temp, true, true);
        let path = root.join("recipes/feature.copy_bios.yaml");
        let mut recipe = fs::read_to_string(&path).unwrap();
        match change {
            "unknown" => recipe = recipe.replace("app_target: system", "app_target: absent"),
            "wrong_kind" => recipe = recipe.replace("app_target: system", "app_target: config"),
            "cross_app_source" => {
                fs::copy(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../authored/apps/armsx1.yaml"),
                    root.join("apps/armsx1.yaml"),
                )
                .unwrap();
                recipe = recipe.replace("steps:\n", "artifacts:\n  foreign_asset:\n    type: app_artifact\n    app_ref: armsx1\n    artifact: apk\nsteps:\n");
                recipe = recipe.replace(
                    "ref: inputs.bios_source_dir",
                    "ref: artifacts.foreign_asset.local_path",
                );
            }
            _ => unreachable!(),
        }
        fs::write(path, recipe).unwrap();
        let response = describe(json!({"authoredRoot": root, "devicePlan": "target.plan"}));
        assert_eq!(response["ok"], true, "{change}: {response:#}");
        let option = target_recipe_option(&response, "feature.copy_bios");
        assert_eq!(option["available"], false, "{change}: {option:#}");
        assert!(
            option.get("requiredCapabilities").is_none(),
            "{change}: {option:#}"
        );
        assert_eq!(option["unavailableCapabilities"], json!([]));
        assert!(
            response["result"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["code"] == expected_code
                    && diagnostic["key"] == "feature.copy_bios"
                    && diagnostic["details"]["step_id"] == "copy_bios_dir"),
            "{change}: {response:#}"
        );
        assert_eq!(
            target_recipe_option(&response, "feature.copy_roms")["available"],
            true
        );
    }
}

#[test]
fn target_errors_block_only_selected_or_dependency_expanded_recipes() {
    for (selection, rom_depends_on_bios, expected_severity) in [
        (vec!["feature.copy_roms"], false, "warning"),
        (vec!["feature.copy_bios"], false, "error"),
        (vec!["feature.copy_roms"], true, "error"),
    ] {
        let temp = TempDir::new().unwrap();
        let root = write_target_availability_root(&temp, true, true);
        let bios_path = root.join("recipes/feature.copy_bios.yaml");
        let bios = fs::read_to_string(&bios_path).unwrap();
        fs::write(
            &bios_path,
            bios.replace("app_target: system", "app_target: absent"),
        )
        .unwrap();
        if rom_depends_on_bios {
            let rom_path = root.join("recipes/feature.copy_roms.yaml");
            let rom = fs::read_to_string(&rom_path).unwrap();
            fs::write(
                rom_path,
                rom.replace(
                    "recipe_dependencies: []",
                    "recipe_dependencies: [feature.copy_bios]",
                ),
            )
            .unwrap();
        }
        let source = temp.path().join("rom-source");
        fs::create_dir(&source).unwrap();
        let selected_recipe_id = selection[0];
        let payload = json!({
            "authoredRoot": root,
            "devicePlan": "target.plan",
            "selectedRecipes": selection,
            "bindings": { "feature.copy_roms/source": source },
        });
        let response = describe(payload.clone());
        let plan = runtime_request("planConfiguration", payload);
        assert_eq!(response["ok"], true, "{response:#}");
        let bios = target_recipe_option(&response, "feature.copy_bios");
        assert_eq!(bios["available"], false, "{response:#}");
        assert!(bios.get("requiredCapabilities").is_none(), "{response:#}");
        let selected_option = target_recipe_option(&response, selected_recipe_id);
        let plan_has_steps = plan["result"]["plan"]["steps"]
            .as_array()
            .is_some_and(|steps| !steps.is_empty());
        assert_eq!(
            selected_option["available"], plan_has_steps,
            "Recipe availability must match the planner graph with the same binding: {plan:#}"
        );
        assert_eq!(
            bios["dependencyRequired"], rom_depends_on_bios,
            "{response:#}"
        );
        let target_diagnostics = response["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "app_target_unknown")
            .collect::<Vec<_>>();
        assert!(!target_diagnostics.is_empty(), "{response:#}");
        assert!(
            target_diagnostics.iter().all(|diagnostic| {
                diagnostic["severity"] == expected_severity
                    && matches!(
                        diagnostic["key"].as_str(),
                        Some("feature.copy_bios" | "feature.copy_roms")
                    )
                    && diagnostic["details"]["step_id"] == "copy_bios_dir"
            }),
            "{response:#}"
        );
        if expected_severity == "warning" {
            assert!(
                response["result"]["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|diagnostic| diagnostic["severity"] != "error"),
                "{response:#}"
            );
            let rom = target_recipe_option(&response, "feature.copy_roms");
            assert_eq!(rom["available"], true, "{response:#}");
            assert_eq!(rom["unavailableCapabilities"], json!([]));
            assert!(rom.get("requiredCapabilities").is_none(), "{rom:#}");
        }
    }
}

#[test]
fn ambiguous_app_definitions_do_not_advertise_target_backed_recipes() {
    let temp = TempDir::new().unwrap();
    let root = write_target_availability_root(&temp, true, true);
    fs::copy(
        root.join("apps/retroarch.yaml"),
        root.join("apps/retroarch-duplicate.yaml"),
    )
    .unwrap();
    let response = describe(json!({
        "authoredRoot": root,
        "devicePlan": "target.plan",
        "selectedRecipes": ["feature.copy_bios"],
    }));
    assert_eq!(response["ok"], true, "{response:#}");
    let bios = target_recipe_option(&response, "feature.copy_bios");
    assert_eq!(bios["available"], false, "{response:#}");
    assert!(bios.get("requiredCapabilities").is_none(), "{response:#}");
    assert!(
        response["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |diagnostic| diagnostic["code"] == "app_definition_ambiguous"
                    && diagnostic["key"] == "feature.copy_bios"
                    && diagnostic["severity"] == "error"
            ),
        "{response:#}"
    );
}
