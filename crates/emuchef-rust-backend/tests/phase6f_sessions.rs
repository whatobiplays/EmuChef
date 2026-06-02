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
    for request_type in [
        "openRecipe",
        "getDocument",
        "saveRecipe",
        "saveRecipeAs",
        "closeDocument",
        "setDocumentAuthoredRoot",
    ] {
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
fn save_recipe_as_writes_target_updates_path_and_preserves_history() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let original_source = fs::read_to_string(&temp_recipe.path).expect("source should be readable");
    let save_as_path = temp_recipe.dir.join("saved_as.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": temp_recipe.path, "authoredRoot": fixture_root()}
        }),
        json!({
            "id": "rename",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": "doc-1",
                "command": {"type": "SetOverviewField", "field": "name", "value": "Save As Name"}
            }
        }),
        json!({
            "id": "save-as",
            "type": "saveRecipeAs",
            "payload": {"documentId": "doc-1", "path": save_as_path, "ignored": true}
        }),
        json!({
            "id": "undo-after-save-as",
            "type": "undo",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "redo-after-save-as",
            "type": "redo",
            "payload": {"documentId": "doc-1"}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(responses[2]["id"], "save-as");
    assert_eq!(responses[2]["ok"], true);
    let saved_as = &responses[2]["result"]["document"];
    assert_eq!(saved_as["documentId"], "doc-1");
    assert_eq!(
        saved_as["path"],
        save_as_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
    assert_eq!(
        saved_as["authoredRoot"],
        Path::new(&fixture_root())
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
    assert_eq!(saved_as["dirty"], false);
    assert_eq!(saved_as["canUndo"], true);
    assert_eq!(saved_as["canRedo"], false);
    assert!(saved_as["diagnostics"].as_array().is_some());
    assert!(save_as_path.exists());
    assert_eq!(
        fs::read_to_string(&save_as_path).expect("save-as target should be readable"),
        saved_as["yaml"].as_str().unwrap()
    );
    assert!(saved_as["yaml"]
        .as_str()
        .unwrap()
        .contains("name: Save As Name"));
    assert_eq!(
        fs::read_to_string(&temp_recipe.path).expect("source should remain readable"),
        original_source,
        "Save As should not mutate the previous source file"
    );

    assert_eq!(
        responses[3]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(responses[3]["result"]["document"]["dirty"], true);
    assert_eq!(responses[3]["result"]["document"]["path"], saved_as["path"]);

    assert_eq!(
        responses[4]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(responses[4]["result"]["document"]["dirty"], false);
    assert_eq!(responses[4]["result"]["document"]["path"], saved_as["path"]);
}

#[test]
fn save_recipe_as_validates_payload_and_unknown_documents() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": temp_recipe.path, "authoredRoot": null}
        }),
        json!({
            "id": "missing-document-id",
            "type": "saveRecipeAs",
            "payload": {"path": temp_recipe.dir.join("ignored.yaml")}
        }),
        json!({
            "id": "wrong-document-id",
            "type": "saveRecipeAs",
            "payload": {"documentId": 123, "path": temp_recipe.dir.join("ignored.yaml")}
        }),
        json!({
            "id": "missing-path",
            "type": "saveRecipeAs",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "wrong-path",
            "type": "saveRecipeAs",
            "payload": {"documentId": "doc-1", "path": 123}
        }),
        json!({
            "id": "non-object-payload",
            "type": "saveRecipeAs",
            "payload": []
        }),
        json!({
            "id": "unknown-document",
            "type": "saveRecipeAs",
            "payload": {"documentId": "missing-document", "path": temp_recipe.dir.join("ignored.yaml")}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 7);
    assert_eq!(responses[0]["ok"], true);
    assert_invalid_field(&responses[1], "documentId");
    assert_invalid_field(&responses[2], "documentId");
    assert_invalid_field(&responses[3], "path");
    assert_invalid_field(&responses[4], "path");
    assert_eq!(responses[5]["ok"], false);
    assert_eq!(responses[5]["error"]["code"], "invalid_request");
    assert_unknown_document(&responses[6], "missing-document");
}

#[test]
fn save_recipe_as_missing_parent_does_not_create_dirs_or_mutate_session() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let original_path = temp_recipe.path.canonicalize().unwrap();
    let missing_parent = temp_recipe.dir.join("missing").join("parent");
    let save_as_path = missing_parent.join("saved.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": temp_recipe.path, "authoredRoot": null}
        }),
        json!({
            "id": "rename",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": "doc-1",
                "command": {"type": "SetOverviewField", "field": "name", "value": "Unsaved Name"}
            }
        }),
        json!({
            "id": "save-as-missing-parent",
            "type": "saveRecipeAs",
            "payload": {"documentId": "doc-1", "path": save_as_path}
        }),
        json!({
            "id": "get-after-failure",
            "type": "getDocument",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "specs-after-failure",
            "type": "listStepSpecs",
            "payload": {}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(responses[2]["id"], "save-as-missing-parent");
    assert_eq!(responses[2]["ok"], false);
    assert_eq!(responses[2]["error"]["code"], "save_failed");
    assert_eq!(
        responses[2]["error"]["details"],
        json!({"documentId": "doc-1", "path": save_as_path})
    );
    assert!(
        !missing_parent.exists(),
        "Save As should not create missing parent directories"
    );
    assert_eq!(responses[3]["id"], "get-after-failure");
    assert_eq!(responses[3]["ok"], true);
    let after_failure = &responses[3]["result"]["document"];
    assert_eq!(
        after_failure["path"],
        original_path.to_string_lossy().to_string()
    );
    assert_eq!(after_failure["dirty"], true);
    assert!(after_failure["yaml"]
        .as_str()
        .unwrap()
        .contains("name: Unsaved Name"));
    assert_eq!(responses[4]["id"], "specs-after-failure");
    assert_eq!(responses[4]["ok"], true);
}

#[test]
fn create_recipe_from_template_writes_destination_and_opens_clean_document() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "emuchef-rust-backend-create-template-{}-{unique}",
        std::process::id()
    ));
    let authored_root = dir.join("authored");
    let template_root = dir.join("templates").join("authored");
    fs::create_dir_all(authored_root.join("recipes")).expect("authored recipes directory");
    fs::create_dir_all(&template_root).expect("template directory");
    let template_path = template_root.join("recipe.template.yaml");
    fs::write(
        &template_path,
        r#"schema_version: 1
kind: recipe
id: template.recipe
name: Template Recipe
inputs: {}
artifacts: {}
artifact_groups: {}
steps: []
"#,
    )
    .expect("template should be written");
    let destination_path = authored_root.join("recipes").join("created_recipe.yaml");
    let input = format!(
        "{}\n{}\n",
        json!({
            "id": "create-template",
            "type": "createRecipeFromTemplate",
            "payload": {
                "templatePath": template_path,
                "destinationPath": destination_path,
                "recipeId": "created.recipe",
                "authoredRoot": authored_root,
            }
        }),
        json!({
            "id": "get-created",
            "type": "getDocument",
            "payload": {"documentId": "doc-1"}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], "create-template");
    assert_eq!(responses[0]["ok"], true);
    let created = &responses[0]["result"]["document"];
    assert_eq!(created["documentId"], "doc-1");
    assert_eq!(created["recipe"]["id"], "created.recipe");
    assert_eq!(created["recipe"]["name"], "Template Recipe");
    assert_eq!(
        created["path"],
        destination_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
    assert_eq!(
        created["authoredRoot"],
        authored_root
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
    assert_eq!(created["dirty"], false);
    assert_eq!(created["canUndo"], false);
    assert_eq!(created["canRedo"], false);
    assert!(destination_path.exists());
    let written = fs::read_to_string(&destination_path).expect("created recipe should be readable");
    assert!(written.contains("id: created.recipe"));
    assert!(written.contains("name: Template Recipe"));

    assert_eq!(responses[1]["id"], "get-created");
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(responses[1]["result"]["document"]["documentId"], "doc-1");
    assert_eq!(responses[1]["result"]["document"], *created);

    let _ = fs::remove_dir_all(&dir);
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

    for request_type in [
        "getDocument",
        "saveRecipe",
        "closeDocument",
        "setDocumentAuthoredRoot",
    ] {
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

    let open = sidecar_response(json!({
        "id": "open",
        "type": "openRecipe",
        "payload": {"path": fixture_path("minimal_recipe.yaml"), "authoredRoot": null}
    }));
    assert_eq!(open["ok"], true);

    let missing_authored_root = sidecar_response(json!({
        "id": "missing-authored-root",
        "type": "setDocumentAuthoredRoot",
        "payload": {"documentId": "doc-1"}
    }));
    assert_invalid_field(&missing_authored_root, "authoredRoot");

    let wrong_authored_root_type = sidecar_response(json!({
        "id": "wrong-authored-root",
        "type": "setDocumentAuthoredRoot",
        "payload": {"documentId": "doc-1", "authoredRoot": 123}
    }));
    assert_invalid_field(&wrong_authored_root_type, "authoredRoot");

    let empty_authored_root = sidecar_response(json!({
        "id": "empty-authored-root",
        "type": "setDocumentAuthoredRoot",
        "payload": {"documentId": "doc-1", "authoredRoot": ""}
    }));
    assert_invalid_field(&empty_authored_root, "authoredRoot");

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
    for request_type in [
        "getDocument",
        "saveRecipe",
        "closeDocument",
        "setDocumentAuthoredRoot",
    ] {
        let response = sidecar_response(json!({
            "id": format!("{request_type}-unknown"),
            "type": request_type,
            "payload": {
                "documentId": "missing-document",
                "authoredRoot": null
            }
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
