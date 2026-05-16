use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::{jsonl, protocol, run_with_args_and_input};
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
            "emuchef-rust-backend-phase6k-{}-{unique}-{sequence}",
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

fn validate_path_request(path: &str) -> Value {
    json!({
        "type": "validateRecipePath",
        "payload": {"path": path, "authoredRoot": null}
    })
}

fn open_request(id: &str, path: &str) -> Value {
    json!({
        "id": id,
        "type": "openRecipe",
        "payload": {"path": path, "authoredRoot": null}
    })
}

fn document_request(id: &str, request_type: &str) -> Value {
    json!({
        "id": id,
        "type": request_type,
        "payload": {"documentId": "doc-1"}
    })
}

fn command_request(id: &str, command: Value) -> Value {
    json!({
        "id": id,
        "type": "applyRecipeCommand",
        "payload": {"documentId": "doc-1", "command": command}
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

fn diagnostics_for_validate_path(fixture: &str) -> Value {
    let response = one_shot_response(validate_path_request(&fixture_path(fixture)));
    assert_eq!(response["ok"], true, "{fixture}");
    response["result"]["diagnostics"].clone()
}

fn diagnostics_for_open_and_validate(fixture: &str) -> (Value, Value) {
    let path = fixture_path(fixture);
    let responses = sidecar_responses(&jsonl_input(vec![
        open_request("open", &path),
        document_request("validate", "validate"),
    ]));
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["ok"], true, "{fixture} open");
    assert_eq!(responses[1]["ok"], true, "{fixture} validate");
    (
        responses[0]["result"]["document"]["diagnostics"].clone(),
        responses[1]["result"]["diagnostics"].clone(),
    )
}

fn assert_python_diagnostic_shape(diagnostic: &Value) {
    let object = diagnostic.as_object().expect("diagnostic should be object");
    let keys = object.keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            "severity",
            "code",
            "message",
            "file",
            "objectKind",
            "objectId",
            "field",
        ]
    );
}

fn assert_limited_context_warning(diagnostics: &Value, object_id: Option<&str>) {
    let warning = &diagnostics.as_array().unwrap()[0];
    assert_python_diagnostic_shape(warning);
    assert_eq!(warning["severity"], "warning");
    assert_eq!(warning["code"], "validation_context_limited");
    assert_eq!(
        warning["objectKind"],
        object_id.map_or(Value::Null, |_| json!("recipe"))
    );
    assert_eq!(
        warning["objectId"],
        object_id.map_or(Value::Null, |id| json!(id))
    );
    assert_eq!(warning["field"], Value::Null);
}

fn assert_error_diagnostic(diagnostics: &Value, code: &str, object_id: &str, field: Option<&str>) {
    let error = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == code)
        .unwrap_or_else(|| panic!("expected diagnostic code {code} in {diagnostics:#?}"));
    assert_python_diagnostic_shape(error);
    assert_eq!(error["severity"], "error");
    assert_eq!(error["objectKind"], "recipe");
    assert_eq!(error["objectId"], object_id);
    assert_eq!(
        error["field"],
        field.map_or(Value::Null, |field| json!(field))
    );
}

fn assert_editor_local_fixture_diagnostics(
    fixture: &str,
    code: &str,
    object_id: &str,
    field: Option<&str>,
) {
    let path_diagnostics = diagnostics_for_validate_path(fixture);
    let (open_diagnostics, session_diagnostics) = diagnostics_for_open_and_validate(fixture);

    // Python ValidatorService serializes warnings before errors. These fixtures
    // intentionally omit authoredRoot to stay editor-local and avoid catalog scans.
    for diagnostics in [&path_diagnostics, &open_diagnostics, &session_diagnostics] {
        assert_limited_context_warning(diagnostics, Some(object_id));
        assert_error_diagnostic(diagnostics, code, object_id, field);
    }
}

#[test]
fn phase6k_capabilities_stay_on_editor_session_surface() {
    assert_eq!(
        protocol::CAPABILITIES,
        &[
            "listStepSpecs",
            "emitRecipeYamlFromPath",
            "validateRecipePath",
            "openRecipe",
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
        ]
    );
}

