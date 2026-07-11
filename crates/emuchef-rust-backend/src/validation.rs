//! Authored recipe and catalog validation.
//!
//! This module returns editor diagnostic DTOs for validation that can be
//! computed from one loaded recipe, embedded StepSpec metadata, and the local
//! recipe surface. With an authored root it also runs catalog-level
//! catalog-context recipe diagnostics. It intentionally does not perform planner
//! graph construction, executor, device, or network validation.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{json, Value};

use crate::catalog;
use crate::model::{ParamValue, Recipe, Step};
use crate::step_specs::{self, StepSpecDto};
use crate::yaml::{self, LoadErrorKind, LoadIssue, RecipeLoadError};

const RUNTIME_ARTIFACT_FIELDS: &[&str] = &[
    "status",
    "local_path",
    "resolved_url",
    "filename",
    "cache_hit",
    "error",
];

pub fn validate_recipe_path_result(path: impl AsRef<Path>, authored_root: Option<&Path>) -> Value {
    let path = path.as_ref();
    let file = yaml::resolved_path_string(path);
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    match yaml::load_yaml_mapping(path) {
        Ok(raw) => match yaml::parse_recipe_mapping(&raw, path) {
            Ok(recipe) => {
                return json!({
                    "diagnostics": diagnostics_for_loaded_recipe(&file, &recipe, path, authored_root)
                });
            }
            Err(error) => {
                if authored_root.is_none() {
                    warnings.push(limited_context_warning(&file, None, None));
                }
                errors.push(load_error_to_diagnostic(&file, path, error));
            }
        },
        Err(error) => {
            if authored_root.is_none() {
                warnings.push(limited_context_warning(&file, None, None));
            }
            errors.push(load_error_to_diagnostic(&file, path, error));
        }
    }

    let diagnostics: Vec<Value> = warnings.into_iter().chain(errors).collect();
    json!({ "diagnostics": diagnostics })
}

pub fn validate_loaded_recipe_result(
    recipe: &Recipe,
    path: impl AsRef<Path>,
    authored_root: Option<&Path>,
) -> Value {
    let path = path.as_ref();
    let file = yaml::resolved_path_string(path);
    json!({ "diagnostics": diagnostics_for_loaded_recipe(&file, recipe, path, authored_root) })
}

fn diagnostics_for_loaded_recipe(
    file: &str,
    recipe: &Recipe,
    path: &Path,
    authored_root: Option<&Path>,
) -> Vec<Value> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if authored_root.is_none() {
        warnings.push(limited_context_warning(
            file,
            Some("recipe"),
            Some(recipe.id.as_str()),
        ));
    }
    errors.extend(validate_step_dependencies(file, recipe));
    errors.extend(validate_step_contracts(file, recipe));
    errors.extend(validate_step_references(file, recipe));
    if let Some(authored_root) = authored_root {
        errors.extend(catalog::validate_recipe_with_catalog(
            file,
            recipe,
            path,
            authored_root,
        ));
    }

    warnings.into_iter().chain(errors).collect()
}

