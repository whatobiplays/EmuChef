use std::path::Path;

use serde_json::{json, Map, Value};

use crate::envelope;
use crate::errors::ApiError;
use crate::protocol;
use crate::session::DocumentSessionManager;
use crate::step_specs;
use crate::user_configuration;
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
        "ping" => Ok(envelope::success(protocol::ping_result())),
        "listStepSpecs" => Ok(envelope::success(step_specs::list_step_specs_result())),
        "emitRecipeYamlFromPath" => handle_emit_recipe_yaml_from_path(object),
        "validateRecipePath" => handle_validate_recipe_path(object),
        "emitUserConfigurationYamlFromPath" => {
            handle_emit_user_configuration_yaml_from_path(object)
        }
        "validateUserConfigurationPath" => handle_validate_user_configuration_path(object),
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
        "ping" => Ok(envelope::success(protocol::ping_result())),
        "listStepSpecs" => Ok(envelope::success(step_specs::list_step_specs_result())),
        "emitRecipeYamlFromPath" => handle_emit_recipe_yaml_from_path(object),
        "validateRecipePath" => handle_validate_recipe_path(object),
        "emitUserConfigurationYamlFromPath" => {
            handle_emit_user_configuration_yaml_from_path(object)
        }
        "validateUserConfigurationPath" => handle_validate_user_configuration_path(object),
        "openUserConfiguration" => handle_open_user_configuration(object, sessions),
        "createUserConfiguration" => handle_create_user_configuration(object, sessions),
        "getUserConfigurationDocument" => handle_get_user_configuration_document(object, sessions),
        "saveUserConfiguration" => handle_save_user_configuration(object, sessions),
        "saveUserConfigurationAs" => handle_save_user_configuration_as(object, sessions),
        "setUserConfigurationBinding" => handle_set_user_configuration_binding(object, sessions),
        "removeUserConfigurationBinding" => {
            handle_remove_user_configuration_binding(object, sessions)
        }
        "setUserConfigurationSelectedRecipes" => {
            handle_set_user_configuration_selected_recipes(object, sessions)
        }
        "setUserConfigurationDevicePlan" => {
            handle_set_user_configuration_device_plan(object, sessions)
        }
        "validateUserConfiguration" => handle_validate_user_configuration(object, sessions),
        "emitUserConfigurationYaml" => handle_emit_user_configuration_yaml(object, sessions),
        "setUserConfigurationAuthoredRoot" => {
            handle_set_user_configuration_authored_root(object, sessions)
        }
        "closeUserConfiguration" => handle_close_user_configuration(object, sessions),
        "openRecipe" => handle_open_recipe(object, sessions),
        "createRecipeFromTemplate" => handle_create_recipe_from_template(object, sessions),
        "getDocument" => handle_get_document(object, sessions),
        "saveRecipe" => handle_save_recipe(object, sessions),
        "saveRecipeAs" => handle_save_recipe_as(object, sessions),
        "applyRecipeCommand" => handle_apply_recipe_command(object, sessions),
        "undo" => handle_undo(object, sessions),
        "redo" => handle_redo(object, sessions),
        "emitYaml" => handle_emit_yaml(object, sessions),
        "validate" => handle_validate(object, sessions),
        "getRefIndex" => handle_get_ref_index(object, sessions),
        "setDocumentAuthoredRoot" => handle_set_document_authored_root(object, sessions),
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
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(validation::validate_recipe_path_result(
        Path::new(path),
        authored_root.map(Path::new),
    )))
}

fn handle_emit_user_configuration_yaml_from_path(
    object: &Map<String, Value>,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = user_configuration_path(payload)?;
    let configuration = user_configuration::load_user_configuration(&path).map_err(|error| {
        ApiError::load_failed(
            format!("Failed to load user configuration: {error}"),
            json!({ "path": path }),
        )
    })?;
    let yaml =
        user_configuration::emit_user_configuration_yaml(&configuration).map_err(|error| {
            ApiError::load_failed(
                format!("Failed to emit user configuration: {error}"),
                json!({ "path": path }),
            )
        })?;
    Ok(envelope::success(json!({ "yaml": yaml })))
}

fn handle_validate_user_configuration_path(object: &Map<String, Value>) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = user_configuration_path(payload)?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    let configuration = user_configuration::load_user_configuration(&path).map_err(|error| {
        ApiError::load_failed(
            format!("Failed to load user configuration: {error}"),
            json!({ "path": path }),
        )
    })?;
    let diagnostics = authored_root.map_or_else(Vec::new, |root| {
        user_configuration::validate_user_configuration_with_catalog(
            &configuration,
            &path,
            Path::new(root),
        )
    });
    Ok(envelope::success(json!({ "diagnostics": diagnostics })))
}

fn handle_open_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = user_configuration_path(payload)?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(sessions.open_user_configuration(
        &path.to_string_lossy(),
        authored_root,
    )?))
}

fn handle_create_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let path = required_string(payload, "path")?;
    let configuration_id = required_string(payload, "configurationId")?;
    let name = required_string(payload, "name")?;
    let device_plan = required_string(payload, "devicePlan")?;
    let selected_recipes = required_string_array(payload, "selectedRecipes")?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(sessions.create_user_configuration(
        path,
        configuration_id,
        name,
        device_plan,
        selected_recipes,
        authored_root,
    )?))
}

fn handle_get_user_configuration_document(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.get_user_configuration_document(required_document_id(payload)?)?,
    ))
}

