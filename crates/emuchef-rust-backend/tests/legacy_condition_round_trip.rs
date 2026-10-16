//! Round-trip coverage for `package_installed` conditions.
//!
//! The document session parses authored YAML and re-emits it in a canonical
//! layout of its own, so a hand-authored file is not byte-identical to the
//! canonical emission. These tests therefore prove the two properties the
//! editor depends on:
//!
//! 1. Legacy conditions keep their authored `package_name` parameter and gain
//!    no app context, while app-aware conditions stay parameterless and
//!    inherit their package from the enclosing step `app_ref`.
//! 2. Saving and reopening a document reproduces the same canonical YAML and
//!    the same document projection, so a load/save cycle never migrates a
//!    condition silently.

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
            "emuchef-rust-backend-condition-round-trip-{}-{unique}-{sequence}",
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

/// Opens one fixture Recipe on a fresh session and returns its projected document.
fn open_recipe(workspace: &TempWorkspace, file_name: &str) -> (String, Value) {
    let mut sessions = DocumentSessionManager::default();
    open_fixture(&mut sessions, workspace, file_name)
}

/// Opens one fixture Recipe on the given session and returns its session id
/// plus the projected document.
fn open_fixture(
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

fn diagnostic_codes(document: &Value) -> Vec<String> {
    document["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str().map(str::to_string))
        .collect()
}

fn step<'a>(document: &'a Value, step_id: &str) -> &'a Value {
    document["recipe"]["steps"]
        .as_array()
        .expect("steps should be an array")
        .iter()
        .find(|step| step["id"] == step_id)
        .unwrap_or_else(|| panic!("step {step_id:?} should exist: {document}"))
}

#[test]
fn legacy_package_condition_round_trips_without_migration() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (document_id, document) =
        open_fixture(&mut sessions, &workspace, "demo.legacy_package_condition.yaml");
    let yaml = document["yaml"].as_str().expect("yaml should be a string");
    assert!(
        yaml.contains("package_name: com.example.demo"),
        "the legacy condition must keep its authored parameter: {yaml}"
    );

    let wait_step = step(&document, "wait_for_legacy");
    assert!(
        wait_step["appRef"].is_null(),
        "a legacy condition must not gain app context: {wait_step}"
    );
    assert_eq!(
        wait_step["verify"][0]["params"]["package_name"], "com.example.demo",
        "the projection must retain the legacy parameter: {wait_step}"
    );
    let codes = diagnostic_codes(&document);
    assert!(
        !codes.contains(&"app_condition_package_parameter_forbidden".to_string()),
        "a legacy condition on a step without an app context must stay accepted: {codes:?}"
    );

    let saved = request::handle_sidecar_value(
        json!({"id": "save", "type": "saveRecipe", "payload": {"documentId": document_id}}),
        &mut sessions,
    );
    assert_eq!(saved["ok"], true, "saveRecipe should succeed: {saved}");
    assert_eq!(
        fs::read_to_string(workspace.recipe_path("demo.legacy_package_condition.yaml"))
            .expect("saved recipe should read"),
        yaml,
        "saving must write the canonical YAML"
    );

    let (_, reopened) = open_recipe(&workspace, "demo.legacy_package_condition.yaml");
    assert_eq!(
        reopened["yaml"], yaml,
        "reopening must reproduce the same canonical YAML"
    );
    assert_eq!(
        reopened["recipe"], document["recipe"],
        "reopening must reproduce the same document projection"
    );
}

