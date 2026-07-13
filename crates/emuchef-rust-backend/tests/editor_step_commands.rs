use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::commands::{decode_recipe_command, RecipeCommand};
use emuchef_rust_backend::errors::ApiErrorCode;
use emuchef_rust_backend::{jsonl, protocol};
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
            "emuchef-rust-backend-phase6j1-{}-{unique}-{sequence}",
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

fn request(id: &str, request_type: &str) -> Value {
    json!({
        "id": id,
        "type": request_type,
        "payload": {"documentId": "doc-1"}
    })
}

fn jsonl_input(requests: Vec<Value>) -> String {
    format!(
        "{}\n",
        requests
            .into_iter()
            .map(|request| request.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn assert_changed(response: &Value) -> &Value {
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["commandResult"],
        json!({"changed": true})
    );
    &response["result"]["document"]
}

fn assert_unchanged(response: &Value) -> &Value {
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["result"]["commandResult"],
        json!({"changed": false})
    );
    &response["result"]["document"]
}

fn assert_invalid_command(response: &Value) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "invalid_command");
}

fn assert_command_failed(response: &Value) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "command_failed");
}

fn step<'a>(document: &'a Value, step_id: &str) -> &'a Value {
    document["recipe"]["steps"]
        .as_array()
        .expect("steps should be an array")
        .iter()
        .find(|step| step["id"] == step_id)
        .unwrap_or_else(|| panic!("step {step_id} should exist"))
}

