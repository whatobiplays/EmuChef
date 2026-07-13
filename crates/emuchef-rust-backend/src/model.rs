//! Authored recipe model used by the Rust runtime and editor protocol.
//!
//! Recipe inputs are the public runtime-configuration declaration surface.
//! Presentation metadata stays semantic and toolkit-neutral so protocol clients
//! can choose controls without recipe-specific behavior.

use indexmap::IndexMap;
use serde_json::Value;

pub type OrderedMap<T> = IndexMap<String, T>;

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub schema_version: i64,
    pub kind: String,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub recipe_dependencies: Vec<String>,
    pub provides: RecipeProvides,
    pub inputs: OrderedMap<InputDeclaration>,
    pub artifacts: OrderedMap<RemoteFileArtifact>,
    pub artifact_groups: OrderedMap<Vec<String>>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecipeProvides {
    pub features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputDeclaration {
    pub type_name: String,
    pub role: String,
    pub label: String,
    pub description: Option<String>,
    pub required: bool,
    pub multiple: bool,
    pub validation: InputValidation,
    pub default: Value,
    pub options: Vec<InputOption>,
    pub sensitive: bool,
    pub advanced: bool,
    pub metadata: OrderedMap<Value>,
}

impl InputDeclaration {
    /// Return whether a JSON value has the authored input's declared shape.
    /// Constraint checks such as enum membership and path prefixes are separate
    /// so callers can report precise diagnostics without reparsing the type.
    pub fn value_matches_type(&self, value: &Value) -> bool {
        if self.multiple {
            return value.as_array().is_some_and(|items| {
                items
                    .iter()
                    .all(|item| scalar_input_value_matches(&self.type_name, item))
            });
        }
        match self.type_name.as_str() {
            "string_list" | "path_list" => value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
            type_name => scalar_input_value_matches(type_name, value),
        }
    }

    /// Flatten a valid single or list-like binding for per-value constraints.
    pub fn binding_items<'a>(&self, value: &'a Value) -> Option<Vec<&'a Value>> {
        if self.multiple || matches!(self.type_name.as_str(), "string_list" | "path_list") {
            return value.as_array().map(|items| items.iter().collect());
        }
        Some(vec![value])
    }
}

fn scalar_input_value_matches(type_name: &str, value: &Value) -> bool {
    match type_name {
        "string" | "enum" | "file" | "directory" | "path" | "device_path" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "string_list" | "path_list" => value.is_array(),
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputOption {
    pub value: Value,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputValidation {
    pub must_exist: bool,
    pub allowed_extensions: Vec<String>,
    pub path_kind: Option<String>,
    pub allowed_prefixes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteFileArtifact {
    pub type_name: String,
    pub url: String,
    pub cache: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub id: String,
    pub type_name: String,
    pub name: String,
    pub description: Option<String>,
    pub user_toggleable: bool,
    pub dependencies: Vec<String>,
    pub constraints: StepConstraints,
    pub skip_if: Vec<StepCondition>,
    pub params: OrderedMap<ParamValue>,
    pub verify: Vec<StepCondition>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepConstraints {
    pub capabilities: Vec<String>,
    pub conflicts_with: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepCondition {
    pub type_name: String,
    pub params: OrderedMap<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Ref(String),
    Literal(Value),
}
