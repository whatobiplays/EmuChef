use std::io::Cursor;
use std::path::{Path, PathBuf};

use emuchef_rust_backend::{
    catalog_source::compute_catalog_sha256, jsonl, protocol, run_with_args_and_input,
};
use serde_json::{json, Value};

fn one_shot_response(request: Value) -> Value {
    let output = run_with_args_and_input(&[request.to_string()], "");
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    let lines = output.stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    serde_json::from_str(lines[0]).expect("one-shot response should be valid JSON")
}

fn sidecar_response(request: Value) -> Value {
    let input = format!("{request}\n");
    let responses = jsonl::process_jsonl(&input)
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).expect("sidecar response should be valid JSON")
        })
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 1);
    responses.into_iter().next().unwrap()
}

fn sidecar_raw_response(request: &str) -> Value {
    let mut output = Vec::new();
    jsonl::run_jsonl_sidecar(Cursor::new(format!("{request}\n")), &mut output)
        .expect("interactive sidecar should process the request");
    let output = String::from_utf8(output).expect("sidecar response should be UTF-8");
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    serde_json::from_str(lines[0]).expect("sidecar response should be valid JSON")
}

fn authored_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("authored")
        .canonicalize()
        .unwrap()
}

#[test]
fn device_profile_generation_protocol_is_side_effect_free_and_rejects_serials() {
    let root = authored_root();
    let before = compute_catalog_sha256(&root).unwrap();
    let facts = json!({
        "manufacturer": "AYANEO",
        "brand": "AYANEO",
        "model": "Pocket S Mini",
        "product": "pocket_s_mini",
        "device": "pocket_s_mini",
        "board": "kalama",
        "hardware": "qcom",
        "abis": ["arm64-v8a"],
        "androidVersion": 13,
        "androidApiLevel": 33,
    });
    let generated = sidecar_response(json!({
        "id": "generate-profile",
        "type": "generateDeviceProfileDraft",
        "payload": { "facts": facts }
    }));
    assert_eq!(generated["ok"], true, "{generated:#}");
    assert_eq!(generated["result"]["profile"]["id"], "ayaneo.pocket_s_mini");
    assert!(generated["result"]["canonicalYaml"]
        .as_str()
        .unwrap()
        .contains("kind: device_profile"));

    let collisions = sidecar_response(json!({
        "id": "profile-collisions",
        "type": "checkGeneratedCatalogCollisions",
        "payload": {
            "authoredRoot": root,
            "facts": facts,
            "profile": generated["result"]["profile"].clone(),
        }
    }));
    assert_eq!(collisions["ok"], true, "{collisions:#}");
    assert_eq!(collisions["result"]["blocking"], true);
    assert_eq!(compute_catalog_sha256(&authored_root()).unwrap(), before);

    let rejected = sidecar_response(json!({
        "id": "serial-rejected",
        "type": "generateDeviceProfileDraft",
        "payload": {
            "facts": {
                "manufacturer": "AYANEO",
                "model": "Pocket S Mini",
                "serial": "SECRET-SERIAL"
            }
        }
    }));
    assert_eq!(rejected["ok"], false);
    assert_eq!(rejected["error"]["code"], "invalid_request");
    assert!(!serde_json::to_string(&rejected)
        .unwrap()
        .contains("SECRET-SERIAL"));
}

fn phase_one_configuration_payload(root: &Path, selected_recipes: Value, serial: &str) -> Value {
    let digest = compute_catalog_sha256(root).expect("authored catalog should be hashable");
    json!({
        "catalog": {
            "root": root,
            "sourceKind": "bundled",
            "sourceId": "emuchef.phase1.bundled",
            "version": "phase1-bundled-1",
            "contentDigest": {
                "algorithm": "sha256",
                "value": digest,
            },
        },
        "devicePlan": "ayaneo.pocket_s_mini.base",
        "selectedRecipes": selected_recipes,
        "bindings": {},
        "deviceContext": {
            "manufacturer": "AYANEO",
            "model": "Pocket S mini",
            "androidVersion": 13,
            "androidApiLevel": 33,
        },
        "targetDevice": {
            "serial": serial,
            "manufacturer": "AYANEO",
            "model": "Pocket S mini",
            "androidApiLevel": 33,
        },
    })
}

