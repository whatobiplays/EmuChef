use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::commands::{decode_recipe_command, RecipeCommand};
use emuchef_rust_backend::errors::ApiErrorCode;
use emuchef_rust_backend::jsonl;
use emuchef_rust_backend::model::ParamValue;
use emuchef_rust_backend::protocol;
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
            "emuchef-rust-backend-phase6j2-{}-{unique}-{sequence}",
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

fn all_refs(document: &Value) -> Vec<String> {
    document["refIndex"]["allRefs"]
        .as_array()
        .expect("allRefs should be an array")
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn phase6j2_capabilities_stay_on_editor_session_surface() {
    assert_eq!(
        protocol::CAPABILITIES,
        &[
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
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
fn command_inventory_covers_current_external_apply_recipe_command_decoders() {
    // Mirrors the external applyRecipeCommand decoder inventory. Core-only
    // document commands absent from that wire protocol are intentionally excluded here.
    let samples = [
        (
            "SetOverviewField",
            json!({"type": "SetOverviewField", "field": "name", "value": "Updated"}),
        ),
        (
            "AddInput",
            json!({"type": "AddInput", "inputId": "extra_input"}),
        ),
        (
            "RenameInput",
            json!({"type": "RenameInput", "inputId": "source_dir", "newInputId": "source"}),
        ),
        (
            "DeleteInput",
            json!({"type": "DeleteInput", "inputId": "source_dir"}),
        ),
        (
            "DuplicateInput",
            json!({"type": "DuplicateInput", "sourceInputId": "source_dir", "newInputId": "source_copy"}),
        ),
        (
            "UpdateInputField",
            json!({"type": "UpdateInputField", "inputId": "source_dir", "field": "label", "value": "Source"}),
        ),
        (
            "AddArtifact",
            json!({"type": "AddArtifact", "artifactId": "extra_zip", "url": "https://example.com/extra.zip"}),
        ),
        (
            "UpdateArtifactField",
            json!({"type": "UpdateArtifactField", "artifactId": "target_zip", "field": "url", "value": "https://example.com/new.zip"}),
        ),
        (
            "RenameArtifact",
            json!({"type": "RenameArtifact", "artifactId": "target_zip", "newArtifactId": "target"}),
        ),
        (
            "DeleteArtifact",
            json!({"type": "DeleteArtifact", "artifactId": "target_zip"}),
        ),
        (
            "DuplicateArtifact",
            json!({"type": "DuplicateArtifact", "sourceArtifactId": "target_zip", "newArtifactId": "target_copy"}),
        ),
        (
            "AddArtifactGroup",
            json!({"type": "AddArtifactGroup", "groupId": "extra_group"}),
        ),
        (
            "RenameArtifactGroup",
            json!({"type": "RenameArtifactGroup", "groupId": "bundle", "newGroupId": "renamed_bundle"}),
        ),
        (
            "DeleteArtifactGroup",
            json!({"type": "DeleteArtifactGroup", "groupId": "bundle"}),
        ),
        (
            "DuplicateArtifactGroup",
            json!({"type": "DuplicateArtifactGroup", "sourceGroupId": "bundle", "newGroupId": "bundle_copy"}),
        ),
        (
            "ReorderArtifactGroup",
            json!({"type": "ReorderArtifactGroup", "groupId": "bundle", "toIndex": 1}),
        ),
        (
            "AddArtifactGroupMember",
            json!({"type": "AddArtifactGroupMember", "groupId": "bundle", "artifactId": "other_zip", "index": 0}),
        ),
        (
            "RemoveArtifactGroupMember",
            json!({"type": "RemoveArtifactGroupMember", "groupId": "bundle", "index": 0}),
        ),
        (
            "ReorderArtifactGroupMember",
            json!({"type": "ReorderArtifactGroupMember", "groupId": "bundle", "index": 0, "toIndex": 1}),
        ),
        (
            "AddStep",
            json!({"type": "AddStep", "stepId": "pause", "stepType": "wait", "name": "Pause"}),
        ),
        (
            "DeleteStep",
            json!({"type": "DeleteStep", "stepId": "resolve"}),
        ),
        (
            "DuplicateStep",
            json!({"type": "DuplicateStep", "sourceStepId": "resolve", "newStepId": "resolve_copy"}),
        ),
        (
            "ReorderStep",
            json!({"type": "ReorderStep", "stepId": "resolve", "toIndex": 1}),
        ),
        (
            "UpdateStepBasics",
            json!({"type": "UpdateStepBasics", "stepId": "resolve", "name": "Resolve", "description": null}),
        ),
        (
            "SetStepUserToggleable",
            json!({"type": "SetStepUserToggleable", "stepId": "resolve", "userToggleable": true}),
        ),
        (
            "UpdateStepDependencies",
            json!({"type": "UpdateStepDependencies", "stepId": "resolve", "dependencies": ["extract"]}),
        ),
        (
            "UpdateStepParams",
            json!({"type": "UpdateStepParams", "stepId": "copy_input", "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/Input"}}),
        ),
        (
            "UpdateStepConstraints",
            json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": {"capabilities": [], "conflictsWith": []}}),
        ),
        (
            "UpdateStepSkipIf",
            json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": []}),
        ),
        (
            "UpdateStepVerify",
            json!({"type": "UpdateStepVerify", "stepId": "copy_input", "verify": []}),
        ),
    ];
    assert_eq!(samples.len(), 30);

    for (command_type, payload) in samples {
        decode_recipe_command(&payload)
            .unwrap_or_else(|error| panic!("{command_type} should decode: {error:?}"));
    }

    let error = decode_recipe_command(&json!({"type": "FutureStepCommand"}))
        .expect_err("future commands should remain invalid_command");
    assert_eq!(error.code, ApiErrorCode::InvalidCommand);
}

#[test]
fn step_internals_command_codec_matches_python_shapes() {
    let params = decode_recipe_command(&json!({
        "type": "UpdateStepParams",
        "stepId": "copy_input",
        "ignoredByPython": true,
        "params": {
            "source": {"ref": "steps.extract.outputs.extracted_paths"},
            "dest": "/sdcard/New",
            "literal_object": {"ref": "nested.literal", "extra": true},
            "literal_list": [{"ref": "nested.in.list"}],
            "literal_null": null
        }
    }))
    .expect("UpdateStepParams should decode");
    let RecipeCommand::UpdateStepParams { step_id, params } = params else {
        panic!("expected UpdateStepParams");
    };
    assert_eq!(step_id, "copy_input");
    assert!(matches!(
        params.get("source"),
        Some(ParamValue::Ref(value)) if value == "steps.extract.outputs.extracted_paths"
    ));
    assert_eq!(
        params.get("literal_object"),
        Some(&ParamValue::Literal(
            json!({"ref": "nested.literal", "extra": true})
        ))
    );
    assert_eq!(
        params.get("literal_list"),
        Some(&ParamValue::Literal(json!([{"ref": "nested.in.list"}])))
    );
    assert_eq!(params.get_index(0).unwrap().0, "source");

    let constraints = decode_recipe_command(&json!({
        "type": "UpdateStepConstraints",
        "stepId": "copy_input",
        "ignoredByPython": true,
        "constraints": {"capabilities": ["shared_storage_write"], "conflictsWith": ["resolve"]}
    }))
    .expect("UpdateStepConstraints should decode");
    let RecipeCommand::UpdateStepConstraints {
        step_id,
        constraints,
    } = constraints
    else {
        panic!("expected UpdateStepConstraints");
    };
    assert_eq!(step_id, "copy_input");
    assert_eq!(constraints.capabilities, vec!["shared_storage_write"]);
    assert_eq!(constraints.conflicts_with, vec!["resolve"]);

    for (payload, expected_variant) in [
        (
            json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "ignoredByPython": true, "skipIf": [{"type": "unknown_condition", "params": {"zeta": 1, "nested_ref": {"ref": "inputs.source_dir"}, "alpha": null}}]}),
            "UpdateStepSkipIf",
        ),
        (
            json!({"type": "UpdateStepVerify", "stepId": "copy_input", "ignoredByPython": true, "verify": [{"type": "path_exists"}]}),
            "UpdateStepVerify",
        ),
    ] {
        let command = decode_recipe_command(&payload).expect("condition command should decode");
        match (expected_variant, command) {
            ("UpdateStepSkipIf", RecipeCommand::UpdateStepSkipIf { step_id, skip_if }) => {
                assert_eq!(step_id, "copy_input");
                assert_eq!(skip_if[0].type_name, "unknown_condition");
                assert_eq!(
                    skip_if[0].params.get("nested_ref"),
                    Some(&json!({"ref": "inputs.source_dir"}))
                );
            }
            ("UpdateStepVerify", RecipeCommand::UpdateStepVerify { step_id, verify }) => {
                assert_eq!(step_id, "copy_input");
                assert_eq!(verify[0].type_name, "path_exists");
                assert!(verify[0].params.is_empty());
            }
            _ => panic!("unexpected decoded command variant"),
        }
    }
}

#[test]
fn malformed_step_internals_commands_are_invalid_command() {
    for payload in [
        json!({"type": "UpdateStepParams", "stepId": "copy_input"}),
        json!({"type": "UpdateStepParams", "stepId": "copy_input", "params": []}),
        json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": null}),
        json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": {"capabilities": "shared_storage_write", "conflictsWith": []}}),
        json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": {"capabilities": [], "conflicts_with": ["resolve"]}}),
        json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": {"capabilities": [], "conflictsWith": [], "custom": true}}),
        json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": null}),
        json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": {"type": "path_exists"}}),
        json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": [{"params": {}}]}),
        json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": [{"type": "path_exists", "params": []}]}),
        json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": [{"type": "path_exists", "params": {}, "custom": true}]}),
        json!({"type": "UpdateStepVerify", "stepId": "copy_input", "verify": null}),
        json!({"type": "UpdateStepVerify", "stepId": "copy_input", "verify": {"type": "path_exists"}}),
        json!({"type": "UpdateStepVerify", "stepId": "copy_input", "verify": [{"params": {}}]}),
        json!({"type": "UpdateStepVerify", "stepId": "copy_input", "verify": [{"type": "path_exists", "params": []}]}),
        json!({"type": "UpdateStepVerify", "stepId": "copy_input", "verify": [{"type": "path_exists", "params": {}, "custom": true}]}),
    ] {
        let error = decode_recipe_command(&payload).expect_err("payload should be invalid");
        assert_eq!(error.code, ApiErrorCode::InvalidCommand);
    }
}

#[test]
fn update_step_params_replaces_values_preserves_literals_and_does_not_add_dependencies_or_refs() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "update-params",
            json!({
                "type": "UpdateStepParams",
                "stepId": "copy_input",
                "ignoredByPython": true,
                "params": {
                    "source": {"ref": "steps.missing.outputs.value"},
                    "dest": "/sdcard/New",
                    "literal_null": null,
                    "literal_bool": true,
                    "literal_number": 7,
                    "literal_object": {"ref": "nested.literal", "extra": ["kept"]},
                    "literal_list": [{"ref": "nested.in.list"}]
                }
            }),
        ),
        request("undo", "undo"),
        request("redo", "redo"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 4);
    let updated = assert_changed(&responses[1]);
    let copy = step(updated, "copy_input");
    assert_eq!(
        copy["params"]["source"],
        json!({"ref": "steps.missing.outputs.value"})
    );
    assert_eq!(copy["params"]["dest"], "/sdcard/New");
    assert_eq!(copy["params"]["literal_null"], Value::Null);
    assert_eq!(copy["params"]["literal_bool"], true);
    assert_eq!(copy["params"]["literal_number"], 7);
    assert_eq!(
        copy["params"]["literal_object"],
        json!({"ref": "nested.literal", "extra": ["kept"]})
    );
    assert_eq!(
        copy["params"]["literal_list"],
        json!([{"ref": "nested.in.list"}])
    );
    assert!(copy["params"].get("copy_policy").is_none());
    assert_eq!(copy["dependencies"], json!([]));
    assert!(!all_refs(updated).contains(&"steps.missing.outputs.value".to_string()));
    assert!(updated["yaml"]
        .as_str()
        .unwrap()
        .contains("ref: steps.missing.outputs.value"));
    assert!(updated["dirty"].as_bool().unwrap());
    assert!(updated["canUndo"].as_bool().unwrap());
    assert_eq!(updated["canRedo"], false);

    let undone = assert_changed(&responses[2]);
    assert_eq!(
        step(undone, "copy_input")["params"]["dest"],
        "/sdcard/Input"
    );
    assert_eq!(undone["canRedo"], true);

    let redone = assert_changed(&responses[3]);
    assert_eq!(step(redone, "copy_input")["params"]["dest"], "/sdcard/New");
}

