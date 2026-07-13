//! Shared parsing and type metadata for authored and normalized runtime refs.
//!
//! Recipe authors use local refs such as `inputs.destination`. Planner output
//! qualifies those refs later; this module deliberately understands only the
//! authored namespaces and never exposes binding-key syntax to recipe YAML.

/// Runtime artifact fields that may be referenced from authored step params.
pub(crate) const RUNTIME_ARTIFACT_FIELDS: &[&str] = &[
    "cache_hit",
    "error",
    "filename",
    "local_path",
    "resolved_url",
    "status",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeRef {
    Input { target_id: String },
    ArtifactField { target_id: String, field: String },
    StepShorthand { target_id: String },
    StepOutput { target_id: String, field: String },
}

impl RuntimeRef {
    pub(crate) fn source_kind(&self) -> &'static str {
        match self {
            Self::Input { .. } => "input_ref",
            Self::ArtifactField { .. } => "artifact_ref",
            Self::StepShorthand { .. } | Self::StepOutput { .. } => "step_output_ref",
        }
    }
}

/// Parse a recipe-local authored ref without qualifying any local identifier.
pub(crate) fn parse_reference(value: &str) -> Result<RuntimeRef, ()> {
    if let Some(target_id) = value.strip_prefix("inputs.") {
        return (!target_id.is_empty())
            .then(|| RuntimeRef::Input {
                target_id: target_id.to_string(),
            })
            .ok_or(());
    }

    if let Some(step_body) = value.strip_prefix("steps.") {
        if let Some((target_id, field)) = step_body.split_once(".outputs.") {
            return (!target_id.is_empty() && !field.is_empty())
                .then(|| RuntimeRef::StepOutput {
                    target_id: target_id.to_string(),
                    field: field.to_string(),
                })
                .ok_or(());
        }
        return (!step_body.is_empty())
            .then(|| RuntimeRef::StepShorthand {
                target_id: step_body.to_string(),
            })
            .ok_or(());
    }

    if let Some(body) = value.strip_prefix("artifacts.") {
        if let Some((target_id, field)) = body.rsplit_once('.') {
            return (!target_id.is_empty() && !field.is_empty())
                .then(|| RuntimeRef::ArtifactField {
                    target_id: target_id.to_string(),
                    field: field.to_string(),
                })
                .ok_or(());
        }
    }

    Err(())
}

pub(crate) fn input_value_type(type_name: &str, multiple: bool) -> &'static str {
    if multiple {
        return match type_name {
            "string" | "enum" => "string_list",
            _ => "path_list",
        };
    }
    match type_name {
        "directory" => "directory_path",
        "file" => "file_path",
        "device_path" => "device_path",
        "path" => "path",
        "string_list" => "string_list",
        "path_list" => "path_list",
        "integer" => "integer",
        "boolean" => "boolean",
        "object" => "object",
        "string" | "enum" => "string",
        _ => "file_path",
    }
}

pub(crate) fn artifact_field_value_type(field: &str) -> Option<&'static str> {
    match field {
        "cache_hit" => Some("boolean"),
        "local_path" => Some("file_path"),
        "error" | "filename" | "resolved_url" | "status" => Some("string"),
        _ => None,
    }
}
