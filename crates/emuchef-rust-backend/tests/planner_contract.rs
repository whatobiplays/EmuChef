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

#[test]
fn capabilities_stay_editor_scoped_and_planner_requests_are_not_routed() {
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
        "type": "__testOnlyUnknownPlannerRequest",
        "payload": {}
    }));
    assert_eq!(one_shot["ok"], false);
    assert_eq!(one_shot["error"]["code"], "invalid_request");

    let sidecar = sidecar_response(json!({
        "id": "plan",
        "type": "__testOnlyUnknownPlannerRequest",
        "payload": {}
    }));
    assert_eq!(sidecar["ok"], false);
    assert_eq!(sidecar["error"]["code"], "invalid_request");
}