#[test]
fn valid_minimal_and_top_level_permissions_keep_python_request_classification() {
    let valid = diagnostics_for_validate_path("minimal_recipe.yaml");
    assert_eq!(valid.as_array().unwrap().len(), 1);
    assert_limited_context_warning(&valid, Some("phase6e.minimal"));

    let permissions = diagnostics_for_validate_path("invalid_top_level_permissions.yaml");
    assert_limited_context_warning(&permissions, None);
    assert_error_diagnostic(
        &permissions,
        "authored_data_invalid",
        "phase6e.invalid_permissions",
        Some("permissions"),
    );

    let open_permissions = sidecar_responses(&jsonl_input(vec![open_request(
        "open",
        &fixture_path("invalid_top_level_permissions.yaml"),
    )]));
    assert_eq!(open_permissions.len(), 1);
    assert_eq!(open_permissions[0]["ok"], false);
    assert_eq!(open_permissions[0]["error"]["code"], "load_failed");
}

#[test]
fn editor_local_step_dependency_diagnostics_match_python_fields() {
    // Mirrors src/emuchef/io/validation.py _annotate_recipe_step_cycle_errors.
    assert_editor_local_fixture_diagnostics(
        "phase6k_missing_step_dependency.yaml",
        "step_not_found",
        "phase6k.missing_dependency",
        Some("steps[0].dependencies[0]"),
    );
    assert_editor_local_fixture_diagnostics(
        "phase6k_self_dependency.yaml",
        "dependency_cycle",
        "phase6k.self_dependency",
        Some("steps"),
    );
}

#[test]
fn stepspec_required_param_diagnostics_match_python_fields() {
    assert_editor_local_fixture_diagnostics(
        "phase6k_missing_required_param.yaml",
        "param_contract_violation",
        "phase6k.missing_required_param",
        Some("steps[0].params.duration_ms"),
    );
}

#[test]
fn top_level_param_refs_validate_only_authored_param_refs() {
    assert_editor_local_fixture_diagnostics(
        "phase6k_unknown_input_ref.yaml",
        "unknown_input_ref",
        "phase6k.unknown_input_ref",
        Some("steps[0].params.app"),
    );
    assert_editor_local_fixture_diagnostics(
        "phase6k_unknown_artifact_ref.yaml",
        "unknown_artifact_ref",
        "phase6k.unknown_artifact_ref",
        Some("steps[0].params.app"),
    );
    assert_editor_local_fixture_diagnostics(
        "phase6k_unknown_artifact_field.yaml",
        "unknown_artifact_field",
        "phase6k.unknown_artifact_field",
        Some("steps[0].params.app"),
    );
    assert_editor_local_fixture_diagnostics(
        "phase6k_unknown_step_ref.yaml",
        "unknown_step_ref",
        "phase6k.unknown_step_ref",
        Some("steps[0].params.source"),
    );
    assert_editor_local_fixture_diagnostics(
        "phase6k_unknown_step_output.yaml",
        "unknown_step_output",
        "phase6k.unknown_step_output",
        Some("steps[1].params.source"),
    );

    let nested = diagnostics_for_validate_path("phase6k_nested_ref_literal.yaml");
    assert_eq!(nested.as_array().unwrap().len(), 1);
    assert_limited_context_warning(&nested, Some("phase6k.nested_ref_literal"));
}

#[test]
fn stepspec_ref_mode_and_invalid_ref_format_diagnostics_are_covered() {
    assert_editor_local_fixture_diagnostics(
        "phase6k_ref_mode_violation.yaml",
        "param_contract_violation",
        "phase6k.ref_mode_violation",
        Some("steps[0].params.source"),
    );

    let temp_recipe = TempRecipe::copy_fixture("phase6i_commands.yaml");
    let path = temp_recipe.path.to_string_lossy().into_owned();
    let responses = sidecar_responses(&jsonl_input(vec![
        open_request("open", path.as_str()),
        command_request(
            "bad-ref-format",
            json!({
                "type": "UpdateStepParams",
                "stepId": "copy_input",
                "params": {
                    "source": {"ref": "not_a_runtime_ref"},
                    "dest": "/sdcard/Input"
                }
            }),
        ),
        document_request("validate", "validate"),
    ]));

    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(responses[1]["ok"], true);
    let diagnostics = &responses[2]["result"]["diagnostics"];
    assert_error_diagnostic(
        diagnostics,
        "invalid_ref_format",
        "phase6i.commands",
        Some("steps[2].params.source"),
    );
}

