use std::path::{Path, PathBuf};

use emuchef_rust_backend::{jsonl, run_with_args_and_input};
use serde_json::{json, Value};

fn fixture_path(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("recipes")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("python_goldens")
        .join(name)
}

fn read_golden(name: &str) -> String {
    std::fs::read_to_string(golden_path(name)).expect("golden should be readable")
}

fn parse_stdout_json(stdout: &str) -> Value {
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "expected exactly one JSON response line");
    serde_json::from_str(lines[0]).expect("response should be valid JSON")
}

fn one_shot_response(request: Value) -> Value {
    let output = run_with_args_and_input(&[request.to_string()], "");
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stderr, "");
    parse_stdout_json(&output.stdout)
}

fn sidecar_response(request: Value) -> Value {
    let responses: Vec<Value> = jsonl::process_jsonl(&format!("{request}\n"))
        .lines()
        .map(|line| serde_json::from_str(line).expect("sidecar response should be valid JSON"))
        .collect();
    assert_eq!(responses.len(), 1);
    responses.into_iter().next().unwrap()
}

fn emit_request(path: &str) -> Value {
    json!({
        "type": "emitRecipeYamlFromPath",
        "payload": {
            "path": path,
            "authoredRoot": null,
        },
    })
}

fn validate_request(path: &str) -> Value {
    json!({
        "type": "validateRecipePath",
        "payload": {
            "path": path,
            "authoredRoot": null,
        },
    })
}

fn assert_yaml_semantically_equal(actual: &str, expected: &str) {
    let actual_yaml: serde_yaml::Value =
        serde_yaml::from_str(actual).expect("actual YAML should parse");
    let expected_yaml: serde_yaml::Value =
        serde_yaml::from_str(expected).expect("expected YAML should parse");
    assert_eq!(actual_yaml, expected_yaml);
}

fn assert_invalid_path_payload(response: &Value) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "invalid_request");
    assert_eq!(response["error"]["details"], json!({"field": "path"}));
}

#[test]
fn one_shot_emit_recipe_yaml_from_path_matches_minimal_python_golden_byte_for_byte() {
    let response = one_shot_response(emit_request(&fixture_path("minimal_recipe.yaml")));

    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["yaml"],
        read_golden("minimal_recipe.emit.yaml")
    );
}

#[test]
fn one_shot_emit_recipe_yaml_from_path_matches_representative_python_golden_semantically() {
    let response = one_shot_response(emit_request(&fixture_path("representative_recipe.yaml")));
    let actual = response["result"]["yaml"].as_str().unwrap();
    let expected = read_golden("representative_recipe.emit.yaml");

    assert_eq!(response["ok"], true);
    assert_yaml_semantically_equal(actual, &expected);
    assert!(actual.find("schema_version:").unwrap() < actual.find("kind:").unwrap());
    assert!(actual.find("recipe_dependencies:").unwrap() < actual.find("provides:").unwrap());
    assert!(!actual.contains("\npermissions:"));
}

#[test]
fn one_shot_emit_recipe_yaml_preserves_top_level_refs_and_nested_literal_ref_objects() {
    let response = one_shot_response(emit_request(&fixture_path("ref_params.yaml")));
    let actual = response["result"]["yaml"].as_str().unwrap();
    let expected = read_golden("ref_params.emit.yaml");

    assert_eq!(response["ok"], true);
    assert_yaml_semantically_equal(actual, &expected);
    assert!(actual.contains("ref: steps.extract_assets.outputs.extracted_paths"));
    assert!(actual.contains("wrapper:\n        ref: nested.literal.in.params"));
    assert!(actual.contains("path:\n        ref: nested.literal.in.condition"));
}

#[test]
fn sidecar_emit_recipe_yaml_from_path_echoes_id_and_returns_yaml() {
    let path = fixture_path("minimal_recipe.yaml");
    let response = sidecar_response(json!({
        "id": "emit-1",
        "type": "emitRecipeYamlFromPath",
        "payload": {"path": path, "authoredRoot": null}
    }));

    assert_eq!(response["id"], "emit-1");
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["yaml"],
        read_golden("minimal_recipe.emit.yaml")
    );
}

