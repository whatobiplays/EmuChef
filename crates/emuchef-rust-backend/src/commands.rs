//! Editor command decoding for the Phase 6G Rust backend slice.
//!
//! Python remains the reference implementation. This module intentionally
//! accepts only the overview field command needed for Phase 6G so the Rust
//! backend does not imply support for input, artifact, step, safe-delete, or ref
//! index command parity before those behaviors are ported and tested.

use serde_json::{json, Map, Value};

use crate::errors::ApiError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecipeCommand {
    SetOverviewField {
        field: OverviewField,
        value: OverviewValue,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverviewField {
    Name,
    Description,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverviewValue {
    Text(String),
    Null,
}

pub fn decode_recipe_command(payload: &Value) -> Result<RecipeCommand, ApiError> {
    let object = match payload {
        Value::Object(object) => object,
        _ => {
            return Err(ApiError::invalid_command_with_details(
                "Command payload must be an object.",
                json!({ "commandType": Value::Null }),
            ));
        }
    };

    let command_type = match object.get("type") {
        Some(Value::String(command_type)) if !command_type.is_empty() => command_type.as_str(),
        value => {
            return Err(ApiError::invalid_command_with_details(
                "Command payload must include a string type.",
                json!({ "commandType": value.cloned().unwrap_or(Value::Null) }),
            ));
        }
    };

    match command_type {
        "SetOverviewField" => decode_set_overview_field(object),
        unknown => Err(ApiError::invalid_command_with_details(
            format!("Unsupported command type: {unknown}"),
            json!({ "commandType": unknown }),
        )),
    }
}

fn decode_set_overview_field(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    reject_unexpected_fields(object, &["type", "field", "value"], "SetOverviewField")?;

    let field = match object.get("field") {
        Some(Value::String(field)) if field == "name" => OverviewField::Name,
        Some(Value::String(field)) if field == "description" => OverviewField::Description,
        Some(Value::String(field)) => {
            return Err(ApiError::invalid_command_with_details(
                format!("Invalid field value: {field}"),
                json!({
                    "commandType": "SetOverviewField",
                    "field": "field",
                    "allowedValues": ["name", "description"],
                }),
            ));
        }
        value => {
            return Err(ApiError::invalid_command_with_details(
                "Command field 'field' must be a string.",
                json!({
                    "commandType": "SetOverviewField",
                    "field": "field",
                    "value": value.cloned().unwrap_or(Value::Null),
                }),
            ));
        }
    };

    let value = match object.get("value") {
        Some(Value::String(value)) => OverviewValue::Text(value.clone()),
        Some(Value::Null) if field == OverviewField::Description => OverviewValue::Null,
        Some(value) => {
            return Err(ApiError::invalid_command_with_details(
                "SetOverviewField value must be a string.",
                json!({
                    "commandType": "SetOverviewField",
                    "field": "value",
                    "value": value,
                }),
            ));
        }
        None => {
            return Err(ApiError::invalid_command_with_details(
                "SetOverviewField command payload is missing required field: value",
                json!({ "commandType": "SetOverviewField", "field": "value" }),
            ));
        }
    };

    Ok(RecipeCommand::SetOverviewField { field, value })
}

fn reject_unexpected_fields(
    object: &Map<String, Value>,
    allowed_fields: &[&str],
    command_type: &str,
) -> Result<(), ApiError> {
    let unexpected_fields: Vec<&String> = object
        .keys()
        .filter(|field| !allowed_fields.contains(&field.as_str()))
        .collect();
    if unexpected_fields.is_empty() {
        return Ok(());
    }

    Err(ApiError::invalid_command_with_details(
        format!("Unexpected field in {command_type} command payload."),
        json!({
            "commandType": command_type,
            "unexpectedFields": unexpected_fields,
            "allowedFields": allowed_fields,
        }),
    ))
}
