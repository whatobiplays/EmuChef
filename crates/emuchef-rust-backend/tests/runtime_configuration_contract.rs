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

fn target_recipe_option<'a>(response: &'a Value, recipe_id: &str) -> &'a Value {
    response["result"]["recipeOptions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["id"] == recipe_id)
        .unwrap_or_else(|| panic!("missing recipe option {recipe_id}: {response:#}"))
}

#[test]
fn target_backed_recipe_options_include_derived_capabilities_and_preserve_generic_rom() {
    let temp = TempDir::new().unwrap();
    let root = write_target_availability_root(&temp, true, true);
    let response = describe(json!({"authoredRoot": root, "devicePlan": "target.plan"}));
    assert_eq!(response["ok"], true, "{response:#}");
    for (recipe_id, capabilities) in [
        (
            "app.retroarch.provision",
            json!([
                "apk_install",
                "app_data_write",
                "app_launch",
                "shared_storage_write"
            ]),
        ),
        ("feature.copy_bios", json!(["shared_storage_write"])),
        ("feature.copy_roms", json!(["shared_storage_write"])),
    ] {
        let option = target_recipe_option(&response, recipe_id);
        assert_eq!(option["available"], true, "{recipe_id}: {option:#}");
        assert_eq!(
            option["requiredCapabilities"], capabilities,
            "{recipe_id}: {option:#}"
        );
        assert_eq!(option["unavailableCapabilities"], json!([]));
    }
}

#[test]
fn target_backed_recipe_options_report_missing_write_capabilities() {
    for (shared_write, app_data_write, retroarch_missing, bios_missing, rom_missing) in [
        (
            false,
            true,
            json!(["shared_storage_write"]),
            json!(["shared_storage_write"]),
            json!(["shared_storage_write"]),
        ),
        (true, false, json!(["app_data_write"]), json!([]), json!([])),
    ] {
        let temp = TempDir::new().unwrap();
        let root = write_target_availability_root(&temp, shared_write, app_data_write);
        let response = describe(json!({"authoredRoot": root, "devicePlan": "target.plan"}));
        assert_eq!(response["ok"], true, "{response:#}");
        for (recipe_id, expected) in [
            ("app.retroarch.provision", retroarch_missing),
            ("feature.copy_bios", bios_missing),
            ("feature.copy_roms", rom_missing),
        ] {
            let option = target_recipe_option(&response, recipe_id);
            assert_eq!(
                option["unavailableCapabilities"], expected,
                "{recipe_id}: {option:#}"
            );
            assert_eq!(option["available"], expected.as_array().unwrap().is_empty());
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
        let response = describe(json!({
            "authoredRoot": root,
            "devicePlan": "target.plan",
            "selectedRecipes": selection,
            "bindings": { "feature.copy_roms/source": source },
        }));
        assert_eq!(response["ok"], true, "{response:#}");
        let bios = target_recipe_option(&response, "feature.copy_bios");
        assert_eq!(bios["available"], false, "{response:#}");
        assert!(bios.get("requiredCapabilities").is_none(), "{response:#}");
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
                    && diagnostic["key"] == "feature.copy_bios"
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
            assert_eq!(rom["requiredCapabilities"], json!(["shared_storage_write"]));
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
