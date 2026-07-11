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
        static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "emuchef-rust-backend-phase6h-{}-{unique}-{sequence}",
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

fn command_request(id: &str, value: &str) -> Value {
    json!({
        "id": id,
        "type": "applyRecipeCommand",
        "payload": {
            "documentId": "doc-1",
            "command": {"type": "SetOverviewField", "field": "name", "value": value}
        }
    })
}

fn expected_representative_ref_index() -> Value {
    json!({
        "inputRefs": ["inputs.bios_source_dir"],
        "artifactRefs": [],
        "stepRefs": ["steps.copy_bios_dir"],
        "stepOutputRefs": ["steps.copy_bios_dir.outputs.copied_paths"],
        "allRefs": [
            "inputs.bios_source_dir",
            "steps.copy_bios_dir",
            "steps.copy_bios_dir.outputs.copied_paths"
        ],
        "candidates": [
            {
                "ref": "inputs.bios_source_dir",
                "label": "Input \u{00b7} bios_source_dir",
                "valueType": "directory_path",
                "sourceKind": "input",
                "sourceId": "bios_source_dir"
            },
            {
                "ref": "steps.copy_bios_dir.outputs.copied_paths",
                "label": "Step Output \u{00b7} copy_bios_dir.copied_paths",
                "valueType": "path_list",
                "sourceKind": "step_output",
                "sourceId": "copy_bios_dir"
            }
        ]
    })
}

fn expected_ref_params_ref_index() -> Value {
    json!({
        "inputRefs": [],
        "artifactRefs": [
            "artifacts.assets_zip.cache_hit",
            "artifacts.assets_zip.error",
            "artifacts.assets_zip.filename",
            "artifacts.assets_zip.local_path",
            "artifacts.assets_zip.resolved_url",
            "artifacts.assets_zip.status"
        ],
        "stepRefs": ["steps.extract_assets", "steps.copy_assets"],
        "stepOutputRefs": [
            "steps.extract_assets.outputs.extracted_paths",
            "steps.copy_assets.outputs.copied_paths"
        ],
        "allRefs": [
            "artifacts.assets_zip.cache_hit",
            "artifacts.assets_zip.error",
            "artifacts.assets_zip.filename",
            "artifacts.assets_zip.local_path",
            "artifacts.assets_zip.resolved_url",
            "artifacts.assets_zip.status",
            "steps.extract_assets",
            "steps.copy_assets",
            "steps.extract_assets.outputs.extracted_paths",
            "steps.copy_assets.outputs.copied_paths"
        ],
        "candidates": [
            {
                "ref": "artifacts.assets_zip.cache_hit",
                "label": "Artifact \u{00b7} assets_zip.cache_hit",
                "valueType": "boolean",
                "sourceKind": "artifact",
                "sourceId": "assets_zip"
            },
            {
                "ref": "artifacts.assets_zip.error",
                "label": "Artifact \u{00b7} assets_zip.error",
                "valueType": "string",
                "sourceKind": "artifact",
                "sourceId": "assets_zip"
            },
            {
                "ref": "artifacts.assets_zip.filename",
                "label": "Artifact \u{00b7} assets_zip.filename",
                "valueType": "string",
                "sourceKind": "artifact",
                "sourceId": "assets_zip"
            },
            {
                "ref": "artifacts.assets_zip.local_path",
                "label": "Artifact \u{00b7} assets_zip.local_path",
                "valueType": "file_path",
                "sourceKind": "artifact",
                "sourceId": "assets_zip"
            },
            {
                "ref": "artifacts.assets_zip.resolved_url",
                "label": "Artifact \u{00b7} assets_zip.resolved_url",
                "valueType": "string",
                "sourceKind": "artifact",
                "sourceId": "assets_zip"
            },
            {
                "ref": "artifacts.assets_zip.status",
                "label": "Artifact \u{00b7} assets_zip.status",
                "valueType": "string",
                "sourceKind": "artifact",
                "sourceId": "assets_zip"
            },
            {
                "ref": "steps.extract_assets.outputs.extracted_paths",
                "label": "Step Output \u{00b7} extract_assets.extracted_paths",
                "valueType": "path_list",
                "sourceKind": "step_output",
                "sourceId": "extract_assets"
            },
            {
                "ref": "steps.copy_assets.outputs.copied_paths",
                "label": "Step Output \u{00b7} copy_assets.copied_paths",
                "valueType": "path_list",
                "sourceKind": "step_output",
                "sourceId": "copy_assets"
            }
        ]
    })
}

fn assert_invalid_request(response: &Value) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "invalid_request");
}

fn assert_unknown_document(response: &Value, document_id: &str) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "unknown_document");
    assert_eq!(
        response["error"]["details"],
        json!({ "documentId": document_id })
    );
}

#[test]
fn one_shot_get_ref_index_is_not_exposed() {
    let response = one_shot_response(json!({
        "type": "getRefIndex",
        "payload": {"documentId": "doc-1"}
    }));

    assert_invalid_request(&response);
}