fn step_ids(document: &Value) -> Vec<String> {
    document["recipe"]["steps"]
        .as_array()
        .expect("steps should be an array")
        .iter()
        .map(|step| step["id"].as_str().unwrap().to_string())
        .collect()
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("value should be an array")
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn capabilities_stay_on_editor_session_surface() {
    assert_eq!(
        protocol::CAPABILITIES,
        &[
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
            "emitUserConfigurationYamlFromPath",
            "validateUserConfigurationPath",
            "describeCatalog",
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
}

#[test]
fn step_command_codec_matches_compatibility_shapes() {
    assert_eq!(
        decode_recipe_command(
            &json!({"type": "AddStep", "stepId": "pause", "stepType": "wait", "name": "Pause", "index": 1})
        )
        .unwrap(),
        RecipeCommand::AddStep {
            step_id: "pause".to_string(),
            step_type: "wait".to_string(),
            name: "Pause".to_string(),
            index: Some(1),
        }
    );
    assert_eq!(
        decode_recipe_command(&json!({"type": "DeleteStep", "stepId": "pause"})).unwrap(),
        RecipeCommand::DeleteStep {
            step_id: "pause".to_string(),
        }
    );
    assert_eq!(
        decode_recipe_command(
            &json!({"type": "DuplicateStep", "sourceStepId": "pause", "newStepId": "pause_copy"})
        )
        .unwrap(),
        RecipeCommand::DuplicateStep {
            source_step_id: "pause".to_string(),
            new_step_id: "pause_copy".to_string(),
        }
    );
    assert_eq!(
        decode_recipe_command(&json!({"type": "ReorderStep", "stepId": "pause", "toIndex": 0}))
            .unwrap(),
        RecipeCommand::ReorderStep {
            step_id: "pause".to_string(),
            to_index: 0,
        }
    );
    assert_eq!(
        decode_recipe_command(
            &json!({"type": "UpdateStepBasics", "stepId": "pause", "name": "Pause", "description": null})
        )
        .unwrap(),
        RecipeCommand::UpdateStepBasics {
            step_id: "pause".to_string(),
            name: "Pause".to_string(),
            description: Value::Null,
        }
    );
    assert_eq!(
        decode_recipe_command(
            &json!({"type": "SetStepUserToggleable", "stepId": "pause", "userToggleable": true})
        )
        .unwrap(),
        RecipeCommand::SetStepUserToggleable {
            step_id: "pause".to_string(),
            user_toggleable: true,
        }
    );
    assert_eq!(
        decode_recipe_command(
            &json!({"type": "UpdateStepDependencies", "stepId": "pause", "dependencies": ["resolve", "missing_step"]})
        )
        .unwrap(),
        RecipeCommand::UpdateStepDependencies {
            step_id: "pause".to_string(),
            dependencies: vec!["resolve".to_string(), "missing_step".to_string()],
        }
    );
}

#[test]
fn malformed_or_out_of_scope_step_commands_are_invalid_command() {
    for payload in [
        json!({"type": "DeleteStep"}),
        json!({"type": "DuplicateStep", "stepId": "pause", "newStepId": "pause_copy"}),
        json!({"type": "ReorderStep", "stepId": "pause", "toIndex": "0"}),
        json!({"type": "UpdateStepBasics", "stepId": "pause", "name": "Pause"}),
        json!({"type": "UpdateStepBasics", "stepId": "pause", "name": "Pause", "description": 1}),
        json!({"type": "SetStepUserToggleable", "stepId": "pause", "userToggleable": "true"}),
        json!({"type": "UpdateStepDependencies", "stepId": "pause", "dependencies": "resolve"}),
        json!({"type": "UpdateStepDependencies", "stepId": "pause", "dependencies": ["resolve", ""]}),
        json!({"type": "UpdateStepParams", "stepId": "pause"}),
        json!({"type": "UpdateStepParams", "stepId": "pause", "params": []}),
        json!({"type": "UpdateStepConstraints", "stepId": "pause", "constraints": null}),
        json!({"type": "UpdateStepConstraints", "stepId": "pause", "constraints": {"capabilities": "shared_storage_write"}}),
        json!({"type": "UpdateStepSkipIf", "stepId": "pause", "skipIf": null}),
        json!({"type": "UpdateStepSkipIf", "stepId": "pause", "skipIf": {"type": "path_exists"}}),
        json!({"type": "UpdateStepVerify", "stepId": "pause", "verify": null}),
        json!({"type": "UpdateStepVerify", "stepId": "pause", "verify": {"type": "path_exists"}}),
    ] {
        let error = decode_recipe_command(&payload).expect_err("payload should be invalid");
        assert_eq!(error.code, ApiErrorCode::InvalidCommand);
    }
}

#[test]
fn step_lifecycle_and_dependency_commands_update_document_state() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "add-step",
            json!({"type": "AddStep", "stepId": "pause", "stepType": "wait", "name": "Pause", "index": 1}),
        ),
        command_request(
            "update-basics",
            json!({"type": "UpdateStepBasics", "stepId": "pause", "name": "Wait for boot", "description": "Delay before launch."}),
        ),
        command_request(
            "set-toggleable",
            json!({"type": "SetStepUserToggleable", "stepId": "pause", "userToggleable": true}),
        ),
        // Dependency updates normalize uniqueness without requiring targets to
        // exist; catalog validation owns target existence checks.
        command_request(
            "update-dependencies",
            json!({"type": "UpdateStepDependencies", "stepId": "pause", "dependencies": ["resolve", "missing_step"]}),
        ),
        command_request(
            "duplicate-step",
            json!({"type": "DuplicateStep", "sourceStepId": "pause", "newStepId": "pause_copy"}),
        ),
        command_request(
            "reorder-step",
            json!({"type": "ReorderStep", "stepId": "pause_copy", "toIndex": 0}),
        ),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 7);
    let added = assert_changed(&responses[1]);
    let pause = step(added, "pause");

    // Adding a step covers supported types and omits default parameters:
    // AddStep creates an empty authored params mapping, not StepSpec defaults.
    assert_eq!(pause["type"], "wait");
    assert_eq!(pause["name"], "Pause");
    assert_eq!(pause["description"], "");
    assert_eq!(pause["userToggleable"], false);
    assert_eq!(pause["dependencies"], json!([]));
    assert_eq!(pause["constraints"]["capabilities"], json!([]));
    assert_eq!(pause["constraints"]["conflictsWith"], json!([]));
    assert_eq!(pause["skipIf"], json!([]));
    assert_eq!(pause["params"], json!({}));
    assert_eq!(pause["verify"], json!([]));
    assert_eq!(step_ids(added)[1], "pause");
    assert!(added["yaml"].as_str().unwrap().contains("id: pause"));
    assert!(added["refIndex"]["stepRefs"]
        .as_array()
        .unwrap()
        .contains(&json!("steps.pause")));

    let updated = assert_changed(&responses[4]);
    let pause = step(updated, "pause");
    assert_eq!(pause["name"], "Wait for boot");
    assert_eq!(pause["description"], "Delay before launch.");
    assert_eq!(pause["userToggleable"], true);
    assert_eq!(pause["dependencies"], json!(["resolve", "missing_step"]));

    let reordered = assert_changed(&responses[6]);
    assert_eq!(step_ids(reordered)[0], "pause_copy");
    let copied = step(reordered, "pause_copy");
    assert_eq!(copied["type"], "wait");
    assert_eq!(copied["name"], "Wait for boot");
    assert_eq!(copied["description"], "Delay before launch.");
    assert_eq!(copied["userToggleable"], true);
    assert_eq!(copied["dependencies"], json!(["resolve", "missing_step"]));
    assert!(reordered["dirty"].as_bool().unwrap());
    assert!(reordered["canUndo"].as_bool().unwrap());
    assert_eq!(reordered["canRedo"], false);
}