#[test]
fn app_aware_parameterless_condition_round_trips_parameterless() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (document_id, document) =
        open_fixture(&mut sessions, &workspace, "demo.app.install.yaml");
    let yaml = document["yaml"].as_str().expect("yaml should be a string");
    assert!(
        yaml.contains("type: package_installed\n    params: {}"),
        "an app-aware condition must stay parameterless: {yaml}"
    );
    assert!(
        !yaml.contains("package_name"),
        "an app-aware condition must never author package_name: {yaml}"
    );

    let install = step(&document, "install_demo");
    assert_eq!(install["appRef"], "demo_app");
    assert_eq!(
        install["verify"][0]["params"],
        json!({}),
        "the condition must inherit its package from the step app_ref: {install}"
    );

    let saved = request::handle_sidecar_value(
        json!({"id": "save", "type": "saveRecipe", "payload": {"documentId": document_id}}),
        &mut sessions,
    );
    assert_eq!(saved["ok"], true, "saveRecipe should succeed: {saved}");
    assert_eq!(
        fs::read_to_string(workspace.recipe_path("demo.app.install.yaml"))
            .expect("saved recipe should read"),
        yaml,
        "saving must write the canonical YAML"
    );

    let (_, reopened) = open_recipe(&workspace, "demo.app.install.yaml");
    assert_eq!(
        reopened["yaml"], yaml,
        "reopening must reproduce the same canonical YAML"
    );
    assert_eq!(
        reopened["recipe"], document["recipe"],
        "reopening must reproduce the same document projection"
    );
}

/// App Target path conditions authored by the Config Editor must round-trip
/// through the document session exactly as written.
///
/// The Config Editor sends `{"type": "path_exists", "params": {"path": {"app_target": ...}}}`
/// shapes for both condition fields. This test proves the session accepts
/// those shapes, keeps them in the projection and the canonical YAML, reports
/// no App Target diagnostics, and never migrates them into another form.
#[test]
fn app_target_conditions_round_trip_without_migration() {
    let workspace = TempWorkspace::copy_fixture("editor_app_catalog");
    let mut sessions = DocumentSessionManager::default();
    let (document_id, document) =
        open_fixture(&mut sessions, &workspace, "demo.target_usage.yaml");
    assert_eq!(step(&document, "copy_payload")["verify"], json!([]));
    assert_eq!(step(&document, "copy_payload")["skipIf"], json!([]));

    let applied = request::handle_sidecar_value(
        json!({
            "id": "verify",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": &document_id,
                "command": {
                    "type": "UpdateStepVerify",
                    "stepId": "copy_payload",
                    "verify": [{"type": "path_exists", "params": {"path": {"app_target": "system"}}}]
                }
            }
        }),
        &mut sessions,
    );
    assert_eq!(applied["ok"], true, "the App Target verify command must succeed: {applied}");
    assert_eq!(
        applied["result"]["commandResult"],
        json!({"changed": true})
    );
    let updated = applied["result"]["document"].clone();
    assert_eq!(
        step(&updated, "copy_payload")["verify"][0]["params"]["path"]["app_target"],
        "system"
    );

    let applied = request::handle_sidecar_value(
        json!({
            "id": "skip_if",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": &document_id,
                "command": {
                    "type": "UpdateStepSkipIf",
                    "stepId": "copy_payload",
                    "skipIf": [{"type": "file_exists", "params": {"path": {"app_target": "config_file"}}}]
                }
            }
        }),
        &mut sessions,
    );
    assert_eq!(applied["ok"], true, "the App Target skip_if command must succeed: {applied}");
    let updated = applied["result"]["document"].clone();
    assert_eq!(
        step(&updated, "copy_payload")["skipIf"][0]["params"]["path"]["app_target"],
        "config_file"
    );
    let codes = diagnostic_codes(&updated);
    assert!(
        codes.iter().all(|code| !code.starts_with("app_target_")),
        "an App Target condition the planner accepts must report no App Target diagnostics: {codes:?}"
    );

    let yaml = updated["yaml"].as_str().expect("yaml should be a string");
    assert!(
        yaml.contains("app_target: system") && yaml.contains("app_target: config_file"),
        "the canonical YAML must keep both App Target conditions: {yaml}"
    );
    let saved = request::handle_sidecar_value(
        json!({"id": "save", "type": "saveRecipe", "payload": {"documentId": &document_id}}),
        &mut sessions,
    );
    assert_eq!(saved["ok"], true, "saveRecipe should succeed: {saved}");

    let (_, reopened) = open_recipe(&workspace, "demo.target_usage.yaml");
    assert_eq!(
        reopened["yaml"], updated["yaml"],
        "reopening must reproduce the same canonical YAML"
    );
    assert_eq!(
        reopened["recipe"], updated["recipe"],
        "reopening must reproduce the same document projection"
    );
}