#[test]
fn update_step_params_omits_only_python_equal_builtin_defaults() {
    let default_noop = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let responses = sidecar_responses(&jsonl_input(vec![
        open_request(&default_noop.path),
        command_request(
            "default-noop",
            json!({
                "type": "UpdateStepParams",
                "stepId": "copy_input",
                "ignoredByPython": true,
                "params": {
                    "source": {"ref": "inputs.source_dir"},
                    "dest": "/sdcard/Input",
                    "copy_policy": "merge"
                }
            }),
        ),
    ]));
    assert_eq!(responses.len(), 2);
    let nooped = assert_unchanged(&responses[1]);
    assert_eq!(nooped["dirty"], false);
    assert_eq!(nooped["canUndo"], false);

    for (name, value) in [
        ("different_string", json!("Merge")),
        ("number", json!(7)),
        ("null", Value::Null),
        ("object", json!({"value": "merge"})),
        ("list", json!(["merge"])),
        (
            "ref_shaped",
            json!({"ref": "steps.extract.outputs.extracted_paths"}),
        ),
    ] {
        let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
        let responses = sidecar_responses(&jsonl_input(vec![
            open_request(&temp_recipe.path),
            command_request(
                name,
                json!({
                    "type": "UpdateStepParams",
                    "stepId": "copy_input",
                    "params": {
                        "source": {"ref": "inputs.source_dir"},
                        "dest": "/sdcard/Input",
                        "copy_policy": value
                    }
                }),
            ),
        ]));
        assert_eq!(responses.len(), 2);
        let changed = assert_changed(&responses[1]);
        assert!(step(changed, "copy_input")["params"]
            .get("copy_policy")
            .is_some());
    }
}

