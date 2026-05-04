use serde_json::{json, Value};
use tauri::State;

use crate::python_bridge::{build_request, run_request};
use crate::sidecar_client::SidecarState;

#[tauri::command]
pub fn list_step_specs() -> Result<Value, String> {
    run_request(build_request("listStepSpecs", None))
}

#[tauri::command]
pub fn open_recipe(path: String, authored_root: Option<String>) -> Result<Value, String> {
    run_request(build_request(
        "openRecipe",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    ))
}

#[tauri::command]
pub fn validate_recipe_path(path: String, authored_root: Option<String>) -> Result<Value, String> {
    run_request(build_request(
        "validateRecipePath",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    ))
}

#[tauri::command]
pub fn emit_recipe_yaml_from_path(
    path: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    run_request(build_request(
        "emitRecipeYamlFromPath",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    ))
}

#[tauri::command]
pub fn sidecar_status(state: State<'_, SidecarState>) -> Result<Value, String> {
    state.status()
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
