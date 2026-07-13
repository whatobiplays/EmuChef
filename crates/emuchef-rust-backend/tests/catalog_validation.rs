use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::{
    jsonl, protocol, request, run_with_args_and_input, session::DocumentSessionManager,
};
use serde_json::{json, Value};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("authored_root")
}

fn backend_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn plain_recipe_path(name: &str) -> PathBuf {
    backend_fixture_root().join("recipes").join(name)
}

fn workspace_root(name: &str) -> PathBuf {
    fixture_root().join(name)
}

fn authored_root(name: &str) -> PathBuf {
    workspace_root(name).join("authored")
}

fn recipe_path(workspace: &str, name: &str) -> PathBuf {
    authored_root(workspace).join("recipes").join(name)
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("compatibility_goldens_v1")
        .join(name)
}

fn read_diagnostic_golden(name: &str) -> Vec<Value> {
    let text = fs::read_to_string(golden_path(name))
        .expect("Compatibility diagnostic fixture should exist");
    serde_json::from_str(&text).expect("Compatibility diagnostic fixture should be valid JSON")
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

fn sidecar_responses(requests: Vec<Value>) -> Vec<Value> {
    let input = format!(
        "{}\n",
        requests
            .into_iter()
            .map(|request| request.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
    jsonl::process_jsonl(&input)
        .lines()
        .map(|line| serde_json::from_str(line).expect("sidecar response should be valid JSON"))
        .collect()
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
            "emuchef-rust-backend-phase6l-{}-{unique}-{sequence}",
            std::process::id()
        ));
        copy_dir_all(&workspace_root(name), &dir);
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
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_all(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).expect("fixture file should copy");
        }
    }
}

fn validate_path(path: &Path, authored_root: Option<&Path>) -> Value {
    let mut payload = json!({ "path": path });
    if let Some(root) = authored_root {
        payload["authoredRoot"] = json!(root);
    }
    one_shot_response(json!({
        "type": "validateRecipePath",
        "payload": payload
    }))
}

fn validate_path_with_null_root(path: &Path) -> Value {
    one_shot_response(json!({
        "type": "validateRecipePath",
        "payload": {"path": path, "authoredRoot": null}
    }))
}

fn diagnostic_fields(diagnostic: &Value) -> Value {
    json!({
        "severity": diagnostic["severity"],
        "code": diagnostic["code"],
        "objectKind": diagnostic["objectKind"],
        "objectId": diagnostic["objectId"],
        "field": diagnostic["field"],
    })
}