#[test]
fn get_ref_index_returns_compatibility_shaped_result_for_open_document() {
    let input = format!(
        "{}\n{}\n",
        open_request(fixture_path("representative_recipe.yaml")),
        json!({
            "id": "get-ref-index",
            "type": "getRefIndex",
            "payload": {"documentId": "doc-1", "ignored": true}
        }),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(
        responses[0]["result"]["document"]["refIndex"],
        expected_representative_ref_index()
    );
    assert_eq!(responses[1]["id"], "get-ref-index");
    assert_eq!(responses[1]["ok"], true);
    assert_eq!(
        responses[1]["result"],
        json!({"refIndex": expected_representative_ref_index()})
    );
}

#[test]
fn ref_index_sources_match_compatibility_for_artifacts_steps_and_outputs() {
    let response = sidecar_response(open_request(fixture_path("ref_params.yaml")));

    assert_eq!(response["ok"], true);
    let ref_index = &response["result"]["document"]["refIndex"];
    assert_eq!(*ref_index, expected_ref_params_ref_index());

    let all_refs = ref_index["allRefs"].as_array().unwrap();
    assert!(!all_refs.contains(&json!("artifact_groups.asset_group")));
    assert!(!all_refs.contains(&json!("provides.features")));
    assert!(!all_refs.contains(&json!("nested.literal.in.params")));
    assert!(!all_refs.contains(&json!("nested.literal.in.condition")));
}

#[test]
fn get_ref_index_validates_payload_and_unknown_documents_like_session_requests() {
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        json!({"id": "missing", "type": "getRefIndex", "payload": {}}),
        json!({"id": "wrong", "type": "getRefIndex", "payload": {"documentId": 123}}),
        json!({"id": "unknown", "type": "getRefIndex", "payload": {"documentId": "missing-document"}}),
        json!({"id": "non-object", "type": "getRefIndex", "payload": []}),
        open_request(fixture_path("minimal_recipe.yaml")),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 5);
    assert_invalid_request(&responses[0]);
    assert_eq!(
        responses[0]["error"]["details"],
        json!({"field": "documentId"})
    );
    assert_invalid_request(&responses[1]);
    assert_eq!(
        responses[1]["error"]["details"],
        json!({"field": "documentId"})
    );
    assert_unknown_document(&responses[2], "missing-document");
    assert_invalid_request(&responses[3]);
    assert_eq!(responses[4]["id"], "open");
    assert_eq!(responses[4]["ok"], true);
}

#[test]
fn document_ref_index_is_current_after_get_save_apply_undo_and_redo() {
    let temp_recipe = TempRecipe::copy_fixture("representative_recipe.yaml");
    let expected_ref_index = expected_representative_ref_index();
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        json!({"id": "get", "type": "getDocument", "payload": {"documentId": "doc-1"}}),
        json!({"id": "save", "type": "saveRecipe", "payload": {"documentId": "doc-1"}}),
        command_request("set-name", "Phase 6H Renamed"),
        json!({"id": "get-ref-after-apply", "type": "getRefIndex", "payload": {"documentId": "doc-1"}}),
        json!({"id": "undo", "type": "undo", "payload": {"documentId": "doc-1"}}),
        json!({"id": "redo", "type": "redo", "payload": {"documentId": "doc-1"}}),
        json!({"id": "validate", "type": "validate", "payload": {"documentId": "doc-1"}}),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 8);
    for index in [0, 1, 2, 3, 5, 6] {
        assert_eq!(responses[index]["ok"], true, "{index}");
        assert_eq!(
            responses[index]["result"]["document"]["refIndex"], expected_ref_index,
            "{index}"
        );
    }
    assert_eq!(
        responses[4]["result"],
        json!({"refIndex": expected_ref_index})
    );
    assert_eq!(responses[7]["ok"], true);
    assert_eq!(
        responses[7]["result"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["diagnostics"]
    );
}

#[test]
fn emit_yaml_result_shape_does_not_include_ref_index() {
    let input = format!(
        "{}\n{}\n",
        open_request(fixture_path("representative_recipe.yaml")),
        json!({"id": "emit", "type": "emitYaml", "payload": {"documentId": "doc-1"}}),
    );

    let responses = sidecar_responses(&input);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[1]["ok"], true);
    assert!(responses[1]["result"]["yaml"].is_string());
    assert!(responses[1]["result"].get("refIndex").is_none());
    assert!(responses[1]["result"].get("document").is_none());
}

#[test]
fn compatibility_results_match_compatibility_goldens_v1() {
    let temp_recipe = TempRecipe::copy_fixture("representative_recipe.yaml");
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        open_request(&temp_recipe.path),
        json!({"id": "get-ref-index", "type": "getRefIndex", "payload": {"documentId": "doc-1"}}),
        command_request("set-name", "Phase 6H Renamed"),
        json!({"id": "undo", "type": "undo", "payload": {"documentId": "doc-1"}}),
        json!({"id": "redo", "type": "redo", "payload": {"documentId": "doc-1"}}),
    );
    let responses = sidecar_responses(&input);
    let expected = [
        ("phase6h_representative_open.result.json", 0),
        ("phase6h_representative_get_ref_index.result.json", 1),
        ("phase6h_representative_set_overview.result.json", 2),
        ("phase6h_representative_undo.result.json", 3),
        ("phase6h_representative_redo.result.json", 4),
    ];

    assert_eq!(responses.len(), 5);
    for (golden, response_index) in expected {
        assert_eq!(responses[response_index]["ok"], true, "{golden}");
        assert_eq!(
            normalize_document_result(&responses[response_index]["result"]),
            read_golden(golden),
            "{golden}"
        );
    }

    let ref_params_input = format!(
        "{}\n{}\n",
        open_request(fixture_path("ref_params.yaml")),
        json!({"id": "get-ref-index", "type": "getRefIndex", "payload": {"documentId": "doc-1"}}),
    );
    let ref_params_responses = sidecar_responses(&ref_params_input);
    assert_eq!(ref_params_responses.len(), 2);
    assert_eq!(ref_params_responses[0]["ok"], true);
    assert_eq!(
        normalize_document_result(&ref_params_responses[1]["result"]),
        read_golden("phase6h_ref_params_get_ref_index.result.json")
    );
}
