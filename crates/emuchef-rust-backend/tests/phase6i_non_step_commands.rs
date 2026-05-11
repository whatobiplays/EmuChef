use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::jsonl;
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
        .join("python_goldens")
        .join(name)
}

fn read_golden(name: &str) -> Value {
    let text = fs::read_to_string(golden_path(name)).expect("Python golden should be readable");
    serde_json::from_str(&text).expect("Python golden should be valid JSON")
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
            "emuchef-rust-backend-phase6i-{}-{unique}-{sequence}",
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

fn input_refs(document: &Value) -> Vec<Value> {
    document["refIndex"]["inputRefs"]
        .as_array()
        .expect("inputRefs should be an array")
        .clone()
}

fn artifact_refs(document: &Value) -> Vec<Value> {
    document["refIndex"]["artifactRefs"]
        .as_array()
        .expect("artifactRefs should be an array")
        .clone()
}

#[test]
fn input_commands_update_dto_yaml_ref_index_and_history() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = jsonl_input(vec![
        open_request(&temp_recipe.path),
        command_request(
            "add-input-extra-key",
            json!({"type": "AddInput", "inputId": "save_dir", "ignoredByPython": true}),
        ),
        command_request(
            "update-label",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "label", "value": 42}),
        ),
        command_request(
            "update-required-false",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "required", "value": ""}),
        ),
        command_request(
            "update-multiple-true",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "multiple", "value": "yes"}),
        ),
        command_request(
            "update-must-exist-false",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "validation.must_exist", "value": 0}),
        ),
        command_request(
            "update-extensions",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "validation.allowed_extensions", "value": "sav, state,, "}),
        ),
        command_request(
            "update-path-kind-null",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "validation.path_kind", "value": null}),
        ),
        command_request(
            "update-type",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "type", "value": "directory"}),
        ),
        command_request(
            "update-role",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "role", "value": "bios"}),
        ),
        command_request(
            "update-description-object",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "description", "value": {"note": ["a", 1, true, null]}}),
        ),
        command_request(
            "update-extensions-list",
            json!({"type": "UpdateInputField", "inputId": "save_dir", "field": "validation.allowed_extensions", "value": [".zip", "", null, 7]}),
        ),
        command_request(
            "rename-input",
            json!({"type": "RenameInput", "inputId": "save_dir", "newInputId": "saves_dir"}),
        ),
        command_request(
            "duplicate-input",
            json!({"type": "DuplicateInput", "sourceInputId": "source_dir", "newInputId": "source_dir_copy"}),
        ),
        command_request(
            "delete-input",
            json!({"type": "DeleteInput", "inputId": "saves_dir"}),
        ),
        command_request(
            "same-id-rename-noop",
            json!({"type": "RenameInput", "inputId": "bios_file", "newInputId": "bios_file"}),
        ),
        request("undo-delete", "undo"),
        request("redo-delete", "redo"),
        request("get", "getDocument"),
    ]);

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 19);

    let added = assert_changed(&responses[1]);
    assert_eq!(added["recipe"]["inputs"]["save_dir"]["type"], "file");
    assert_eq!(added["recipe"]["inputs"]["save_dir"]["role"], "generic");
    assert_eq!(added["recipe"]["inputs"]["save_dir"]["label"], "save_dir");
    assert_eq!(added["recipe"]["inputs"]["save_dir"]["required"], false);
    assert_eq!(added["recipe"]["inputs"]["save_dir"]["multiple"], false);
    assert_eq!(
        added["recipe"]["inputs"]["save_dir"]["validation"]["pathKind"],
        "file"
    );
    assert!(input_refs(added).contains(&json!("inputs.save_dir")));
    assert!(added["yaml"].as_str().unwrap().contains("save_dir:"));

    let updated = assert_changed(&responses[11]);
    assert_eq!(updated["recipe"]["inputs"]["save_dir"]["type"], "directory");
    assert_eq!(updated["recipe"]["inputs"]["save_dir"]["role"], "bios");
    assert_eq!(updated["recipe"]["inputs"]["save_dir"]["label"], "42");
    assert_eq!(
        updated["recipe"]["inputs"]["save_dir"]["description"],
        "{'note': ['a', 1, True, None]}"
    );
    assert_eq!(updated["recipe"]["inputs"]["save_dir"]["required"], false);
    assert_eq!(updated["recipe"]["inputs"]["save_dir"]["multiple"], true);
    assert_eq!(
        updated["recipe"]["inputs"]["save_dir"]["validation"]["mustExist"],
        false
    );
    assert_eq!(
        updated["recipe"]["inputs"]["save_dir"]["validation"]["allowedExtensions"],
        json!([".zip", "None", "7"])
    );
    assert_eq!(
        updated["recipe"]["inputs"]["save_dir"]["validation"]["pathKind"],
        Value::Null
    );

    let renamed = assert_changed(&responses[12]);
    assert!(renamed["recipe"]["inputs"].get("save_dir").is_none());
    assert!(renamed["recipe"]["inputs"].get("saves_dir").is_some());
    assert!(input_refs(renamed).contains(&json!("inputs.saves_dir")));
    assert!(!input_refs(renamed).contains(&json!("inputs.save_dir")));

    let duplicated = assert_changed(&responses[13]);
    assert_eq!(
        duplicated["recipe"]["inputs"]["source_dir_copy"]["metadata"],
        duplicated["recipe"]["inputs"]["source_dir"]["metadata"]
    );
    assert_eq!(
        duplicated["recipe"]["inputs"]["source_dir_copy"]["validation"],
        duplicated["recipe"]["inputs"]["source_dir"]["validation"]
    );

    let deleted = assert_changed(&responses[14]);
    assert!(deleted["recipe"]["inputs"].get("saves_dir").is_none());
    assert!(!input_refs(deleted).contains(&json!("inputs.saves_dir")));
    assert_eq!(deleted["dirty"], true);
    assert_eq!(deleted["canUndo"], true);
    assert_eq!(deleted["canRedo"], false);

    let noop = assert_unchanged(&responses[15]);
    assert_eq!(noop, deleted);

    let undone = assert_changed(&responses[16]);
    assert!(undone["recipe"]["inputs"].get("saves_dir").is_some());
    assert_eq!(undone["canRedo"], true);

    let redone = assert_changed(&responses[17]);
    assert!(redone["recipe"]["inputs"].get("saves_dir").is_none());
    assert_eq!(responses[18]["result"]["document"], *redone);
}

