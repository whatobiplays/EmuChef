use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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

fn fixture_root() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .to_string_lossy()
        .into_owned()
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

fn sidecar_responses(input: &str) -> Vec<Value> {
    jsonl::process_jsonl(input)
        .lines()
        .map(|line| serde_json::from_str(line).expect("sidecar response should be valid JSON"))
        .collect()
}

fn sidecar_response(request: Value) -> Value {
    let responses = sidecar_responses(&format!("{request}\n"));
    assert_eq!(responses.len(), 1);
    responses.into_iter().next().unwrap()
}

struct TempRecipe {
    dir: PathBuf,
    path: PathBuf,
}

impl TempRecipe {
    fn copy_fixture(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "emuchef-rust-backend-phase6f-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp directory should be created");
        let path = dir.join(name);
        fs::copy(fixture_path(name), &path).expect("fixture should copy to temp path");
        Self { dir, path }
    }
}

impl Drop for TempRecipe {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn assert_invalid_field(response: &Value, field: &str) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "invalid_request");
    assert_eq!(response["error"]["details"], json!({ "field": field }));
}

fn assert_unknown_document(response: &Value, document_id: &str) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "unknown_document");
    assert_eq!(
        response["error"]["message"],
        format!("Unknown document id: {document_id}")
    );
    assert_eq!(
        response["error"]["details"],
        json!({ "documentId": document_id })
    );
}

#[test]
fn one_shot_session_requests_are_not_exposed() {
    for request_type in ["openRecipe", "getDocument", "saveRecipe", "closeDocument"] {
        let response = one_shot_response(json!({
            "type": request_type,
            "payload": {}
        }));

        assert_eq!(response["ok"], false, "{request_type}");
        assert_eq!(
            response["error"]["code"], "invalid_request",
            "{request_type}"
        );
    }
}

