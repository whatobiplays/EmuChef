use std::path::Path;

use serde_json::{json, Map, Value};

use crate::envelope;
use crate::errors::ApiError;
use crate::protocol;
use crate::session::DocumentSessionManager;
use crate::step_specs;
use crate::validation;
use crate::yaml;

/// Validate and dispatch a one-shot request object.
pub fn handle_one_shot_value(request: Value) -> Value {
    match handle_request(request) {
        Ok(response) => response,
        Err(error) => envelope::failure(error),
    }
}

/// Validate and dispatch a sidecar request object, including sidecar id rules.
pub fn handle_sidecar_value(request: Value, sessions: &mut DocumentSessionManager) -> Value {
    let mut request_id = None;
    let response = match validate_request_object(request) {
        Ok(object) => match validate_sidecar_id(&object) {
            Ok(id) => {
                request_id = Some(id);
                handle_validated_sidecar_object(&object, sessions).unwrap_or_else(envelope::failure)
            }
            Err(error) => envelope::failure(error),
        },
        Err(error) => envelope::failure(error),
    };

    envelope::with_id(response, request_id)
}

fn handle_request(request: Value) -> Result<Value, ApiError> {
    let object = validate_request_object(request)?;
    handle_validated_object(&object)
}

fn handle_validated_object(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let request_type = validate_request_type(object)?;
    validate_payload(object)?;

    match request_type {
        "hello" => Ok(envelope::success(protocol::hello_result())),
        "listStepSpecs" => Ok(envelope::success(step_specs::list_step_specs_result())),
        "emitRecipeYamlFromPath" => handle_emit_recipe_yaml_from_path(object),
        "validateRecipePath" => handle_validate_recipe_path(object),
        unknown => Err(ApiError::invalid_request(format!(
            "Unknown request type: {unknown}"
        ))),
    }
}

fn handle_validated_sidecar_object(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let request_type = validate_request_type(object)?;
    validate_payload(object)?;

    match request_type {
        "hello" => Ok(envelope::success(protocol::hello_result())),
        "listStepSpecs" => Ok(envelope::success(step_specs::list_step_specs_result())),
        "emitRecipeYamlFromPath" => handle_emit_recipe_yaml_from_path(object),
        "validateRecipePath" => handle_validate_recipe_path(object),
        "openRecipe" => handle_open_recipe(object, sessions),
        "getDocument" => handle_get_document(object, sessions),
        "saveRecipe" => handle_save_recipe(object, sessions),
        "applyRecipeCommand" => handle_apply_recipe_command(object, sessions),
        "undo" => handle_undo(object, sessions),
        "redo" => handle_redo(object, sessions),
        "emitYaml" => handle_emit_yaml(object, sessions),
        "validate" => handle_validate(object, sessions),
        "getRefIndex" => handle_get_ref_index(object, sessions),
        "closeDocument" => handle_close_document(object, sessions),
        unknown => Err(ApiError::invalid_request(format!(
            "Unknown request type: {unknown}"
        ))),
    }
}

fn validate_request_object(request: Value) -> Result<Map<String, Value>, ApiError> {
    match request {
        Value::Object(object) => Ok(object),
        _ => Err(ApiError::invalid_request("Request must be a JSON object.")),
    }
}

fn validate_sidecar_id(object: &Map<String, Value>) -> Result<String, ApiError> {
    match object.get("id") {
        Some(Value::String(id)) if !id.is_empty() => Ok(id.clone()),
        _ => Err(ApiError::invalid_request(
            "Sidecar request must include a non-empty string id.",
        )),
    }
}

fn validate_request_type(object: &Map<String, Value>) -> Result<&str, ApiError> {
    match object.get("type") {
        Some(Value::String(request_type)) if !request_type.is_empty() => Ok(request_type),
        _ => Err(ApiError::invalid_request(
            "Request must include a string type.",
        )),
    }
}

fn validate_payload(object: &Map<String, Value>) -> Result<(), ApiError> {
    match object.get("payload") {
        None | Some(Value::Null) | Some(Value::Object(_)) => Ok(()),
        _ => Err(ApiError::invalid_request(
            "Request payload must be an object.",
        )),
    }
}

fn handle_emit_recipe_yaml_from_path(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_path(payload)?;
    match yaml::emit_recipe_yaml_from_path(Path::new(path)) {
        Ok(yaml) => Ok(envelope::success(json!({ "yaml": yaml }))),
        Err(error) => Err(ApiError::load_failed(
            format!("Failed to emit recipe YAML: {error}"),
            json!({ "path": path }),
        )),
    }
}

fn handle_validate_recipe_path(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_path(payload)?;
    let authored_root_provided = payload
        .get("authoredRoot")
        .is_some_and(|value| !value.is_null());
    Ok(envelope::success(validation::validate_recipe_path_result(
        Path::new(path),
        authored_root_provided,
    )))
}

fn handle_open_recipe(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_path(payload)?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(
        sessions.open_recipe(path, authored_root)?,
    ))
}

fn handle_get_document(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.get_document(document_id)?))
}

fn handle_save_recipe(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.save_recipe(document_id)?))
}

fn handle_apply_recipe_command(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let command = payload.get("command").unwrap_or(&Value::Null);
    Ok(envelope::success(
        sessions.apply_recipe_command(document_id, command)?,
    ))
}

fn handle_undo(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.undo(document_id)?))
}

fn handle_redo(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.redo(document_id)?))
}

fn handle_emit_yaml(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.emit_yaml(document_id)?))
}

fn handle_validate(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.validate(document_id)?))
}

fn handle_get_ref_index(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.get_ref_index(document_id)?))
}

fn handle_close_document(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    Ok(envelope::success(sessions.close_document(document_id)?))
}

fn payload_object(object: &Map<String, Value>) -> Result<&Map<String, Value>, ApiError> {
    match object.get("payload") {
        Some(Value::Object(payload)) => Ok(payload),
        None | Some(Value::Null) => {
            static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
            Ok(EMPTY.get_or_init(Map::new))
        }
        _ => Err(ApiError::invalid_request(
            "Request payload must be an object.",
        )),
    }
}

fn required_path(payload: &Map<String, Value>) -> Result<&str, ApiError> {
    match payload.get("path") {
        Some(Value::String(path)) if !path.is_empty() => Ok(path),
        _ => Err(ApiError::invalid_request_with_details(
            "Request payload is missing required field: path",
            json!({ "field": "path" }),
        )),
    }
}

fn required_document_id(payload: &Map<String, Value>) -> Result<&str, ApiError> {
    match payload.get("documentId") {
        Some(Value::String(document_id)) if !document_id.is_empty() => Ok(document_id),
        _ => Err(ApiError::invalid_request_with_details(
            "Request payload is missing required field: documentId",
            json!({ "field": "documentId" }),
        )),
    }
}

fn optional_string<'a>(
    payload: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, ApiError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        _ => Err(ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be a non-empty string when provided."),
            json!({ "field": field }),
        )),
    }
}
