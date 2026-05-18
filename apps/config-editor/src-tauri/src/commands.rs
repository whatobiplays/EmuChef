use serde_json::{json, Value};
use tauri::State;

use crate::sidecar_client::SidecarState;

#[tauri::command]
pub fn list_step_specs(state: State<'_, SidecarState>) -> Result<Value, String> {
    stateless_request(&state, "listStepSpecs", None)
}

#[tauri::command]
pub fn open_recipe(
    state: State<'_, SidecarState>,
    path: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    stateless_request(
        &state,
        "openRecipe",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    )
}

#[tauri::command]
pub fn validate_recipe_path(
    state: State<'_, SidecarState>,
    path: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    stateless_request(
        &state,
        "validateRecipePath",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    )
}

#[tauri::command]
pub fn emit_recipe_yaml_from_path(
    state: State<'_, SidecarState>,
    path: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    stateless_request(
        &state,
        "emitRecipeYamlFromPath",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    )
}

fn stateless_request(
    state: &SidecarState,
    request_type: &str,
    payload: Option<Value>,
) -> Result<Value, String> {
    state
        .request(request_type, payload)
        .map(strip_sidecar_transport_id)
}

fn strip_sidecar_transport_id(mut response: Value) -> Value {
    if let Some(object) = response.as_object_mut() {
        object.remove("id");
    }
    response
}

#[tauri::command]
pub fn sidecar_status(state: State<'_, SidecarState>) -> Result<Value, String> {
    state.status()
}

#[tauri::command]
pub fn sidecar_ping(state: State<'_, SidecarState>) -> Result<Value, String> {
    state.request("ping", None)
}

#[tauri::command]
pub fn sidecar_restart(state: State<'_, SidecarState>) -> Result<Value, String> {
    state.restart()
}

#[tauri::command]
pub fn sidecar_list_step_specs(state: State<'_, SidecarState>) -> Result<Value, String> {
    state.request("listStepSpecs", None)
}

#[tauri::command]
pub fn sidecar_open_recipe(
    state: State<'_, SidecarState>,
    path: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    state.request(
        "openRecipe",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    )
}

#[tauri::command]
pub fn sidecar_get_document(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    state.request(
        "getDocument",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn sidecar_apply_recipe_command(
    state: State<'_, SidecarState>,
    document_id: String,
    command: Value,
) -> Result<Value, String> {
    state.request(
        "applyRecipeCommand",
        Some(json!({
            "documentId": document_id,
            "command": command,
        })),
    )
}

#[tauri::command]
pub fn sidecar_undo(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    state.request(
        "undo",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn sidecar_redo(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    state.request(
        "redo",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn sidecar_save_recipe(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    state.request(
        "saveRecipe",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn sidecar_save_recipe_as(
    state: State<'_, SidecarState>,
    document_id: String,
    path: String,
) -> Result<Value, String> {
    state.request(
        "saveRecipeAs",
        Some(json!({
            "documentId": document_id,
            "path": path,
        })),
    )
}

#[tauri::command]
pub fn sidecar_validate(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    state.request(
        "validate",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn sidecar_emit_yaml(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    state.request(
        "emitYaml",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn sidecar_get_ref_index(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    state.request(
        "getRefIndex",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn sidecar_set_document_authored_root(
    state: State<'_, SidecarState>,
    document_id: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    state.request(
        "setDocumentAuthoredRoot",
        Some(json!({
            "documentId": document_id,
            "authoredRoot": authored_root,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn former_one_shot_responses_do_not_expose_sidecar_transport_id() {
        let response = strip_sidecar_transport_id(json!({
            "id": "req-1",
            "ok": true,
            "result": {"stepSpecs": []}
        }));

        assert_eq!(
            response,
            json!({
                "ok": true,
                "result": {"stepSpecs": []}
            })
        );
    }

    #[test]
    fn sidecar_failure_envelope_shape_is_preserved_without_transport_id() {
        let response = strip_sidecar_transport_id(json!({
            "id": "req-1",
            "ok": false,
            "error": {
                "code": "load_failed",
                "message": "bad recipe",
                "details": {"path": "missing.yaml"}
            }
        }));

        assert_eq!(
            response,
            json!({
                "ok": false,
                "error": {
                    "code": "load_failed",
                    "message": "bad recipe",
                    "details": {"path": "missing.yaml"}
                }
            })
        );
    }
}
