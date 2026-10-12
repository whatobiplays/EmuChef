//! Validation and resolution for App Definition permission actions.
//!
//! This module turns selected permission sets into immutable, step-local plans
//! and derives their applicability before capability-based step selection.

use super::*;
use crate::authored_models::AppPermissionSet;

pub(super) fn add_derived_step_capability(capabilities: &mut Vec<String>, capability: &str) {
    if !capabilities.iter().any(|existing| existing == capability) {
        capabilities.push(capability.to_string());
    }
}

pub(super) fn is_app_permission_step(step: &Step) -> bool {
    step.type_name == "grant_permissions"
        && (step.app_ref.is_some() || step.params.contains_key("permission_set"))
}

pub(super) fn validate_app_permission_step_contracts(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    for recipe_id in expanded_recipe_refs {
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        for step in &recipe.steps {
            if !is_app_permission_step(step) {
                continue;
            }
            if step.app_ref.is_none() {
                errors.push(PlannerMessage {
                    code: "permission_app_ref_required".to_string(),
                    message: format!(
                        "App-aware permission step '{}' must select an App Definition with app_ref.",
                        step.id
                    ),
                    details: json!({
                        "recipe_ref": recipe_id,
                        "step_id": step.id,
                        "param": "app_ref",
                    }),
                });
            }
            let Some(spec) = step_specs::authored_step_spec_for(step) else {
                continue;
            };
            let normalized = params_with_defaults(step);
            errors.extend(validate_unknown_params(recipe_id, step, &spec));
            errors.extend(validate_spec_param_contracts(
                recipes,
                recipe_id,
                step,
                &spec,
                &normalized,
            ));
            errors.extend(validate_focused_param_values(
                recipes,
                recipe_id,
                step,
                &normalized,
            ));
        }
    }
    errors
}

