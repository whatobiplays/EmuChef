//! Editor command decoding for the experimental Rust backend slice.
//!
//! Python remains the reference implementation. This module intentionally
//! accepts only the command families that have explicit parity tests in the
//! current Rust backend migration slice.

use serde_json::{json, Map, Value};

use crate::errors::ApiError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecipeCommand {
    SetOverviewField {
        field: OverviewField,
        value: OverviewValue,
    },
    AddInput {
        input_id: String,
    },
    RenameInput {
        input_id: String,
        new_input_id: String,
    },
    UpdateInputField {
        input_id: String,
        field: InputField,
        value: Value,
    },
    DeleteInput {
        input_id: String,
    },
    DuplicateInput {
        source_input_id: String,
        new_input_id: String,
    },
    AddArtifact {
        artifact_id: String,
        url: String,
    },
    RenameArtifact {
        artifact_id: String,
        new_artifact_id: String,
    },
    UpdateArtifactField {
        artifact_id: String,
        field: ArtifactField,
        value: Value,
    },
    DeleteArtifact {
        artifact_id: String,
    },
    DuplicateArtifact {
        source_artifact_id: String,
        new_artifact_id: String,
    },
    AddArtifactGroup {
        group_id: String,
    },
    RenameArtifactGroup {
        group_id: String,
        new_group_id: String,
    },
    DeleteArtifactGroup {
        group_id: String,
    },
    DuplicateArtifactGroup {
        source_group_id: String,
        new_group_id: String,
    },
    ReorderArtifactGroup {
        group_id: String,
        to_index: i64,
    },
    AddArtifactGroupMember {
        group_id: String,
        artifact_id: String,
        index: Option<i64>,
    },
    RemoveArtifactGroupMember {
        group_id: String,
        index: i64,
    },
    ReorderArtifactGroupMember {
        group_id: String,
        index: i64,
        to_index: i64,
    },
    AddStep {
        step_id: String,
        step_type: String,
        name: String,
        index: Option<i64>,
    },
    DeleteStep {
        step_id: String,
    },
    DuplicateStep {
        source_step_id: String,
        new_step_id: String,
    },
    ReorderStep {
        step_id: String,
        to_index: i64,
    },
    UpdateStepBasics {
        step_id: String,
        name: String,
        description: Value,
    },
    SetStepUserToggleable {
        step_id: String,
        user_toggleable: bool,
    },
    UpdateStepDependencies {
        step_id: String,
        dependencies: Vec<String>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputField {
    Type,
    Role,
    Label,
    Description,
    Required,
    Multiple,
    ValidationMustExist,
    ValidationAllowedExtensions,
    ValidationPathKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactField {
    Url,
    Cache,
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
        "AddInput" => decode_add_input(object),
        "RenameInput" => decode_rename_input(object),
        "DeleteInput" => decode_delete_input(object),
        "DuplicateInput" => decode_duplicate_input(object),
        "UpdateInputField" => decode_update_input_field(object),
        "AddArtifact" => decode_add_artifact(object),
        "UpdateArtifactField" => decode_update_artifact_field(object),
        "RenameArtifact" => decode_rename_artifact(object),
        "DeleteArtifact" => decode_delete_artifact(object),
        "DuplicateArtifact" => decode_duplicate_artifact(object),
        "AddArtifactGroup" => decode_add_artifact_group(object),
        "RenameArtifactGroup" => decode_rename_artifact_group(object),
        "DeleteArtifactGroup" => decode_delete_artifact_group(object),
        "DuplicateArtifactGroup" => decode_duplicate_artifact_group(object),
        "ReorderArtifactGroup" => decode_reorder_artifact_group(object),
        "AddArtifactGroupMember" => decode_add_artifact_group_member(object),
        "RemoveArtifactGroupMember" => decode_remove_artifact_group_member(object),
        "ReorderArtifactGroupMember" => decode_reorder_artifact_group_member(object),
        "AddStep" => decode_add_step(object),
        "DeleteStep" => decode_delete_step(object),
        "DuplicateStep" => decode_duplicate_step(object),
        "ReorderStep" => decode_reorder_step(object),
        "UpdateStepBasics" => decode_update_step_basics(object),
        "SetStepUserToggleable" => decode_set_step_user_toggleable(object),
        "UpdateStepDependencies" => decode_update_step_dependencies(object),
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

fn decode_add_input(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::AddInput {
        input_id: required_str(object, "inputId", "AddInput")?,
    })
}

fn decode_rename_input(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::RenameInput {
        input_id: required_str(object, "inputId", "RenameInput")?,
        new_input_id: required_str(object, "newInputId", "RenameInput")?,
    })
}

fn decode_delete_input(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::DeleteInput {
        input_id: required_str(object, "inputId", "DeleteInput")?,
    })
}

fn decode_duplicate_input(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::DuplicateInput {
        source_input_id: required_str(object, "sourceInputId", "DuplicateInput")?,
        new_input_id: required_str(object, "newInputId", "DuplicateInput")?,
    })
}