#[test]
fn invalid_step_internals_keep_load_error_classification() {
    for fixture in [
        "phase6k_invalid_constraints.yaml",
        "phase6k_invalid_skip_if.yaml",
        "phase6k_invalid_verify.yaml",
    ] {
        let diagnostics = diagnostics_for_validate_path(fixture);
        assert_limited_context_warning(&diagnostics, None);
        let error = diagnostics
            .as_array()
            .unwrap()
            .iter()
            .find(|diagnostic| diagnostic["code"] == "authored_data_invalid")
            .unwrap_or_else(|| panic!("expected authored_data_invalid in {fixture}"));
        assert_python_diagnostic_shape(error);
        assert_eq!(error["severity"], "error");

        let open = sidecar_responses(&jsonl_input(vec![open_request(
            "open",
            &fixture_path(fixture),
        )]));
        assert_eq!(open.len(), 1, "{fixture}");
        assert_eq!(open[0]["ok"], false, "{fixture}");
        assert_eq!(open[0]["error"]["code"], "load_failed", "{fixture}");
    }
}

#[test]
fn diagnostics_refresh_after_commands_undo_redo_and_save() {
    let temp_recipe = TempRecipe::copy_fixture("minimal_recipe.yaml");
    let path = temp_recipe.path.to_string_lossy().into_owned();
    let responses = sidecar_responses(&jsonl_input(vec![
        open_request("open", path.as_str()),
        command_request(
            "add-invalid-wait",
            json!({
                "type": "AddStep",
                "stepId": "wait",
                "stepType": "wait",
                "name": "Wait"
            }),
        ),
        document_request("validate-after-command", "validate"),
        document_request("undo", "undo"),
        document_request("redo", "redo"),
        document_request("save", "saveRecipe"),
    ]));

    assert_eq!(responses.len(), 6);
    assert_eq!(responses[0]["ok"], true);
    let opened = &responses[0]["result"]["document"];
    assert_eq!(opened["dirty"], false);
    assert_eq!(opened["diagnostics"].as_array().unwrap().len(), 1);
    assert_limited_context_warning(&opened["diagnostics"], Some("phase6e.minimal"));

    let changed = &responses[1]["result"]["document"];
    assert_eq!(
        responses[1]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(changed["dirty"], true);
    assert_error_diagnostic(
        &changed["diagnostics"],
        "param_contract_violation",
        "phase6e.minimal",
        Some("steps[0].params.duration_ms"),
    );
    assert_eq!(
        responses[2]["result"]["diagnostics"],
        changed["diagnostics"]
    );

    let undone = &responses[3]["result"]["document"];
    assert_eq!(
        responses[3]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(undone["dirty"], false);
    assert_eq!(undone["yaml"], opened["yaml"]);
    assert_eq!(undone["diagnostics"].as_array().unwrap().len(), 1);
    assert_limited_context_warning(&undone["diagnostics"], Some("phase6e.minimal"));

    let redone = &responses[4]["result"]["document"];
    assert_eq!(
        responses[4]["result"]["commandResult"],
        json!({"changed": true})
    );
    assert_eq!(redone["dirty"], true);
    assert_error_diagnostic(
        &redone["diagnostics"],
        "param_contract_violation",
        "phase6e.minimal",
        Some("steps[0].params.duration_ms"),
    );

    let saved = &responses[5]["result"]["document"];
    assert_eq!(saved["dirty"], false);
    assert_eq!(saved["yaml"], redone["yaml"]);
    assert_error_diagnostic(
        &saved["diagnostics"],
        "param_contract_violation",
        "phase6e.minimal",
        Some("steps[0].params.duration_ms"),
    );
}
