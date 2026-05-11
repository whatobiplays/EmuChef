//! Temporary StepSpec DTO parity source for Phase 6D.
//!
//! These step specs are a static parity copy of the Python built-ins generated
//! from the Python `listStepSpecs` API. Python remains the reference
//! implementation until Rust backend replacement is explicitly approved.
//! Regenerate and compare the fixture whenever Python step specs change.
//!
//! This fixture-backed source is Phase 6D scaffolding, not the final Rust step
//! registry. Later phases should replace it with Rust-native schema builders
//! before planner or executor behavior is ported.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const PYTHON_STEP_SPECS_RESULT: &str = include_str!("../tests/fixtures/python_step_specs.json");

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StepSpecsResult {
    #[serde(rename = "stepSpecs")]
    pub step_specs: Vec<StepSpecDto>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StepSpecDto {
    #[serde(rename = "type")]
    pub type_name: String,
    pub label: String,
    pub supported: bool,
    #[serde(rename = "primaryOutputName")]
    pub primary_output_name: Option<String>,
    pub outputs: Vec<StepOutputDto>,
    #[serde(rename = "paramOrder")]
    pub param_order: Vec<String>,
    pub params: BTreeMap<String, StepParamDto>,
    pub defaults: BTreeMap<String, Value>,
    #[serde(rename = "refFilters")]
    pub ref_filters: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StepOutputDto {
    pub name: String,
    #[serde(rename = "valueType")]
    pub value_type: String,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StepParamDto {
    pub mode: String,
    pub required: bool,
    #[serde(rename = "enumValues")]
    pub enum_values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<Value>,
}

pub fn list_step_specs_result() -> Value {
    serde_json::to_value(step_specs_result())
        .expect("StepSpec DTO surface should serialize to JSON")
}

pub fn step_specs_result() -> StepSpecsResult {
    serde_json::from_str(PYTHON_STEP_SPECS_RESULT)
        .expect("embedded Python StepSpec fixture must match the Rust DTO surface")
}

pub fn step_spec_for(type_name: &str) -> Option<StepSpecDto> {
    step_specs_result()
        .step_specs
        .into_iter()
        .find(|spec| spec.type_name == type_name)
}

pub fn is_supported_step_type(type_name: &str) -> bool {
    step_spec_for(type_name).is_some()
}