fn decode_update_input_field(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    let field = match required_str(object, "field", "UpdateInputField")?.as_str() {
        "type" => InputField::Type,
        "role" => InputField::Role,
        "label" => InputField::Label,
        "description" => InputField::Description,
        "required" => InputField::Required,
        "multiple" => InputField::Multiple,
        "validation.must_exist" => InputField::ValidationMustExist,
        "validation.allowed_extensions" => InputField::ValidationAllowedExtensions,
        "validation.path_kind" => InputField::ValidationPathKind,
        value => {
            return Err(ApiError::invalid_command_with_details(
                format!("Command field 'field' has unsupported value: {value}"),
                json!({
                    "commandType": "UpdateInputField",
                    "field": "field",
                    "allowedValues": [
                        "type",
                        "role",
                        "label",
                        "description",
                        "required",
                        "multiple",
                        "validation.must_exist",
                        "validation.allowed_extensions",
                        "validation.path_kind"
                    ],
                }),
            ));
        }
    };
    Ok(RecipeCommand::UpdateInputField {
        input_id: required_str(object, "inputId", "UpdateInputField")?,
        field,
        value: required_value(object, "value", "UpdateInputField")?.clone(),
    })
}

fn decode_add_artifact(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::AddArtifact {
        artifact_id: required_str(object, "artifactId", "AddArtifact")?,
        url: required_str(object, "url", "AddArtifact")?,
    })
}

fn decode_update_artifact_field(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    let field = match required_str(object, "field", "UpdateArtifactField")?.as_str() {
        "url" => ArtifactField::Url,
        "cache" => ArtifactField::Cache,
        value => {
            return Err(ApiError::invalid_command_with_details(
                format!("Command field 'field' has unsupported value: {value}"),
                json!({
                    "commandType": "UpdateArtifactField",
                    "field": "field",
                    "allowedValues": ["url", "cache"],
                }),
            ));
        }
    };
    Ok(RecipeCommand::UpdateArtifactField {
        artifact_id: required_str(object, "artifactId", "UpdateArtifactField")?,
        field,
        value: required_value(object, "value", "UpdateArtifactField")?.clone(),
    })
}

fn decode_rename_artifact(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::RenameArtifact {
        artifact_id: required_str(object, "artifactId", "RenameArtifact")?,
        new_artifact_id: required_str(object, "newArtifactId", "RenameArtifact")?,
    })
}

fn decode_delete_artifact(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::DeleteArtifact {
        artifact_id: required_str(object, "artifactId", "DeleteArtifact")?,
    })
}

fn decode_duplicate_artifact(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::DuplicateArtifact {
        source_artifact_id: required_str(object, "sourceArtifactId", "DuplicateArtifact")?,
        new_artifact_id: required_str(object, "newArtifactId", "DuplicateArtifact")?,
    })
}

