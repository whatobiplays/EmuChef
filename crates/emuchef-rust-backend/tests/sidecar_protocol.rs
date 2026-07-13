use std::io::Cursor;

use emuchef_rust_backend::{jsonl, protocol, run_with_args_and_input};
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
