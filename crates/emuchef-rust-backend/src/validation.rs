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
use crate::runtime_refs::{
    artifact_field_value_type, input_value_type, parse_reference, RuntimeRef,
};
use crate::step_specs::{self, StepParamDto, StepSpecDto};
use crate::yaml::{self, LoadErrorKind, LoadIssue, RecipeLoadError};

/// Validate and canonicalize a trusted SHA-256 value.
///
/// Only ASCII whitespace surrounding the complete digest is ignored. The
/// digest itself must contain exactly 64 hexadecimal characters; prefixes,
/// separators, and embedded whitespace are deliberately rejected.
pub(crate) fn normalize_expected_sha256(value: &str) -> Option<String> {
    let trimmed = value.trim_matches(|character: char| character.is_ascii_whitespace());
    (trimmed.len() == 64 && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| trimmed.to_ascii_uppercase())
}

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
        match value {
            ParamValue::Literal(_) if !accepts_source(param_spec, "literal") => {
                errors.push(param_contract_error(
                    file,
                    recipe,
                    step_index,
                    param_name,
                    &format!(
                        "Param {} does not accept literal values for step type {}.",
                        single_quote(param_name),
                        single_quote(&step.type_name)
                    ),
                ))
            }
            _ => {}
        }
        if step.type_name == "install_apk" && param_name == "expected_package_name" {
            if let ParamValue::Literal(value) = value {
                if !matches!(value, Value::String(value) if !value.trim().is_empty()) {
                    errors.push(param_contract_error(
                        file,
                        recipe,
                        step_index,
                        param_name,
                        "Param 'expected_package_name' must be a non-empty string literal for step type 'install_apk'.",
                    ));
                }
            }
        }
        if step.type_name == "install_apk" && param_name == "expected_sha256" {
            if let ParamValue::Literal(value) = value {
                if !matches!(value, Value::String(value) if normalize_expected_sha256(value).is_some())
                {
                    errors.push(param_contract_error(
                        file,
                        recipe,
                        step_index,
                        param_name,
                        "Param 'expected_sha256' must be a 64-character hexadecimal string literal for step type 'install_apk'.",
                    ));
                }
            }
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
        let step_spec = step_specs::step_spec_for(&step.type_name);
        for (param_name, value) in &step.params {
            let ParamValue::Ref(ref_value) = value else {
                continue;
            };
            let param_spec = step_spec
                .as_ref()
                .and_then(|spec| spec.params.get(param_name));
            match parse_reference(ref_value) {
                Ok(RuntimeRef::Input { target_id }) => {
                    if source_is_rejected(
                        file,
                        recipe,
                        step_index,
                        param_name,
                        ref_value,
                        param_spec,
                        "input_ref",
                        &mut errors,
                    ) {
                        continue;
                    }
                    if let Some(input) = recipe.inputs.get(&target_id) {
                        validate_ref_value_type(
                            file,
                            recipe,
                            step_index,
                            param_name,
                            ref_value,
                            param_spec,
                            input_value_type(&input.type_name, input.multiple),
                            &mut errors,
                        );
                    } else {
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
                    if source_is_rejected(
                        file,
                        recipe,
                        step_index,
                        param_name,
                        ref_value,
                        param_spec,
                        "artifact_ref",
                        &mut errors,
                    ) {
                        continue;
                    }
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
                    } else if let Some(value_type) = artifact_field_value_type(&field) {
                        validate_ref_value_type(
                            file,
                            recipe,
                            step_index,
                            param_name,
                            ref_value,
                            param_spec,
                            value_type,
                            &mut errors,
                        );
                    } else {
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
                    if source_is_rejected(
                        file,
                        recipe,
                        step_index,
                        param_name,
                        ref_value,
                        param_spec,
                        "step_output_ref",
                        &mut errors,
                    ) {
                        continue;
                    }
                    let Some(target_step) = step_by_id.get(target_id.as_str()) else {
                        errors.push(unknown_step_ref_error(
                            file, recipe, step_index, step, param_name, &target_id,
                        ));
                        continue;
                    };
                    let Some(output) = primary_output(target_step) else {
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
                        continue;
                    };
                    validate_ref_value_type(
                        file,
                        recipe,
                        step_index,
                        param_name,
                        ref_value,
                        param_spec,
                        &output.value_type,
                        &mut errors,
                    );
                }
                Ok(RuntimeRef::StepOutput { target_id, field }) => {
                    if source_is_rejected(
                        file,
                        recipe,
                        step_index,
                        param_name,
                        ref_value,
                        param_spec,
                        "step_output_ref",
                        &mut errors,
                    ) {
                        continue;
                    }
                    let Some(target_step) = step_by_id.get(target_id.as_str()) else {
                        errors.push(unknown_step_ref_error(
                            file, recipe, step_index, step, param_name, &target_id,
                        ));
                        continue;
                    };
                    let output =
                        step_specs::step_spec_for(&target_step.type_name).and_then(|spec| {
                            spec.outputs.into_iter().find(|output| output.name == field)
                        });
                    if let Some(output) = output {
                        validate_ref_value_type(
                            file,
                            recipe,
                            step_index,
                            param_name,
                            ref_value,
                            param_spec,
                            &output.value_type,
                            &mut errors,
                        );
                    } else {
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

fn primary_output(step: &Step) -> Option<crate::step_specs::StepOutputDto> {
    let spec = step_specs::step_spec_for(&step.type_name)?;
    let primary = spec.primary_output_name?;
    spec.outputs
        .into_iter()
        .find(|output| output.name == primary)
}

fn accepts_source(spec: &StepParamDto, source: &str) -> bool {
    spec.accepted_sources
        .iter()
        .any(|accepted| accepted == source)
}

#[allow(clippy::too_many_arguments)]
fn source_is_rejected(
    file: &str,
    recipe: &Recipe,
    step_index: usize,
    param_name: &str,
    ref_value: &str,
    spec: Option<&StepParamDto>,
    source: &str,
    errors: &mut Vec<Value>,
) -> bool {
    if spec.is_none_or(|spec| accepts_source(spec, source)) {
        return false;
    }
    errors.push(ref_error(
        file,
        recipe,
        step_index,
        param_name,
        "param_source_not_accepted",
        &format!(
            "Param {} on step {} does not accept {} ref {}.",
            single_quote(param_name),
            single_quote(&recipe.steps[step_index].id),
            source,
            single_quote(ref_value)
        ),
    ));
    true
}

#[allow(clippy::too_many_arguments)]
fn validate_ref_value_type(
    file: &str,
    recipe: &Recipe,
    step_index: usize,
    param_name: &str,
    ref_value: &str,
    spec: Option<&StepParamDto>,
    actual_type: &str,
    errors: &mut Vec<Value>,
) {
    let Some(spec) = spec else {
        return;
    };
    if spec.accepted_value_types.is_empty()
        || spec
            .accepted_value_types
            .iter()
            .any(|accepted| accepted == actual_type)
    {
        return;
    }
    errors.push(ref_error(
        file,
        recipe,
        step_index,
        param_name,
        "param_value_type_not_accepted",
        &format!(
            "Param {} on step {} does not accept ref {} with value type {}.",
            single_quote(param_name),
            single_quote(&recipe.steps[step_index].id),
            single_quote(ref_value),
            single_quote(actual_type)
        ),
    ));
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