fn decode_add_artifact_group(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::AddArtifactGroup {
        group_id: required_str(object, "groupId", "AddArtifactGroup")?,
    })
}

fn decode_rename_artifact_group(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::RenameArtifactGroup {
        group_id: required_str(object, "groupId", "RenameArtifactGroup")?,
        new_group_id: required_str(object, "newGroupId", "RenameArtifactGroup")?,
    })
}

fn decode_delete_artifact_group(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::DeleteArtifactGroup {
        group_id: required_str(object, "groupId", "DeleteArtifactGroup")?,
    })
}

fn decode_duplicate_artifact_group(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::DuplicateArtifactGroup {
        source_group_id: required_str(object, "sourceGroupId", "DuplicateArtifactGroup")?,
        new_group_id: required_str(object, "newGroupId", "DuplicateArtifactGroup")?,
    })
}

fn decode_reorder_artifact_group(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::ReorderArtifactGroup {
        group_id: required_str(object, "groupId", "ReorderArtifactGroup")?,
        to_index: required_index(object, "toIndex", "ReorderArtifactGroup")?,
    })
}

fn decode_add_artifact_group_member(
    object: &Map<String, Value>,
) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::AddArtifactGroupMember {
        group_id: required_str(object, "groupId", "AddArtifactGroupMember")?,
        artifact_id: required_str(object, "artifactId", "AddArtifactGroupMember")?,
        index: optional_index(object, "index", "AddArtifactGroupMember")?,
    })
}

fn decode_remove_artifact_group_member(
    object: &Map<String, Value>,
) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::RemoveArtifactGroupMember {
        group_id: required_str(object, "groupId", "RemoveArtifactGroupMember")?,
        index: required_index(object, "index", "RemoveArtifactGroupMember")?,
    })
}

fn decode_reorder_artifact_group_member(
    object: &Map<String, Value>,
) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::ReorderArtifactGroupMember {
        group_id: required_str(object, "groupId", "ReorderArtifactGroupMember")?,
        index: required_index(object, "index", "ReorderArtifactGroupMember")?,
        to_index: required_index(object, "toIndex", "ReorderArtifactGroupMember")?,
    })
}

fn decode_add_step(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::AddStep {
        step_id: required_str(object, "stepId", "AddStep")?,
        step_type: required_str(object, "stepType", "AddStep")?,
        name: required_str(object, "name", "AddStep")?,
        index: optional_index(object, "index", "AddStep")?,
    })
}

fn decode_delete_step(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::DeleteStep {
        step_id: required_str(object, "stepId", "DeleteStep")?,
    })
}

fn decode_duplicate_step(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::DuplicateStep {
        source_step_id: required_str(object, "sourceStepId", "DuplicateStep")?,
        new_step_id: required_str(object, "newStepId", "DuplicateStep")?,
    })
}

fn decode_reorder_step(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::ReorderStep {
        step_id: required_str(object, "stepId", "ReorderStep")?,
        to_index: required_index(object, "toIndex", "ReorderStep")?,
    })
}

fn decode_update_step_basics(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::UpdateStepBasics {
        step_id: required_str(object, "stepId", "UpdateStepBasics")?,
        name: required_str(object, "name", "UpdateStepBasics")?,
        description: required_optional_str(object, "description", "UpdateStepBasics")?,
    })
}

fn decode_set_step_user_toggleable(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::SetStepUserToggleable {
        step_id: required_str(object, "stepId", "SetStepUserToggleable")?,
        user_toggleable: required_bool(object, "userToggleable", "SetStepUserToggleable")?,
    })
}