#[test]
fn step_command_noops_do_not_push_undo_or_clear_redo() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "set-description",
            json!({"type": "UpdateStepBasics", "stepId": "resolve", "name": "Resolve", "description": "Temporary description."}),
        ),
        request("undo", "undo"),
        // Commands are no-ops when emitted YAML is unchanged. Optional text
        // clears null or blank values.
        command_request(
            "noop-null-description",
            json!({"type": "UpdateStepBasics", "stepId": "resolve", "name": "Resolve", "description": null}),
        ),
        request("redo", "redo"),
        command_request(
            "clear-empty-description",
            json!({"type": "UpdateStepBasics", "stepId": "resolve", "name": "Resolve", "description": ""}),
        ),
        command_request(
            "noop-empty-description",
            json!({"type": "UpdateStepBasics", "stepId": "resolve", "name": "Resolve", "description": null}),
        ),
        command_request(
            "noop-toggleable",
            json!({"type": "SetStepUserToggleable", "stepId": "resolve", "userToggleable": false}),
        ),
        command_request(
            "noop-dependencies",
            json!({"type": "UpdateStepDependencies", "stepId": "resolve", "dependencies": []}),
        ),
        command_request(
            "noop-reorder",
            json!({"type": "ReorderStep", "stepId": "resolve", "toIndex": 0}),
        ),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 10);

    let described = assert_changed(&responses[1]);
    assert_eq!(
        step(described, "resolve")["description"],
        "Temporary description."
    );

    let undone = assert_changed(&responses[2]);
    assert_eq!(step(undone, "resolve")["description"], "");
    assert_eq!(undone["canRedo"], true);

    let nooped = assert_unchanged(&responses[3]);
    assert_eq!(nooped["canRedo"], true);
    assert_eq!(step(nooped, "resolve")["description"], "");

    let redone = assert_changed(&responses[4]);
    assert_eq!(
        step(redone, "resolve")["description"],
        "Temporary description."
    );

    let cleared = assert_changed(&responses[5]);
    assert_eq!(step(cleared, "resolve")["description"], "");
    assert!(!cleared["yaml"]
        .as_str()
        .unwrap()
        .contains("Temporary description."));
    assert!(!cleared["yaml"]
        .as_str()
        .unwrap()
        .contains("description: ''"));

    assert_unchanged(&responses[6]);
    assert_unchanged(&responses[7]);
    assert_unchanged(&responses[8]);
    assert_unchanged(&responses[9]);
}