#[test]
fn artifact_commands_update_refs_and_cleanup_supported_usages() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request(
            "add-artifact",
            json!({"type": "AddArtifact", "artifactId": "new_zip", "url": " https://example.com/new.zip "})
        ),
        command_request(
            "update-url",
            json!({"type": "UpdateArtifactField", "artifactId": "new_zip", "field": "url", "value": "https://example.com/newer.zip"})
        ),
        command_request(
            "update-cache",
            json!({"type": "UpdateArtifactField", "artifactId": "new_zip", "field": "cache", "value": "none"})
        ),
        command_request(
            "rename-target",
            json!({"type": "RenameArtifact", "artifactId": "target_zip", "newArtifactId": "renamed_zip"})
        ),
        command_request(
            "duplicate-artifact",
            json!({"type": "DuplicateArtifact", "sourceArtifactId": "new_zip", "newArtifactId": "new_zip_copy"})
        ),
        // Mirrors Python cleanup covered by tests/test_editor_core.py:
        // test_delete_artifact_cleans_group_membership_step_selection_and_param_ref_in_one_command.
        command_request(
            "delete-renamed",
            json!({"type": "DeleteArtifact", "artifactId": "renamed_zip"})
        ),
        command_request(
            "same-id-rename-noop",
            json!({"type": "RenameArtifact", "artifactId": "other_zip", "newArtifactId": "other_zip"})
        ),
        request("undo-delete", "undo"),
        request("redo-delete", "redo"),
    );

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 10);

    let added = assert_changed(&responses[1]);
    assert_eq!(
        added["recipe"]["artifacts"]["new_zip"]["type"],
        "remote_file"
    );
    assert_eq!(
        added["recipe"]["artifacts"]["new_zip"]["url"],
        "https://example.com/new.zip"
    );
    assert_eq!(added["recipe"]["artifacts"]["new_zip"]["cache"], "default");
    assert!(artifact_refs(added).contains(&json!("artifacts.new_zip.local_path")));

    let updated = assert_changed(&responses[3]);
    assert_eq!(
        updated["recipe"]["artifacts"]["new_zip"]["url"],
        "https://example.com/newer.zip"
    );
    assert_eq!(updated["recipe"]["artifacts"]["new_zip"]["cache"], "none");

    let renamed = assert_changed(&responses[4]);
    assert!(renamed["recipe"]["artifacts"].get("target_zip").is_none());
    assert!(renamed["recipe"]["artifacts"].get("renamed_zip").is_some());
    assert_eq!(
        renamed["recipe"]["artifactGroups"]["bundle"],
        json!(["renamed_zip", "other_zip"])
    );
    assert_eq!(
        renamed["recipe"]["steps"][0]["params"]["artifacts"],
        json!(["renamed_zip", "other_zip"])
    );
    assert_eq!(
        renamed["recipe"]["steps"][1]["params"]["archive"],
        json!({"ref": "artifacts.renamed_zip.local_path"})
    );
    assert!(artifact_refs(renamed).contains(&json!("artifacts.renamed_zip.local_path")));
    assert!(!artifact_refs(renamed).contains(&json!("artifacts.target_zip.local_path")));

    let duplicated = assert_changed(&responses[5]);
    assert_eq!(
        duplicated["recipe"]["artifacts"]["new_zip_copy"]["id"],
        "new_zip_copy"
    );
    assert_eq!(
        duplicated["recipe"]["artifacts"]["new_zip_copy"]["type"],
        "remote_file"
    );
    assert_eq!(
        duplicated["recipe"]["artifacts"]["new_zip_copy"]["url"],
        duplicated["recipe"]["artifacts"]["new_zip"]["url"]
    );
    assert_eq!(
        duplicated["recipe"]["artifacts"]["new_zip_copy"]["cache"],
        duplicated["recipe"]["artifacts"]["new_zip"]["cache"]
    );

    let deleted = assert_changed(&responses[6]);
    assert!(deleted["recipe"]["artifacts"].get("renamed_zip").is_none());
    assert_eq!(
        deleted["recipe"]["artifactGroups"]["bundle"],
        json!(["other_zip"])
    );
    assert_eq!(
        deleted["recipe"]["steps"][0]["params"]["artifacts"],
        json!(["other_zip"])
    );
    assert!(deleted["recipe"]["steps"][1]["params"]
        .get("archive")
        .is_none());
    assert!(!deleted["yaml"]
        .as_str()
        .unwrap()
        .contains("artifacts.renamed_zip.local_path"));

    let noop = assert_unchanged(&responses[7]);
    assert_eq!(noop, deleted);

    let undone = assert_changed(&responses[8]);
    assert!(undone["recipe"]["artifacts"].get("renamed_zip").is_some());
    assert_eq!(undone["canRedo"], true);

    let redone = assert_changed(&responses[9]);
    assert!(redone["recipe"]["artifacts"].get("renamed_zip").is_none());
}

