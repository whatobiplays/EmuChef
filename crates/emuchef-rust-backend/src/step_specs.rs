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
    #[serde(rename = "acceptedSources")]
    pub accepted_sources: Vec<String>,
    #[serde(rename = "acceptedValueTypes")]
    pub accepted_value_types: Vec<String>,
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
                "resolve_remote_release",
                "Resolve Remote Release",
                Some("download_url"),
                vec![
                    output("download_url", "string", true),
                    output("asset_name", "string", false),
                    output("release_tag", "string", false),
                    output("published_at", "string", false),
                    output("size", "integer", false),
                ],
                vec![
                    "provider",
                    "base_url",
                    "repository",
                    "include_prereleases",
                    "asset_pattern",
                ],
                map(vec![
                    (
                        "provider",
                        param(
                            &["literal"],
                            &["string"],
                            true,
                            vec!["github", "gitlab", "forgejo"],
                            None,
                        ),
                    ),
                    (
                        "base_url",
                        param(&["literal"], &["string"], true, vec![], None),
                    ),
                    (
                        "repository",
                        param(&["literal"], &["string"], true, vec![], None),
                    ),
                    (
                        "include_prereleases",
                        param(&["literal"], &["boolean"], false, vec![], None),
                    ),
                    (
                        "asset_pattern",
                        param(&["literal"], &["string"], true, vec![], None),
                    ),
                ]),
                map(vec![("include_prereleases", json!(false))]),
            ),
            spec(
                "resolve_github_release",
                "Resolve GitHub Release",
                Some("download_url"),
                vec![
                    output("download_url", "string", true),
                    output("asset_name", "string", false),
                    output("release_tag", "string", false),
                    output("published_at", "string", false),
                    output("size", "integer", false),
                ],
                vec!["repository", "include_prereleases", "asset_pattern"],
                map(vec![
                    (
                        "repository",
                        param(&["literal"], &["string"], true, vec![], None),
                    ),
                    (
                        "include_prereleases",
                        param(&["literal"], &["boolean"], false, vec![], None),
                    ),
                    (
                        "asset_pattern",
                        param(&["literal"], &["string"], true, vec![], None),
                    ),
                ]),
                map(vec![("include_prereleases", json!(false))]),
            ),
            spec(
                "download_remote_file",
                "Download Remote File",
                Some("local_path"),
                vec![
                    output("local_path", "file_path", true),
                    output("filename", "string", false),
                ],
                vec!["url", "cache"],
                map(vec![
                    (
                        "url",
                        param(
                            &["literal", "step_output_ref"],
                            &["string"],
                            true,
                            vec![],
                            None,
                        ),
                    ),
                    (
                        "cache",
                        param(
                            &["literal"],
                            &["string"],
                            false,
                            vec!["default", "none"],
                            None,
                        ),
                    ),
                ]),
                map(vec![("cache", json!("default"))]),
            ),
            spec(
                "resolve_artifacts",
                "Resolve Artifacts",
                None,
                vec![],
                vec!["artifacts", "artifact_groups"],
                map(vec![
                    (
                        "artifact_groups",
                        param(
                            &["literal"],
                            &["string_list"],
                            false,
                            vec![],
                            Some(artifact_group_list_shape()),
                        ),
                    ),
                    (
                        "artifacts",
                        param(
                            &["literal"],
                            &["string_list"],
                            false,
                            vec![],
                            Some(artifact_list_shape()),
                        ),
                    ),
                ]),
                map(vec![]),
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
                        param(
                            &["literal"],
                            &["string_list"],
                            false,
                            vec![],
                            Some(artifact_group_list_shape()),
                        ),
                    ),
                    (
                        "artifacts",
                        param(
                            &["literal"],
                            &["string_list"],
                            false,
                            vec![],
                            Some(artifact_list_shape()),
                        ),
                    ),
                    (
                        "extract_on",
                        param(
                            &["literal"],
                            &["string"],
                            false,
                            vec!["host", "device"],
                            None,
                        ),
                    ),
                ]),
                map(vec![("extract_on", json!("host"))]),
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
                    (
                        "archive",
                        param(
                            &["input_ref", "artifact_ref", "step_output_ref"],
                            &["file_path"],
                            true,
                            vec![],
                            None,
                        ),
                    ),
                    (
                        "cleanup",
                        param(&["literal"], &["boolean"], false, vec![], None),
                    ),
                    (
                        "dest",
                        param(&["literal"], &["string"], false, vec![], None),
                    ),
                    (
                        "device_temp_path",
                        param(&["literal"], &["device_path"], false, vec![], None),
                    ),
                    (
                        "extract_on",
                        param(
                            &["literal"],
                            &["string"],
                            false,
                            vec!["host", "device"],
                            None,
                        ),
                    ),
                ]),
                map(vec![
                    ("cleanup", json!(true)),
                    ("extract_on", json!("host")),
                ]),
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
                        param(
                            &["literal", "input_ref"],
                            &["string"],
                            false,
                            vec!["merge", "sync", "replace"],
                            None,
                        ),
                    ),
                    (
                        "dest",
                        param(
                            &["literal", "input_ref"],
                            &["device_path"],
                            true,
                            vec![],
                            None,
                        ),
                    ),
                    (
                        "source",
                        param(
                            &["input_ref", "artifact_ref", "step_output_ref"],
                            &["file_path", "directory_path", "path_list"],
                            true,
                            vec![],
                            None,
                        ),
                    ),
                ]),
                map(vec![("copy_policy", json!("merge"))]),
            ),
            spec(
                "install_apk",
                "Install APK",
                None,
                vec![],
                vec!["app", "expected_package_name", "replace_existing"],
                map(vec![
                    (
                        "app",
                        param(
                            &["input_ref", "artifact_ref", "step_output_ref"],
                            &["file_path"],
                            true,
                            vec![],
                            None,
                        ),
                    ),
                    (
                        "expected_package_name",
                        param(&["literal"], &["string"], false, vec![], None),
                    ),
                    (
                        "replace_existing",
                        param(&["literal"], &["boolean"], false, vec![], None),
                    ),
                ]),
                map(vec![("replace_existing", json!(false))]),
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
                        param(
                            &["literal"],
                            &["object"],
                            false,
                            vec![],
                            Some(appops_shape()),
                        ),
                    ),
                    (
                        "policy",
                        param(
                            &["literal"],
                            &["object"],
                            false,
                            vec![],
                            Some(policy_shape()),
                        ),
                    ),
                    (
                        "runtime",
                        param(
                            &["literal"],
                            &["object"],
                            false,
                            vec![],
                            Some(runtime_permissions_shape()),
                        ),
                    ),
                ]),
                map(vec![]),
            ),
            spec(
                "launch_app",
                "Launch App",
                None,
                vec![],
                vec!["package_name", "activity"],
                map(vec![
                    (
                        "activity",
                        param(&["literal"], &["string"], false, vec![], None),
                    ),
                    (
                        "package_name",
                        param(&["literal"], &["string"], true, vec![], None),
                    ),
                ]),
                map(vec![]),
            ),
            spec(
                "wait",
                "Wait",
                None,
                vec![],
                vec!["duration_ms"],
                map(vec![(
                    "duration_ms",
                    param(&["literal"], &["integer"], true, vec![], None),
                )]),
                map(vec![]),
            ),
            spec(
                "force_stop_app",
                "Force Stop",
                None,
                vec![],
                vec!["package_name"],
                map(vec![(
                    "package_name",
                    param(&["literal"], &["string"], true, vec![], None),
                )]),
                map(vec![]),
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
    }
}

fn output(name: &str, value_type: &str, primary: bool) -> StepOutputDto {
    StepOutputDto {
        name: name.to_string(),
        value_type: value_type.to_string(),
        primary,
    }
}

fn param(
    accepted_sources: &[&str],
    accepted_value_types: &[&str],
    required: bool,
    enum_values: Vec<&str>,
    shape: Option<Value>,
) -> StepParamDto {
    StepParamDto {
        accepted_sources: accepted_sources.iter().map(ToString::to_string).collect(),
        accepted_value_types: accepted_value_types
            .iter()
            .map(ToString::to_string)
            .collect(),
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