#[test]
fn update_step_constraints_updates_casing_and_application_failures_preserve_history() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "update-constraints",
            json!({
                "type": "UpdateStepConstraints",
                "stepId": "copy_input",
                "ignoredByPython": true,
                "constraints": {
                    "capabilities": ["shared_storage_write"],
                    "conflictsWith": ["resolve"]
                }
            }),
        ),
        request("undo", "undo"),
        command_request(
            "duplicate-normalized-capability",
            json!({
                "type": "UpdateStepConstraints",
                "stepId": "copy_input",
                "constraints": {
                    "capabilities": ["shared_storage_write", " shared_storage_write "],
                    "conflictsWith": []
                }
            }),
        ),
        command_request(
            "blank-after-trim-conflict",
            json!({
                "type": "UpdateStepConstraints",
                "stepId": "copy_input",
                "constraints": {
                    "capabilities": [],
                    "conflictsWith": [" "]
                }
            }),
        ),
        command_request(
            "snake-case-invalid",
            json!({
                "type": "UpdateStepConstraints",
                "stepId": "copy_input",
                "constraints": {"capabilities": [], "conflicts_with": ["resolve"]}
            }),
        ),
        request("get", "getDocument"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 7);
    let updated = assert_changed(&responses[1]);
    let copy = step(updated, "copy_input");
    assert_eq!(
        copy["constraints"],
        json!({"capabilities": ["shared_storage_write"], "conflictsWith": ["resolve"]})
    );
    assert!(updated["yaml"]
        .as_str()
        .unwrap()
        .contains("conflicts_with:"));

    let undone = assert_changed(&responses[2]).clone();
    assert_eq!(
        step(&undone, "copy_input")["constraints"]["capabilities"],
        json!([])
    );
    assert_eq!(undone["canRedo"], true);

    assert_command_failed(&responses[3]);
    assert_command_failed(&responses[4]);
    assert_invalid_command(&responses[5]);
    assert_eq!(responses[6]["result"]["document"], undone);
}

