use std::fs;
use std::path::{Path, PathBuf};

use emuchef_rust_backend::{request, session::DocumentSessionManager};
use serde_json::{json, Value};

const EXPECTED_AUTHORED_RECIPES: &[&str] = &[
    "app.obtainium.install.yaml",
    "app.retroarch.provision.yaml",
    "app.xaniteog.install.yaml",
    "feature.copy_bios.yaml",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should resolve from crate manifest dir")
}

fn repo_authored_root() -> PathBuf {
    repo_root().join("authored")
}

fn repo_authored_recipes_dir() -> PathBuf {
    repo_authored_root().join("recipes")
}

fn authored_recipe_names() -> Vec<String> {
    let mut names = fs::read_dir(repo_authored_recipes_dir())
        .expect("repo authored recipes directory should be readable")
        .map(|entry| {
            entry
                .expect("authored recipe entry should be readable")
                .path()
        })
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| matches!(extension, "yaml" | "yml"))
        })
        .map(|path| {
            path.file_name()
                .expect("authored recipe path should have a file name")
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn recipe_id_from_name(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .expect("recipe file name should have a stem")
        .to_string_lossy()
        .to_string()
}

fn sidecar_response(
    sessions: &mut DocumentSessionManager,
    id: &str,
    request_type: &str,
    payload: Value,
) -> Value {
    request::handle_sidecar_value(
        json!({
            "id": id,
            "type": request_type,
            "payload": payload,
        }),
        sessions,
    )
}

fn assert_success(response: &Value, id: &str) {
    assert_eq!(response["id"], id);
    assert_eq!(response["ok"], true, "{id}: {response}");
    assert!(
        response["result"].is_object(),
        "{id}: success response should include a result object"
    );
}

fn non_empty_string<'a>(value: &'a Value, label: &str) -> &'a str {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{label} should be a string"));
    assert!(!text.is_empty(), "{label} should not be empty");
    text
}

fn assert_ref_index_shape(ref_index: &Value, label: &str) {
    for field in [
        "allRefs",
        "inputRefs",
        "artifactRefs",
        "stepRefs",
        "stepOutputRefs",
        "candidates",
    ] {
        assert!(
            ref_index[field].as_array().is_some(),
            "{label}.{field} should be an array"
        );
    }
}

fn assert_document_shape(
    document: &Value,
    expected_recipe_id: &str,
    expected_authored_root: Option<&Path>,
    label: &str,
) -> String {
    let document_id = non_empty_string(&document["documentId"], &format!("{label}.documentId"));
    non_empty_string(&document["path"], &format!("{label}.path"));
    assert_eq!(
        document["recipe"]["id"], expected_recipe_id,
        "{label}.recipe.id"
    );
    non_empty_string(&document["recipe"]["name"], &format!("{label}.recipe.name"));
    non_empty_string(&document["yaml"], &format!("{label}.yaml"));
    assert!(
        document["diagnostics"].as_array().is_some(),
        "{label}.diagnostics should be an array"
    );
    assert_ref_index_shape(&document["refIndex"], &format!("{label}.refIndex"));

    if let Some(authored_root) = expected_authored_root {
        assert_eq!(
            document["authoredRoot"],
            authored_root
                .canonicalize()
                .expect("explicit authored root should canonicalize")
                .to_string_lossy()
                .to_string(),
            "{label}.authoredRoot"
        );
    }

    document_id.to_string()
}

fn run_editor_session_flow(recipe_name: &str, authored_root: Value) {
    let recipe_path = repo_authored_recipes_dir().join(recipe_name);
    let expected_recipe_id = recipe_id_from_name(recipe_name);
    let expected_authored_root = if authored_root.is_null() {
        None
    } else {
        Some(repo_authored_root())
    };
    let mut sessions = DocumentSessionManager::default();

    let open = sidecar_response(
        &mut sessions,
        "open",
        "openRecipe",
        json!({"path": recipe_path, "authoredRoot": authored_root}),
    );
    assert_success(&open, "open");
    let document_id = assert_document_shape(
        &open["result"]["document"],
        &expected_recipe_id,
        expected_authored_root.as_deref(),
        "open.document",
    );

    let get_document = sidecar_response(
        &mut sessions,
        "get-document",
        "getDocument",
        json!({"documentId": document_id}),
    );
    assert_success(&get_document, "get-document");
    let document_id = assert_document_shape(
        &get_document["result"]["document"],
        &expected_recipe_id,
        expected_authored_root.as_deref(),
        "getDocument.document",
    );

    let validate = sidecar_response(
        &mut sessions,
        "validate",
        "validate",
        json!({"documentId": document_id}),
    );
    assert_success(&validate, "validate");
    assert!(
        validate["result"]["diagnostics"].as_array().is_some(),
        "validate.diagnostics should be an array"
    );

    let emit_yaml = sidecar_response(
        &mut sessions,
        "emit-yaml",
        "emitYaml",
        json!({"documentId": document_id}),
    );
    assert_success(&emit_yaml, "emit-yaml");
    non_empty_string(&emit_yaml["result"]["yaml"], "emitYaml.yaml");

    let get_ref_index = sidecar_response(
        &mut sessions,
        "get-ref-index",
        "getRefIndex",
        json!({"documentId": document_id}),
    );
    assert_success(&get_ref_index, "get-ref-index");
    assert_ref_index_shape(&get_ref_index["result"]["refIndex"], "getRefIndex.refIndex");
}

#[test]
fn real_authored_recipe_corpus_is_explicit() {
    assert_eq!(authored_recipe_names(), EXPECTED_AUTHORED_RECIPES);
}

#[test]
fn real_authored_recipes_open_validate_emit_and_index_through_editor_sessions() {
    // This is Rust editor/session smoke coverage for the real authored corpus.
    // It intentionally does not exercise planner, executor, apply, device,
    // network or Tauri GUI behavior.
    for recipe_name in authored_recipe_names() {
        run_editor_session_flow(&recipe_name, Value::Null);
        run_editor_session_flow(&recipe_name, json!(repo_authored_root()));
    }
}
