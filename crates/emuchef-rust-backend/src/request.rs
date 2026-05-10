use serde_json::{Map, Value};

use crate::envelope;
use crate::errors::ApiError;
use crate::protocol;
use crate::step_specs;

/// Validate and dispatch a one-shot request object.
pub fn handle_one_shot_value(request: Value) -> Value {
    match handle_request(request) {
        Ok(response) => response,
        Err(error) => envelope::failure(error),
    }
}

/// Validate and dispatch a sidecar request object, including sidecar id rules.
pub fn handle_sidecar_value(request: Value) -> Value {
    let mut request_id = None;
    let response = match validate_request_object(request) {
        Ok(object) => match validate_sidecar_id(&object) {
            Ok(id) => {
                request_id = Some(id);
                handle_validated_object(&object).unwrap_or_else(envelope::failure)
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
