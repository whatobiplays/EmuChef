//! Basic Python-shaped authored recipe validation for Phase 6E.
//!
//! This module returns editor diagnostic DTOs for focused YAML/load checks only.
//! It does not perform catalog, planner-contract, dependency graph, ref
//! existence, artifact expansion, device, or executor validation.

use std::path::Path;

use serde_json::{json, Value};

use crate::yaml::{self, LoadErrorKind, LoadIssue, RecipeLoadError};

pub fn validate_recipe_path_result(path: impl AsRef<Path>, authored_root_provided: bool) -> Value {
    let path = path.as_ref();
    let file = yaml::resolved_path_string(path);
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    match yaml::load_yaml_mapping(path) {
        Ok(raw) => match yaml::parse_recipe_mapping(&raw, path) {
            Ok(recipe) => {
                if !authored_root_provided {
                    warnings.push(limited_context_warning(
                        &file,
                        Some("recipe"),
                        Some(recipe.id.as_str()),
                    ));
                }
                if let Some(issue) = yaml::unsupported_step_issue(&recipe) {
                    errors.push(issue_to_diagnostic("error", &file, &issue));
                }
            }
            Err(error) => {
                if !authored_root_provided {
                    warnings.push(limited_context_warning(&file, None, None));
                }
                errors.push(load_error_to_diagnostic(&file, path, error));
            }
        },
        Err(error) => {
            if !authored_root_provided {
                warnings.push(limited_context_warning(&file, None, None));
            }
            errors.push(load_error_to_diagnostic(&file, path, error));
        }
    }

    let diagnostics: Vec<Value> = warnings.into_iter().chain(errors).collect();
    json!({ "diagnostics": diagnostics })
}

fn limited_context_warning(
    file: &str,
    object_kind: Option<&str>,
    object_id: Option<&str>,
) -> Value {
    diagnostic(
        "warning",
        "validation_context_limited",
        "Cross-file validation was limited because no authored_root was provided.",
        file,
        object_kind,
        object_id,
        None,
    )
}

fn load_error_to_diagnostic(file: &str, path: &Path, error: RecipeLoadError) -> Value {
    if let Some(issue) = error.issue {
        return issue_to_diagnostic("error", file, &issue);
    }

    match error.kind {
        LoadErrorKind::YamlParse => diagnostic(
            "error",
            "authored_data_invalid",
            &format!(
                "File '{}' could not be parsed as YAML: {}.",
                path.file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_else(|| path.to_string_lossy()),
                error.message
            ),
            file,
            None,
            None,
            None,
        ),
        LoadErrorKind::Io => diagnostic(
            "error",
            "authored_data_invalid",
            &format!("File {} was not found.", path.display()),
            file,
            None,
            None,
            None,
        ),
        LoadErrorKind::AuthoredData => diagnostic(
            "error",
            "authored_data_invalid",
            &error.message,
            file,
            None,
            None,
            None,
        ),
    }
}

fn issue_to_diagnostic(severity: &str, file: &str, issue: &LoadIssue) -> Value {
    diagnostic(
        severity,
        issue.code,
        &issue.message,
        file,
        issue.object_kind.as_deref(),
        issue.object_id.as_deref(),
        issue.field.as_deref(),
    )
}

fn diagnostic(
    severity: &str,
    code: &str,
    message: &str,
    file: &str,
    object_kind: Option<&str>,
    object_id: Option<&str>,
    field: Option<&str>,
) -> Value {
    json!({
        "severity": severity,
        "code": code,
        "message": message,
        "file": file,
        "objectKind": object_kind,
        "objectId": object_id,
        "field": field,
    })
}