fn diagnostic_set(response: &Value) -> Vec<Value> {
    response["result"]["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .map(diagnostic_fields)
        .collect()
}

fn document_diagnostic_set(response: &Value) -> Vec<Value> {
    response["result"]["document"]["diagnostics"]
        .as_array()
        .expect("document diagnostics should be an array")
        .iter()
        .map(diagnostic_fields)
        .collect()
}

fn normalized_path(path: &Path) -> Value {
    json!(path.canonicalize().unwrap().to_string_lossy().to_string())
}

#[test]
fn capabilities_include_catalog_context_session_update() {
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
fn validate_recipe_path_uses_explicit_authored_root_but_does_not_infer_it() {
    let path = recipe_path("complete", "main.yaml");
    let inferred = validate_path(&path, None);
    assert_eq!(inferred["ok"], true);
    let inferred_diagnostics = diagnostic_set(&inferred);
    assert_eq!(
        inferred_diagnostics,
        read_diagnostic_golden("phase6l_complete_null_root.diagnostics.json")
    );

    let null_root = validate_path_with_null_root(&path);
    assert_eq!(null_root["ok"], true);
    let null_diagnostics = diagnostic_set(&null_root);
    assert_eq!(
        null_diagnostics,
        read_diagnostic_golden("phase6l_complete_null_root.diagnostics.json")
    );

    let explicit = validate_path(&path, Some(&authored_root("complete")));
    assert_eq!(explicit["ok"], true);
    assert_eq!(
        diagnostic_set(&explicit),
        read_diagnostic_golden("phase6l_complete_explicit_root.diagnostics.json")
    );
}

#[test]
fn nonexistent_non_null_authored_root_is_empty_catalog_context_not_request_failure() {
    let path = recipe_path("complete", "main.yaml");
    let missing_root = workspace_root("complete").join("missing-authored-root");
    let response = validate_path(&path, Some(&missing_root));

    assert_eq!(response["ok"], true);
    let diagnostics = diagnostic_set(&response);
    assert_eq!(
        diagnostics,
        read_diagnostic_golden("phase6l_missing_authored_root.diagnostics.json")
    );
}

#[test]
fn valid_authored_root_reports_recipe_dependency_diagnostics() {
    let response = validate_path(
        &recipe_path("missing_dependency", "missing_dependency.yaml"),
        Some(&authored_root("missing_dependency")),
    );

    assert_eq!(response["ok"], true);
    let diagnostics = diagnostic_set(&response);
    assert_eq!(
        diagnostics,
        read_diagnostic_golden("phase6l_missing_dependency.diagnostics.json")
    );
}

#[test]
fn recipe_dependency_cycles_use_validation_local_graph_checks() {
    let response = validate_path(
        &recipe_path("dependency_cycle", "cycle_a.yaml"),
        Some(&authored_root("dependency_cycle")),
    );

    assert_eq!(response["ok"], true);
    let diagnostics = diagnostic_set(&response);
    assert_eq!(
        diagnostics,
        read_diagnostic_golden("phase6l_dependency_cycle.diagnostics.json")
    );
}

#[test]
fn catalog_scan_uses_only_compatibility_verified_top_level_globs() {
    // Catalog validation scans only these top-level authored globs:
    // apps/*.y*ml, recipes/*.y*ml, device_profiles/*.y*ml, device_plans/*.y*ml.
    // This nested recipe is intentionally ignored, so the dependency is missing.
    let response = validate_path(
        &recipe_path("nested_ignored", "main.yaml"),
        Some(&authored_root("nested_ignored")),
    );

    assert_eq!(response["ok"], true);
    let diagnostics = diagnostic_set(&response);
    assert_eq!(
        diagnostics,
        read_diagnostic_golden("phase6l_nested_ignored.diagnostics.json")
    );
}

#[test]
fn open_recipe_infers_and_normalizes_authored_root_like_compatibility() {
    let inferred_path = recipe_path("complete", "main.yaml");
    let repo_root = workspace_root("complete");
    let responses = sidecar_responses(vec![
        json!({
            "id": "open-inferred",
            "type": "openRecipe",
            "payload": {"path": inferred_path}
        }),
        json!({
            "id": "open-null-inferred",
            "type": "openRecipe",
            "payload": {"path": inferred_path, "authoredRoot": null}
        }),
        json!({
            "id": "open-normalized",
            "type": "openRecipe",
            "payload": {"path": inferred_path, "authoredRoot": repo_root}
        }),
    ]);

    assert_eq!(responses.len(), 3);
    for response in &responses {
        assert_eq!(response["ok"], true);
        assert_eq!(
            response["result"]["document"]["authoredRoot"],
            authored_root("complete")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        );
        assert_eq!(
            document_diagnostic_set(response),
            read_diagnostic_golden("phase6l_complete_explicit_root.diagnostics.json")
        );
    }
}

#[test]
fn set_document_authored_root_updates_null_session_context_without_reopening() {
    let responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": plain_recipe_path("minimal_recipe.yaml"), "authoredRoot": null}
        }),
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": backend_fixture_root()}
        }),
    ]);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(
        responses[0]["result"]["document"]["authoredRoot"],
        Value::Null
    );
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(
        responses[1]["result"]["document"]["authoredRoot"],
        normalized_path(&backend_fixture_root())
    );
    assert_eq!(document_diagnostic_set(&responses[1]), Vec::<Value>::new());
    assert_eq!(
        responses[1]["result"]["document"]["refIndex"]["allRefs"],
        json!([])
    );
}

#[test]
fn set_document_authored_root_switches_catalog_context() {
    let responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {
                "path": recipe_path("complete", "main.yaml"),
                "authoredRoot": authored_root("complete")
            }
        }),
        json!({
            "id": "switch-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {
                "documentId": "doc-1",
                "authoredRoot": authored_root("missing_dependency")
            }
        }),
    ]);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(
        responses[1]["result"]["document"]["authoredRoot"],
        normalized_path(&authored_root("missing_dependency"))
    );
    assert_eq!(
        document_diagnostic_set(&responses[1]),
        vec![json!({
            "severity": "error",
            "code": "recipe_not_found",
            "objectKind": "recipe",
            "objectId": "phase6l.main",
            "field": "recipe_dependencies[0]",
        })]
    );
}

#[test]
fn set_document_authored_root_null_clears_context_without_inference() {
    let responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {
                "path": recipe_path("complete", "main.yaml"),
                "authoredRoot": authored_root("complete")
            }
        }),
        json!({
            "id": "clear-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": null}
        }),
    ]);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(
        responses[1]["result"]["document"]["authoredRoot"],
        Value::Null
    );
    assert_eq!(
        document_diagnostic_set(&responses[1]),
        read_diagnostic_golden("phase6l_complete_null_root.diagnostics.json")
    );
}

#[test]
fn set_document_authored_root_preserves_clean_dirty_and_history_state() {
    let clean_responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": recipe_path("complete", "main.yaml"), "authoredRoot": null}
        }),
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": authored_root("complete")}
        }),
    ]);
    let clean_document = &clean_responses[1]["result"]["document"];
    assert_eq!(clean_document["dirty"], false);
    assert_eq!(clean_document["canUndo"], false);
    assert_eq!(clean_document["canRedo"], false);

    let dirty_responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": recipe_path("complete", "main.yaml"), "authoredRoot": null}
        }),
        json!({
            "id": "edit",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": "doc-1",
                "command": {"type": "SetOverviewField", "field": "name", "value": "Unsaved Name"}
            }
        }),
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": authored_root("complete")}
        }),
    ]);
    let dirty_document = &dirty_responses[2]["result"]["document"];
    assert_eq!(dirty_document["dirty"], true);
    assert_eq!(dirty_document["canUndo"], true);
    assert_eq!(dirty_document["canRedo"], false);
    assert_eq!(dirty_document["recipe"]["name"], "Unsaved Name");
}