#[test]
fn update_step_skip_if_and_verify_replace_conditions_preserving_literal_params() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "update-skip-if",
            json!({
                "type": "UpdateStepSkipIf",
                "stepId": "copy_input",
                "ignoredByPython": true,
                "skipIf": [
                    {
                        "type": "unknown_condition",
                        "params": {
                            "zeta": 1,
                            "nested_ref": {"ref": "steps.missing.outputs.value"},
                            "alpha": null,
                            "items": [{"ref": "inputs.source_dir"}],
                            "enabled": false
                        }
                    }
                ]
            }),
        ),
        command_request(
            "update-verify",
            json!({
                "type": "UpdateStepVerify",
                "stepId": "copy_input",
                "ignoredByPython": true,
                "verify": [
                    {
                        "type": "path_exists",
                        "params": {"path": "/sdcard/Input", "expected": true}
                    },
                    {"type": "custom_without_params"}
                ]
            }),
        ),
        request("undo-verify", "undo"),
        request("redo-verify", "redo"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 5);
    let skipped = assert_changed(&responses[1]);
    let copy = step(skipped, "copy_input");
    assert_eq!(copy["skipIf"][0]["type"], "unknown_condition");
    assert_eq!(
        copy["skipIf"][0]["params"]["nested_ref"],
        json!({"ref": "steps.missing.outputs.value"})
    );
    assert_eq!(
        copy["skipIf"][0]["params"]["items"],
        json!([{"ref": "inputs.source_dir"}])
    );
    assert_eq!(copy["dependencies"], json!([]));
    assert!(!all_refs(skipped).contains(&"steps.missing.outputs.value".to_string()));
    assert!(skipped["yaml"].as_str().unwrap().contains("skip_if:"));

    let verified = assert_changed(&responses[2]);
    let copy = step(verified, "copy_input");
    assert_eq!(copy["verify"][0]["type"], "path_exists");
    assert_eq!(copy["verify"][0]["params"]["expected"], true);
    assert_eq!(copy["verify"][1]["params"], json!({}));
    assert!(verified["yaml"].as_str().unwrap().contains("verify:"));

    assert_eq!(
        step(assert_changed(&responses[3]), "copy_input")["verify"],
        json!([])
    );
    assert_eq!(
        step(assert_changed(&responses[4]), "copy_input")["verify"][0]["type"],
        "path_exists"
    );
}

