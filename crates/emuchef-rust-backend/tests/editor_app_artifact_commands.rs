use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::{request, session::DocumentSessionManager};
use serde_json::{json, Value};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("authored_root")
}

struct TempWorkspace {
    dir: PathBuf,
}

impl TempWorkspace {
    fn copy_fixture(name: &str) -> Self {
        static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "emuchef-rust-backend-app-artifact-{}-{unique}-{sequence}",
            std::process::id()
        ));
        copy_dir_all(&fixture_root().join(name), &dir);
        Self { dir }
    }

    fn authored_root(&self) -> PathBuf {
        self.dir.join("authored")
    }

    fn recipe_path(&self, name: &str) -> PathBuf {
        self.authored_root().join("recipes").join(name)
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn copy_dir_all(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("destination directory should be created");
    for entry in fs::read_dir(source).expect("source directory should be readable") {
        let entry = entry.expect("source entry should be readable");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("file type should be readable").is_dir() {
            copy_dir_all(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("fixture file should copy");
        }
    }
}

fn open_app_aware_recipe(sessions: &mut DocumentSessionManager, workspace: &TempWorkspace) -> String {
    let response = request::handle_sidecar_value(
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {
                "path": workspace.recipe_path("demo.app.install.yaml"),
                "authoredRoot": workspace.authored_root().to_string_lossy(),
            }
        }),
        sessions,
    );
    assert_eq!(response["ok"], true, "openRecipe should succeed: {response}");
    response["result"]["document"]["documentId"]
        .as_str()
        .expect("document id should be a string")
        .to_string()
}

fn apply_command(
    sessions: &mut DocumentSessionManager,
    document_id: &str,
    command: Value,
) -> Value {
    request::handle_sidecar_value(
        json!({
            "id": "edit",
            "type": "applyRecipeCommand",
            "payload": {"documentId": document_id, "command": command}
        }),
        sessions,
    )
}

fn command_document(response: &Value) -> &Value {
    assert_eq!(response["ok"], true, "command should succeed: {response}");
    &response["result"]["document"]
}

fn artifact<'a>(document: &'a Value, artifact_id: &str) -> &'a Value {
    &document["recipe"]["artifacts"][artifact_id]
}

fn step<'a>(document: &'a Value, step_id: &str) -> &'a Value {
    document["recipe"]["steps"]
        .as_array()
        .expect("steps should be an array")
        .iter()
        .find(|step| step["id"] == step_id)
        .unwrap_or_else(|| panic!("step {step_id} should exist"))
}

fn undo(sessions: &mut DocumentSessionManager, document_id: &str) -> Value {
    request::handle_sidecar_value(
        json!({"id": "undo", "type": "undo", "payload": {"documentId": document_id}}),
        sessions,
    )
}

fn redo(sessions: &mut DocumentSessionManager, document_id: &str) -> Value {
    request::handle_sidecar_value(
        json!({"id": "redo", "type": "redo", "payload": {"documentId": document_id}}),
        sessions,
    )
}

fn diagnostic_codes(document: &Value) -> Vec<String> {
    document["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn add_app_artifact_binds_existing_input_without_creating_inputs() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let document_id = open_app_aware_recipe(&mut sessions, &workspace);

    let response = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "AddAppArtifact",
            "artifactId": "extra_config",
            "appRef": "demo_app",
            "artifact": "config",
            "input": "config"
        }),
    );
    let document = command_document(&response);
    assert_eq!(response["result"]["commandResult"]["changed"], true);

    let extra = artifact(document, "extra_config");
    assert_eq!(extra["type"], "app_artifact");
    assert_eq!(extra["appRef"], "demo_app");
    assert_eq!(extra["artifact"], "config");
    assert_eq!(extra["input"], "config");

    let input_ids = document["recipe"]["inputs"]
        .as_object()
        .expect("inputs should be an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(input_ids, vec!["config".to_string()]);

    let yaml = document["yaml"].as_str().expect("yaml should be a string");
    assert!(yaml.contains("input:\n      ref: inputs.config"), "yaml should bind the input: {yaml}");

    let undone = request::handle_sidecar_value(
        json!({"id": "undo", "type": "undo", "payload": {"documentId": document_id}}),
        &mut sessions,
    );
    assert_eq!(undone["ok"], true);
    assert!(artifact(&undone["result"]["document"], "extra_config").is_null());

    let redone = request::handle_sidecar_value(
        json!({"id": "redo", "type": "redo", "payload": {"documentId": document_id}}),
        &mut sessions,
    );
    assert_eq!(redone["ok"], true);
    assert_eq!(artifact(&redone["result"]["document"], "extra_config")["input"], "config");
}