fn handle_save_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.save_user_configuration(required_document_id(payload)?)?,
    ))
}

fn handle_save_user_configuration_as(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let path = required_path(payload)?;
    Ok(envelope::success(
        sessions.save_user_configuration_as(document_id, path)?,
    ))
}

fn handle_set_user_configuration_binding(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let key = required_string(payload, "key")?;
    let value = payload.get("value").cloned().ok_or_else(|| {
        ApiError::invalid_request_with_details(
            "Request payload is missing required field: value",
            json!({ "field": "value" }),
        )
    })?;
    Ok(envelope::success(sessions.set_user_configuration_binding(
        document_id,
        key,
        Some(value),
    )?))
}

fn handle_remove_user_configuration_binding(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.set_user_configuration_binding(
        required_document_id(payload)?,
        required_string(payload, "key")?,
        None,
    )?))
}

fn handle_set_user_configuration_selected_recipes(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.set_user_configuration_selected_recipes(
            required_document_id(payload)?,
            required_string_array(payload, "selectedRecipes")?,
        )?,
    ))
}

fn handle_set_user_configuration_device_plan(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.set_user_configuration_device_plan(
            required_document_id(payload)?,
            required_string(payload, "devicePlan")?.to_string(),
        )?,
    ))
}

fn handle_validate_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.validate_user_configuration(
        required_document_id(payload)?,
    )?))
}

fn handle_emit_user_configuration_yaml(
    object: &Map<String, Value>,
    sessions: &DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.emit_user_configuration_yaml(
        required_document_id(payload)?,
    )?))
}

fn handle_set_user_configuration_authored_root(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(
        sessions.set_user_configuration_authored_root(
            required_document_id(payload)?,
            required_nullable_string(payload, "authoredRoot")?,
        )?,
    ))
}

fn handle_close_user_configuration(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    Ok(envelope::success(sessions.close_user_configuration(
        required_document_id(payload)?,
    )?))
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

fn handle_create_recipe_from_template(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let template_path = required_string(payload, "templatePath")?;
    let destination_path = required_string(payload, "destinationPath")?;
    let recipe_id = required_string(payload, "recipeId")?;
    let authored_root = optional_string(payload, "authoredRoot")?;
    Ok(envelope::success(sessions.create_recipe_from_template(
        template_path,
        destination_path,
        recipe_id,
        authored_root,
    )?))
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

fn handle_save_recipe_as(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let path = required_path(payload)?;
    Ok(envelope::success(
        sessions.save_recipe_as(document_id, path)?,
    ))
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

fn handle_set_document_authored_root(
    object: &Map<String, Value>,
    sessions: &mut DocumentSessionManager,
) -> Result<Value, ApiError> {
    let payload = payload_object(object)?;
    let document_id = required_document_id(payload)?;
    let authored_root = required_nullable_string(payload, "authoredRoot")?;
    Ok(envelope::success(
        sessions.set_document_authored_root(document_id, authored_root)?,
    ))
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
    required_string(payload, "path")
}

fn required_string<'a>(payload: &'a Map<String, Value>, field: &str) -> Result<&'a str, ApiError> {
    match payload.get(field) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(ApiError::invalid_request_with_details(
            format!("Request payload is missing required field: {field}"),
            json!({ "field": field }),
        )),
    }
}

fn required_document_id(payload: &Map<String, Value>) -> Result<&str, ApiError> {
    required_string(payload, "documentId")
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

fn required_nullable_string<'a>(
    payload: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, ApiError> {
    match payload.get(field) {
        None => Err(ApiError::invalid_request_with_details(
            format!("Request payload is missing required field: {field}"),
            json!({ "field": field }),
        )),
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        _ => Err(ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be a non-empty string when provided."),
            json!({ "field": field }),
        )),
    }
}

fn required_string_array(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<Vec<String>, ApiError> {
    let Some(Value::Array(values)) = payload.get(field) else {
        return Err(ApiError::invalid_request_with_details(
            format!("Request field '{field}' must be an array of strings."),
            json!({ "field": field }),
        ));
    };
    values
        .iter()
        .map(|value| match value {
            Value::String(value) if !value.is_empty() => Ok(value.clone()),
            _ => Err(ApiError::invalid_request_with_details(
                format!("Request field '{field}' must be an array of non-empty strings."),
                json!({ "field": field }),
            )),
        })
        .collect()
}

fn user_configuration_path(payload: &Map<String, Value>) -> Result<std::path::PathBuf, ApiError> {
    let path = optional_string(payload, "path")?;
    let reference = optional_string(payload, "userConfiguration")?;
    let value = match (path, reference) {
        (Some(path), None) | (None, Some(path)) => path,
        (Some(_), Some(_)) => {
            return Err(ApiError::invalid_request_with_details(
                "Request must provide only one of 'path' or 'userConfiguration'.",
                json!({ "fields": ["path", "userConfiguration"] }),
            ));
        }
        (None, None) => {
            return Err(ApiError::invalid_request_with_details(
                "Request must provide 'path' or 'userConfiguration'.",
                json!({ "fields": ["path", "userConfiguration"] }),
            ));
        }
    };
    let configuration_root = optional_string(payload, "configurationRoot")?;
    user_configuration::resolve_user_configuration_path(configuration_root.map(Path::new), value)
        .map_err(|error| {
            ApiError::invalid_request_with_details(
                format!("Invalid user-configuration reference: {error}"),
                json!({ "userConfiguration": value }),
            )
        })
}