#[test]
fn sidecar_runtime_configuration_rejects_duplicate_raw_binding_keys_without_values() {
    for operation in ["describeConfiguration", "planConfiguration"] {
        let request = format!(
            r#"{{"id":"req-1","type":"{operation}","payload":{{"authoredRoot":"/tmp/authored","devicePlan":"example.plan","bindings":{{"feature.copy_roms/policy":"merge","feature.copy_roms/policy":"sync"}}}}}}"#
        );
        let response = sidecar_raw_response(&request);
        assert_eq!(
            response,
            json!({
                "id": "req-1",
                "ok": false,
                "error": {
                    "code": "invalid_request",
                    "message": "Request field 'bindings' contains a duplicate key.",
                    "details": {
                        "reason": "duplicate_binding_key",
                        "field": "bindings",
                        "key": "feature.copy_roms/policy",
                    },
                },
            })
        );
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains("merge"));
        assert!(!serialized.contains("sync"));
    }
}

#[test]
fn keeps_executor_internal_and_protocol_capabilities_editor_scoped() {
    assert_eq!(
        protocol::CAPABILITIES,
        &[
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
            "emitUserConfigurationYamlFromPath",
            "validateUserConfigurationPath",
            "describeCatalog",
            "listAdbDevices",
            "probeDevice",
            "qualifyDevice",
            "checkRoot",
            "inspectApk",
            "generateAppRecipeDraft",
            "generateRemoteAppRecipeDraft",
            "generateDeviceProfileDraft",
            "checkGeneratedCatalogCollisions",
            "matchDevice",
            "negotiateCapabilities",
            "openUserConfiguration",
            "createUserConfiguration",
            "getUserConfigurationDocument",
            "saveUserConfiguration",
            "saveUserConfigurationAs",
            "setUserConfigurationBinding",
            "removeUserConfigurationBinding",
            "setUserConfigurationSelectedRecipes",
            "setUserConfigurationDevicePlan",
            "validateUserConfiguration",
            "emitUserConfigurationYaml",
            "setUserConfigurationAuthoredRoot",
            "closeUserConfiguration",
            "describeConfiguration",
            "planConfiguration",
            "startExecution",
            "getExecution",
            "getExecutionEvents",
            "cancelExecution",
            "launchExecutionApp",
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
            "ping",
        ]
    );

    let one_shot = one_shot_response(json!({
        "type": "__testOnlyUnknownExecutorRequest",
        "payload": {}
    }));
    assert_eq!(one_shot["ok"], false);
    assert_eq!(one_shot["error"]["code"], "invalid_request");

    let sidecar = sidecar_response(json!({
        "id": "executor",
        "type": "__testOnlyUnknownExecutorRequest",
        "payload": {}
    }));
    assert_eq!(sidecar["ok"], false);
    assert_eq!(sidecar["error"]["code"], "invalid_request");
}

#[test]
fn keeps_filesystem_executor_internal_and_protocol_capabilities_editor_scoped() {
    assert_eq!(
        protocol::CAPABILITIES,
        &[
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
            "emitUserConfigurationYamlFromPath",
            "validateUserConfigurationPath",
            "describeCatalog",
            "listAdbDevices",
            "probeDevice",
            "qualifyDevice",
            "checkRoot",
            "inspectApk",
            "generateAppRecipeDraft",
            "generateRemoteAppRecipeDraft",
            "generateDeviceProfileDraft",
            "checkGeneratedCatalogCollisions",
            "matchDevice",
            "negotiateCapabilities",
            "openUserConfiguration",
            "createUserConfiguration",
            "getUserConfigurationDocument",
            "saveUserConfiguration",
            "saveUserConfigurationAs",
            "setUserConfigurationBinding",
            "removeUserConfigurationBinding",
            "setUserConfigurationSelectedRecipes",
            "setUserConfigurationDevicePlan",
            "validateUserConfiguration",
            "emitUserConfigurationYaml",
            "setUserConfigurationAuthoredRoot",
            "closeUserConfiguration",
            "describeConfiguration",
            "planConfiguration",
            "startExecution",
            "getExecution",
            "getExecutionEvents",
            "cancelExecution",
            "launchExecutionApp",
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
            "ping",
        ]
    );

    let one_shot = one_shot_response(json!({
        "type": "__testOnlyUnknownPhase6PExecutorRequest",
        "payload": {}
    }));
    assert_eq!(one_shot["ok"], false);
    assert_eq!(one_shot["error"]["code"], "invalid_request");

    let sidecar = sidecar_response(json!({
        "id": "executor-phase6p",
        "type": "__testOnlyUnknownPhase6PExecutorRequest",
        "payload": {}
    }));
    assert_eq!(sidecar["ok"], false);
    assert_eq!(sidecar["error"]["code"], "invalid_request");
}