#[test]
fn add_app_artifact_adds_parameterless_binding_variants() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let document_id = open_app_aware_recipe(&mut sessions, &workspace);

    let response = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "AddAppArtifact",
            "artifactId": "extra_apk",
            "appRef": "second_app",
            "artifact": "apk"
        }),
    );
    let document = command_document(&response);
    let extra = artifact(document, "extra_apk");
    assert_eq!(extra["appRef"], "second_app");
    assert_eq!(extra["artifact"], "apk");
    assert!(extra["input"].is_null(), "an omitted input should stay absent: {extra}");
    assert_eq!(extra["cache"], "default");
}

#[test]
fn add_app_artifact_rejects_invalid_identifier_syntax_without_mutating_the_document() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let document_id = open_app_aware_recipe(&mut sessions, &workspace);

    for (artifact_id, app_ref, artifact_name) in [
        ("Bad Id", "demo_app", "config"),
        ("extra", "Bad Ref", "config"),
        ("extra", "demo_app", "Bad Artifact"),
    ] {
        let response = apply_command(
            &mut sessions,
            &document_id,
            json!({
                "type": "AddAppArtifact",
                "artifactId": artifact_id,
                "appRef": app_ref,
                "artifact": artifact_name
            }),
        );
        assert_eq!(response["ok"], false, "invalid identifiers should be rejected: {response}");
    }

    let document = request::handle_sidecar_value(
        json!({"id": "get", "type": "getDocument", "payload": {"documentId": document_id}}),
        &mut sessions,
    );
    let document = &document["result"]["document"];
    assert_eq!(document["canUndo"], false);
    assert!(artifact(document, "extra").is_null());
}

#[test]
fn update_app_artifact_fields_retain_dependent_values_and_surface_diagnostics() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let document_id = open_app_aware_recipe(&mut sessions, &workspace);

    apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "AddAppArtifact",
            "artifactId": "extra",
            "appRef": "demo_app",
            "artifact": "apk"
        }),
    );

    let rebound = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateArtifactField",
            "artifactId": "extra",
            "field": "appRef",
            "value": "second_app"
        }),
    );
    let document = command_document(&rebound);
    let extra = artifact(document, "extra");
    assert_eq!(extra["appRef"], "second_app");
    assert_eq!(extra["artifact"], "apk", "dependent values must not be rewritten");
    assert!(
        !diagnostic_codes(document).contains(&"app_artifact_unknown".to_string()),
        "second_app defines apk: {:?}",
        diagnostic_codes(document)
    );

    let unknown_artifact = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateArtifactField",
            "artifactId": "extra",
            "field": "artifact",
            "value": "config"
        }),
    );
    let document = command_document(&unknown_artifact);
    let extra = artifact(document, "extra");
    assert_eq!(extra["artifact"], "config", "the retained value stays visible");
    assert!(
        diagnostic_codes(document).contains(&"app_artifact_unknown".to_string()),
        "an App Definition that does not define the artifact must produce an actionable diagnostic: {:?}",
        diagnostic_codes(document)
    );
}

#[test]
fn update_app_artifact_input_binding_binds_clears_and_rejects_remote_files() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let document_id = open_app_aware_recipe(&mut sessions, &workspace);

    apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "AddAppArtifact",
            "artifactId": "extra",
            "appRef": "demo_app",
            "artifact": "config"
        }),
    );

    let bound = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateArtifactField",
            "artifactId": "extra",
            "field": "input",
            "value": "config"
        }),
    );
    assert_eq!(artifact(command_document(&bound), "extra")["input"], "config");

    let cleared = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateArtifactField",
            "artifactId": "extra",
            "field": "input",
            "value": null
        }),
    );
    assert!(artifact(command_document(&cleared), "extra")["input"].is_null());

    let remote_file = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateArtifactField",
            "artifactId": "extra",
            "field": "input",
            "value": "Bad Input"
        }),
    );
    assert_eq!(remote_file["ok"], false, "invalid input syntax should be rejected");

    let installer = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateArtifactField",
            "artifactId": "installer",
            "field": "appRef",
            "value": "second_app"
        }),
    );
    assert_eq!(installer["ok"], true, "app artifacts accept App-owned field updates");

    let unknown_field = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateArtifactField",
            "artifactId": "extra",
            "field": "url",
            "value": "https://example.com/extra.apk"
        }),
    );
    assert_eq!(
        unknown_field["ok"], false,
        "App Definition artifact sources stay owned by the App Definition"
    );
}

