use serde_json::{json, Value};
use tauri::State;

use crate::sidecar_client::SidecarState;

#[tauri::command]
pub fn list_step_specs(state: State<'_, SidecarState>) -> Result<Value, String> {
    request_without_transport_id(&state, "listStepSpecs", None)
}

#[tauri::command]
pub fn open_recipe(
    state: State<'_, SidecarState>,
    path: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    request_without_transport_id(
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
    request_without_transport_id(
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
    request_without_transport_id(
        &state,
        "emitRecipeYamlFromPath",
        Some(json!({
            "path": path,
            "authoredRoot": authored_root,
        })),
    )
}

#[tauri::command]
pub fn open_user_configuration(
    state: State<'_, SidecarState>,
    path: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "openUserConfiguration",
        Some(json!({ "path": path, "authoredRoot": authored_root })),
    )
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_user_configuration(
    state: State<'_, SidecarState>,
    path: String,
    configuration_id: String,
    name: String,
    device_plan: String,
    selected_recipes: Vec<String>,
    authored_root: Option<String>,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "createUserConfiguration",
        Some(json!({
            "path": path,
            "configurationId": configuration_id,
            "name": name,
            "devicePlan": device_plan,
            "selectedRecipes": selected_recipes,
            "authoredRoot": authored_root,
        })),
    )
}

#[tauri::command]
pub fn get_user_configuration_document(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "getUserConfigurationDocument",
        Some(json!({ "documentId": document_id })),
    )
}

#[tauri::command]
pub fn save_user_configuration(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "saveUserConfiguration",
        Some(json!({ "documentId": document_id })),
    )
}

#[tauri::command]
pub fn save_user_configuration_as(
    state: State<'_, SidecarState>,
    document_id: String,
    path: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "saveUserConfigurationAs",
        Some(json!({ "documentId": document_id, "path": path })),
    )
}

#[tauri::command]
pub fn set_user_configuration_binding(
    state: State<'_, SidecarState>,
    document_id: String,
    key: String,
    value: Value,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "setUserConfigurationBinding",
        Some(json!({ "documentId": document_id, "key": key, "value": value })),
    )
}

#[tauri::command]
pub fn remove_user_configuration_binding(
    state: State<'_, SidecarState>,
    document_id: String,
    key: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "removeUserConfigurationBinding",
        Some(json!({ "documentId": document_id, "key": key })),
    )
}

#[tauri::command]
pub fn set_user_configuration_selected_recipes(
    state: State<'_, SidecarState>,
    document_id: String,
    selected_recipes: Vec<String>,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "setUserConfigurationSelectedRecipes",
        Some(json!({
            "documentId": document_id,
            "selectedRecipes": selected_recipes,
        })),
    )
}

#[tauri::command]
pub fn set_user_configuration_device_plan(
    state: State<'_, SidecarState>,
    document_id: String,
    device_plan: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "setUserConfigurationDevicePlan",
        Some(json!({ "documentId": document_id, "devicePlan": device_plan })),
    )
}

#[tauri::command]
pub fn validate_user_configuration(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "validateUserConfiguration",
        Some(json!({ "documentId": document_id })),
    )
}

#[tauri::command]
pub fn emit_user_configuration_yaml(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "emitUserConfigurationYaml",
        Some(json!({ "documentId": document_id })),
    )
}

#[tauri::command]
pub fn set_user_configuration_authored_root(
    state: State<'_, SidecarState>,
    document_id: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "setUserConfigurationAuthoredRoot",
        Some(json!({ "documentId": document_id, "authoredRoot": authored_root })),
    )
}

#[tauri::command]
pub fn close_user_configuration(
    state: State<'_, SidecarState>,
    document_id: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "closeUserConfiguration",
        Some(json!({ "documentId": document_id })),
    )
}

fn request_without_transport_id(
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
    request_without_transport_id(&state, "ping", None)
}

#[tauri::command]
pub fn sidecar_restart(state: State<'_, SidecarState>) -> Result<Value, String> {
    state.restart()
}

#[tauri::command]
pub fn get_document(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "getDocument",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn apply_recipe_command(
    state: State<'_, SidecarState>,
    document_id: String,
    command: Value,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "applyRecipeCommand",
        Some(json!({
            "documentId": document_id,
            "command": command,
        })),
    )
}

#[tauri::command]
pub fn undo(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "undo",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn redo(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "redo",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn save_recipe(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "saveRecipe",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn save_recipe_as(
    state: State<'_, SidecarState>,
    document_id: String,
    path: String,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "saveRecipeAs",
        Some(json!({
            "documentId": document_id,
            "path": path,
        })),
    )
}

#[tauri::command]
pub fn validate(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "validate",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn emit_yaml(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "emitYaml",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn get_ref_index(state: State<'_, SidecarState>, document_id: String) -> Result<Value, String> {
    request_without_transport_id(
        &state,
        "getRefIndex",
        Some(json!({
            "documentId": document_id,
        })),
    )
}

#[tauri::command]
pub fn set_document_authored_root(
    state: State<'_, SidecarState>,
    document_id: String,
    authored_root: Option<String>,
) -> Result<Value, String> {
    request_without_transport_id(
        &state,
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
    fn product_responses_do_not_expose_sidecar_transport_id() {
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