#[test]
fn step_internals_noops_do_not_push_undo_or_clear_redo() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "make-redo",
            json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": {"capabilities": ["shared_storage_write"], "conflictsWith": []}}),
        ),
        request("undo", "undo"),
        command_request(
            "noop-params",
            json!({"type": "UpdateStepParams", "stepId": "copy_input", "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/Input"}}),
        ),
        command_request(
            "noop-constraints",
            json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": {"capabilities": [], "conflictsWith": []}}),
        ),
        command_request(
            "noop-skip-if",
            json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": []}),
        ),
        command_request(
            "noop-verify",
            json!({"type": "UpdateStepVerify", "stepId": "copy_input", "verify": []}),
        ),
        request("redo", "redo"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 8);
    assert_changed(&responses[1]);
    assert_eq!(assert_changed(&responses[2])["canRedo"], true);
    for response in &responses[3..=6] {
        let document = assert_unchanged(response);
        assert_eq!(document["canRedo"], true);
    }
    assert_eq!(
        step(assert_changed(&responses[7]), "copy_input")["constraints"]["capabilities"],
        json!(["shared_storage_write"])
    );
}

#[test]
fn missing_step_and_invalid_step_internals_commands_leave_document_and_history_unchanged() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "make-redo",
            json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": {"capabilities": ["shared_storage_write"], "conflictsWith": []}}),
        ),
        request("undo", "undo"),
        command_request(
            "missing-params-step",
            json!({"type": "UpdateStepParams", "stepId": "missing", "params": {}}),
        ),
        command_request(
            "missing-constraints-step",
            json!({"type": "UpdateStepConstraints", "stepId": "missing", "constraints": {}}),
        ),
        command_request(
            "missing-skip-if-step",
            json!({"type": "UpdateStepSkipIf", "stepId": "missing", "skipIf": []}),
        ),
        command_request(
            "missing-verify-step",
            json!({"type": "UpdateStepVerify", "stepId": "missing", "verify": []}),
        ),
        command_request(
            "invalid-payload",
            json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": [{"type": "path_exists", "params": [], "custom": true}]}),
        ),
        request("get", "getDocument"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 9);
    assert_changed(&responses[1]);
    let undone = assert_changed(&responses[2]).clone();
    assert_eq!(undone["canRedo"], true);
    for response in &responses[3..=6] {
        assert_command_failed(response);
    }
    assert_invalid_command(&responses[7]);
    assert_eq!(responses[8]["result"]["document"], undone);
}