fn validate_step_dependencies(file: &str, recipe: &Recipe) -> Vec<Value> {
    let by_id = recipe
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| (step.id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut permanent = HashSet::new();
    let mut temporary = HashSet::new();
    let mut missing_dependencies = HashSet::new();
    let mut errors = Vec::new();

    for step in &recipe.steps {
        visit_step_dependency(
            file,
            recipe,
            &by_id,
            &step.id,
            &mut Vec::new(),
            &mut permanent,
            &mut temporary,
            &mut missing_dependencies,
            &mut errors,
        );
    }

    errors
}

#[allow(clippy::too_many_arguments)]
fn visit_step_dependency(
    file: &str,
    recipe: &Recipe,
    by_id: &HashMap<String, usize>,
    step_id: &str,
    stack: &mut Vec<String>,
    permanent: &mut HashSet<String>,
    temporary: &mut HashSet<String>,
    missing_dependencies: &mut HashSet<(String, String)>,
    errors: &mut Vec<Value>,
) {
    if permanent.contains(step_id) {
        return;
    }
    if temporary.contains(step_id) {
        errors.push(diagnostic(
            "error",
            "dependency_cycle",
            &format!(
                "Step dependency cycle detected in recipe {}.",
                single_quote(&recipe.id)
            ),
            file,
            Some("recipe"),
            Some(&recipe.id),
            Some("steps"),
        ));
        return;
    }
    let Some(&index) = by_id.get(step_id) else {
        let Some(dependent_step_id) = stack.last() else {
            return;
        };
        let missing_key = (dependent_step_id.clone(), step_id.to_string());
        if !missing_dependencies.insert(missing_key) {
            return;
        }
        let field = dependency_field(recipe, dependent_step_id, step_id)
            .unwrap_or_else(|| "steps".to_string());
        errors.push(diagnostic(
            "error",
            "step_not_found",
            &format!(
                "Step {} depends on unknown step {}.",
                single_quote(dependent_step_id),
                single_quote(step_id)
            ),
            file,
            Some("recipe"),
            Some(&recipe.id),
            Some(&field),
        ));
        return;
    };

    temporary.insert(step_id.to_string());
    stack.push(step_id.to_string());
    for dependency in &recipe.steps[index].dependencies {
        visit_step_dependency(
            file,
            recipe,
            by_id,
            dependency,
            stack,
            permanent,
            temporary,
            missing_dependencies,
            errors,
        );
    }
    stack.pop();
    temporary.remove(step_id);
    permanent.insert(step_id.to_string());
}

fn dependency_field(recipe: &Recipe, step_id: &str, dependency_id: &str) -> Option<String> {
    let step_index = recipe.steps.iter().position(|step| step.id == step_id)?;
    let dependency_index = recipe.steps[step_index]
        .dependencies
        .iter()
        .position(|dependency| dependency == dependency_id)?;
    Some(format!(
        "steps[{step_index}].dependencies[{dependency_index}]"
    ))
}

fn validate_step_contracts(file: &str, recipe: &Recipe) -> Vec<Value> {
    let mut errors = Vec::new();
    for (step_index, step) in recipe.steps.iter().enumerate() {
        let Some(spec) = step_specs::step_spec_for(&step.type_name) else {
            errors.push(diagnostic(
                "error",
                "param_contract_violation",
                &format!("Unsupported step type {}.", single_quote(&step.type_name)),
                file,
                Some("recipe"),
                Some(&recipe.id),
                None,
            ));
            continue;
        };
        errors.extend(validate_step_params(file, recipe, step_index, step, &spec));
    }
    errors
}

fn validate_step_params(
    file: &str,
    recipe: &Recipe,
    step_index: usize,
    step: &Step,
    spec: &StepSpecDto,
) -> Vec<Value> {
    let mut errors = Vec::new();
    let expected = spec.params.keys().cloned().collect::<HashSet<_>>();
    let mut unexpected = step
        .params
        .keys()
        .filter(|param_name| !expected.contains(*param_name))
        .cloned()
        .collect::<Vec<_>>();
    unexpected.sort();
    for param_name in unexpected {
        errors.push(param_contract_error(
            file,
            recipe,
            step_index,
            &param_name,
            &format!(
                "Unexpected param {} for step type {}.",
                single_quote(&param_name),
                single_quote(&step.type_name)
            ),
        ));
    }

    for (param_name, param_spec) in &spec.params {
        let has_value =
            step.params.contains_key(param_name) || spec.defaults.contains_key(param_name);
        if param_spec.required && !has_value {
            errors.push(param_contract_error(
                file,
                recipe,
                step_index,
                param_name,
                &format!(
                    "Missing required param {} for step type {}.",
                    single_quote(param_name),
                    single_quote(&step.type_name)
                ),
            ));
            continue;
        }

        let Some(value) = step.params.get(param_name) else {
            continue;
        };
        match (param_spec.mode.as_str(), value) {
            ("ref", ParamValue::Literal(_)) => errors.push(param_contract_error(
                file,
                recipe,
                step_index,
                param_name,
                &format!(
                    "Param {} must use {{ref: ...}} for step type {}.",
                    single_quote(param_name),
                    single_quote(&step.type_name)
                ),
            )),
            ("literal", ParamValue::Ref(_)) => errors.push(param_contract_error(
                file,
                recipe,
                step_index,
                param_name,
                &format!(
                    "Param {} must remain a literal for step type {}.",
                    single_quote(param_name),
                    single_quote(&step.type_name)
                ),
            )),
            _ => {}
        }
    }
    errors
}

fn validate_step_references(file: &str, recipe: &Recipe) -> Vec<Value> {
    let mut errors = Vec::new();
    let step_by_id = recipe
        .steps
        .iter()
        .map(|step| (step.id.as_str(), step))
        .collect::<HashMap<_, _>>();

    for (step_index, step) in recipe.steps.iter().enumerate() {
        for (param_name, value) in &step.params {
            let ParamValue::Ref(ref_value) = value else {
                continue;
            };
            match parse_reference(ref_value) {
                Ok(RuntimeRef::Input { target_id }) => {
                    if !recipe.inputs.contains_key(&target_id) {
                        errors.push(ref_error(
                            file,
                            recipe,
                            step_index,
                            param_name,
                            "unknown_input_ref",
                            &format!(
                                "Step {} references unknown input {}.",
                                single_quote(&step.id),
                                single_quote(&target_id)
                            ),
                        ));
                    }
                }
                Ok(RuntimeRef::ArtifactField { target_id, field }) => {
                    if !recipe.artifacts.contains_key(&target_id) {
                        errors.push(ref_error(
                            file,
                            recipe,
                            step_index,
                            param_name,
                            "unknown_artifact_ref",
                            &format!(
                                "Step {} references unknown artifact {}.",
                                single_quote(&step.id),
                                single_quote(&target_id)
                            ),
                        ));
                    } else if !RUNTIME_ARTIFACT_FIELDS.contains(&field.as_str()) {
                        errors.push(ref_error(
                            file,
                            recipe,
                            step_index,
                            param_name,
                            "unknown_artifact_field",
                            &format!(
                                "Artifact ref {} uses unknown field {}.",
                                single_quote(ref_value),
                                single_quote(&field)
                            ),
                        ));
                    }
                }
                Ok(RuntimeRef::StepShorthand { target_id }) => {
                    let Some(target_step) = step_by_id.get(target_id.as_str()) else {
                        errors.push(unknown_step_ref_error(
                            file, recipe, step_index, step, param_name, &target_id,
                        ));
                        continue;
                    };
                    if primary_output_name(target_step).is_none() {
                        errors.push(ref_error(
                            file,
                            recipe,
                            step_index,
                            param_name,
                            "unknown_step_output",
                            &format!(
                                "Step shorthand ref {} targets step type {}, which has no primary output.",
                                single_quote(ref_value),
                                single_quote(&target_step.type_name)
                            ),
                        ));
                    }
                }
                Ok(RuntimeRef::StepOutput { target_id, field }) => {
                    let Some(target_step) = step_by_id.get(target_id.as_str()) else {
                        errors.push(unknown_step_ref_error(
                            file, recipe, step_index, step, param_name, &target_id,
                        ));
                        continue;
                    };
                    if primary_output_name(target_step).as_deref() != Some(field.as_str()) {
                        errors.push(ref_error(
                            file,
                            recipe,
                            step_index,
                            param_name,
                            "unknown_step_output",
                            &format!(
                                "Step output ref {} targets unsupported output {}.",
                                single_quote(ref_value),
                                single_quote(&field)
                            ),
                        ));
                    }
                }
                Err(()) => errors.push(ref_error(
                    file,
                    recipe,
                    step_index,
                    param_name,
                    "invalid_ref_format",
                    &format!(
                        "Param {} on step {} has an invalid ref {}.",
                        single_quote(param_name),
                        single_quote(&step.id),
                        single_quote(ref_value)
                    ),
                )),
            }
        }
    }

    errors
}

fn unknown_step_ref_error(
    file: &str,
    recipe: &Recipe,
    step_index: usize,
    step: &Step,
    param_name: &str,
    target_id: &str,
) -> Value {
    ref_error(
        file,
        recipe,
        step_index,
        param_name,
        "unknown_step_ref",
        &format!(
            "Step {} references unknown step {}.",
            single_quote(&step.id),
            single_quote(target_id)
        ),
    )
}

fn primary_output_name(step: &Step) -> Option<String> {
    step_specs::step_spec_for(&step.type_name).and_then(|spec| spec.primary_output_name)
}

enum RuntimeRef {
    Input { target_id: String },
    ArtifactField { target_id: String, field: String },
    StepShorthand { target_id: String },
    StepOutput { target_id: String, field: String },
}

fn parse_reference(value: &str) -> Result<RuntimeRef, ()> {
    if let Some(target_id) = value.strip_prefix("inputs.") {
        if !target_id.is_empty() {
            return Ok(RuntimeRef::Input {
                target_id: target_id.to_string(),
            });
        }
        return Err(());
    }

    if let Some(step_body) = value.strip_prefix("steps.") {
        if let Some((step_id, output_name)) = step_body.split_once(".outputs.") {
            if !step_id.is_empty() && !output_name.is_empty() {
                return Ok(RuntimeRef::StepOutput {
                    target_id: step_id.to_string(),
                    field: output_name.to_string(),
                });
            }
            return Err(());
        }
        if !step_body.is_empty() {
            return Ok(RuntimeRef::StepShorthand {
                target_id: step_body.to_string(),
            });
        }
        return Err(());
    }

    if let Some(body) = value.strip_prefix("artifacts.") {
        if let Some((artifact_id, field)) = body.rsplit_once('.') {
            if !artifact_id.is_empty() && !field.is_empty() {
                return Ok(RuntimeRef::ArtifactField {
                    target_id: artifact_id.to_string(),
                    field: field.to_string(),
                });
            }
        }
        return Err(());
    }

    Err(())
}

fn param_contract_error(
    file: &str,
    recipe: &Recipe,
    step_index: usize,
    param_name: &str,
    message: &str,
) -> Value {
    diagnostic(
        "error",
        "param_contract_violation",
        message,
        file,
        Some("recipe"),
        Some(&recipe.id),
        Some(&format!("steps[{step_index}].params.{param_name}")),
    )
}

fn ref_error(
    file: &str,
    recipe: &Recipe,
    step_index: usize,
    param_name: &str,
    code: &str,
    message: &str,
) -> Value {
    diagnostic(
        "error",
        code,
        message,
        file,
        Some("recipe"),
        Some(&recipe.id),
        Some(&format!("steps[{step_index}].params.{param_name}")),
    )
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

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}