#[test]
fn artifact_group_commands_preserve_order_cleanup_and_ref_index_scope() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request(
            "add-group",
            json!({"type": "AddArtifactGroup", "groupId": "third_group"})
        ),
        command_request(
            "add-member-append",
            json!({"type": "AddArtifactGroupMember", "groupId": "third_group", "artifactId": "other_zip"})
        ),
        command_request(
            "add-member-index",
            json!({"type": "AddArtifactGroupMember", "groupId": "third_group", "artifactId": "target_zip", "index": 0})
        ),
        command_request(
            "reorder-member",
            json!({"type": "ReorderArtifactGroupMember", "groupId": "third_group", "index": 1, "toIndex": 0})
        ),
        command_request(
            "remove-member",
            json!({"type": "RemoveArtifactGroupMember", "groupId": "third_group", "index": 1})
        ),
        command_request(
            "duplicate-group",
            json!({"type": "DuplicateArtifactGroup", "sourceGroupId": "third_group", "newGroupId": "third_group_copy"})
        ),
        command_request(
            "reorder-group",
            json!({"type": "ReorderArtifactGroup", "groupId": "third_group_copy", "toIndex": 0})
        ),
        command_request(
            "same-index-reorder-noop",
            json!({"type": "ReorderArtifactGroup", "groupId": "third_group_copy", "toIndex": 0})
        ),
        // Mirrors Python cleanup covered by tests/test_editor_core.py:
        // test_delete_input_and_artifact_group_remove_supported_structured_refs_only and
        // test_rename_commands_rewrite_supported_structured_usages.
        command_request(
            "rename-bundle",
            json!({"type": "RenameArtifactGroup", "groupId": "bundle", "newGroupId": "renamed_bundle"})
        ),
        command_request(
            "delete-renamed-bundle",
            json!({"type": "DeleteArtifactGroup", "groupId": "renamed_bundle"})
        ),
        request("undo-delete", "undo"),
        request("redo-delete", "redo"),
    );

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 13);

    let ordered = assert_changed(&responses[7]);
    let yaml = ordered["yaml"].as_str().unwrap();
    assert!(yaml.find("  third_group_copy:").unwrap() < yaml.find("  bundle:").unwrap());
    assert!(yaml.find("  bundle:").unwrap() < yaml.find("  other_bundle:").unwrap());
    assert!(yaml.find("  other_bundle:").unwrap() < yaml.find("  third_group:").unwrap());
    assert_eq!(
        ordered["recipe"]["artifactGroups"]["third_group"],
        json!(["other_zip"])
    );
    assert_eq!(
        ordered["recipe"]["artifactGroups"]["third_group_copy"],
        json!(["other_zip"])
    );

    let noop = assert_unchanged(&responses[8]);
    assert_eq!(noop, ordered);

    let renamed = assert_changed(&responses[9]);
    assert!(renamed["recipe"]["artifactGroups"].get("bundle").is_none());
    assert_eq!(
        renamed["recipe"]["steps"][0]["params"]["artifact_groups"],
        json!(["renamed_bundle", "other_bundle"])
    );
    assert!(!renamed["refIndex"]["allRefs"]
        .as_array()
        .unwrap()
        .contains(&json!("artifact_groups.renamed_bundle")));

    let deleted = assert_changed(&responses[10]);
    assert!(deleted["recipe"]["artifactGroups"]
        .get("renamed_bundle")
        .is_none());
    assert_eq!(
        deleted["recipe"]["steps"][0]["params"]["artifact_groups"],
        json!(["other_bundle"])
    );
    assert!(!deleted["refIndex"]["allRefs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value
            .as_str()
            .unwrap_or_default()
            .starts_with("artifact_groups.")));

    let undone = assert_changed(&responses[11]);
    assert!(undone["recipe"]["artifactGroups"]
        .get("renamed_bundle")
        .is_some());
    assert_eq!(undone["canRedo"], true);

    let redone = assert_changed(&responses[12]);
    assert!(redone["recipe"]["artifactGroups"]
        .get("renamed_bundle")
        .is_none());
}

