//! Static Recipe validation coverage for App Definition authority.
//!
//! Every test opens an authored Recipe through the sidecar document-session
//! path used by the Config Editor and inspects the diagnostics that ordinary
//! Recipe validation reports without running device planning. The catalog
//! fixture root is copied to a temporary directory per test, so tests never
//! mutate the shared fixture tree.

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
            "emuchef-rust-backend-app-authority-{}-{unique}-{sequence}",
            std::process::id()
        ));
        copy_dir_all(&fixture_root().join(name), &dir);
        Self { dir }
    }

    fn authored_root(&self) -> PathBuf {
        self.dir.join("authored")
    }

    fn recipe_path(&self, file_name: &str) -> PathBuf {
        self.authored_root().join("recipes").join(file_name)
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

/// Opens one fixture Recipe and returns its session id plus the projected document.
fn open_recipe(
    sessions: &mut DocumentSessionManager,
    workspace: &TempWorkspace,
    file_name: &str,
) -> (String, Value) {
    let response = request::handle_sidecar_value(
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {
                "path": workspace.recipe_path(file_name),
                "authoredRoot": workspace.authored_root().to_string_lossy(),
            }
        }),
        sessions,
    );
    assert_eq!(response["ok"], true, "openRecipe should succeed: {response}");
    let document = response["result"]["document"].clone();
    let document_id = document["documentId"]
        .as_str()
        .expect("document id should be a string")
        .to_string();
    (document_id, document)
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

fn diagnostic_codes(document: &Value) -> Vec<String> {
    document["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str().map(str::to_string))
        .collect()
}

fn assert_has_code(document: &Value, code: &str) {
    let codes = diagnostic_codes(document);
    assert!(
        codes.contains(&code.to_string()),
        "expected diagnostic {code:?}: {codes:?}"
    );
}

fn assert_missing_code(document: &Value, code: &str) {
    let codes = diagnostic_codes(document);
    assert!(
        !codes.contains(&code.to_string()),
        "expected no diagnostic {code:?}: {codes:?}"
    );
}

fn document_yaml(document: &Value) -> &str {
    document["yaml"].as_str().expect("yaml should be a string")
}

#[test]
fn undefined_permission_set_is_reported_without_device_planning() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.undefined_permission_set.yaml",
    );
    assert_has_code(&document, "permission_set_missing");
    assert!(
        document_yaml(&document).contains("permission_set: elevated"),
        "the authored selection must stay visible: {}",
        document_yaml(&document)
    );
}

#[test]
fn missing_permission_set_selection_is_reported() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.missing_permission_set.yaml",
    );
    assert_has_code(&document, "permission_set_required");
}

#[test]
fn permission_set_diagnostics_refresh_after_editor_commands() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (document_id, document) =
        open_recipe(&mut sessions, &workspace, "demo.app.install.yaml");
    assert_missing_code(&document, "permission_set_missing");

    let selected = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateStepParams",
            "stepId": "grant_demo",
            "params": {
                "permission_set": "elevated",
                "policy": {"on_failure": "warn"}
            }
        }),
    );
    let document = command_document(&selected);
    assert_has_code(document, "permission_set_missing");
    assert!(
        document_yaml(document).contains("permission_set: elevated"),
        "the selected value must be retained: {}",
        document_yaml(document)
    );

    let restored = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateStepParams",
            "stepId": "grant_demo",
            "params": {
                "permission_set": "baseline",
                "policy": {"on_failure": "warn"}
            }
        }),
    );
    assert_missing_code(command_document(&restored), "permission_set_missing");
}

#[test]
fn unknown_app_reference_is_retained_with_actionable_diagnostic() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.retained_unknown_app.yaml",
    );
    assert_has_code(&document, "app_definition_unknown");
    assert_eq!(
        document["recipe"]["steps"][0]["appRef"], "ghost_app",
        "the stale app selection must stay visible: {document}"
    );
    assert!(
        document_yaml(&document).contains("app_ref: ghost_app"),
        "the stale app selection must serialize unchanged: {}",
        document_yaml(&document)
    );
}

#[test]
fn unused_unknown_app_artifact_is_reported() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.unused_unknown_artifact.yaml",
    );
    assert_has_code(&document, "app_artifact_unknown");
    assert_eq!(
        document["recipe"]["artifacts"]["ghost"]["artifact"], "missing_artifact",
        "the retained artifact selection must stay visible: {document}"
    );
}

