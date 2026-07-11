//! Rust-owned StepSpec DTO metadata for the editor protocol.
//!
//! This module owns the static StepSpec surface returned by `listStepSpecs` and
//! consumed by editor validation, ref indexing, planner scaffolding, and the
//! Tauri frontend. Keep the DTO shapes stable unless the editor protocol and
//! frontend expectations change together.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    StepSpecsResult {
        step_specs: vec![
            spec(
                "resolve_artifacts",
                "Resolve Artifacts",
                None,
                vec![],
                vec!["artifacts", "artifact_groups"],
                map(vec![
                    (
                        "artifact_groups",
                        param("literal", false, vec![], Some(artifact_group_list_shape())),
                    ),
                    (
                        "artifacts",
                        param("literal", false, vec![], Some(artifact_list_shape())),
                    ),
                ]),
                map(vec![]),
                ref_filters(vec![]),
            ),
            spec(
                "extract_artifacts",
                "Extract Artifacts",
                Some("extracted_paths"),
                vec![output("extracted_paths", "path_list", true)],
                vec!["artifacts", "artifact_groups", "extract_on"],
                map(vec![
                    (
                        "artifact_groups",
                        param("literal", false, vec![], Some(artifact_group_list_shape())),
                    ),
                    (
                        "artifacts",
                        param("literal", false, vec![], Some(artifact_list_shape())),
                    ),
                    (
                        "extract_on",
                        param("literal", false, vec!["host", "device"], None),
                    ),
                ]),
                map(vec![("extract_on", json!("host"))]),
                ref_filters(vec![]),
            ),
            spec(
                "extract_archive",
                "Extract Archive",
                Some("extracted_path"),
                vec![output("extracted_path", "directory_path", true)],
                vec![
                    "archive",
                    "extract_on",
                    "dest",
                    "device_temp_path",
                    "cleanup",
                ],
                map(vec![
                    ("archive", param("ref", true, vec![], None)),
                    ("cleanup", param("literal", false, vec![], None)),
                    ("dest", param("literal", false, vec![], None)),
                    ("device_temp_path", param("literal", false, vec![], None)),
                    (
                        "extract_on",
                        param("literal", false, vec!["host", "device"], None),
                    ),
                ]),
                map(vec![
                    ("cleanup", json!(true)),
                    ("extract_on", json!("host")),
                ]),
                ref_filters(vec![("archive", vec!["file_path"])]),
            ),
            spec(
                "copy_files",
                "Copy Files",
                Some("copied_paths"),
                vec![output("copied_paths", "path_list", true)],
                vec!["source", "dest", "copy_policy"],
                map(vec![
                    (
                        "copy_policy",
                        param("literal", false, vec!["merge", "sync", "replace"], None),
                    ),
                    ("dest", param("literal", true, vec![], None)),
                    ("source", param("ref", true, vec![], None)),
                ]),
                map(vec![("copy_policy", json!("merge"))]),
                ref_filters(vec![(
                    "source",
                    vec!["file_path", "directory_path", "path_list"],
                )]),
            ),
            spec(
                "install_apk",
                "Install APK",
                None,
                vec![],
                vec!["app", "replace_existing"],
                map(vec![
                    ("app", param("ref", true, vec![], None)),
                    ("replace_existing", param("literal", false, vec![], None)),
                ]),
                map(vec![("replace_existing", json!(false))]),
                ref_filters(vec![("app", vec!["file_path"])]),
            ),
            spec(
                "grant_permissions",
                "Grant Permissions",
                None,
                vec![],
                vec!["runtime", "appops", "policy"],
                map(vec![
                    (
                        "appops",
                        param("literal", false, vec![], Some(appops_shape())),
                    ),
                    (
                        "policy",
                        param("literal", false, vec![], Some(policy_shape())),
                    ),
                    (
                        "runtime",
                        param("literal", false, vec![], Some(runtime_permissions_shape())),
                    ),
                ]),
                map(vec![]),
                ref_filters(vec![]),
            ),
            spec(
                "launch_app",
                "Launch App",
                None,
                vec![],
                vec!["package_name", "activity"],
                map(vec![
                    ("activity", param("literal", false, vec![], None)),
                    ("package_name", param("literal", true, vec![], None)),
                ]),
                map(vec![]),
                ref_filters(vec![]),
            ),
            spec(
                "wait",
                "Wait",
                None,
                vec![],
                vec!["duration_ms"],
                map(vec![("duration_ms", param("literal", true, vec![], None))]),
                map(vec![]),
                ref_filters(vec![]),
            ),
            spec(
                "force_stop_app",
                "Force Stop",
                None,
                vec![],
                vec!["package_name"],
                map(vec![("package_name", param("literal", true, vec![], None))]),
                map(vec![]),
                ref_filters(vec![]),
            ),
        ],
    }
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