#[test]
fn keeps_fake_device_executor_internal_and_protocol_capabilities_editor_scoped() {
    assert_eq!(
        protocol::CAPABILITIES,
        &[
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
            "emitUserConfigurationYamlFromPath",
            "validateUserConfigurationPath",
            "describeCatalog",
            "listAdbDevices",
            "probeDevice",
            "qualifyDevice",
            "checkRoot",
            "inspectApk",
            "generateAppRecipeDraft",
            "generateRemoteAppRecipeDraft",
            "generateDeviceProfileDraft",
            "checkGeneratedCatalogCollisions",
            "matchDevice",
            "negotiateCapabilities",
            "openUserConfiguration",
            "createUserConfiguration",
            "getUserConfigurationDocument",
            "saveUserConfiguration",
            "saveUserConfigurationAs",
            "setUserConfigurationBinding",
            "removeUserConfigurationBinding",
            "setUserConfigurationSelectedRecipes",
            "setUserConfigurationDevicePlan",
            "validateUserConfiguration",
            "emitUserConfigurationYaml",
            "setUserConfigurationAuthoredRoot",
            "closeUserConfiguration",
            "describeConfiguration",
            "planConfiguration",
            "startExecution",
            "getExecution",
            "getExecutionEvents",
            "cancelExecution",
            "launchExecutionApp",
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
            "ping",
        ]
    );

    let one_shot = one_shot_response(json!({
        "type": "__testOnlyUnknownPhase6QExecutorRequest",
        "payload": {}
    }));
    assert_eq!(one_shot["ok"], false);
    assert_eq!(one_shot["error"]["code"], "invalid_request");

    let sidecar = sidecar_response(json!({
        "id": "executor-phase6q",
        "type": "__testOnlyUnknownPhase6QExecutorRequest",
        "payload": {}
    }));
    assert_eq!(sidecar["ok"], false);
    assert_eq!(sidecar["error"]["code"], "invalid_request");
}

#[test]
fn keeps_real_adb_executor_internal_and_protocol_capabilities_editor_scoped() {
    assert_eq!(
        protocol::CAPABILITIES,
        &[
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
            "emitUserConfigurationYamlFromPath",
            "validateUserConfigurationPath",
            "describeCatalog",
            "listAdbDevices",
            "probeDevice",
            "qualifyDevice",
            "checkRoot",
            "inspectApk",
            "generateAppRecipeDraft",
            "generateRemoteAppRecipeDraft",
            "generateDeviceProfileDraft",
            "checkGeneratedCatalogCollisions",
            "matchDevice",
            "negotiateCapabilities",
            "openUserConfiguration",
            "createUserConfiguration",
            "getUserConfigurationDocument",
            "saveUserConfiguration",
            "saveUserConfigurationAs",
            "setUserConfigurationBinding",
            "removeUserConfigurationBinding",
            "setUserConfigurationSelectedRecipes",
            "setUserConfigurationDevicePlan",
            "validateUserConfiguration",
            "emitUserConfigurationYaml",
            "setUserConfigurationAuthoredRoot",
            "closeUserConfiguration",
            "describeConfiguration",
            "planConfiguration",
            "startExecution",
            "getExecution",
            "getExecutionEvents",
            "cancelExecution",
            "launchExecutionApp",
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
            "ping",
        ]
    );

    let one_shot = one_shot_response(json!({
        "type": "__testOnlyUnknownPhase6RExecutorRequest",
        "payload": {}
    }));
    assert_eq!(one_shot["ok"], false);
    assert_eq!(one_shot["error"]["code"], "invalid_request");

    let sidecar = sidecar_response(json!({
        "id": "executor-phase6r",
        "type": "__testOnlyUnknownPhase6RExecutorRequest",
        "payload": {}
    }));
    assert_eq!(sidecar["ok"], false);
    assert_eq!(sidecar["error"]["code"], "invalid_request");
}