#[test]
fn each_step_command_family_supports_snapshot_undo_redo() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "add-step",
            json!({"type": "AddStep", "stepId": "pause", "stepType": "wait", "name": "Pause"}),
        ),
        request("undo-add", "undo"),
        request("redo-add", "redo"),
        command_request(
            "update-basics",
            json!({"type": "UpdateStepBasics", "stepId": "pause", "name": "Pause Updated", "description": "Updated."}),
        ),
        request("undo-basics", "undo"),
        request("redo-basics", "redo"),
        command_request(
            "set-toggleable",
            json!({"type": "SetStepUserToggleable", "stepId": "pause", "userToggleable": true}),
        ),
        request("undo-toggleable", "undo"),
        request("redo-toggleable", "redo"),
        command_request(
            "update-dependencies",
            json!({"type": "UpdateStepDependencies", "stepId": "pause", "dependencies": ["resolve", "missing_step"]}),
        ),
        request("undo-dependencies", "undo"),
        request("redo-dependencies", "redo"),
        command_request(
            "duplicate-step",
            json!({"type": "DuplicateStep", "sourceStepId": "pause", "newStepId": "pause_copy"}),
        ),
        request("undo-duplicate", "undo"),
        request("redo-duplicate", "redo"),
        command_request(
            "reorder-step",
            json!({"type": "ReorderStep", "stepId": "pause_copy", "toIndex": 0}),
        ),
        request("undo-reorder", "undo"),
        request("redo-reorder", "redo"),
        command_request(
            "delete-step",
            json!({"type": "DeleteStep", "stepId": "pause_copy"}),
        ),
        request("undo-delete", "undo"),
        request("redo-delete", "redo"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 22);

    assert!(step_ids(assert_changed(&responses[1])).contains(&"pause".to_string()));
    assert!(!step_ids(assert_changed(&responses[2])).contains(&"pause".to_string()));
    assert!(step_ids(assert_changed(&responses[3])).contains(&"pause".to_string()));

    assert_eq!(
        step(assert_changed(&responses[4]), "pause")["name"],
        "Pause Updated"
    );
    assert_eq!(
        step(assert_changed(&responses[5]), "pause")["name"],
        "Pause"
    );
    assert_eq!(
        step(assert_changed(&responses[6]), "pause")["name"],
        "Pause Updated"
    );

    assert_eq!(
        step(assert_changed(&responses[7]), "pause")["userToggleable"],
        true
    );
    assert_eq!(
        step(assert_changed(&responses[8]), "pause")["userToggleable"],
        false
    );
    assert_eq!(
        step(assert_changed(&responses[9]), "pause")["userToggleable"],
        true
    );

    assert_eq!(
        step(assert_changed(&responses[10]), "pause")["dependencies"],
        json!(["resolve", "missing_step"])
    );
    assert_eq!(
        step(assert_changed(&responses[11]), "pause")["dependencies"],
        json!([])
    );
    assert_eq!(
        step(assert_changed(&responses[12]), "pause")["dependencies"],
        json!(["resolve", "missing_step"])
    );

    assert!(step_ids(assert_changed(&responses[13])).contains(&"pause_copy".to_string()));
    assert!(!step_ids(assert_changed(&responses[14])).contains(&"pause_copy".to_string()));
    assert!(step_ids(assert_changed(&responses[15])).contains(&"pause_copy".to_string()));

    assert_eq!(step_ids(assert_changed(&responses[16]))[0], "pause_copy");
    assert_ne!(step_ids(assert_changed(&responses[17]))[0], "pause_copy");
    assert_eq!(step_ids(assert_changed(&responses[18]))[0], "pause_copy");

    assert!(!step_ids(assert_changed(&responses[19])).contains(&"pause_copy".to_string()));
    assert!(step_ids(assert_changed(&responses[20])).contains(&"pause_copy".to_string()));
    assert!(!step_ids(assert_changed(&responses[21])).contains(&"pause_copy".to_string()));
}

#[test]
fn delete_step_cleans_compatibility_verified_modeled_refs() {
    let temp_recipe = TempRecipe::copy_fixture("phase6j1_step_cleanup.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        // Step deletion removes supported dependencies, conflicts, and
        // structured references.
        command_request(
            "delete-prepare",
            json!({"type": "DeleteStep", "stepId": "prepare"}),
        ),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 2);
    let deleted = assert_changed(&responses[1]);
    assert_eq!(step_ids(deleted), vec!["consume", "consume_shorthand"]);

    let consume = step(deleted, "consume");
    assert_eq!(consume["dependencies"], json!([]));
    assert_eq!(consume["constraints"]["conflictsWith"], json!([]));
    assert!(consume["params"].get("source").is_none());

    let shorthand = step(deleted, "consume_shorthand");
    assert!(shorthand["params"].get("source").is_none());
    assert!(!deleted["yaml"].as_str().unwrap().contains("steps.prepare"));
}