pub(super) fn resolve_app_permission_plans(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    app_definitions: &HashMap<String, AppDefinitionV1>,
    android_api_level: Option<i64>,
) -> Result<HashMap<String, PermissionPlan>, Vec<PlannerMessage>> {
    let mut plans = HashMap::new();
    let mut errors = Vec::new();
    for recipe_id in expanded_recipe_refs {
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        for step in &recipe.steps {
            if !is_app_permission_step(step) {
                continue;
            }
            let execution_step_id = make_execution_step_id(recipe_id, &step.id);
            let Some(app_ref) = step.app_ref.as_deref() else {
                continue;
            };
            let Some(app) = app_definitions.get(app_ref) else {
                continue;
            };
            let permission_set = match step.params.get("permission_set") {
                Some(ParamValue::Literal(Value::String(value))) => match value.as_str() {
                    "baseline" => PermissionSetKind::Baseline,
                    "elevated" => PermissionSetKind::Elevated,
                    _ => {
                        errors.push(permission_planning_message(
                            "permission_set_unknown",
                            format!(
                                "Permission step '{}' selects unknown set '{}'; choose baseline or elevated.",
                                step.id, value
                            ),
                            recipe_id,
                            step,
                            json!({ "permission_set": value }),
                        ));
                        continue;
                    }
                },
                _ => {
                    errors.push(permission_planning_message(
                        "permission_set_required",
                        format!(
                            "Permission step '{}' must select the literal set baseline or elevated.",
                            step.id
                        ),
                        recipe_id,
                        step,
                        json!({ "param": "permission_set" }),
                    ));
                    continue;
                }
            };
            let Some(app_permission_sets) = app.permission_sets.as_ref() else {
                errors.push(permission_planning_message(
                    "permission_set_missing",
                    format!(
                        "App Definition '{}' does not define a permission set; add the selected set or choose an app that provides it.",
                        app.id
                    ),
                    recipe_id,
                    step,
                    json!({ "app_ref": app_ref, "permission_set": permission_set }),
                ));
                continue;
            };
            let selected_set: Option<&AppPermissionSet> = match permission_set {
                PermissionSetKind::Baseline => app_permission_sets.baseline.as_ref(),
                PermissionSetKind::Elevated => app_permission_sets.elevated.as_ref(),
            };
            let Some(selected_set) = selected_set else {
                errors.push(permission_planning_message(
                    "permission_set_missing",
                    format!(
                        "App Definition '{}' has no '{}' permission set; add it or select a set that exists.",
                        app.id,
                        match permission_set {
                            PermissionSetKind::Baseline => "baseline",
                            PermissionSetKind::Elevated => "elevated",
                        }
                    ),
                    recipe_id,
                    step,
                    json!({ "app_ref": app_ref, "permission_set": permission_set }),
                ));
                continue;
            };
            let on_failure = match step.params.get("policy") {
                Some(ParamValue::Literal(Value::Object(policy))) => {
                    match policy.get("on_failure").and_then(Value::as_str) {
                        Some("warn") => PermissionFailurePolicy::Warn,
                        Some("fail") => PermissionFailurePolicy::Fail,
                        _ => {
                            errors.push(permission_planning_message(
                                "permission_failure_policy_required",
                                format!(
                                    "Permission step '{}' must set policy.on_failure to warn or fail.",
                                    step.id
                                ),
                                recipe_id,
                                step,
                                json!({ "param": "policy.on_failure" }),
                            ));
                            continue;
                        }
                    }
                }
                _ => {
                    errors.push(permission_planning_message(
                        "permission_failure_policy_required",
                        format!(
                            "Permission step '{}' must set policy.on_failure to warn or fail.",
                            step.id
                        ),
                        recipe_id,
                        step,
                        json!({ "param": "policy.on_failure" }),
                    ));
                    continue;
                }
            };
            let mut actions =
                Vec::with_capacity(selected_set.runtime.len() + selected_set.app_ops.len());
            let mut action_errors = Vec::new();
            for (index, action) in selected_set.runtime.iter().enumerate() {
                match permission_action_applicability(
                    android_api_level,
                    action.android_api_min,
                    action.android_api_max,
                ) {
                    Ok(applicability) => actions.push(PermissionPlanAction::RuntimePermission {
                        package_name: app.package_id.clone(),
                        permission: action.permission.clone(),
                        android_api_min: action.android_api_min,
                        android_api_max: action.android_api_max,
                        applicability,
                    }),
                    Err(()) => action_errors.push(permission_planning_message(
                        "permission_api_level_unknown",
                        format!(
                            "Permission step '{}' cannot resolve runtime action '{}' because the Android API level is unknown; provide the API level or remove the action's API bounds.",
                            step.id, action.permission
                        ),
                        recipe_id,
                        step,
                        json!({
                            "app_ref": app_ref,
                            "permission_set": permission_set,
                            "source_section": "runtime",
                            "action_index": index,
                            "permission": action.permission,
                            "android_api_min": action.android_api_min,
                            "android_api_max": action.android_api_max,
                        }),
                    )),
                }
            }
            for (index, action) in selected_set.app_ops.iter().enumerate() {
                match permission_action_applicability(
                    android_api_level,
                    action.android_api_min,
                    action.android_api_max,
                ) {
                    Ok(applicability) => actions.push(PermissionPlanAction::AppOp {
                        package_name: app.package_id.clone(),
                        op: action.op.clone(),
                        mode: action.mode,
                        android_api_min: action.android_api_min,
                        android_api_max: action.android_api_max,
                        applicability,
                    }),
                    Err(()) => action_errors.push(permission_planning_message(
                        "permission_api_level_unknown",
                        format!(
                            "Permission step '{}' cannot resolve app-op '{}' because the Android API level is unknown; provide the API level or remove the action's API bounds.",
                            step.id, action.op
                        ),
                        recipe_id,
                        step,
                        json!({
                            "app_ref": app_ref,
                            "permission_set": permission_set,
                            "source_section": "app_ops",
                            "action_index": index,
                            "op": action.op,
                            "android_api_min": action.android_api_min,
                            "android_api_max": action.android_api_max,
                        }),
                    )),
                }
            }
            if action_errors.is_empty() {
                plans.insert(
                    execution_step_id,
                    PermissionPlan {
                        permission_set,
                        on_failure,
                        actions,
                    },
                );
            } else {
                errors.extend(action_errors);
            }
        }
    }
    if errors.is_empty() {
        Ok(plans)
    } else {
        Err(errors)
    }
}