#[test]
fn sidecar_open_get_save_close_lifecycle_persists_document_state() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": temp_recipe.path, "authoredRoot": null}
        }),
        json!({
            "id": "get",
            "type": "getDocument",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "save",
            "type": "saveRecipe",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "close",
            "type": "closeDocument",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "after-close",
            "type": "getDocument",
            "payload": {"documentId": "doc-1"}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["id"], "open");
    assert_eq!(responses[0]["ok"], true);
    let opened = &responses[0]["result"]["document"];
    assert_eq!(opened["documentId"], "doc-1");
    assert_eq!(
        opened["path"],
        temp_recipe
            .path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
    assert_eq!(opened["authoredRoot"], Value::Null);
    assert_eq!(opened["dirty"], false);
    assert_eq!(opened["canUndo"], false);
    assert_eq!(opened["canRedo"], false);
    assert!(opened["yaml"]
        .as_str()
        .unwrap()
        .contains("id: phase6e.minimal"));
    assert_eq!(opened["recipe"]["schemaVersion"], 1);
    assert_eq!(opened["recipe"]["kind"], "recipe");
    assert_eq!(opened["recipe"]["id"], "phase6e.minimal");
    assert!(opened["recipe"].get("permissions").is_none());
    assert!(opened.get("stepSpecs").is_none());
    assert!(opened["diagnostics"].as_array().is_some());
    assert_eq!(
        opened["refIndex"],
        json!({
            "inputRefs": [],
            "artifactRefs": [],
            "stepRefs": [],
            "stepOutputRefs": [],
            "allRefs": [],
            "candidates": [],
        })
    );

    assert_eq!(responses[1]["id"], "get");
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(responses[1]["result"]["document"], *opened);

    assert_eq!(responses[2]["id"], "save");
    assert_eq!(responses[2]["ok"], true);
    assert_eq!(responses[2]["result"]["document"]["dirty"], false);
    let saved_yaml = fs::read_to_string(&temp_recipe.path).expect("saved file should be readable");
    assert_eq!(
        saved_yaml,
        responses[2]["result"]["document"]["yaml"].as_str().unwrap()
    );

    assert_eq!(responses[3]["id"], "close");
    assert_eq!(responses[3]["ok"], true);
    assert_eq!(responses[3]["result"], json!({}));

    assert_eq!(responses[4]["id"], "after-close");
    assert_unknown_document(&responses[4], "doc-1");
}

#[test]
fn open_recipe_matches_python_authored_root_fixture_behavior() {
    let input = format!(
        "{}\n{}\n{}\n",
        json!({
            "id": "null-root",
            "type": "openRecipe",
            "payload": {"path": fixture_path("minimal_recipe.yaml"), "authoredRoot": null}
        }),
        json!({
            "id": "omitted-root",
            "type": "openRecipe",
            "payload": {"path": fixture_path("minimal_recipe.yaml")}
        }),
        json!({
            "id": "string-root",
            "type": "openRecipe",
            "payload": {"path": fixture_path("minimal_recipe.yaml"), "authoredRoot": fixture_root()}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_eq!(response["ok"], true);
    }

    let null_root = &responses[0]["result"]["document"];
    assert_eq!(null_root["authoredRoot"], Value::Null);
    assert_eq!(
        null_root["diagnostics"][0]["code"],
        "validation_context_limited"
    );

    let omitted_root = &responses[1]["result"]["document"];
    assert_eq!(omitted_root["authoredRoot"], Value::Null);
    assert_eq!(
        omitted_root["diagnostics"][0]["code"],
        "validation_context_limited"
    );

    let string_root = &responses[2]["result"]["document"];
    assert_eq!(
        string_root["authoredRoot"],
        Path::new(&fixture_root())
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
    assert_eq!(string_root["diagnostics"], json!([]));
}

#[test]
fn dto_projection_uses_python_shaped_recipe_fields_and_param_refs() {
    let response = sidecar_response(json!({
        "id": "open-ref-params",
        "type": "openRecipe",
        "payload": {"path": fixture_path("ref_params.yaml"), "authoredRoot": null}
    }));

    assert_eq!(response["ok"], true);
    let document = &response["result"]["document"];
    let recipe = &document["recipe"];
    assert_eq!(
        recipe["artifactGroups"],
        json!({"asset_group": ["assets_zip"]})
    );

    let steps = recipe["steps"].as_array().unwrap();
    let copy_step = steps
        .iter()
        .find(|step| step["id"] == "copy_assets")
        .expect("copy_assets step should be projected");

    assert_eq!(copy_step["type"], "copy_files");
    assert_eq!(copy_step["userToggleable"], true);
    assert_eq!(copy_step["constraints"]["conflictsWith"], json!([]));
    assert_eq!(
        copy_step["params"]["source"],
        json!({"ref": "steps.extract_assets.outputs.extracted_paths"})
    );
    assert_eq!(
        copy_step["params"]["literal_nested"],
        json!({"wrapper": {"ref": "nested.literal.in.params"}})
    );
    assert_eq!(copy_step["skipIf"][0]["type"], "path_exists");
    assert_eq!(
        copy_step["skipIf"][0]["params"]["path"],
        json!({"ref": "nested.literal.in.condition"})
    );

    assert!(document["yaml"].as_str().unwrap().contains("skip_if:"));
    assert!(document["yaml"]
        .as_str()
        .unwrap()
        .contains("conflicts_with:"));
    assert!(!document["yaml"]
        .as_str()
        .unwrap()
        .contains("artifactGroups:"));
}

#[test]
fn session_requests_validate_payloads_and_ignore_unknown_payload_keys() {
    let missing_path = sidecar_response(json!({
        "id": "missing-path",
        "type": "openRecipe",
        "payload": {}
    }));
    assert_invalid_field(&missing_path, "path");

    let wrong_path_type = sidecar_response(json!({
        "id": "wrong-path",
        "type": "openRecipe",
        "payload": {"path": 123}
    }));
    assert_invalid_field(&wrong_path_type, "path");

    let non_object_payload = sidecar_response(json!({
        "id": "non-object",
        "type": "getDocument",
        "payload": []
    }));
    assert_eq!(non_object_payload["ok"], false);
    assert_eq!(non_object_payload["error"]["code"], "invalid_request");

    for request_type in ["getDocument", "saveRecipe", "closeDocument"] {
        let missing = sidecar_response(json!({
            "id": format!("{request_type}-missing"),
            "type": request_type,
            "payload": {}
        }));
        assert_invalid_field(&missing, "documentId");

        let wrong_type = sidecar_response(json!({
            "id": format!("{request_type}-wrong"),
            "type": request_type,
            "payload": {"documentId": 123}
        }));
        assert_invalid_field(&wrong_type, "documentId");
    }

    let unknown_key = sidecar_response(json!({
        "id": "unknown-key",
        "type": "openRecipe",
        "payload": {
            "path": fixture_path("minimal_recipe.yaml"),
            "authoredRoot": null,
            "ignored": true
        }
    }));
    assert_eq!(unknown_key["ok"], true);
}

#[test]
fn unknown_document_errors_match_python_shape() {
    for request_type in ["getDocument", "saveRecipe", "closeDocument"] {
        let response = sidecar_response(json!({
            "id": format!("{request_type}-unknown"),
            "type": request_type,
            "payload": {"documentId": "missing-document"}
        }));

        assert_unknown_document(&response, "missing-document");
    }
}

#[test]
fn sidecar_continues_after_session_request_errors() {
    let input = format!(
        "{}\n{}\n",
        json!({
            "id": "bad-get",
            "type": "getDocument",
            "payload": {"documentId": "missing-document"}
        }),
        json!({
            "id": "open-after-error",
            "type": "openRecipe",
            "payload": {"path": fixture_path("minimal_recipe.yaml"), "authoredRoot": null}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 2);
    assert_unknown_document(&responses[0], "missing-document");
    assert_eq!(responses[1]["id"], "open-after-error");
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(responses[1]["result"]["document"]["documentId"], "doc-1");
}