#[test]
fn invalid_and_failed_non_step_commands_leave_document_unchanged_without_history() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request("missing-input-id", json!({"type": "AddInput"})),
        command_request(
            "wrong-input-id-type",
            json!({"type": "AddInput", "inputId": null})
        ),
        command_request(
            "unsupported-input-field",
            json!({"type": "UpdateInputField", "inputId": "source_dir", "field": "metadata", "value": {}})
        ),
        command_request(
            "bad-member-index-type",
            json!({"type": "ReorderArtifactGroupMember", "groupId": "bundle", "index": 0, "toIndex": "1"})
        ),
        command_request(
            "unsupported-step-command",
            json!({"type": "AddStep", "stepId": "new_step"})
        ),
        command_request(
            "duplicate-input",
            json!({"type": "AddInput", "inputId": "source_dir"})
        ),
        command_request(
            "duplicate-member",
            json!({"type": "AddArtifactGroupMember", "groupId": "bundle", "artifactId": "target_zip"})
        ),
        command_request(
            "missing-member-artifact",
            json!({"type": "AddArtifactGroupMember", "groupId": "bundle", "artifactId": "missing_zip"})
        ),
        request("undo-after-errors", "undo"),
        request("get", "getDocument"),
    );

    let responses = sidecar_responses(&input);
    assert_eq!(responses.len(), 11);
    let opened = responses[0]["result"]["document"].clone();

    for index in 1..=5 {
        assert_invalid_command(&responses[index]);
    }
    for index in 6..=8 {
        assert_command_failed(&responses[index]);
    }
    assert_eq!(assert_unchanged(&responses[9]), &opened);
    assert_eq!(responses[10]["result"]["document"], opened);
}