fn decode_update_step_dependencies(object: &Map<String, Value>) -> Result<RecipeCommand, ApiError> {
    Ok(RecipeCommand::UpdateStepDependencies {
        step_id: required_str(object, "stepId", "UpdateStepDependencies")?,
        dependencies: required_string_list(object, "dependencies", "UpdateStepDependencies")?,
    })
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

fn required_value<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    command_type: &str,
) -> Result<&'a Value, ApiError> {
    object.get(field).ok_or_else(|| {
        ApiError::invalid_command_with_details(
            format!("Command payload is missing required field: {field}"),
            json!({ "commandType": command_type, "field": field }),
        )
    })
}

fn required_str(
    object: &Map<String, Value>,
    field: &str,
    command_type: &str,
) -> Result<String, ApiError> {
    match required_value(object, field, command_type)? {
        Value::String(value) if !value.is_empty() => Ok(value.clone()),
        value => Err(ApiError::invalid_command_with_details(
            format!("Command field '{field}' must be a non-empty string."),
            json!({
                "commandType": command_type,
                "field": field,
                "value": value,
            }),
        )),
    }
}

fn required_index(
    object: &Map<String, Value>,
    field: &str,
    command_type: &str,
) -> Result<i64, ApiError> {
    match required_value(object, field, command_type)? {
        Value::Number(value) => value.as_i64().ok_or_else(|| {
            ApiError::invalid_command_with_details(
                format!("Command field '{field}' must be an integer."),
                json!({
                    "commandType": command_type,
                    "field": field,
                }),
            )
        }),
        value => Err(ApiError::invalid_command_with_details(
            format!("Command field '{field}' must be an integer."),
            json!({
                "commandType": command_type,
                "field": field,
                "value": value,
            }),
        )),
    }
}

fn optional_index(
    object: &Map<String, Value>,
    field: &str,
    command_type: &str,
) -> Result<Option<i64>, ApiError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value.as_i64().map(Some).ok_or_else(|| {
            ApiError::invalid_command_with_details(
                format!("Command field '{field}' must be an integer."),
                json!({
                    "commandType": command_type,
                    "field": field,
                }),
            )
        }),
        Some(value) => Err(ApiError::invalid_command_with_details(
            format!("Command field '{field}' must be an integer."),
            json!({
                "commandType": command_type,
                "field": field,
                "value": value,
            }),
        )),
    }
}

fn required_bool(
    object: &Map<String, Value>,
    field: &str,
    command_type: &str,
) -> Result<bool, ApiError> {
    match required_value(object, field, command_type)? {
        Value::Bool(value) => Ok(*value),
        value => Err(ApiError::invalid_command_with_details(
            format!("Command field '{field}' must be a boolean."),
            json!({
                "commandType": command_type,
                "field": field,
                "value": value,
            }),
        )),
    }
}

fn required_optional_str(
    object: &Map<String, Value>,
    field: &str,
    command_type: &str,
) -> Result<Value, ApiError> {
    match required_value(object, field, command_type)? {
        Value::Null => Ok(Value::Null),
        Value::String(value) => Ok(Value::String(value.clone())),
        value => Err(ApiError::invalid_command_with_details(
            format!("Command field '{field}' must be a string or null."),
            json!({
                "commandType": command_type,
                "field": field,
                "value": value,
            }),
        )),
    }
}

fn required_string_list(
    object: &Map<String, Value>,
    field: &str,
    command_type: &str,
) -> Result<Vec<String>, ApiError> {
    let value = required_value(object, field, command_type)?;
    let Value::Array(items) = value else {
        return Err(ApiError::invalid_command_with_details(
            format!("Command field '{field}' must be a list."),
            json!({
                "commandType": command_type,
                "field": field,
                "value": value,
            }),
        ));
    };
    let mut strings = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Value::String(value) if !value.is_empty() => strings.push(value.clone()),
            _ => {
                return Err(ApiError::invalid_command_with_details(
                    format!("Command field '{field}' must contain only non-empty strings."),
                    json!({
                        "commandType": command_type,
                        "field": field,
                        "value": value,
                    }),
                ));
            }
        }
    }
    Ok(strings)
}
