use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("compatibility_goldens_v1")
        .join(name)
}

fn read_golden(name: &str) -> Value {
    let text =
        fs::read_to_string(golden_path(name)).expect("Compatibility fixture should be readable");
    serde_json::from_str(&text).expect("Compatibility fixture should be valid JSON")
}

fn normalize_document_result(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut normalized = serde_json::Map::new();
            for (key, item) in object {
                let value = match key.as_str() {
                    "documentId" => json!("<documentId>"),
                    "path" => json!("<path>"),
                    "authoredRoot" if !item.is_null() => json!("<authoredRoot>"),
                    "file" if !item.is_null() => json!("<path>"),
                    _ => normalize_document_result(item),
                };
                normalized.insert(key.clone(), value);
            }
            Value::Object(normalized)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize_document_result).collect()),
        _ => value.clone(),
    }
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

struct TempRecipe {
    dir: PathBuf,
    path: PathBuf,
}

impl TempRecipe {
    fn copy_fixture(name: &str) -> Self {
        static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "emuchef-rust-backend-phase6g-{}-{unique}-{sequence}",
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

fn open_request(path: impl AsRef<Path>) -> Value {
    json!({
        "id": "open",
        "type": "openRecipe",
        "payload": {"path": path.as_ref(), "authoredRoot": fixture_root()}
    })
}

fn command_request(id: &str, command: Value) -> Value {
    json!({
        "id": id,
        "type": "applyRecipeCommand",
        "payload": {"documentId": "doc-1", "command": command}
    })
}

fn undo_request(id: &str) -> Value {
    json!({
        "id": id,
        "type": "undo",
        "payload": {"documentId": "doc-1"}
    })
}

fn redo_request(id: &str) -> Value {
    json!({
        "id": id,
        "type": "redo",
        "payload": {"documentId": "doc-1"}
    })
}

fn assert_invalid_request(response: &Value) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "invalid_request");
}

fn assert_invalid_command(response: &Value) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "invalid_command");
}

fn assert_unknown_document(response: &Value, document_id: &str) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "unknown_document");
    assert_eq!(
        response["error"]["details"],
        json!({ "documentId": document_id })
    );
}

fn assert_command_failed(response: &Value) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "command_failed");
}

#[test]
fn one_shot_session_requests_return_invalid_request() {
    for request_type in ["applyRecipeCommand", "undo", "redo", "emitYaml", "validate"] {
        let response = one_shot_response(json!({
            "type": request_type,
            "payload": {}
        }));

        assert_invalid_request(&response);
    }
}

#[test]
fn apply_recipe_command_validates_request_and_command_payloads() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        json!({
            "id": "missing-document-id",
            "type": "applyRecipeCommand",
            "payload": {"command": {"type": "SetOverviewField", "field": "name", "value": "Ignored"}}
        }),
        json!({
            "id": "wrong-document-id",
            "type": "applyRecipeCommand",
            "payload": {"documentId": 123, "command": {"type": "SetOverviewField", "field": "name", "value": "Ignored"}}
        }),
        json!({
            "id": "missing-command",
            "type": "applyRecipeCommand",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "wrong-command-type",
            "type": "applyRecipeCommand",
            "payload": {"documentId": "doc-1", "command": []}
        }),
        json!({
            "id": "unknown-document",
            "type": "applyRecipeCommand",
            "payload": {"documentId": "missing-document", "command": {"type": "SetOverviewField", "field": "name", "value": "Ignored"}}
        }),
        command_request(
            "unknown-command",
            json!({"type": "RenameRecipe", "value": "Ignored"})
        ),
        command_request(
            "invalid-field",
            json!({"type": "SetOverviewField", "field": "id", "value": "new.id"})
        ),
        command_request(
            "invalid-value",
            json!({"type": "SetOverviewField", "field": "name", "value": 123})
        ),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 9);
    assert_eq!(responses[0]["ok"], true);
    assert_invalid_request(&responses[1]);
    assert_invalid_request(&responses[2]);
    assert_invalid_command(&responses[3]);
    assert_invalid_command(&responses[4]);
    assert_unknown_document(&responses[5], "missing-document");
    assert_invalid_command(&responses[6]);
    assert_invalid_command(&responses[7]);
    assert_invalid_command(&responses[8]);
}