#[test]
fn save_after_non_step_command_preserves_history_and_updates_baseline() {
    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        command_request(
            "add-input",
            json!({"type": "AddInput", "inputId": "save_after_input"})
        ),
        request("undo", "undo"),
        request("save", "saveRecipe"),
        request("redo", "redo"),
    );

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
    assert!(redone["recipe"]["inputs"].get("save_after_input").is_some());

    let saved_yaml = fs::read_to_string(&temp_recipe.path).expect("saved YAML should be readable");
    assert!(!saved_yaml.contains("save_after_input"));
}

#[test]
fn focused_phase6i_ref_index_results_match_python_goldens() {
    let input_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let input_responses = sidecar_responses(&format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&input_recipe.path),
        command_request(
            "add-input",
            json!({"type": "AddInput", "inputId": "gold_input"})
        ),
        command_request(
            "rename-input",
            json!({"type": "RenameInput", "inputId": "source_dir", "newInputId": "src_dir"})
        ),
        command_request(
            "duplicate-input",
            json!({"type": "DuplicateInput", "sourceInputId": "bios_file", "newInputId": "bios_copy"})
        ),
        command_request(
            "delete-input",
            json!({"type": "DeleteInput", "inputId": "gold_input"})
        ),
        request("get-ref-index", "getRefIndex"),
    ));
    assert_eq!(input_responses.len(), 6);
    assert_eq!(
        input_responses[5]["result"],
        read_golden("phase6i_after_inputs_get_ref_index.result.json")
    );

    let artifact_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let artifact_responses = sidecar_responses(&format!(
        "{}\n{}\n{}\n{}\n{}\n",
        open_request(&artifact_recipe.path),
        command_request(
            "add-artifact",
            json!({"type": "AddArtifact", "artifactId": "new_zip", "url": "https://example.com/new.zip"})
        ),
        command_request(
            "rename-artifact",
            json!({"type": "RenameArtifact", "artifactId": "target_zip", "newArtifactId": "renamed_zip"})
        ),
        command_request(
            "delete-artifact",
            json!({"type": "DeleteArtifact", "artifactId": "renamed_zip"})
        ),
        request("get-ref-index", "getRefIndex"),
    ));
    assert_eq!(artifact_responses.len(), 5);
    assert_eq!(
        artifact_responses[4]["result"],
        read_golden("phase6i_after_artifacts_get_ref_index.result.json")
    );

    let group_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let group_responses = sidecar_responses(&format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&group_recipe.path),
        command_request(
            "add-group",
            json!({"type": "AddArtifactGroup", "groupId": "gold_group"})
        ),
        command_request(
            "add-member",
            json!({"type": "AddArtifactGroupMember", "groupId": "gold_group", "artifactId": "other_zip"})
        ),
        command_request(
            "rename-group",
            json!({"type": "RenameArtifactGroup", "groupId": "bundle", "newGroupId": "renamed_bundle"})
        ),
        command_request(
            "delete-group",
            json!({"type": "DeleteArtifactGroup", "groupId": "renamed_bundle"})
        ),
        request("get-ref-index", "getRefIndex"),
    ));
    assert_eq!(group_responses.len(), 6);
    assert_eq!(
        group_responses[5]["result"],
        read_golden("phase6i_after_groups_get_ref_index.result.json")
    );
}