#[test]
fn validate_recipe_path_valid_fixture_matches_python_diagnostics() {
    let response = one_shot_response(validate_request(&fixture_path("minimal_recipe.yaml")));
    let expected: Value =
        serde_json::from_str(&read_golden("minimal_recipe.validate.json")).unwrap();

    assert_eq!(response["ok"], true);
    assert_eq!(response["result"], expected);
}

#[test]
fn validate_recipe_path_top_level_permissions_matches_python_diagnostics() {
    let response = one_shot_response(validate_request(&fixture_path(
        "invalid_top_level_permissions.yaml",
    )));
    let expected: Value =
        serde_json::from_str(&read_golden("invalid_top_level_permissions.validate.json")).unwrap();

    assert_eq!(response["ok"], true);
    assert_eq!(response["result"], expected);
}

#[test]
fn validate_recipe_path_unsupported_step_type_matches_python_diagnostics() {
    let response = one_shot_response(validate_request(&fixture_path(
        "unsupported_step_type.yaml",
    )));
    let expected: Value =
        serde_json::from_str(&read_golden("unsupported_step_type.validate.json")).unwrap();

    assert_eq!(response["ok"], true);
    assert_eq!(response["result"], expected);
}

#[test]
fn validate_recipe_path_malformed_yaml_returns_python_shaped_diagnostics() {
    let response = one_shot_response(validate_request(&fixture_path("malformed.yaml")));
    let diagnostics = response["result"]["diagnostics"].as_array().unwrap();

    assert_eq!(response["ok"], true);
    assert_eq!(diagnostics[0]["code"], "validation_context_limited");
    assert_eq!(diagnostics[1]["severity"], "error");
    assert_eq!(diagnostics[1]["code"], "authored_data_invalid");
    assert_eq!(diagnostics[1]["objectKind"], Value::Null);
    assert_eq!(diagnostics[1]["objectId"], Value::Null);
    // PyYAML and serde_yaml format parser messages differently; Phase 6E
    // matches the diagnostic shape/code while documenting the message drift.
    assert!(diagnostics[1]["message"]
        .as_str()
        .unwrap()
        .contains("could not be parsed as YAML"));
}

#[test]
fn sidecar_validate_recipe_path_echoes_id_and_returns_diagnostics() {
    let path = fixture_path("minimal_recipe.yaml");
    let response = sidecar_response(json!({
        "id": "validate-1",
        "type": "validateRecipePath",
        "payload": {"path": path, "authoredRoot": null}
    }));

    assert_eq!(response["id"], "validate-1");
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["diagnostics"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn emit_recipe_yaml_from_path_returns_load_failed_for_expected_input_failures() {
    for fixture in [
        "invalid_top_level_permissions.yaml",
        "unsupported_step_type.yaml",
        "malformed.yaml",
    ] {
        let response = one_shot_response(emit_request(&fixture_path(fixture)));

        assert_eq!(response["ok"], false, "{fixture}");
        assert_eq!(response["error"]["code"], "load_failed", "{fixture}");
        assert_eq!(response["error"]["details"]["path"], fixture_path(fixture));
    }
}

#[test]
fn new_requests_reject_non_object_payload_and_missing_path_like_python() {
    for request_type in ["emitRecipeYamlFromPath", "validateRecipePath"] {
        let non_object = one_shot_response(json!({"type": request_type, "payload": []}));
        assert_eq!(non_object["ok"], false);
        assert_eq!(non_object["error"]["code"], "invalid_request");

        assert_invalid_path_payload(&one_shot_response(json!({"type": request_type})));
        assert_invalid_path_payload(&one_shot_response(json!({
            "type": request_type,
            "payload": null
        })));
        assert_invalid_path_payload(&one_shot_response(json!({
            "type": request_type,
            "payload": {}
        })));
    }
}

#[test]
fn new_requests_ignore_unknown_payload_keys_like_python() {
    let path = fixture_path("minimal_recipe.yaml");

    let emit = one_shot_response(json!({
        "type": "emitRecipeYamlFromPath",
        "payload": {"path": path, "authoredRoot": null, "ignored": true}
    }));
    assert_eq!(emit["ok"], true);

    let validate = one_shot_response(json!({
        "type": "validateRecipePath",
        "payload": {"path": fixture_path("minimal_recipe.yaml"), "ignored": true}
    }));
    assert_eq!(validate["ok"], true);
}