#[test]
fn save_after_each_step_internals_command_preserves_history_and_updates_baseline() {
    for (name, command, yaml_marker) in [
        (
            "params",
            json!({"type": "UpdateStepParams", "stepId": "copy_input", "params": {"source": {"ref": "inputs.source_dir"}, "dest": "/sdcard/SaveParams"}}),
            "/sdcard/SaveParams",
        ),
        (
            "constraints",
            json!({"type": "UpdateStepConstraints", "stepId": "copy_input", "constraints": {"capabilities": ["shared_storage_write"], "conflictsWith": []}}),
            "shared_storage_write",
        ),
        (
            "skip-if",
            json!({"type": "UpdateStepSkipIf", "stepId": "copy_input", "skipIf": [{"type": "path_exists", "params": {"path": "/tmp/skip"}}]}),
            "/tmp/skip",
        ),
        (
            "verify",
            json!({"type": "UpdateStepVerify", "stepId": "copy_input", "verify": [{"type": "path_exists", "params": {"path": "/tmp/verify"}}]}),
            "/tmp/verify",
        ),
    ] {
        let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
        let responses = sidecar_responses(&jsonl_input(vec![
            open_request(&temp_recipe.path),
            command_request(name, command),
            request("undo", "undo"),
            request("save", "saveRecipe"),
            request("redo", "redo"),
        ]));
        assert_eq!(responses.len(), 5);
        assert_changed(&responses[1]);
        let undone = assert_changed(&responses[2]);
        assert_eq!(undone["dirty"], false);
        assert_eq!(undone["canRedo"], true);
        let saved = &responses[3]["result"]["document"];
        assert_eq!(saved["dirty"], false);
        assert_eq!(saved["canRedo"], true);
        assert!(step(assert_changed(&responses[4]), "copy_input")["params"]
            .as_object()
            .is_some());
        let saved_yaml =
            fs::read_to_string(&temp_recipe.path).expect("saved YAML should be readable");
        assert!(
            !saved_yaml.contains(yaml_marker),
            "save after undo should not persist marker for {name}"
        );
    }
}