fn permission_action_applicability(
    android_api_level: Option<i64>,
    android_api_min: Option<i64>,
    android_api_max: Option<i64>,
) -> Result<PermissionActionApplicability, ()> {
    if android_api_min.is_none() && android_api_max.is_none() {
        return Ok(PermissionActionApplicability::Applicable);
    }
    let Some(actual) = android_api_level else {
        return Err(());
    };
    if let Some(minimum) = android_api_min.filter(|minimum| actual < *minimum) {
        return Ok(PermissionActionApplicability::NotApplicable {
            reason: PermissionNotApplicableReason::AndroidApiBelowMinimum { actual, minimum },
        });
    }
    if let Some(maximum) = android_api_max.filter(|maximum| actual > *maximum) {
        return Ok(PermissionActionApplicability::NotApplicable {
            reason: PermissionNotApplicableReason::AndroidApiAboveMaximum { actual, maximum },
        });
    }
    Ok(PermissionActionApplicability::Applicable)
}

fn permission_planning_message(
    code: &str,
    message: String,
    recipe_id: &str,
    step: &Step,
    details: Value,
) -> PlannerMessage {
    let mut details = details.as_object().cloned().unwrap_or_default();
    details.insert("recipe_ref".to_string(), json!(recipe_id));
    details.insert("step_id".to_string(), json!(step.id));
    PlannerMessage {
        code: code.to_string(),
        message,
        details: Value::Object(details),
    }
}

pub(super) fn validate_app_permission_param_values(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    match normalized.get("permission_set") {
        Some(ParamValue::Literal(Value::String(value)))
            if matches!(value.as_str(), "baseline" | "elevated") => {}
        Some(ParamValue::Literal(Value::String(value))) => errors.push(param_contract_message(
            "invalid_enum_value",
            "Param 'permission_set' must be one of baseline, elevated for step type 'grant_permissions'.".to_string(),
            recipe_id,
            step,
            "permission_set",
            json!(["baseline", "elevated"]),
            json!(value),
        )),
        Some(actual) => errors.push(param_contract_message(
            "invalid_param_value",
            "Param 'permission_set' must be a literal set name for step type 'grant_permissions'.".to_string(),
            recipe_id,
            step,
            "permission_set",
            json!(["baseline", "elevated"]),
            param_value_to_json(actual),
        )),
        None => errors.push(param_contract_message(
            "missing_required_param",
            "Param 'permission_set' is required for app-aware step type 'grant_permissions'.".to_string(),
            recipe_id,
            step,
            "permission_set",
            json!(["baseline", "elevated"]),
            Value::Null,
        )),
    }

    let Some(policy_value) = normalized.get("policy") else {
        errors.push(param_contract_message(
            "missing_required_param",
            "Param 'policy' is required for app-aware step type 'grant_permissions'.".to_string(),
            recipe_id,
            step,
            "policy",
            json!({ "on_failure": ["warn", "fail"] }),
            Value::Null,
        ));
        return errors;
    };
    let ParamValue::Literal(Value::Object(policy)) = policy_value else {
        errors.push(param_contract_message(
            "invalid_param_value",
            "Param 'policy' must be an object for app-aware step type 'grant_permissions'."
                .to_string(),
            recipe_id,
            step,
            "policy",
            json!({ "on_failure": ["warn", "fail"] }),
            param_value_to_json(policy_value),
        ));
        return errors;
    };
    for field_name in sorted_unsupported_fields(policy, &["on_failure"]) {
        errors.push(param_contract_message(
            "invalid_param_value",
            format!("Param 'policy' contains unsupported field '{field_name}' for app-aware permission steps."),
            recipe_id,
            step,
            &format!("policy.{field_name}"),
            json!(["on_failure"]),
            policy.get(&field_name).cloned().unwrap_or(Value::Null),
        ));
    }
    match policy.get("on_failure") {
        Some(Value::String(value)) if matches!(value.as_str(), "warn" | "fail") => {}
        Some(actual) => errors.push(param_contract_message(
            "invalid_enum_value",
            "Param 'policy.on_failure' must be one of warn, fail for step type 'grant_permissions'.".to_string(),
            recipe_id,
            step,
            "policy.on_failure",
            json!(["warn", "fail"]),
            actual.clone(),
        )),
        None => errors.push(param_contract_message(
            "missing_required_param",
            "Param 'policy.on_failure' is required for app-aware step type 'grant_permissions'.".to_string(),
            recipe_id,
            step,
            "policy.on_failure",
            json!(["warn", "fail"]),
            Value::Null,
        )),
    }
    errors
}
