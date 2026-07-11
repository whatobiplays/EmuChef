//! Authored recipe model used by the Rust runtime and editor protocol.
//!
//! These types intentionally cover only the authored YAML sections needed for
//! load/emit parity tests. They are not a planner, executor, or editor document
//! model, and they should not grow those responsibilities in this phase.

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
    pub metadata: OrderedMap<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputValidation {
    pub must_exist: bool,
    pub allowed_extensions: Vec<String>,
    pub path_kind: Option<String>,
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