#[test]
fn ref_index_updates_after_step_duplicate_reorder_and_delete() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "duplicate-extract",
            json!({"type": "DuplicateStep", "sourceStepId": "extract", "newStepId": "extract_copy"}),
        ),
        command_request(
            "reorder-extract-copy",
            json!({"type": "ReorderStep", "stepId": "extract_copy", "toIndex": 0}),
        ),
        command_request(
            "delete-extract-copy",
            json!({"type": "DeleteStep", "stepId": "extract_copy"}),
        ),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 4);

    let duplicated = assert_changed(&responses[1]);
    let duplicated_step_refs = string_array(&duplicated["refIndex"]["stepRefs"]);
    let duplicated_output_refs = string_array(&duplicated["refIndex"]["stepOutputRefs"]);
    assert!(duplicated_step_refs.contains(&"steps.extract_copy".to_string()));
    assert!(duplicated_output_refs
        .iter()
        .any(|reference| reference.starts_with("steps.extract_copy.outputs.")));

    let reordered = assert_changed(&responses[2]);
    let reordered_step_refs = string_array(&reordered["refIndex"]["stepRefs"]);
    let reordered_output_refs = string_array(&reordered["refIndex"]["stepOutputRefs"]);
    assert_eq!(reordered_step_refs[0], "steps.extract_copy");
    assert!(reordered_output_refs[0].starts_with("steps.extract_copy.outputs."));

    let deleted = assert_changed(&responses[3]);
    let deleted_step_refs = string_array(&deleted["refIndex"]["stepRefs"]);
    let deleted_output_refs = string_array(&deleted["refIndex"]["stepOutputRefs"]);
    assert!(!deleted_step_refs.contains(&"steps.extract_copy".to_string()));
    assert!(!deleted_output_refs
        .iter()
        .any(|reference| reference.starts_with("steps.extract_copy.outputs.")));
}

#[test]
fn step_command_failures_leave_document_and_history_unchanged() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "invalid-step-params",
            json!({"type": "UpdateStepParams", "stepId": "resolve", "params": []}),
        ),
        command_request(
            "duplicate-step-id",
            json!({"type": "AddStep", "stepId": "resolve", "stepType": "wait", "name": "Duplicate"}),
        ),
        command_request(
            "unknown-step-type",
            json!({"type": "AddStep", "stepId": "new_step", "stepType": "missing_type", "name": "Missing"}),
        ),
        command_request(
            "missing-delete",
            json!({"type": "DeleteStep", "stepId": "missing"}),
        ),
        command_request(
            "duplicate-new-id",
            json!({"type": "DuplicateStep", "sourceStepId": "resolve", "newStepId": "extract"}),
        ),
        command_request(
            "reorder-out-of-range",
            json!({"type": "ReorderStep", "stepId": "resolve", "toIndex": 99}),
        ),
        command_request(
            "duplicate-dependencies",
            json!({"type": "UpdateStepDependencies", "stepId": "resolve", "dependencies": ["extract", "extract"]}),
        ),
        request("undo-after-errors", "undo"),
        request("get", "getDocument"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 10);
    let opened = responses[0]["result"]["document"].clone();

    assert_invalid_command(&responses[1]);
    for response in &responses[2..=7] {
        assert_command_failed(response);
    }
    assert_eq!(assert_unchanged(&responses[8]), &opened);
    assert_eq!(responses[9]["result"]["document"], opened);
}

#[test]
fn save_after_step_command_preserves_history_and_updates_baseline() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "add-step",
            json!({"type": "AddStep", "stepId": "save_after_step", "stepType": "wait", "name": "Save After"}),
        ),
        request("undo", "undo"),
        request("save", "saveRecipe"),
        request("redo", "redo"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 5);

    let added = assert_changed(&responses[1]);
    assert!(added["dirty"].as_bool().unwrap());
    assert!(added["canUndo"].as_bool().unwrap());

    let undone = assert_changed(&responses[2]);
    assert_eq!(undone["dirty"], false);
    assert_eq!(undone["canUndo"], false);
    assert_eq!(undone["canRedo"], true);

    assert_eq!(responses[3]["ok"], true);
    let saved = &responses[3]["result"]["document"];
    assert_eq!(saved["dirty"], false);
    assert_eq!(saved["canUndo"], false);
    assert_eq!(saved["canRedo"], true);

    let redone = assert_changed(&responses[4]);
    assert_eq!(redone["dirty"], true);
    assert_eq!(redone["canUndo"], true);
    assert_eq!(redone["canRedo"], false);
    assert!(step_ids(redone).contains(&"save_after_step".to_string()));

    let saved_yaml = fs::read_to_string(&temp_recipe.path).expect("saved YAML should be readable");
    assert!(!saved_yaml.contains("save_after_step"));
}