// Step specifications are static declarative records. Keeping every field
// visible at each call site is clearer than a mutable builder with defaults.
#[allow(clippy::too_many_arguments)]
fn spec(
    type_name: &str,
    label: &str,
    primary_output_name: Option<&str>,
    outputs: Vec<StepOutputDto>,
    param_order: Vec<&str>,
    params: BTreeMap<String, StepParamDto>,
    defaults: BTreeMap<String, Value>,
    ref_filters: BTreeMap<String, Vec<String>>,
) -> StepSpecDto {
    StepSpecDto {
        type_name: type_name.to_string(),
        label: label.to_string(),
        supported: true,
        primary_output_name: primary_output_name.map(str::to_string),
        outputs,
        param_order: param_order.into_iter().map(str::to_string).collect(),
        params,
        defaults,
        ref_filters,
    }
}

fn output(name: &str, value_type: &str, primary: bool) -> StepOutputDto {
    StepOutputDto {
        name: name.to_string(),
        value_type: value_type.to_string(),
        primary,
    }
}

fn param(mode: &str, required: bool, enum_values: Vec<&str>, shape: Option<Value>) -> StepParamDto {
    StepParamDto {
        mode: mode.to_string(),
        required,
        enum_values: enum_values.into_iter().map(str::to_string).collect(),
        shape,
    }
}

fn map<T>(entries: Vec<(&str, T)>) -> BTreeMap<String, T> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn ref_filters(entries: Vec<(&str, Vec<&str>)>) -> BTreeMap<String, Vec<String>> {
    entries
        .into_iter()
        .map(|(key, values)| {
            (
                key.to_string(),
                values.into_iter().map(str::to_string).collect(),
            )
        })
        .collect()
}

fn artifact_list_shape() -> Value {
    json!({
        "fields": {},
        "itemKind": "string",
        "kind": "list",
        "ordered": true,
        "target": "artifact",
        "unique": true,
    })
}

fn artifact_group_list_shape() -> Value {
    json!({
        "fields": {},
        "itemKind": "string",
        "kind": "list",
        "ordered": true,
        "target": "artifact_group",
        "unique": true,
    })
}

fn runtime_permissions_shape() -> Value {
    json!({
        "fields": {
            "name": {
                "enumValues": [],
                "kind": "string",
                "required": true,
            },
            "package_name": {
                "enumValues": [],
                "kind": "string",
                "required": true,
            },
        },
        "itemKind": "object",
        "kind": "list",
        "ordered": true,
        "unique": false,
    })
}

fn appops_shape() -> Value {
    json!({
        "fields": {
            "mode": {
                "enumValues": [],
                "kind": "string",
                "required": true,
            },
            "op": {
                "enumValues": [],
                "kind": "string",
                "required": true,
            },
            "package_name": {
                "enumValues": [],
                "kind": "string",
                "required": true,
            },
        },
        "itemKind": "object",
        "kind": "list",
        "ordered": true,
        "unique": false,
    })
}

fn policy_shape() -> Value {
    json!({
        "fields": {
            "on_failure": {
                "default": "warn",
                "enumValues": ["warn", "fail"],
                "kind": "string",
                "required": false,
            },
            "require_all": {
                "default": false,
                "enumValues": [],
                "kind": "boolean",
                "required": false,
            },
        },
        "kind": "object",
        "ordered": false,
        "unique": false,
    })
}