#[test]
fn overview_noops_and_invalid_names_do_not_push_history() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request(
            "noop-name",
            json!({"type": "SetOverviewField", "field": "name", "value": "Phase 6E Minimal"})
        ),
        undo_request("undo-after-name-noop"),
        command_request(
            "noop-description-null",
            json!({"type": "SetOverviewField", "field": "description", "value": null})
        ),
        undo_request("undo-after-description-noop"),
        command_request(
            "empty-name",
            json!({"type": "SetOverviewField", "field": "name", "value": ""})
        ),
        command_request(
            "blank-name",
            json!({"type": "SetOverviewField", "field": "name", "value": "   "})
        ),
        command_request(
            "kind-field",
            json!({"type": "SetOverviewField", "field": "kind", "value": "recipe"})
        ),
        command_request(
            "schema-version-field",
            json!({"type": "SetOverviewField", "field": "schema_version", "value": "1"})
        ),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 9);
    let opened = responses[0]["result"]["document"].clone();
    assert_eq!(
        responses[1]["result"]["commandResult"],
        json!({"changed": false})
    );
    assert_eq!(responses[1]["result"]["document"], opened);
    assert_eq!(
        responses[2]["result"]["commandResult"],
        json!({"changed": false})
    );
    assert_eq!(responses[2]["result"]["document"], opened);
    assert_eq!(
        responses[3]["result"]["commandResult"],
        json!({"changed": false})
    );
    assert_eq!(responses[3]["result"]["document"], opened);
    assert_eq!(
        responses[4]["result"]["commandResult"],
        json!({"changed": false})
    );
    assert_eq!(responses[4]["result"]["document"], opened);
    assert_command_failed(&responses[5]);
    assert_command_failed(&responses[6]);
    assert_invalid_command(&responses[7]);
    assert_invalid_command(&responses[8]);
}