#[test]
fn set_document_authored_root_preserves_in_memory_content_when_disk_changes() {
    let workspace = TempWorkspace::copy_fixture("complete");
    let recipe_path = workspace.recipe_path("main.yaml");
    let original_disk_yaml =
        fs::read_to_string(&recipe_path).expect("copied recipe should be readable");
    let mut sessions = DocumentSessionManager::default();

    let opened = request::handle_sidecar_value(
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": recipe_path, "authoredRoot": null}
        }),
        &mut sessions,
    );
    assert_eq!(opened["ok"], true);

    let edited = request::handle_sidecar_value(
        json!({
            "id": "edit",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": "doc-1",
                "command": {"type": "AddInput", "inputId": "unsaved_input"}
            }
        }),
        &mut sessions,
    );
    assert_eq!(edited["ok"], true);

    fs::write(
        &recipe_path,
        original_disk_yaml.replace("Phase 6L Main", "Disk Reloaded Name"),
    )
    .expect("backing file should be mutable");

    let response = request::handle_sidecar_value(
        json!({
            "id": "set-root",
            "type": "setDocumentAuthoredRoot",
            "payload": {"documentId": "doc-1", "authoredRoot": workspace.authored_root()}
        }),
        &mut sessions,
    );

    assert_eq!(response["ok"], true);
    let document = &response["result"]["document"];
    assert_eq!(document["dirty"], true);
    assert_eq!(document["recipe"]["name"], "Phase 6L Main");
    assert!(document["yaml"]
        .as_str()
        .unwrap()
        .contains("unsaved_input:"));
    assert!(!document["yaml"]
        .as_str()
        .unwrap()
        .contains("Disk Reloaded Name"));
    assert!(document["refIndex"]["inputRefs"]
        .as_array()
        .unwrap()
        .contains(&json!("inputs.unsaved_input")));
}

#[test]
fn open_recipe_reports_duplicate_recipe_id_conflict_against_catalog() {
    let responses = sidecar_responses(vec![json!({
        "id": "open",
        "type": "openRecipe",
        "payload": {
            "path": recipe_path("duplicate", "target_duplicate.yaml"),
            "authoredRoot": authored_root("duplicate")
        }
    })]);

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(
        document_diagnostic_set(&responses[0]),
        read_diagnostic_golden("phase6l_duplicate_open.diagnostics.json")
    );
}

#[test]
fn duplicate_recipe_id_conflict_matches_compatibility_first_file_replacement_semantics() {
    let responses = sidecar_responses(vec![json!({
        "id": "open",
        "type": "openRecipe",
        "payload": {
            "path": recipe_path("duplicate_reverse", "a_target_duplicate.yaml"),
            "authoredRoot": authored_root("duplicate_reverse")
        }
    })]);

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(
        document_diagnostic_set(&responses[0]),
        read_diagnostic_golden("phase6l_duplicate_reverse_open.diagnostics.json")
    );
}

#[test]
fn commands_undo_redo_save_and_session_validate_reuse_stored_authored_root() {
    let workspace = TempWorkspace::copy_fixture("missing_dependency");
    let path = workspace.recipe_path("missing_dependency.yaml");
    let responses = sidecar_responses(vec![
        json!({
            "id": "open",
            "type": "openRecipe",
            "payload": {"path": path}
        }),
        json!({
            "id": "command",
            "type": "applyRecipeCommand",
            "payload": {
                "documentId": "doc-1",
                "command": {"type": "SetOverviewField", "field": "name", "value": "Renamed Missing Dependency"}
            }
        }),
        json!({
            "id": "validate-after-command",
            "type": "validate",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "undo",
            "type": "undo",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "redo",
            "type": "redo",
            "payload": {"documentId": "doc-1"}
        }),
        json!({
            "id": "save",
            "type": "saveRecipe",
            "payload": {"documentId": "doc-1"}
        }),
    ]);

    assert_eq!(responses.len(), 6);
    for response in &responses {
        assert_eq!(response["ok"], true, "{response:#?}");
        let diagnostics = if response["result"].get("document").is_some() {
            document_diagnostic_set(response)
        } else {
            diagnostic_set(response)
        };
        assert_eq!(
            diagnostics,
            read_diagnostic_golden("phase6l_missing_dependency_open.diagnostics.json")
        );
    }

    let expected_root = workspace.authored_root().canonicalize().unwrap();
    assert_eq!(
        responses[0]["result"]["document"]["authoredRoot"],
        expected_root.to_string_lossy().to_string()
    );
    for response in [&responses[1], &responses[3], &responses[4], &responses[5]] {
        assert_eq!(
            response["result"]["document"]["authoredRoot"],
            expected_root.to_string_lossy().to_string()
        );
    }
}