#[test]
fn phase_one_device_matching_returns_exact_and_safe_generic_choices() {
    let root = authored_root();
    let response = sidecar_response(json!({
        "id": "match",
        "type": "matchDevice",
        "payload": {
            "catalog": {
                "root": root,
                "sourceKind": "bundled",
                "sourceId": "test.catalog",
                "version": "1",
                "cacheKey": null,
                "contentDigest": null
            },
            "facts": {
                "serial": "trusted-internal-serial",
                "manufacturer": "AYANEO",
                "brand": "AYANEO",
                "model": "Pocket S mini",
                "android_version": 13,
                "android_api_level": 33,
                "device_tags": []
            }
        }
    }));
    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["confidence"], "exact");
    assert_eq!(
        response["result"]["recommendedPlanId"],
        "ayaneo.pocket_s_mini.base"
    );
    assert!(response["result"]["safeGenericPlans"].is_array());
    assert!(!response["result"]["blocked"].as_bool().unwrap());
}

#[test]
fn phase_one_describes_pocket_s_mini_defaults_with_internal_target_and_safe_missing_inputs() {
    let root = authored_root();
    let serial = "trusted-internal-serial";
    let response = sidecar_response(json!({
        "id": "describe-pocket-s-mini",
        "type": "describeConfiguration",
        "payload": phase_one_configuration_payload(&root, Value::Null, serial),
    }));

    assert_eq!(response["ok"], true, "{response:#}");
    let result = &response["result"];
    assert_eq!(result["devicePlan"], "ayaneo.pocket_s_mini.base");
    assert_eq!(
        result["selectedRecipes"],
        json!(["app.retroarch.provision"])
    );
    assert_eq!(
        result["expandedRecipes"],
        json!(["app.retroarch.provision"])
    );
    assert_eq!(result["targetDevice"]["serial"], serial);
    assert_eq!(result["targetDevice"]["androidApiLevel"], 33);
    assert!(result["targetDevice"].get("android_api_level").is_none());
    assert_eq!(result["catalog"]["sourceId"], "emuchef.phase1.bundled");
    assert_eq!(result["catalog"]["contentDigest"]["algorithm"], "sha256");
    assert_eq!(
        result["catalog"]["contentDigest"]["value"]
            .as_str()
            .unwrap()
            .len(),
        64
    );

    let options = result["recipeOptions"].as_array().unwrap();
    let retroarch = options
        .iter()
        .find(|recipe| recipe["id"] == "app.retroarch.provision")
        .expect("default RetroArch recipe option should be present");
    assert_eq!(retroarch["selected"], true);
    assert_eq!(retroarch["recommended"], true);
    let inputs = result["inputs"].as_array().unwrap();
    let config = inputs
        .iter()
        .find(|input| input["key"] == "app.retroarch.provision/retroarch_cfg")
        .expect("RetroArch input descriptor should be present");
    assert_eq!(config["type"], "file");
    assert_eq!(config["value"], Value::Null);
    assert_eq!(config["diagnostics"], json!([]));

    let missing_input_response = sidecar_response(json!({
        "id": "describe-pocket-s-mini-missing-input",
        "type": "describeConfiguration",
        "payload": phase_one_configuration_payload(
            &root,
            json!(["app.retroarch.provision", "feature.copy_roms"]),
            serial,
        ),
    }));
    assert_eq!(
        missing_input_response["ok"], true,
        "{missing_input_response:#}"
    );
    let missing = missing_input_response["result"]["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["key"] == "feature.copy_roms/source")
        .expect("required missing input should remain an input descriptor");
    assert_eq!(missing["value"], Value::Null);
    assert!(missing["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "binding_missing"));
}

#[cfg(unix)]
#[test]
fn phase_one_fake_adb_inventory_and_probe_cover_product_states() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let fake = temp.path().join("adb");
    fs::write(
        &fake,
        r#"#!/bin/sh
if [ "$1" = "devices" ]; then
  printf 'List of devices attached\navailable-1 device model:Pocket_S_mini transport_id:1\nunauthorized-1 unauthorized usb:1\noffline-1 offline\n'
  exit 0
fi
printf '[ro.product.manufacturer]: [AYANEO]\n[ro.product.brand]: [AYANEO]\n[ro.product.model]: [Pocket S mini]\n[ro.build.version.release]: [13]\n[ro.build.version.sdk]: [33]\n'
"#,
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

    let inventory = sidecar_response(json!({
        "id": "devices",
        "type": "listAdbDevices",
        "payload": { "adbPath": fake }
    }));
    assert_eq!(inventory["ok"], true);
    assert_eq!(inventory["result"]["devices"][0]["state"], "available");
    assert_eq!(inventory["result"]["devices"][1]["state"], "unauthorized");
    assert_eq!(inventory["result"]["devices"][2]["state"], "offline");

    let probe = sidecar_response(json!({
        "id": "probe",
        "type": "probeDevice",
        "payload": { "adbPath": fake, "serial": "available-1" }
    }));
    assert_eq!(probe["ok"], true);
    assert_eq!(probe["result"]["serial"], "available-1");
    assert_eq!(probe["result"]["manufacturer"], "AYANEO");
    assert_eq!(probe["result"]["android_api_level"], 33);
}

#[test]
fn local_apk_generation_and_native_inspection_protocols_are_separate_and_safe() {
    let generated = sidecar_response(json!({
        "id": "generate-app-recipe",
        "type": "generateAppRecipeDraft",
        "payload": {
            "facts": {
                "packageName": "com.example.player",
                "applicationLabel": "Example Player",
                "launcherActivities": ["com.example.player/.MainActivity"],
                "split": false,
                "base": true
            }
        }
    }));
    assert_eq!(generated["ok"], true, "{generated:#}");
    assert_eq!(
        generated["result"]["app"]["install_source"]["type"],
        "user_provided_apk"
    );
    assert_eq!(
        generated["result"]["app"]["artifacts"]["byo_apk"]["required"],
        true
    );
    assert_eq!(
        generated["result"]["recipe"]["id"],
        "app.example_player.install"
    );
    assert_eq!(
        generated["result"]["recipe"]["inputs"]["example_player_apk"]["role"],
        "apk"
    );
    assert!(!generated.to_string().contains("/tmp/example.apk"));

    let legacy_rejected = sidecar_response(json!({
        "id": "inspect-legacy-rejected",
        "type": "inspectApk",
        "payload": {
            "analyzer": "apkanalyzer",
            "facts": { "packageName": "com.example.player" }
        }
    }));
    assert_eq!(legacy_rejected["ok"], false);
    assert_eq!(legacy_rejected["error"]["code"], "invalid_request");
    assert_eq!(
        legacy_rejected["error"]["message"],
        "APK inspection input is invalid."
    );

    let missing = sidecar_response(json!({
        "id": "inspect-native-missing",
        "type": "inspectApk",
        "payload": {
            "apkPath": "/Users/private/secret-source-name.apk"
        }
    }));
    assert_eq!(missing["ok"], false);
    assert_eq!(missing["error"]["code"], "command_failed");
    assert_eq!(
        missing["error"]["details"]["reason"],
        "apk_manifest_inspection_failed"
    );
    assert!(!missing.to_string().contains("/Users/private"));
    assert!(!missing.to_string().contains("secret-source-name"));

    let mixed_contract_rejected = sidecar_response(json!({
        "id": "inspect-mixed-rejected",
        "type": "inspectApk",
        "payload": {
            "apkPath": "/tmp/example.apk",
            "facts": { "packageName": "com.example.player" }
        }
    }));
    assert_eq!(mixed_contract_rejected["ok"], false);
    assert_eq!(mixed_contract_rejected["error"]["code"], "invalid_request");
    assert!(!mixed_contract_rejected
        .to_string()
        .contains("/tmp/example.apk"));
}