#[test]
fn cross_app_install_provenance_is_rejected() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.cross_app_artifact.yaml",
    );
    assert_has_code(&document, "app_install_artifact_provenance_invalid");
}

#[test]
fn launcher_unavailable_is_reported() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.launcher_unavailable.yaml",
    );
    assert_has_code(&document, "app_launcher_activity_required");
}

#[test]
fn app_context_inapplicable_is_reported_after_editor_command() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (document_id, _) = open_recipe(&mut sessions, &workspace, "demo.app.install.yaml");

    let response = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "SetStepAppRef",
            "stepId": "resolve_demo_apk",
            "appRef": "demo_app"
        }),
    );
    let document = command_document(&response);
    assert_has_code(document, "app_context_inapplicable");
    let step = document["recipe"]["steps"]
        .as_array()
        .expect("steps should be an array")
        .iter()
        .find(|step| step["id"] == "resolve_demo_apk")
        .expect("resolve_demo_apk should exist");
    assert_eq!(
        step["appRef"], "demo_app",
        "the selected app must stay visible: {document}"
    );
}

#[test]
fn parameterless_condition_without_app_context_is_reported() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (document_id, _) = open_recipe(&mut sessions, &workspace, "demo.target_usage.yaml");

    apply_command(
        &mut sessions,
        &document_id,
        json!({"type": "SetStepAppRef", "stepId": "copy_payload", "appRef": null}),
    );
    let response = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "UpdateStepVerify",
            "stepId": "copy_payload",
            "verify": [{"type": "package_installed", "params": {}}]
        }),
    );
    assert_has_code(command_document(&response), "app_condition_context_required");
}

#[test]
fn valid_app_aware_recipe_has_no_app_authority_diagnostics() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    for file_name in ["demo.app.install.yaml", "demo.target_usage.yaml"] {
        let (_, document) = open_recipe(&mut sessions, &workspace, file_name);
        let codes = diagnostic_codes(&document);
        assert!(
            codes
                .iter()
                .all(|code| !code.starts_with("app_") && !code.starts_with("permission_")),
            "{file_name} should report no app authority diagnostics: {codes:?}"
        );
    }
}

#[test]
fn app_target_usage_is_valid_and_retained_after_app_change() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (document_id, document) = open_recipe(&mut sessions, &workspace, "demo.target_usage.yaml");
    let codes = diagnostic_codes(&document);
    assert!(
        codes.iter().all(|code| !code.starts_with("app_target_")),
        "the authored target usage should validate: {codes:?}"
    );

    let response = apply_command(
        &mut sessions,
        &document_id,
        json!({
            "type": "SetStepAppRef",
            "stepId": "copy_payload",
            "appRef": "second_app"
        }),
    );
    let document = command_document(&response);
    assert_has_code(document, "app_target_unknown");
    assert!(
        document_yaml(document).contains("app_target: config_file"),
        "the dependent target selection must be retained: {}",
        document_yaml(document)
    );
}

/// Assert one artifact-scoped diagnostic for a specific Recipe artifact.
fn assert_artifact_code(document: &Value, artifact_id: &str, code: &str) {
    let diagnostics = document["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array");
    let matched = diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == code
            && diagnostic["objectKind"] == "artifact"
            && diagnostic["objectId"] == artifact_id
    });
    assert!(
        matched,
        "expected diagnostic {code:?} for artifact {artifact_id:?}: {diagnostics:?}"
    );
}

#[test]
fn unused_user_provided_artifact_without_binding_is_reported() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.unused_missing_binding.yaml",
    );
    assert_artifact_code(&document, "config", "app_artifact_input_binding_required");
}

#[test]
fn unused_binding_to_unknown_recipe_input_is_reported() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.unknown_input_binding.yaml",
    );
    assert_artifact_code(&document, "config", "app_artifact_input_binding_invalid");
}

#[test]
fn unused_bindings_to_incompatible_recipe_inputs_are_reported() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(
        &mut sessions,
        &workspace,
        "demo.incompatible_input_binding.yaml",
    );
    assert_artifact_code(&document, "config_folder", "app_artifact_input_binding_invalid");
    assert_artifact_code(&document, "config_many", "app_artifact_input_binding_invalid");
}

#[test]
fn binding_on_non_user_provided_artifact_is_reported() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (_, document) = open_recipe(&mut sessions, &workspace, "demo.unsupported_binding.yaml");
    assert_artifact_code(&document, "installer", "app_artifact_input_binding_unsupported");
}


