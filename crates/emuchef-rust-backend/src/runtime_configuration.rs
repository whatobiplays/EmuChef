//! Side-effect-free preparation and discovery for runtime recipe configuration.
//!
//! This module resolves request overrides, saved configuration, device-plan
//! selection, recipe dependencies, and binding precedence. It performs no ADB,
//! download, extraction, copy, device-write, or persistence operations.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::{json, Value};

use crate::dto;
use crate::model::{OrderedMap, Recipe};
use crate::planner::{
    expand_recipe_dependencies, resolve_runtime_bindings, BindingDiagnostic, BindingResolution,
    BindingSource, PlannerLoadError, PlannerMessage,
};
use crate::planner_device_plan;
use crate::user_configuration::{self, UserConfigurationLoadError};

#[derive(Clone, Debug)]
pub(crate) struct ConfigurationContextRequest {
    pub authored_root: PathBuf,
    pub configuration_root: Option<PathBuf>,
    pub user_configuration: Option<String>,
    pub device_plan: Option<String>,
    pub selected_recipes: Option<Vec<String>>,
    pub explicit_bindings: OrderedMap<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfigurationDiagnostic {
    pub severity: &'static str,
    pub code: String,
    pub message: String,
    pub key: Option<String>,
    pub provenance: Option<BindingSource>,
    pub details: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedConfiguration {
    pub effective_device_plan: String,
    pub recipes: Vec<Recipe>,
    pub selected_recipe_refs: Vec<String>,
    pub expanded_recipe_refs: Vec<String>,
    pub binding_resolution: BindingResolution,
    pub diagnostics: Vec<RuntimeConfigurationDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationDescription {
    pub device_plan: String,
    pub selected_recipes: Vec<String>,
    pub expanded_recipes: Vec<String>,
    pub inputs: Vec<Value>,
    pub diagnostics: Vec<RuntimeConfigurationDiagnostic>,
}

#[derive(Debug)]
pub(crate) enum ConfigurationContextError {
    MissingDevicePlan,
    UserConfiguration(UserConfigurationLoadError),
    Catalog(PlannerLoadError),
}

impl std::fmt::Display for ConfigurationContextError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDevicePlan => formatter.write_str(
                "Configuration context requires devicePlan or a saved user configuration.",
            ),
            Self::UserConfiguration(error) => error.fmt(formatter),
            Self::Catalog(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ConfigurationContextError {}

pub(crate) fn prepare_configuration(
    request: ConfigurationContextRequest,
) -> Result<PreparedConfiguration, ConfigurationContextError> {
    let recipes = crate::planner::load_top_level_recipes(&request.authored_root)
        .map_err(ConfigurationContextError::Catalog)?;
    let recipe_map = recipes
        .iter()
        .cloned()
        .map(|recipe| (recipe.id.clone(), recipe))
        .collect::<HashMap<_, _>>();

    let saved = request
        .user_configuration
        .as_deref()
        .map(|reference| {
            user_configuration::load_user_configuration_reference(
                request.configuration_root.as_deref(),
                reference,
            )
            .map(|(_, configuration)| configuration)
            .map_err(ConfigurationContextError::UserConfiguration)
        })
        .transpose()?;
    let effective_device_plan = request
        .device_plan
        .clone()
        .or_else(|| {
            saved
                .as_ref()
                .map(|configuration| configuration.device_plan.clone())
        })
        .ok_or(ConfigurationContextError::MissingDevicePlan)?;

    let mut diagnostics = Vec::new();
    let device_plan_parts = match planner_device_plan::load_planner_input_parts(
        &request.authored_root,
        &effective_device_plan,
        &recipes,
    ) {
        Ok(parts) => Some(parts),
        Err(error) => {
            diagnostics.push(planner_load_diagnostic(&error, &effective_device_plan));
            None
        }
    };
    let selected_recipe_refs = request
        .selected_recipes
        .clone()
        .or_else(|| {
            saved
                .as_ref()
                .map(|configuration| configuration.selected_recipes.clone())
        })
        .or_else(|| {
            device_plan_parts
                .as_ref()
                .map(|parts| parts.selected_recipe_refs.clone())
        })
        .unwrap_or_default();
    let (expanded_recipe_refs, dependency_errors) =
        expand_recipe_dependencies(&recipe_map, &selected_recipe_refs);
    diagnostics.extend(
        dependency_errors
            .into_iter()
            .map(planner_message_diagnostic),
    );

    let user_configuration_bindings = saved
        .map(|configuration| configuration.bindings)
        .unwrap_or_default();
    let device_plan_input_bindings = device_plan_parts
        .as_ref()
        .map(|parts| parts.device_plan_input_bindings.clone())
        .unwrap_or_default();
    let binding_resolution = resolve_runtime_bindings(
        &recipe_map,
        &expanded_recipe_refs,
        &request.explicit_bindings,
        &user_configuration_bindings,
        &device_plan_input_bindings,
    );
    diagnostics.extend(
        binding_resolution
            .diagnostics
            .iter()
            .map(binding_diagnostic),
    );

    Ok(PreparedConfiguration {
        effective_device_plan,
        recipes,
        selected_recipe_refs,
        expanded_recipe_refs,
        binding_resolution,
        diagnostics,
    })
}

pub(crate) fn describe_configuration(
    request: ConfigurationContextRequest,
) -> Result<ConfigurationDescription, ConfigurationContextError> {
    let prepared = prepare_configuration(request)?;
    let recipes = prepared
        .recipes
        .iter()
        .map(|recipe| (recipe.id.as_str(), recipe))
        .collect::<HashMap<_, _>>();
    let inputs = prepared
        .binding_resolution
        .resolved_inputs
        .iter()
        .filter_map(|resolved| {
            let declaration = recipes
                .get(resolved.recipe_id.as_str())?
                .inputs
                .get(&resolved.input_id)?;
            let mut dto = dto::input_to_dto(&resolved.recipe_id, &resolved.input_id, declaration);
            let object = dto.as_object_mut()?;
            object.insert(
                "value".to_string(),
                resolved.value.clone().unwrap_or(Value::Null),
            );
            object.insert(
                "valueSource".to_string(),
                resolved.source.map_or(Value::Null, |source| {
                    serde_json::to_value(source).expect("binding source should serialize")
                }),
            );
            object.insert(
                "diagnostics".to_string(),
                json!(prepared
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.key.as_deref() == Some(&resolved.key))
                    .collect::<Vec<_>>()),
            );
            Some(dto)
        })
        .collect();

    Ok(ConfigurationDescription {
        device_plan: prepared.effective_device_plan,
        selected_recipes: prepared.selected_recipe_refs,
        expanded_recipes: prepared.expanded_recipe_refs,
        inputs,
        diagnostics: prepared.diagnostics,
    })
}

fn binding_diagnostic(diagnostic: &BindingDiagnostic) -> RuntimeConfigurationDiagnostic {
    RuntimeConfigurationDiagnostic {
        severity: "error",
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        key: Some(diagnostic.key.clone()),
        provenance: diagnostic.provenance,
        details: diagnostic.details.clone(),
    }
}

fn planner_message_diagnostic(message: PlannerMessage) -> RuntimeConfigurationDiagnostic {
    RuntimeConfigurationDiagnostic {
        severity: "error",
        code: message.code,
        message: message.message,
        key: None,
        provenance: None,
        details: message.details,
    }
}

fn planner_load_diagnostic(
    error: &PlannerLoadError,
    device_plan: &str,
) -> RuntimeConfigurationDiagnostic {
    RuntimeConfigurationDiagnostic {
        severity: "error",
        code: error.code().to_string(),
        message: error.to_string(),
        key: None,
        provenance: Some(BindingSource::DevicePlan),
        details: json!({ "devicePlan": device_plan }),
    }
}