#[test]
fn overview_name_and_description_commands_update_document_state() {
    let temp_recipe = TempRecipe::copy_fixture("representative_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request(
            "set-name",
            json!({"type": "SetOverviewField", "field": "name", "value": "Updated Recipe Name"})
        ),
        command_request(
            "set-description",
            json!({"type": "SetOverviewField", "field": "description", "value": "Updated description"})
        ),
        command_request(
            "clear-description",
            json!({"type": "SetOverviewField", "field": "description", "value": null})
        ),
        command_request(
            "noop-name",
            json!({"type": "SetOverviewField", "field": "name", "value": "Updated Recipe Name"})
        ),
        json!({
            "id": "emit",
            "type": "emitYaml",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "validate",
            "type": "validate",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "get",
            "type": "getDocument",
            "payload": {"documentId": "doc-1"}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 8);
    let opened = &responses[0]["result"]["document"];
    assert_eq!(opened["dirty"], false);
    assert_eq!(opened["canUndo"], false);
    assert_eq!(opened["canRedo"], false);

    let renamed = &responses[1]["result"];
    assert_eq!(renamed["commandResult"], json!({"changed": true}));
    assert_eq!(renamed["document"]["recipe"]["name"], "Updated Recipe Name");
    assert!(renamed["document"]["yaml"]
        .as_str()
        .unwrap()
        .contains("name: Updated Recipe Name"));
    assert_eq!(renamed["document"]["dirty"], true);
    assert_eq!(renamed["document"]["canUndo"], true);
    assert_eq!(renamed["document"]["canRedo"], false);
    assert_eq!(renamed["document"]["diagnostics"], json!([]));

    let described = &responses[2]["result"];
    assert_eq!(described["commandResult"], json!({"changed": true}));
    assert_eq!(
        described["document"]["recipe"]["description"],
        "Updated description"
    );
    assert!(described["document"]["yaml"]
        .as_str()
        .unwrap()
        .contains("description: Updated description"));

    let cleared = &responses[3]["result"];
    assert_eq!(cleared["commandResult"], json!({"changed": true}));
    assert_eq!(cleared["document"]["recipe"]["description"], "");
    assert!(!cleared["document"]["yaml"]
        .as_str()
        .unwrap()
        .lines()
        .any(|line| line.starts_with("description:")));

    let noop = &responses[4]["result"];
    assert_eq!(noop["commandResult"], json!({"changed": false}));
    assert_eq!(noop["document"]["canUndo"], true);
    assert_eq!(noop["document"]["canRedo"], false);

    assert_eq!(responses[5]["result"]["yaml"], noop["document"]["yaml"]);
    assert_eq!(responses[6]["result"], json!({"diagnostics": []}));
    assert_eq!(responses[7]["result"]["document"], noop["document"]);
}

#[test]
fn invalid_command_leaves_document_unchanged_and_does_not_push_history() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request(
            "invalid",
            json!({"type": "SetOverviewField", "field": "schemaVersion", "value": "2"})
        ),
        undo_request("undo"),
        json!({
            "id": "get",
            "type": "getDocument",
            "payload": {"documentId": "doc-1"}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 4);
    let opened = responses[0]["result"]["document"].clone();
    assert_invalid_command(&responses[1]);
    assert_eq!(responses[2]["ok"], true);
    assert_eq!(
        responses[2]["result"]["commandResult"],
        json!({"changed": false})
    );
    assert_eq!(responses[2]["result"]["document"], opened);
    assert_eq!(responses[3]["result"]["document"], opened);
}

#[test]
fn undo_redo_snapshot_history_and_save_baseline_match_compatibility_behavior() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        undo_request("empty-undo"),
        redo_request("empty-redo"),
        command_request(
            "set-one",
            json!({"type": "SetOverviewField", "field": "name", "value": "First Name"})
        ),
        undo_request("undo-one"),
        redo_request("redo-one"),
        undo_request("undo-before-branch"),
        command_request(
            "set-two",
            json!({"type": "SetOverviewField", "field": "name", "value": "Second Name"})
        ),
        redo_request("redo-cleared"),
        json!({
            "id": "save",
            "type": "saveRecipe",
            "payload": {"documentId": "doc-1"}
        }),
        undo_request("undo-after-save"),
        redo_request("redo-after-save"),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 12);
    let original_yaml = responses[0]["result"]["document"]["yaml"].as_str().unwrap();

    assert_eq!(
        responses[1]["result"]["commandResult"],
        json!({"changed": false})
    );
    assert_eq!(responses[1]["result"]["document"]["canUndo"], false);
    assert_eq!(responses[1]["result"]["document"]["canRedo"], false);
    assert_eq!(
        responses[2]["result"]["commandResult"],
        json!({"changed": false})
    );

    assert_eq!(
        responses[3]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert!(responses[3]["result"]["document"]["yaml"]
        .as_str()
        .unwrap()
        .contains("name: First Name"));
    assert_eq!(responses[3]["result"]["document"]["canUndo"], true);

    assert_eq!(
        responses[4]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(
        responses[4]["result"]["document"]["yaml"].as_str().unwrap(),
        original_yaml
    );
    assert_eq!(responses[4]["result"]["document"]["canRedo"], true);

    assert_eq!(
        responses[5]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert!(responses[5]["result"]["document"]["yaml"]
        .as_str()
        .unwrap()
        .contains("name: First Name"));

    assert_eq!(
        responses[6]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(
        responses[7]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert!(responses[7]["result"]["document"]["yaml"]
        .as_str()
        .unwrap()
        .contains("name: Second Name"));
    assert_eq!(responses[7]["result"]["document"]["canRedo"], false);
    assert_eq!(
        responses[8]["result"]["commandResult"],
        json!({"changed": false})
    );
    assert_eq!(responses[8]["result"]["document"]["canRedo"], false);

    assert_eq!(responses[9]["ok"], true);
    assert_eq!(responses[9]["result"]["document"]["dirty"], false);
    assert_eq!(responses[9]["result"]["document"]["canUndo"], true);
    let saved_yaml = fs::read_to_string(&temp_recipe.path).expect("saved YAML should be readable");
    assert_eq!(
        saved_yaml,
        responses[9]["result"]["document"]["yaml"].as_str().unwrap()
    );

    assert_eq!(
        responses[10]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(responses[10]["result"]["document"]["dirty"], true);
    assert_eq!(responses[10]["result"]["document"]["canRedo"], true);

    assert_eq!(
        responses[11]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(responses[11]["result"]["document"]["dirty"], false);
}

#[test]
fn save_preserves_non_empty_redo_stack_without_pushing_history() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request(
            "set-name",
            json!({"type": "SetOverviewField", "field": "name", "value": "Redo After Save"})
        ),
        undo_request("undo"),
        json!({
            "id": "save",
            "type": "saveRecipe",
            "payload": {"documentId": "doc-1"}
        }),
        redo_request("redo-after-save"),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 5);
    assert_eq!(
        responses[1]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(
        responses[2]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(responses[2]["result"]["document"]["canUndo"], false);
    assert_eq!(responses[2]["result"]["document"]["canRedo"], true);

    assert_eq!(responses[3]["ok"], true);
    assert_eq!(responses[3]["result"]["document"]["dirty"], false);
    assert_eq!(responses[3]["result"]["document"]["canUndo"], false);
    assert_eq!(responses[3]["result"]["document"]["canRedo"], true);

    assert_eq!(
        responses[4]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(responses[4]["result"]["document"]["dirty"], true);
    assert_eq!(responses[4]["result"]["document"]["canUndo"], true);
    assert_eq!(responses[4]["result"]["document"]["canRedo"], false);
    assert!(responses[4]["result"]["document"]["yaml"]
        .as_str()
        .unwrap()
        .contains("name: Redo After Save"));
}

#[test]
fn emit_yaml_and_validate_validate_payloads_and_unknown_documents() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request(
            "set-name",
            json!({"type": "SetOverviewField", "field": "name", "value": "In Memory Name"})
        ),
        json!({"id": "emit-missing", "type": "emitYaml", "payload": {}}),
        json!({"id": "emit-wrong", "type": "emitYaml", "payload": {"documentId": 123}}),
        json!({"id": "emit-unknown", "type": "emitYaml", "payload": {"documentId": "missing-document"}}),
        json!({"id": "validate-unknown", "type": "validate", "payload": {"documentId": "missing-document"}}),
        json!({"id": "emit-current", "type": "emitYaml", "payload": {"documentId": "doc-1"}}),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 7);
    assert_eq!(responses[1]["ok"], true);
    assert_invalid_request(&responses[2]);
    assert_invalid_request(&responses[3]);
    assert_unknown_document(&responses[4], "missing-document");
    assert_unknown_document(&responses[5], "missing-document");
    assert!(responses[6]["result"]["yaml"]
        .as_str()
        .unwrap()
        .contains("name: In Memory Name"));
}

#[test]
fn sidecar_continues_after_request_errors() {
    let input = format!(
        "{}\n{}\n",
        json!({
            "id": "bad-command",
            "type": "applyRecipeCommand",
            "payload": {"documentId": "missing-document", "command": {"type": "Unknown"}}
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

#[test]
fn compatibility_results_match_compatibility_goldens_v1() {
    let temp_recipe = TempRecipe::copy_fixture("phase6g_golden.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        undo_request("empty-undo"),
        redo_request("empty-redo"),
        command_request(
            "set-name",
            json!({"type": "SetOverviewField", "field": "name", "value": "Python Golden Name"})
        ),
        command_request(
            "set-description",
            json!({"type": "SetOverviewField", "field": "description", "value": "Python Golden Description"})
        ),
        command_request(
            "clear-description",
            json!({"type": "SetOverviewField", "field": "description", "value": null})
        ),
        command_request(
            "noop",
            json!({"type": "SetOverviewField", "field": "name", "value": "Python Golden Name"})
        ),
        undo_request("undo"),
        redo_request("redo"),
        json!({"id": "emit", "type": "emitYaml", "payload": {"documentId": "doc-1"}}),
        json!({"id": "validate", "type": "validate", "payload": {"documentId": "doc-1"}}),
    );

    let responses = sidecar_responses(&input);
    let expected = [
        ("phase6g_empty_undo.result.json", 1),
        ("phase6g_empty_redo.result.json", 2),
        ("phase6g_set_overview_name.result.json", 3),
        ("phase6g_set_overview_description.result.json", 4),
        ("phase6g_set_overview_description_null.result.json", 5),
        ("phase6g_set_overview_noop.result.json", 6),
        ("phase6g_undo_after_overview.result.json", 7),
        ("phase6g_redo_after_overview.result.json", 8),
        ("phase6g_emit_yaml_after_overview.result.json", 9),
        ("phase6g_validate_after_overview.result.json", 10),
    ];

    assert_eq!(responses.len(), 11);
    for (golden, response_index) in expected {
        assert_eq!(responses[response_index]["ok"], true, "{golden}");
        assert_eq!(
            normalize_document_result(&responses[response_index]["result"]),
            read_golden(golden),
            "{golden}"
        );
    }
}
