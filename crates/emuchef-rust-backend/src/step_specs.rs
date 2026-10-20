//! Rust-owned StepSpec DTO metadata for the editor protocol.
//!
//! This module owns the static StepSpec surface returned by `listStepSpecs` and
//! consumed by editor validation, ref indexing, planner scaffolding, and the
//! Tauri frontend. Keep the DTO shapes stable unless the editor protocol and
//! frontend expectations change together.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::Step;

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
    #[serde(rename = "appContext")]
    pub app_context: AppContextRequirement,
    #[serde(rename = "primaryOutputName")]
    pub primary_output_name: Option<String>,
    pub outputs: Vec<StepOutputDto>,
    #[serde(rename = "paramOrder")]
    pub param_order: Vec<String>,
    pub params: BTreeMap<String, StepParamDto>,
    pub defaults: BTreeMap<String, Value>,
}

/// Whether a step type can use an App Definition as its execution context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppContextRequirement {
    None,
    Optional,
    Required,
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
                            &["literal", "input_ref", "app_target"],
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
                vec!["app", "replace_existing"],
                map(vec![
                    (
                        "app",
                        param(&["artifact_ref"], &["file_path"], true, vec![], None),
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
                vec!["permission_set", "policy"],
                map(vec![
                    (
                        "permission_set",
                        param(
                            &["literal"],
                            &["string"],
                            true,
                            vec!["baseline", "elevated"],
                            None,
                        ),
                    ),
                    (
                        "policy",
                        param(
                            &["literal"],
                            &["object"],
                            true,
                            vec![],
                            Some(permission_policy_shape()),
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
                vec![],
                map(vec![]),
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
                vec![],
                map(vec![]),
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

/// Return the canonical authoring contract for an authored step.
pub(crate) fn authored_step_spec_for(step: &Step) -> Option<StepSpecDto> {
    step_spec_for(&step.type_name)
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
        app_context: match type_name {
            "install_apk" | "grant_permissions" | "launch_app" | "force_stop_app" => {
                AppContextRequirement::Required
            }
            "copy_files" | "package_installed" => AppContextRequirement::Optional,
            _ => AppContextRequirement::None,
        },
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

fn permission_policy_shape() -> Value {
    json!({
        "fields": {
            "on_failure": {
                "enumValues": ["warn", "fail"],
                "kind": "string",
                "required": true,
            },
        },
        "kind": "object",
        "ordered": false,
        "unique": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ParamValue;

    fn permission_step(app_ref: Option<&str>, permission_set: Option<&str>) -> Step {
        let mut params = crate::model::OrderedMap::new();
        if let Some(permission_set) = permission_set {
            params.insert(
                "permission_set".to_string(),
                ParamValue::Literal(json!(permission_set)),
            );
        }
        Step {
            id: "grant".to_string(),
            type_name: "grant_permissions".to_string(),
            name: "Grant permissions".to_string(),
            description: None,
            progress_note: None,
            user_toggleable: false,
            app_ref: app_ref.map(str::to_string),
            dependencies: Vec::new(),
            constraints: crate::model::StepConstraints {
                capabilities: Vec::new(),
                conflicts_with: Vec::new(),
            },
            skip_if: Vec::new(),
            params,
            verify: Vec::new(),
        }
    }

    #[test]
    fn app_context_requirement_is_typed_and_exposed_in_step_specs() {
        assert_eq!(
            serde_json::to_value(AppContextRequirement::None).unwrap(),
            json!("none")
        );
        assert_eq!(
            serde_json::to_value(AppContextRequirement::Optional).unwrap(),
            json!("optional")
        );
        assert_eq!(
            serde_json::to_value(AppContextRequirement::Required).unwrap(),
            json!("required")
        );

        for step_type in [
            "install_apk",
            "grant_permissions",
            "launch_app",
            "force_stop_app",
        ] {
            assert_eq!(
                step_spec_for(step_type).unwrap().app_context,
                AppContextRequirement::Required,
                "{step_type} uses app context in the App Definition form"
            );
        }
        assert_eq!(
            step_spec_for("copy_files").unwrap().app_context,
            AppContextRequirement::Optional
        );
        assert_eq!(
            step_spec_for("wait").unwrap().app_context,
            AppContextRequirement::None
        );
    }

    #[test]
    fn lifecycle_step_specs_expose_only_app_context_as_authored_identity() {
        for step_type in ["launch_app", "force_stop_app"] {
            let spec = step_spec_for(step_type).expect("lifecycle step spec");
            assert_eq!(spec.app_context, AppContextRequirement::Required);
            assert!(
                spec.params.is_empty(),
                "{step_type} exposes identity params"
            );
            assert!(
                spec.param_order.is_empty(),
                "{step_type} orders identity params"
            );
        }
    }

    #[test]
    fn permission_step_spec_exposes_the_closed_app_definition_form() {
        let spec = step_spec_for("grant_permissions").expect("permission step spec");

        assert_eq!(spec.param_order, ["permission_set", "policy"]);
        assert_eq!(
            spec.params.keys().cloned().collect::<Vec<_>>(),
            spec.param_order
        );
        assert_eq!(
            spec.params["permission_set"].enum_values,
            ["baseline", "elevated"]
        );
        assert!(spec.params["permission_set"].required);
        assert!(spec.params["policy"].required);
        assert_eq!(spec.defaults.len(), 0);
    }

    #[test]
    fn permission_steps_use_only_the_app_definition_contract() {
        let spec =
            authored_step_spec_for(&permission_step(None, None)).expect("permission step contract");

        assert_eq!(spec.param_order, ["permission_set", "policy"]);
        assert!(spec.params.contains_key("permission_set"));
        assert!(!spec.params.contains_key("runtime"));
        assert!(!spec.params.contains_key("appops"));
        assert!(spec.params["policy"].shape.as_ref().unwrap()["fields"]
            .get("require_all")
            .is_none());
    }

    #[test]
    fn app_definition_permission_steps_use_the_public_permission_set_contract() {
        let spec = authored_step_spec_for(&permission_step(Some("armsx1"), Some("elevated")))
            .expect("App Definition permission step contract");

        assert_eq!(spec.param_order, ["permission_set", "policy"]);
        assert!(spec.params.contains_key("permission_set"));
        assert!(!spec.params.contains_key("runtime"));
        assert!(!spec.params.contains_key("appops"));
    }

    #[test]
    fn source_specific_release_steps_are_removed_but_generic_download_remains() {
        let specs = step_specs_result()
            .step_specs
            .into_iter()
            .map(|spec| spec.type_name)
            .collect::<Vec<_>>();

        assert!(!specs.iter().any(|name| name == "resolve_remote_release"));
        assert!(!specs.iter().any(|name| name == "resolve_github_release"));
        assert!(specs.iter().any(|name| name == "download_remote_file"));
    }
}