#[test]
fn app_artifact_rename_duplicate_and_delete_rewrite_recipe_use_with_undo() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let document_id = open_app_aware_recipe(&mut sessions, &workspace);

    // A group that selects the installer artifact gives rename and delete a
    // second dependent structure to rewrite.
    apply_command(
        &mut sessions,
        &document_id,
        json!({"type": "AddArtifactGroup", "groupId": "bundle"}),
    );
    apply_command(
        &mut sessions,
        &document_id,
        json!({"type": "AddArtifactGroupMember", "groupId": "bundle", "artifactId": "installer"}),
    );

    // Renaming rewrites group membership and every structured step reference.
    let renamed = apply_command(
        &mut sessions,
        &document_id,
        json!({"type": "RenameArtifact", "artifactId": "installer", "newArtifactId": "installer_v2"}),
    );
    let document = command_document(&renamed);
    assert!(artifact(document, "installer").is_null());
    assert_eq!(artifact(document, "installer_v2")["appRef"], "demo_app");
    assert_eq!(document["recipe"]["artifactGroups"]["bundle"], json!(["installer_v2"]));
    assert_eq!(
        step(document, "install_demo")["params"]["app"]["ref"],
        "artifacts.installer_v2.local_path"
    );
    assert_eq!(step(document, "resolve_demo_apk")["params"]["artifacts"], json!(["installer_v2"]));

    // Undo restores every rewritten reference; redo re-applies the rename.
    let undone = undo(&mut sessions, &document_id);
    let document = command_document(&undone);
    assert!(!artifact(document, "installer").is_null(), "undo restores the artifact");
    assert_eq!(document["recipe"]["artifactGroups"]["bundle"], json!(["installer"]));
    assert_eq!(
        step(document, "install_demo")["params"]["app"]["ref"],
        "artifacts.installer.local_path"
    );
    assert_eq!(step(document, "resolve_demo_apk")["params"]["artifacts"], json!(["installer"]));

    let redone = redo(&mut sessions, &document_id);
    let document = command_document(&redone);
    assert!(!artifact(document, "installer_v2").is_null(), "redo re-applies the rename");

    // Duplicate clones the artifact only: dependent step references keep
    // pointing at the source artifact.
    let duplicated = apply_command(
        &mut sessions,
        &document_id,
        json!({"type": "DuplicateArtifact", "sourceArtifactId": "installer_v2", "newArtifactId": "installer_copy"}),
    );
    let document = command_document(&duplicated);
    assert_eq!(artifact(document, "installer_copy")["appRef"], "demo_app");
    assert_eq!(artifact(document, "installer_copy")["artifact"], "apk");
    assert_eq!(
        step(document, "install_demo")["params"]["app"]["ref"],
        "artifacts.installer_v2.local_path",
        "duplication never rewrites dependent references"
    );

    // Delete removes the artifact, its group membership, and step references
    // in one undoable command.
    let deleted = apply_command(
        &mut sessions,
        &document_id,
        json!({"type": "DeleteArtifact", "artifactId": "installer_v2"}),
    );
    let document = command_document(&deleted);
    assert!(artifact(document, "installer_v2").is_null());
    assert_eq!(document["recipe"]["artifactGroups"]["bundle"], json!([]));
    assert!(step(document, "install_demo")["params"]["app"].is_null());
    assert_eq!(step(document, "resolve_demo_apk")["params"]["artifacts"], json!([]));
    assert!(!artifact(document, "installer_copy").is_null(), "other artifacts stay");

    let restored = undo(&mut sessions, &document_id);
    let document = command_document(&restored);
    assert!(!artifact(document, "installer_v2").is_null(), "undo restores the deletion");
    assert_eq!(document["recipe"]["artifactGroups"]["bundle"], json!(["installer_v2"]));
    assert_eq!(
        step(document, "install_demo")["params"]["app"]["ref"],
        "artifacts.installer_v2.local_path"
    );
}

