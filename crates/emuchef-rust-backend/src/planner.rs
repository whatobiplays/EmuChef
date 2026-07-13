//! Declarative execution-plan emission for authored recipes.
//!
//! This module builds `PlanningResult` and `ExecutionPlan` values without
//! exposing protocol requests, running executor operations, probing devices, or
//! mutating authored files.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Map;
use serde_json::{json, Value};

use crate::model::{OrderedMap, ParamValue, Recipe, Step, StepCondition, StepConstraints};
use crate::planner_device_plan;
use crate::runtime_refs::{
    artifact_field_value_type, input_value_type, parse_reference, RuntimeRef,
};
use crate::step_specs;
use crate::yaml;

const SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlanningResult {
    pub status: PlanningStatus,
    pub warnings: Vec<PlannerMessage>,
    pub errors: Vec<PlannerMessage>,
    pub execution_plan: Option<ExecutionPlan>,
    pub schema_version: i64,
    pub kind: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningStatus {
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlannerMessage {
    pub code: String,
    pub message: String,
    pub details: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionPlan {
    pub id: String,
    pub source: ExecutionPlanSource,
    pub device_context: DeviceContext,
    pub runtime_capabilities: RuntimeCapabilities,
    pub inputs: Vec<ExecutionInputValue>,
    pub artifacts: Vec<ExecutionArtifact>,
    pub steps: Vec<ExecutionStep>,
    pub schema_version: i64,
    pub kind: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionPlanSource {
    pub device_profile_ref: String,
    pub device_plan_ref: String,
    pub selected_recipe_refs: Vec<String>,
    pub expanded_recipe_refs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeviceContext {
    pub manufacturer: String,
    pub model: String,
    pub android_version: i64,
    pub android_api_level: Option<i64>,
    pub device_tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RuntimeCapabilities {
    pub adb_available: bool,
    pub apk_install: bool,
    pub shared_storage_write: bool,
    pub app_launch: bool,
    pub shell_command: bool,
    pub package_remove_for_user: bool,
    pub root_shell: bool,
    pub app_data_write: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionInputValue {
    pub id: String,
    pub value: RuntimeValue,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RuntimeValue {
    #[serde(rename = "type")]
    pub type_name: String,
    pub value: Value,
    pub location: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionArtifact {
    pub id: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub url: String,
    pub cache: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionStep {
    pub id: String,
    pub recipe_ref: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub name: String,
    pub dependencies: Vec<String>,
    pub constraints: ExecutionStepConstraints,
    pub params: OrderedMap<ExecutionParamValue>,
    pub skip_if: Vec<ExecutionStepCondition>,
    pub verify: Vec<ExecutionStepCondition>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionStepConstraints {
    pub capabilities: Vec<String>,
    pub conflicts_with: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionStepCondition {
    #[serde(rename = "type")]
    pub type_name: String,
    pub params: OrderedMap<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ExecutionParamValue {
    Ref {
        #[serde(rename = "ref")]
        ref_value: String,
    },
    Literal {
        value: Value,
    },
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PermissionIntentPlan {
    pub grants: Vec<PermissionIntentGrant>,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PermissionIntentGrant {
    pub recipe_ref: String,
    pub step_id: String,
    pub execution_step_id: String,
    pub policy: PermissionIntentPolicy,
    pub actions: Vec<PermissionIntentAction>,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PermissionIntentPolicy {
    pub on_failure: String,
    pub require_all: bool,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PermissionIntentAction {
    RuntimePermission {
        package_name: String,
        permission: String,
        required: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        when: Option<Value>,
        source_section: String,
    },
    Appop {
        package_name: String,
        op: String,
        desired_mode: String,
        required: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        when: Option<Value>,
        source_section: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannerInput {
    pub recipes: Vec<Recipe>,
    pub selected_recipe_refs: Vec<String>,
    pub input_bindings: OrderedMap<Value>,
    pub plan_id: String,
    pub device_plan_ref: String,
    pub device_profile_ref: String,
    pub device_context: DeviceContext,
    pub runtime_capabilities: RuntimeCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannerLoadError {
    code: String,
    message: String,
}

impl PlannerInput {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn from_authored_root(
        authored_root: impl AsRef<Path>,
        selected_recipe_refs: Vec<String>,
        plan_id: String,
        device_plan_ref: String,
        device_profile_ref: String,
        device_context: DeviceContext,
        runtime_capabilities: RuntimeCapabilities,
    ) -> Result<Self, PlannerLoadError> {
        let recipes = load_top_level_recipes(authored_root.as_ref())?;
        Ok(Self {
            recipes,
            selected_recipe_refs,
            input_bindings: OrderedMap::new(),
            plan_id,
            device_plan_ref,
            device_profile_ref,
            device_context,
            runtime_capabilities,
        })
    }

    pub(crate) fn from_authored_device_plan(
        authored_root: impl AsRef<Path>,
        device_plan_ref: &str,
        plan_id: String,
        input_bindings: OrderedMap<Value>,
    ) -> Result<Self, PlannerLoadError> {
        let authored_root = authored_root.as_ref();
        let recipes = load_top_level_recipes(authored_root)?;
        let parts = planner_device_plan::load_planner_input_parts(
            authored_root,
            device_plan_ref,
            &recipes,
        )?;
        let recipe_ids = recipes
            .iter()
            .map(|recipe| recipe.id.as_str())
            .collect::<HashSet<_>>();
        for recipe_ref in &parts.recipe_refs {
            if !recipe_ids.contains(recipe_ref.as_str()) {
                return Err(PlannerLoadError::new(
                    "recipe_not_found",
                    format!(
                        "Recipe '{recipe_ref}' referenced by device plan '{}' was not found.",
                        parts.device_plan_ref
                    ),
                ));
            }
        }
        let mut merged_input_bindings = parts.override_input_bindings;
        for (input_id, value) in input_bindings {
            merged_input_bindings.insert(input_id, value);
        }

        Ok(Self {
            recipes,
            selected_recipe_refs: parts.selected_recipe_refs,
            input_bindings: merged_input_bindings,
            plan_id,
            device_plan_ref: parts.device_plan_ref,
            device_profile_ref: parts.device_profile_ref,
            device_context: parts.device_context,
            runtime_capabilities: parts.runtime_capabilities,
        })
    }
}

impl PlannerLoadError {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }
}

impl std::fmt::Display for PlannerLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PlannerLoadError {}

pub fn plan_execution(input: PlannerInput) -> PlanningResult {
    let recipes = input
        .recipes
        .iter()
        .map(|recipe| (recipe.id.clone(), recipe.clone()))
        .collect::<HashMap<_, _>>();

    let (expanded_recipe_refs, dependency_errors) =
        expand_recipe_dependencies(&recipes, &input.selected_recipe_refs);
    if !dependency_errors.is_empty() {
        return error_result(dependency_errors);
    }

    let (selected_step_ids, selection_errors) = selected_step_ids(
        &recipes,
        &expanded_recipe_refs,
        &input.input_bindings,
        &input.runtime_capabilities,
    );
    if !selection_errors.is_empty() {
        return error_result(selection_errors);
    }
    let selected_input_ids =
        selected_input_ids(&recipes, &expanded_recipe_refs, &selected_step_ids);
    let input_errors =
        validate_input_bindings(&recipes, &selected_input_ids, &input.input_bindings);
    if !input_errors.is_empty() {
        return error_result(input_errors);
    }

    let authored_steps = authored_steps(&recipes, &expanded_recipe_refs, &selected_step_ids);
    let step_param_errors = validate_step_param_contracts(&recipes, &authored_steps);
    if !step_param_errors.is_empty() {
        return error_result(step_param_errors);
    }
    let step_dependency_errors = validate_emitted_step_dependencies(&authored_steps);
    if !step_dependency_errors.is_empty() {
        return error_result(step_dependency_errors);
    }
    let artifact_selection_errors = validate_artifact_selections(&recipes, &authored_steps);
    if !artifact_selection_errors.is_empty() {
        return error_result(artifact_selection_errors);
    }
    let step_ref_errors = validate_step_refs(&authored_steps);
    if !step_ref_errors.is_empty() {
        return error_result(step_ref_errors);
    }
    let (ordered_steps, step_errors) = topologically_sort_steps(authored_steps);
    if !step_errors.is_empty() {
        return error_result(step_errors);
    }
    if ordered_steps.is_empty() {
        return error_result(vec![PlannerMessage {
            code: "empty_execution_plan".to_string(),
            message: "Execution plan emission produced no runnable steps.".to_string(),
            details: json!({ "plan_id": input.plan_id }),
        }]);
    }

    let execution_plan = ExecutionPlan {
        id: input.plan_id,
        source: ExecutionPlanSource {
            device_profile_ref: input.device_profile_ref,
            device_plan_ref: input.device_plan_ref,
            selected_recipe_refs: input.selected_recipe_refs,
            expanded_recipe_refs: expanded_recipe_refs.clone(),
        },
        device_context: input.device_context,
        runtime_capabilities: input.runtime_capabilities,
        inputs: emit_execution_inputs(&recipes, &selected_input_ids, &input.input_bindings),
        artifacts: emit_execution_artifacts(&recipes, &expanded_recipe_refs),
        steps: ordered_steps
            .into_iter()
            .filter_map(|(step_id, recipe_id, step)| {
                let recipe = recipes.get(&recipe_id)?;
                Some(ExecutionStep {
                    id: step_id,
                    recipe_ref: recipe_id,
                    type_name: step.type_name.clone(),
                    name: step.name.clone(),
                    dependencies: step
                        .dependencies
                        .iter()
                        .map(|dependency| make_execution_step_id(&recipe.id, dependency))
                        .collect(),
                    constraints: execution_constraints(&step.constraints, &recipe.id),
                    params: normalize_step_params_for_execution(recipe, &step),
                    skip_if: step.skip_if.iter().map(execution_condition).collect(),
                    verify: step.verify.iter().map(execution_condition).collect(),
                })
            })
            .collect(),
        schema_version: SCHEMA_VERSION,
        kind: "execution_plan",
    };

    PlanningResult {
        status: PlanningStatus::Success,
        warnings: Vec::new(),
        errors: Vec::new(),
        execution_plan: Some(execution_plan),
        schema_version: SCHEMA_VERSION,
        kind: "planning_result",
    }
}

fn load_top_level_recipes(authored_root: &Path) -> Result<Vec<Recipe>, PlannerLoadError> {
    let recipe_root = authored_root.join("recipes");
    let entries = fs::read_dir(&recipe_root).map_err(|error| {
        PlannerLoadError::new(
            "io",
            format!(
                "Failed to read recipe directory {}: {error}",
                recipe_root.display()
            ),
        )
    })?;
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_yaml_extension(path))
        .collect::<Vec<_>>();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            yaml::load_recipe_from_path(&path).map_err(|error| {
                PlannerLoadError::new(
                    "authored_data_invalid",
                    format!("Failed to load planner recipe {}: {error}", path.display()),
                )
            })
        })
        .collect()
}

fn is_yaml_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    )
}

fn error_result(errors: Vec<PlannerMessage>) -> PlanningResult {
    PlanningResult {
        status: PlanningStatus::Error,
        warnings: Vec::new(),
        errors,
        execution_plan: None,
        schema_version: SCHEMA_VERSION,
        kind: "planning_result",
    }
}

fn expand_recipe_dependencies(
    recipes: &HashMap<String, Recipe>,
    selected_recipe_refs: &[String],
) -> (Vec<String>, Vec<PlannerMessage>) {
    let mut ordered = Vec::new();
    let mut permanent = HashSet::new();
    let mut temporary = HashSet::new();
    let mut errors = Vec::new();

    for recipe_ref in selected_recipe_refs {
        visit_recipe_dependency(
            recipes,
            recipe_ref,
            &mut Vec::new(),
            &mut ordered,
            &mut permanent,
            &mut temporary,
            &mut errors,
        );
    }

    (ordered, errors)
}

fn visit_recipe_dependency(
    recipes: &HashMap<String, Recipe>,
    recipe_ref: &str,
    stack: &mut Vec<String>,
    ordered: &mut Vec<String>,
    permanent: &mut HashSet<String>,
    temporary: &mut HashSet<String>,
    errors: &mut Vec<PlannerMessage>,
) {
    if permanent.contains(recipe_ref) {
        return;
    }
    if temporary.contains(recipe_ref) {
        let mut cycle = stack.clone();
        cycle.push(recipe_ref.to_string());
        errors.push(PlannerMessage {
            code: "dependency_cycle".to_string(),
            message: format!("Recipe dependency cycle detected at '{}'.", recipe_ref),
            details: json!({ "cycle": cycle }),
        });
        return;
    }
    let Some(recipe) = recipes.get(recipe_ref) else {
        let details = if let Some(dependent_recipe_ref) = stack.last() {
            json!({
                "recipe_ref": recipe_ref,
                "dependency_ref": recipe_ref,
                "dependent_recipe_ref": dependent_recipe_ref,
                "source": "recipe_dependencies"
            })
        } else {
            json!({
                "recipe_ref": recipe_ref,
                "selected_recipe_ref": recipe_ref,
                "source": "selected_recipe_refs"
            })
        };
        errors.push(PlannerMessage {
            code: "recipe_not_found".to_string(),
            message: format!("Recipe '{recipe_ref}' was not found."),
            details,
        });
        return;
    };

    temporary.insert(recipe_ref.to_string());
    stack.push(recipe_ref.to_string());
    for dependency_ref in &recipe.recipe_dependencies {
        visit_recipe_dependency(
            recipes,
            dependency_ref,
            stack,
            ordered,
            permanent,
            temporary,
            errors,
        );
    }
    stack.pop();
    temporary.remove(recipe_ref);
    permanent.insert(recipe_ref.to_string());
    if !ordered.iter().any(|item| item == recipe_ref) {
        ordered.push(recipe_ref.to_string());
    }
}

fn authored_steps(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    selected_step_ids: &HashSet<String>,
) -> Vec<(String, String, Step)> {
    let mut result = Vec::new();
    for recipe_id in expanded_recipe_refs {
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        for step in &recipe.steps {
            let step_id = make_execution_step_id(recipe_id, &step.id);
            if !selected_step_ids.contains(&step_id) {
                continue;
            }
            result.push((step_id, recipe_id.clone(), step.clone()));
        }
    }
    result
}

#[cfg(test)]
pub(crate) fn build_permission_intent(steps: &[(String, String, Step)]) -> PermissionIntentPlan {
    let mut grants = Vec::new();

    for (execution_step_id, recipe_id, step) in steps {
        if step.type_name != "grant_permissions" {
            continue;
        }

        let normalized = params_with_defaults(step);
        let mut actions = Vec::new();

        for (index, item) in literal_object_items(normalized.get("runtime"))
            .iter()
            .enumerate()
        {
            actions.push(PermissionIntentAction::RuntimePermission {
                package_name: string_field(item, "package_name"),
                permission: string_field(item, "name"),
                required: bool_field(item, "required").unwrap_or(true),
                when: item.get("when").cloned(),
                source_section: format!("params.runtime[{index}]"),
            });
        }

        for (index, item) in literal_object_items(normalized.get("appops"))
            .iter()
            .enumerate()
        {
            actions.push(PermissionIntentAction::Appop {
                package_name: string_field(item, "package_name"),
                op: string_field(item, "op"),
                desired_mode: string_field(item, "mode"),
                required: bool_field(item, "required").unwrap_or(true),
                when: item.get("when").cloned(),
                source_section: format!("params.appops[{index}]"),
            });
        }

        if actions.is_empty() {
            continue;
        }

        grants.push(PermissionIntentGrant {
            recipe_ref: recipe_id.clone(),
            step_id: step.id.clone(),
            execution_step_id: execution_step_id.clone(),
            policy: permission_intent_policy(normalized.get("policy")),
            actions,
        });
    }

    PermissionIntentPlan { grants }
}

#[cfg(test)]
fn permission_intent_policy(value: Option<&ParamValue>) -> PermissionIntentPolicy {
    let Some(ParamValue::Literal(Value::Object(policy))) = value else {
        return PermissionIntentPolicy {
            on_failure: "warn".to_string(),
            require_all: false,
        };
    };

    PermissionIntentPolicy {
        on_failure: policy
            .get("on_failure")
            .and_then(Value::as_str)
            .unwrap_or("warn")
            .to_string(),
        require_all: policy
            .get("require_all")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

#[cfg(test)]
fn literal_object_items(value: Option<&ParamValue>) -> Vec<&Map<String, Value>> {
    let Some(ParamValue::Literal(Value::Array(items))) = value else {
        return Vec::new();
    };
    items.iter().filter_map(Value::as_object).collect()
}

#[cfg(test)]
fn string_field(item: &Map<String, Value>, field: &str) -> String {
    item.get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
fn bool_field(item: &Map<String, Value>, field: &str) -> Option<bool> {
    item.get(field).and_then(Value::as_bool)
}

// Planner terminology follows the current Rust pipeline:
// - available steps satisfy static runtime capability constraints;
// - selected steps are available steps retained after dependency expansion and
//   optional-input pruning;
// - emitted steps are selected authored steps materialized by `authored_steps`
//   and serialized into the execution plan.
fn topologically_sort_steps(
    steps: Vec<(String, String, Step)>,
) -> (Vec<(String, String, Step)>, Vec<PlannerMessage>) {
    let index_by_id = steps
        .iter()
        .enumerate()
        .map(|(index, (step_id, _, _))| (step_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let step_by_id = steps
        .iter()
        .map(|(step_id, recipe_id, step)| (step_id.clone(), (recipe_id.clone(), step.clone())))
        .collect::<HashMap<_, _>>();
    let mut reverse_graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut indegree: HashMap<String, usize> = HashMap::new();

    for (step_id, recipe_id, step) in &steps {
        let dependency_ids = unique_dependency_ids(&step.dependencies);
        indegree.insert(step_id.clone(), dependency_ids.len());
        reverse_graph.entry(step_id.clone()).or_default();
        for dependency in dependency_ids {
            let dependency_id = make_execution_step_id(recipe_id, dependency);
            reverse_graph
                .entry(dependency_id)
                .or_default()
                .push(step_id.clone());
        }
    }

    let mut ready = indegree
        .iter()
        .filter_map(|(step_id, degree)| (*degree == 0).then_some(step_id.clone()))
        .collect::<Vec<_>>();
    ready.sort_by_key(|step_id| index_by_id[step_id]);
    let mut queue = VecDeque::from(ready);
    let mut result = Vec::new();

    while let Some(step_id) = queue.pop_front() {
        if let Some((recipe_id, step)) = step_by_id.get(&step_id) {
            result.push((step_id.clone(), recipe_id.clone(), step.clone()));
        }

        let mut dependents = reverse_graph.remove(&step_id).unwrap_or_default();
        dependents.sort_by_key(|dependent| index_by_id[dependent]);
        for dependent in dependents {
            let Some(degree) = indegree.get_mut(&dependent) else {
                continue;
            };
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(dependent);
            }
        }
    }

    if result.len() != steps.len() {
        let step_ids = steps
            .iter()
            .map(|(step_id, _, _)| Value::String(step_id.clone()))
            .collect::<Vec<_>>();
        return (
            Vec::new(),
            vec![PlannerMessage {
                code: "dependency_cycle".to_string(),
                message: "Execution step dependency cycle detected.".to_string(),
                details: json!({ "step_ids": step_ids }),
            }],
        );
    }

    (result, Vec::new())
}

fn selected_step_ids(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    input_bindings: &OrderedMap<Value>,
    runtime_capabilities: &RuntimeCapabilities,
) -> (HashSet<String>, Vec<PlannerMessage>) {
    let mut selected = HashSet::new();
    let mut availability_by_step_id = HashMap::new();
    let mut errors = Vec::new();

    for recipe_id in expanded_recipe_refs {
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        let mut available_by_local_step_id = HashMap::new();
        let mut requested_local_step_ids = Vec::new();

        for step in &recipe.steps {
            let available =
                step.constraints.capabilities.iter().all(|capability| {
                    runtime_capability_available(runtime_capabilities, capability)
                });
            let step_id = make_execution_step_id(recipe_id, &step.id);
            availability_by_step_id.insert(step_id, available);
            available_by_local_step_id.insert(step.id.clone(), available);
            if available {
                requested_local_step_ids.push(step.id.clone());
            }
        }

        let (local_step_ids, selection_errors) = select_local_step_ids(
            recipe,
            &requested_local_step_ids,
            &available_by_local_step_id,
        );
        errors.extend(selection_errors);
        for local_step_id in local_step_ids {
            selected.insert(make_execution_step_id(recipe_id, &local_step_id));
        }
    }

    if !errors.is_empty() {
        return (HashSet::new(), errors);
    }

    (
        prune_optional_input_steps(
            recipes,
            expanded_recipe_refs,
            selected,
            &availability_by_step_id,
            input_bindings,
        ),
        Vec::new(),
    )
}

fn runtime_capability_available(capabilities: &RuntimeCapabilities, capability: &str) -> bool {
    match capability {
        "adb_available" => capabilities.adb_available,
        "apk_install" => capabilities.apk_install,
        "shared_storage_write" => capabilities.shared_storage_write,
        "app_launch" => capabilities.app_launch,
        "shell_command" => capabilities.shell_command,
        "package_remove_for_user" => capabilities.package_remove_for_user,
        "root_shell" => capabilities.root_shell,
        "app_data_write" => capabilities.app_data_write,
        _ => false,
    }
}

fn select_local_step_ids(
    recipe: &Recipe,
    requested_local_step_ids: &[String],
    available_by_local_step_id: &HashMap<String, bool>,
) -> (HashSet<String>, Vec<PlannerMessage>) {
    let by_id = recipe
        .steps
        .iter()
        .map(|step| (step.id.clone(), step))
        .collect::<HashMap<_, _>>();
    let dependency_errors =
        validate_available_step_dependencies(recipe, requested_local_step_ids, &by_id);
    if !dependency_errors.is_empty() {
        return (HashSet::new(), dependency_errors);
    }
    let mut selected = HashSet::new();
    let mut can_select_cache = HashMap::new();
    let mut errors = Vec::new();

    for requested_id in requested_local_step_ids {
        if can_select_local_step(
            recipe,
            requested_id,
            &by_id,
            available_by_local_step_id,
            &mut can_select_cache,
            &mut HashSet::new(),
            &mut errors,
        ) {
            add_local_step_with_dependencies(requested_id, &by_id, &mut selected);
        }
    }

    if errors.is_empty() {
        (selected, Vec::new())
    } else {
        (HashSet::new(), errors)
    }
}

fn can_select_local_step(
    recipe: &Recipe,
    step_id: &str,
    by_id: &HashMap<String, &Step>,
    available_by_local_step_id: &HashMap<String, bool>,
    cache: &mut HashMap<String, bool>,
    temporary: &mut HashSet<String>,
    errors: &mut Vec<PlannerMessage>,
) -> bool {
    if let Some(cached) = cache.get(step_id) {
        return *cached;
    }
    if !temporary.insert(step_id.to_string()) {
        if errors.is_empty() {
            errors.push(PlannerMessage {
                code: "dependency_cycle".to_string(),
                message: format!("Step dependency cycle detected in recipe '{}'.", recipe.id),
                details: json!({ "recipe_ref": recipe.id, "step_id": step_id }),
            });
        }
        return false;
    }
    let Some(step) = by_id.get(step_id) else {
        temporary.remove(step_id);
        cache.insert(step_id.to_string(), false);
        return false;
    };
    let allowed = *available_by_local_step_id.get(step_id).unwrap_or(&false)
        && step.dependencies.iter().all(|dependency| {
            can_select_local_step(
                recipe,
                dependency,
                by_id,
                available_by_local_step_id,
                cache,
                temporary,
                errors,
            )
        });
    temporary.remove(step_id);
    cache.insert(step_id.to_string(), allowed);
    allowed
}

fn add_local_step_with_dependencies(
    step_id: &str,
    by_id: &HashMap<String, &Step>,
    selected: &mut HashSet<String>,
) {
    if selected.contains(step_id) {
        return;
    }
    let Some(step) = by_id.get(step_id) else {
        return;
    };
    for dependency in &step.dependencies {
        add_local_step_with_dependencies(dependency, by_id, selected);
    }
    selected.insert(step_id.to_string());
}

fn prune_optional_input_steps(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    selected_step_ids: HashSet<String>,
    availability_by_step_id: &HashMap<String, bool>,
    input_bindings: &OrderedMap<Value>,
) -> HashSet<String> {
    let mut selected = selected_step_ids;

    loop {
        let mut removed_step_ids = HashSet::new();
        for recipe_id in expanded_recipe_refs {
            let Some(recipe) = recipes.get(recipe_id) else {
                continue;
            };
            for step in &recipe.steps {
                let step_id = make_execution_step_id(recipe_id, &step.id);
                if !selected.contains(&step_id)
                    || !availability_by_step_id
                        .get(&step_id)
                        .copied()
                        .unwrap_or(false)
                {
                    continue;
                }
                let unbound_optional_inputs = referenced_input_ids(recipe_id, step)
                    .into_iter()
                    .filter(|input_id| optional_input_is_unbound(recipes, input_id, input_bindings))
                    .collect::<Vec<_>>();
                if unbound_optional_inputs.is_empty() {
                    continue;
                }
                removed_step_ids.insert(step_id);
                for dependent_id in dependent_step_ids(recipe, &step.id) {
                    removed_step_ids.insert(make_execution_step_id(recipe_id, &dependent_id));
                }
            }
        }

        if removed_step_ids.is_empty() {
            return selected;
        }
        for step_id in removed_step_ids {
            selected.remove(&step_id);
        }
    }
}

fn optional_input_is_unbound(
    recipes: &HashMap<String, Recipe>,
    input_id: &str,
    input_bindings: &OrderedMap<Value>,
) -> bool {
    let Some((recipe_id, local_input_id)) = input_id.split_once('/') else {
        return false;
    };
    let Some(declaration) = recipes
        .get(recipe_id)
        .and_then(|recipe| recipe.inputs.get(local_input_id))
    else {
        return false;
    };
    !declaration.required && !input_bindings.contains_key(input_id) && declaration.default.is_null()
}

fn referenced_input_ids(recipe_id: &str, step: &Step) -> Vec<String> {
    let mut input_ids = Vec::new();
    for value in step.params.values() {
        let ParamValue::Ref(ref_value) = value else {
            continue;
        };
        if let Ok(RuntimeRef::Input { target_id }) = parse_reference(ref_value) {
            let input_id = make_execution_input_id(recipe_id, &target_id);
            if !input_ids.iter().any(|existing| existing == &input_id) {
                input_ids.push(input_id);
            }
        }
    }
    input_ids
}

fn dependent_step_ids(recipe: &Recipe, dependency_id: &str) -> HashSet<String> {
    let mut dependents = HashSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        for step in &recipe.steps {
            if step.id == dependency_id || dependents.contains(&step.id) {
                continue;
            }
            if step.dependencies.iter().any(|item| item == dependency_id)
                || step
                    .dependencies
                    .iter()
                    .any(|item| dependents.contains(item))
            {
                dependents.insert(step.id.clone());
                changed = true;
            }
        }
    }
    dependents
}

fn selected_input_ids(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
    selected_step_ids: &HashSet<String>,
) -> Vec<String> {
    let mut input_ids = Vec::new();
    for recipe_id in expanded_recipe_refs {
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        for step in &recipe.steps {
            let step_id = make_execution_step_id(recipe_id, &step.id);
            if !selected_step_ids.contains(&step_id) {
                continue;
            }
            for input_id in referenced_input_ids(recipe_id, step) {
                if !input_ids.iter().any(|existing| existing == &input_id) {
                    input_ids.push(input_id);
                }
            }
        }
    }
    input_ids
}

fn validate_input_bindings(
    recipes: &HashMap<String, Recipe>,
    input_ids: &[String],
    input_bindings: &OrderedMap<Value>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    for input_id in input_ids {
        let Some((recipe_id, local_input_id)) = input_id.split_once('/') else {
            continue;
        };
        let Some(declaration) = recipes
            .get(recipe_id)
            .and_then(|recipe| recipe.inputs.get(local_input_id))
        else {
            continue;
        };
        let Some(value) = binding_value(declaration, input_id, input_bindings) else {
            if !declaration.required {
                continue;
            }
            errors.push(PlannerMessage {
                code: "binding_missing".to_string(),
                message: format!("Required binding '{input_id}' is missing."),
                details: json!({ "input_id": input_id }),
            });
            continue;
        };
        errors.extend(validate_binding_value(input_id, declaration, &value));
    }
    errors
}

fn validate_binding_value(
    input_id: &str,
    declaration: &crate::model::InputDeclaration,
    value: &Value,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    let values = if declaration.multiple {
        let Some(values) = value.as_array() else {
            return vec![PlannerMessage {
                code: "binding_validation_failed".to_string(),
                message: format!("Input '{input_id}' requires multiple values."),
                details: json!({ "input_id": input_id }),
            }];
        };
        values.iter().collect::<Vec<_>>()
    } else {
        if !value.is_string() {
            return vec![PlannerMessage {
                code: "binding_validation_failed".to_string(),
                message: format!("Input '{input_id}' requires a single string path value."),
                details: json!({ "input_id": input_id }),
            }];
        }
        vec![value]
    };

    if declaration.required && values.is_empty() {
        errors.push(PlannerMessage {
            code: "binding_validation_failed".to_string(),
            message: format!("Input '{input_id}' requires at least one value."),
            details: json!({ "input_id": input_id }),
        });
        return errors;
    }

    for raw_value in values {
        let Some(raw_path) = raw_value.as_str() else {
            errors.push(PlannerMessage {
                code: "binding_validation_failed".to_string(),
                message: format!("Input '{input_id}' values must be string paths."),
                details: json!({ "input_id": input_id }),
            });
            continue;
        };
        if !declaration.validation.allowed_extensions.is_empty()
            && extension_is_disallowed(raw_path, &declaration.validation.allowed_extensions)
        {
            errors.push(PlannerMessage {
                code: "binding_validation_failed".to_string(),
                message: format!("Input path '{raw_path}' has an unsupported extension."),
                details: json!({ "input_id": input_id, "path": raw_path }),
            });
        }
    }
    errors
}

fn emit_execution_inputs(
    recipes: &HashMap<String, Recipe>,
    input_ids: &[String],
    input_bindings: &OrderedMap<Value>,
) -> Vec<ExecutionInputValue> {
    input_ids
        .iter()
        .filter_map(|input_id| {
            let (recipe_id, local_input_id) = input_id.split_once('/')?;
            let declaration = recipes.get(recipe_id)?.inputs.get(local_input_id)?;
            let value = binding_value(declaration, input_id, input_bindings)?;
            Some(ExecutionInputValue {
                id: input_id.clone(),
                value: binding_to_runtime_value(
                    declaration_type(declaration),
                    declaration.multiple,
                    value,
                ),
            })
        })
        .collect()
}

fn binding_value(
    declaration: &crate::model::InputDeclaration,
    input_id: &str,
    input_bindings: &OrderedMap<Value>,
) -> Option<Value> {
    let value = input_bindings
        .get(input_id)
        .cloned()
        .or_else(|| (!declaration.default.is_null()).then(|| declaration.default.clone()))?;
    Some(normalize_binding_value(declaration, value))
}

fn normalize_binding_value(declaration: &crate::model::InputDeclaration, value: Value) -> Value {
    let expected_kind = declaration_type(declaration);
    if expected_kind != "file" && expected_kind != "directory" {
        return value;
    }
    if declaration.multiple {
        let Value::Array(items) = value else {
            return value;
        };
        return Value::Array(
            items
                .into_iter()
                .map(|item| match item {
                    Value::String(path) => Value::String(normalize_path_string(&path)),
                    other => other,
                })
                .collect(),
        );
    }
    match value {
        Value::String(path) => Value::String(normalize_path_string(&path)),
        other => other,
    }
}

fn normalize_path_string(raw_value: &str) -> String {
    let expanded = expand_user_path(raw_value);
    let path = Path::new(&expanded);
    if path.is_absolute() {
        return expanded;
    }
    std::env::current_dir()
        .map(|current_dir| current_dir.join(path).to_string_lossy().to_string())
        .unwrap_or(expanded)
}

fn expand_user_path(raw_value: &str) -> String {
    if raw_value == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| raw_value.to_string());
    }
    let Some(rest) = raw_value.strip_prefix("~/") else {
        return raw_value.to_string();
    };
    std::env::var("HOME")
        .map(|home| Path::new(&home).join(rest).to_string_lossy().to_string())
        .unwrap_or_else(|_| raw_value.to_string())
}

fn extension_is_disallowed(raw_path: &str, allowed_extensions: &[String]) -> bool {
    let Some(extension) = Path::new(raw_path)
        .extension()
        .and_then(|extension| extension.to_str())
    else {
        return false;
    };
    let normalized = format!(".{}", extension.to_lowercase());
    !allowed_extensions
        .iter()
        .filter_map(|allowed| normalize_allowed_extension(allowed))
        .any(|allowed| allowed == normalized)
}

fn normalize_allowed_extension(extension: &str) -> Option<String> {
    let normalized = extension.trim().to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if normalized.starts_with('.') {
        Some(normalized)
    } else {
        Some(format!(".{normalized}"))
    }
}

fn declaration_type(declaration: &crate::model::InputDeclaration) -> &str {
    declaration
        .validation
        .path_kind
        .as_deref()
        .unwrap_or(&declaration.type_name)
}

fn binding_to_runtime_value(type_name: &str, multiple: bool, value: Value) -> RuntimeValue {
    if multiple {
        return RuntimeValue {
            type_name: "path_list".to_string(),
            value,
            location: Some("host".to_string()),
        };
    }
    if type_name == "file" {
        return RuntimeValue {
            type_name: "file_path".to_string(),
            value,
            location: Some("host".to_string()),
        };
    }
    if type_name == "directory" {
        return RuntimeValue {
            type_name: "directory_path".to_string(),
            value,
            location: Some("host".to_string()),
        };
    }
    let runtime_type = if value.is_null() {
        "null"
    } else if value.is_boolean() {
        "boolean"
    } else if value.is_i64() || value.is_u64() {
        "integer"
    } else if value.is_string() {
        "string"
    } else {
        "object"
    };
    RuntimeValue {
        type_name: runtime_type.to_string(),
        value,
        location: None,
    }
}

fn emit_execution_artifacts(
    recipes: &HashMap<String, Recipe>,
    expanded_recipe_refs: &[String],
) -> Vec<ExecutionArtifact> {
    let mut artifacts = Vec::new();
    for recipe_id in expanded_recipe_refs {
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        for (artifact_id, artifact) in &recipe.artifacts {
            artifacts.push(ExecutionArtifact {
                id: make_execution_artifact_id(recipe_id, artifact_id),
                type_name: artifact.type_name.clone(),
                url: artifact.url.clone(),
                cache: artifact.cache.clone(),
            });
        }
    }
    artifacts
}

fn execution_constraints(
    constraints: &StepConstraints,
    recipe_id: &str,
) -> ExecutionStepConstraints {
    ExecutionStepConstraints {
        capabilities: constraints.capabilities.clone(),
        conflicts_with: constraints
            .conflicts_with
            .iter()
            .map(|conflict_id| make_execution_step_id(recipe_id, conflict_id))
            .collect(),
    }
}

fn execution_condition(condition: &StepCondition) -> ExecutionStepCondition {
    ExecutionStepCondition {
        type_name: condition.type_name.clone(),
        params: condition.params.clone(),
    }
}

fn validate_artifact_selections(
    recipes: &HashMap<String, Recipe>,
    steps: &[(String, String, Step)],
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    for (_, recipe_id, step) in steps {
        if !matches!(
            step.type_name.as_str(),
            "resolve_artifacts" | "extract_artifacts"
        ) {
            continue;
        }
        let Some(recipe) = recipes.get(recipe_id) else {
            continue;
        };
        errors.extend(validate_artifact_selection(recipe, step));
    }
    errors
}

fn validate_artifact_selection(recipe: &Recipe, step: &Step) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    let mut expanded = Vec::<(String, &'static str)>::new();

    for artifact_id in string_list(step.params.get("artifacts")) {
        if !recipe.artifacts.contains_key(&artifact_id) {
            errors.push(PlannerMessage {
                code: "unknown_artifact_ref".to_string(),
                message: format!(
                    "Step '{}' references unknown artifact '{}'.",
                    step.id, artifact_id
                ),
                details: json!({
                    "recipe_ref": recipe.id,
                    "step_id": step.id,
                    "param": "artifacts",
                    "artifact_id": artifact_id,
                }),
            });
        }
        expanded.push((artifact_id, "artifacts"));
    }

    for group_id in string_list(step.params.get("artifact_groups")) {
        let Some(group_artifacts) = recipe.artifact_groups.get(&group_id) else {
            errors.push(PlannerMessage {
                code: "unknown_artifact_group_ref".to_string(),
                message: format!(
                    "Step '{}' references unknown artifact group '{}'.",
                    step.id, group_id
                ),
                details: json!({
                    "recipe_ref": recipe.id,
                    "step_id": step.id,
                    "param": "artifact_groups",
                    "group_id": group_id,
                }),
            });
            continue;
        };
        for artifact_id in group_artifacts {
            if !recipe.artifacts.contains_key(artifact_id) {
                errors.push(PlannerMessage {
                    code: "unknown_artifact_ref".to_string(),
                    message: format!(
                        "Step '{}' references unknown artifact '{}' in artifact group '{}'.",
                        step.id, artifact_id, group_id
                    ),
                    details: json!({
                        "recipe_ref": recipe.id,
                        "step_id": step.id,
                        "param": "artifact_groups",
                        "group_id": group_id,
                        "artifact_id": artifact_id,
                    }),
                });
            }
            expanded.push((artifact_id.clone(), "artifact_groups"));
        }
    }

    let mut seen = HashMap::<String, &'static str>::new();
    let mut duplicate_ids = HashSet::<String>::new();
    for (artifact_id, param) in expanded {
        if seen.insert(artifact_id.clone(), param).is_some()
            && duplicate_ids.insert(artifact_id.clone())
        {
            errors.push(PlannerMessage {
                code: "duplicate_artifact_selection".to_string(),
                message: format!(
                    "Step '{}' resolves duplicate artifact id '{}' after artifact group expansion.",
                    step.id, artifact_id
                ),
                details: json!({
                    "recipe_ref": recipe.id,
                    "step_id": step.id,
                    "param": param,
                    "duplicate_artifact_id": artifact_id,
                }),
            });
        }
    }

    errors
}

fn validate_step_param_contracts(
    recipes: &HashMap<String, Recipe>,
    steps: &[(String, String, Step)],
) -> Vec<PlannerMessage> {
    // This pass validates only the emitted-step contract matrix. Ref-valued params are
    // checked for authored `{ref: ...}` shape here; target parsing, shorthand
    // refs, and explicit output rewrites remain owned by the reference-rewrite pass.
    let mut errors = Vec::new();
    for (_, recipe_id, step) in steps {
        if !is_emitted_step_type(&step.type_name) {
            continue;
        }
        let Some(spec) = step_specs::step_spec_for(&step.type_name) else {
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
    errors
}

fn is_emitted_step_type(step_type: &str) -> bool {
    matches!(
        step_type,
        "copy_files"
            | "extract_artifacts"
            | "extract_archive"
            | "install_apk"
            | "wait"
            | "grant_permissions"
    )
}

fn validate_unknown_params(
    recipe_id: &str,
    step: &Step,
    spec: &step_specs::StepSpecDto,
) -> Vec<PlannerMessage> {
    let expected = spec.params.keys().cloned().collect::<HashSet<_>>();
    let mut unexpected = step
        .params
        .keys()
        .filter(|param_name| !expected.contains(*param_name))
        .cloned()
        .collect::<Vec<_>>();
    unexpected.sort();

    unexpected
        .into_iter()
        .map(|param_name| {
            param_contract_message(
                "unknown_param",
                format!(
                    "Param '{}' is not supported for step type '{}'.",
                    param_name, step.type_name
                ),
                recipe_id,
                step,
                &param_name,
                expected_param_names(spec),
                step.params
                    .get(&param_name)
                    .map(param_value_to_json)
                    .unwrap_or(Value::Null),
            )
        })
        .collect()
}

fn validate_spec_param_contracts(
    recipes: &HashMap<String, Recipe>,
    recipe_id: &str,
    step: &Step,
    spec: &step_specs::StepSpecDto,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    for (param_name, param_spec) in &spec.params {
        let Some(value) = normalized.get(param_name) else {
            continue;
        };
        match value {
            ParamValue::Literal(_) if !accepts_param_source(param_spec, "literal") => {
                errors.push(param_contract_message(
                    "invalid_param_source",
                    format!(
                        "Param '{}' does not accept literal values for step type '{}'.",
                        param_name, step.type_name
                    ),
                    recipe_id,
                    step,
                    param_name,
                    json!(param_spec.accepted_sources),
                    param_value_to_json(value),
                ));
            }
            ParamValue::Ref(ref_value) => errors.extend(validate_ref_param_contract(
                recipes, recipe_id, step, param_name, ref_value, param_spec,
            )),
            _ => {}
        }
    }
    errors
}

fn accepts_param_source(spec: &step_specs::StepParamDto, source: &str) -> bool {
    spec.accepted_sources
        .iter()
        .any(|accepted| accepted == source)
}

fn validate_ref_param_contract(
    recipes: &HashMap<String, Recipe>,
    recipe_id: &str,
    step: &Step,
    param_name: &str,
    ref_value: &str,
    spec: &step_specs::StepParamDto,
) -> Vec<PlannerMessage> {
    let Ok(reference) = parse_reference(ref_value) else {
        return Vec::new();
    };
    let source = reference.source_kind();
    if !accepts_param_source(spec, source) {
        return vec![param_contract_message(
            "invalid_param_source",
            format!(
                "Param '{}' does not accept {} values for step type '{}'.",
                param_name, source, step.type_name
            ),
            recipe_id,
            step,
            param_name,
            json!(spec.accepted_sources),
            param_value_to_json(&ParamValue::Ref(ref_value.to_string())),
        )];
    }

    let Some(recipe) = recipes.get(recipe_id) else {
        return Vec::new();
    };
    let value_type = match reference {
        RuntimeRef::Input { target_id } => recipe
            .inputs
            .get(&target_id)
            .map(|input| input_value_type(&input.type_name, input.multiple).to_string()),
        RuntimeRef::ArtifactField { target_id, field } => recipe
            .artifacts
            .contains_key(&target_id)
            .then(|| artifact_field_value_type(&field).map(ToString::to_string))
            .flatten(),
        RuntimeRef::StepShorthand { target_id } => recipe
            .steps
            .iter()
            .find(|candidate| candidate.id == target_id)
            .and_then(primary_output_value_type),
        RuntimeRef::StepOutput { target_id, field } => recipe
            .steps
            .iter()
            .find(|candidate| candidate.id == target_id)
            .and_then(|candidate| step_output_value_type(candidate, &field)),
    };
    let Some(value_type) = value_type else {
        return Vec::new();
    };
    if spec
        .accepted_value_types
        .iter()
        .any(|accepted| accepted == &value_type)
    {
        return Vec::new();
    }
    vec![param_contract_message(
        "invalid_param_value_type",
        format!(
            "Param '{}' does not accept value type '{}' for step type '{}'.",
            param_name, value_type, step.type_name
        ),
        recipe_id,
        step,
        param_name,
        json!(spec.accepted_value_types),
        json!({ "ref": ref_value, "valueType": value_type }),
    )]
}

fn primary_output_value_type(step: &Step) -> Option<String> {
    let spec = step_specs::step_spec_for(&step.type_name)?;
    let primary = spec.primary_output_name?;
    spec.outputs
        .into_iter()
        .find(|output| output.name == primary)
        .map(|output| output.value_type)
}

fn step_output_value_type(step: &Step, field: &str) -> Option<String> {
    step_specs::step_spec_for(&step.type_name)?
        .outputs
        .into_iter()
        .find(|output| output.name == field)
        .map(|output| output.value_type)
}

fn validate_focused_param_values(
    recipes: &HashMap<String, Recipe>,
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    match step.type_name.as_str() {
        "copy_files" => validate_copy_files_param_values(recipe_id, step, normalized),
        "extract_artifacts" => {
            validate_extract_artifacts_param_values(recipes, recipe_id, step, normalized)
        }
        "extract_archive" => validate_extract_archive_param_values(recipe_id, step, normalized),
        "install_apk" => validate_install_apk_param_values(recipe_id, step, normalized),
        "grant_permissions" => validate_grant_permissions_param_values(recipe_id, step, normalized),
        "wait" => validate_wait_param_values(recipe_id, step, normalized),
        _ => Vec::new(),
    }
}

fn validate_copy_files_param_values(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    errors.extend(require_param(
        recipe_id,
        step,
        normalized,
        "source",
        json!(["input_ref", "artifact_ref", "step_output_ref"]),
    ));
    errors.extend(require_param(
        recipe_id,
        step,
        normalized,
        "dest",
        json!(["literal", "input_ref"]),
    ));
    errors.extend(validate_enum_param(
        recipe_id,
        step,
        normalized,
        "copy_policy",
        &["merge", "replace", "sync"],
    ));
    errors
}

fn validate_extract_artifacts_param_values(
    recipes: &HashMap<String, Recipe>,
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    errors.extend(validate_literal_list_param(
        recipe_id,
        step,
        normalized,
        "artifacts",
    ));
    errors.extend(validate_literal_list_param(
        recipe_id,
        step,
        normalized,
        "artifact_groups",
    ));
    errors.extend(validate_enum_param(
        recipe_id,
        step,
        normalized,
        "extract_on",
        &["host", "device"],
    ));
    if errors.is_empty() && normalized_artifact_selection_is_empty(recipes, recipe_id, normalized) {
        errors.push(param_contract_message(
            "missing_required_param",
            "Param 'artifacts' must select at least one artifact directly or through artifact_groups."
                .to_string(),
            recipe_id,
            step,
            "artifacts",
            json!("literal list via artifacts or artifact_groups"),
            Value::Null,
        ));
    }
    errors
}

fn validate_extract_archive_param_values(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    errors.extend(require_param(
        recipe_id,
        step,
        normalized,
        "archive",
        json!("ref"),
    ));
    errors.extend(validate_enum_param(
        recipe_id,
        step,
        normalized,
        "extract_on",
        &["host", "device"],
    ));
    errors.extend(validate_bool_param(recipe_id, step, normalized, "cleanup"));

    match literal_string(normalized.get("extract_on")).as_deref() {
        Some("device") => {
            errors.extend(require_param(
                recipe_id,
                step,
                normalized,
                "dest",
                json!("literal when extract_on is device"),
            ));
            errors.extend(validate_literal_mode_if_present(
                recipe_id,
                step,
                normalized,
                "dest",
                json!("literal when extract_on is device"),
            ));
            errors.extend(validate_literal_mode_if_present(
                recipe_id,
                step,
                normalized,
                "device_temp_path",
                json!("literal"),
            ));
        }
        Some("host") => {
            for param_name in ["dest", "device_temp_path"] {
                if let Some(value) = step.params.get(param_name) {
                    errors.push(param_contract_message(
                        "invalid_param_value",
                        format!(
                            "Param '{}' is only valid when extract_archive.extract_on is 'device'.",
                            param_name
                        ),
                        recipe_id,
                        step,
                        param_name,
                        json!("only valid when extract_on is device"),
                        param_value_to_json(value),
                    ));
                }
            }
        }
        _ => {}
    }

    errors
}

fn validate_install_apk_param_values(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    errors.extend(require_param(
        recipe_id,
        step,
        normalized,
        "app",
        json!("ref"),
    ));
    errors.extend(validate_bool_param(
        recipe_id,
        step,
        normalized,
        "replace_existing",
    ));
    errors
}

fn validate_wait_param_values(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    errors.extend(require_param(
        recipe_id,
        step,
        normalized,
        "duration_ms",
        json!("literal positive integer"),
    ));
    if let Some(value) = normalized.get("duration_ms") {
        if matches!(value, ParamValue::Ref(_)) {
            return errors;
        }
        if !matches!(value, ParamValue::Literal(value) if is_positive_integer(value)) {
            errors.push(param_contract_message(
                "invalid_param_value",
                "Param 'duration_ms' must be a positive integer for step type 'wait'.".to_string(),
                recipe_id,
                step,
                "duration_ms",
                json!("literal positive integer"),
                param_value_to_json(value),
            ));
        }
    }
    errors
}

fn validate_grant_permissions_param_values(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let mut errors = Vec::new();
    errors.extend(validate_permission_items(
        recipe_id,
        step,
        normalized,
        "runtime",
        &["package_name", "name"],
        &["package_name", "name", "required", "when"],
    ));
    errors.extend(validate_permission_items(
        recipe_id,
        step,
        normalized,
        "appops",
        &["package_name", "op", "mode"],
        &["package_name", "op", "mode", "required", "when"],
    ));
    errors.extend(validate_permission_policy(recipe_id, step, normalized));
    errors
}

fn validate_permission_items(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
    param_name: &str,
    required_fields: &[&str],
    allowed_fields: &[&str],
) -> Vec<PlannerMessage> {
    let Some(value) = normalized.get(param_name) else {
        return Vec::new();
    };
    if matches!(value, ParamValue::Ref(_)) {
        return Vec::new();
    }
    let ParamValue::Literal(Value::Array(items)) = value else {
        return vec![param_contract_message(
            "invalid_param_value",
            format!(
                "Param '{}' must be a literal list for step type 'grant_permissions'.",
                param_name
            ),
            recipe_id,
            step,
            param_name,
            json!("literal list"),
            param_value_to_json(value),
        )];
    };

    let mut errors = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let field_prefix = format!("{param_name}[{index}]");
        let Some(item) = item.as_object() else {
            errors.push(param_contract_message(
                "invalid_param_value",
                format!(
                    "Param '{field_prefix}' must be an object for step type 'grant_permissions'."
                ),
                recipe_id,
                step,
                &field_prefix,
                json!("object"),
                item.clone(),
            ));
            continue;
        };

        for field_name in sorted_unsupported_fields(item, allowed_fields) {
            errors.push(param_contract_message(
                "invalid_param_value",
                format!("Param '{field_prefix}' contains unsupported field '{field_name}'."),
                recipe_id,
                step,
                &format!("{field_prefix}.{field_name}"),
                json!(allowed_fields),
                item.get(&field_name).cloned().unwrap_or(Value::Null),
            ));
        }

        for field_name in required_fields {
            let param = format!("{field_prefix}.{field_name}");
            match item.get(*field_name) {
                Some(Value::String(value)) if !value.trim().is_empty() => {}
                Some(actual) => errors.push(param_contract_message(
                    "invalid_param_value",
                    format!("Param '{param}' must be a non-empty string."),
                    recipe_id,
                    step,
                    &param,
                    json!("non-empty string"),
                    actual.clone(),
                )),
                None => errors.push(param_contract_message(
                    "missing_required_param",
                    format!("Param '{param}' is required for step type 'grant_permissions'."),
                    recipe_id,
                    step,
                    &param,
                    json!("non-empty string"),
                    Value::Null,
                )),
            }
        }

        if let Some(required) = item.get("required") {
            if !required.is_boolean() {
                errors.push(param_contract_message(
                    "invalid_param_value",
                    format!("Param '{field_prefix}.required' must be a boolean."),
                    recipe_id,
                    step,
                    &format!("{field_prefix}.required"),
                    json!("literal bool"),
                    required.clone(),
                ));
            }
        }

        if let Some(when) = item.get("when") {
            errors.extend(validate_permission_when(
                recipe_id,
                step,
                &format!("{field_prefix}.when"),
                when,
            ));
        }
    }
    errors
}

fn validate_permission_policy(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
) -> Vec<PlannerMessage> {
    let Some(value) = normalized.get("policy") else {
        return Vec::new();
    };
    if matches!(value, ParamValue::Ref(_)) {
        return Vec::new();
    }
    let ParamValue::Literal(Value::Object(policy)) = value else {
        return vec![param_contract_message(
            "invalid_param_value",
            "Param 'policy' must be an object for step type 'grant_permissions'.".to_string(),
            recipe_id,
            step,
            "policy",
            json!("object"),
            param_value_to_json(value),
        )];
    };

    let mut errors = Vec::new();
    for field_name in sorted_unsupported_fields(policy, &["on_failure", "require_all"]) {
        errors.push(param_contract_message(
            "invalid_param_value",
            format!("Param 'policy' contains unsupported field '{field_name}'."),
            recipe_id,
            step,
            &format!("policy.{field_name}"),
            json!(["on_failure", "require_all"]),
            policy.get(&field_name).cloned().unwrap_or(Value::Null),
        ));
    }
    if let Some(on_failure) = policy.get("on_failure") {
        if !matches!(on_failure.as_str(), Some("warn" | "fail")) {
            errors.push(param_contract_message(
                "invalid_enum_value",
                "Param 'policy.on_failure' must be one of warn, fail for step type 'grant_permissions'.".to_string(),
                recipe_id,
                step,
                "policy.on_failure",
                json!(["warn", "fail"]),
                on_failure.clone(),
            ));
        }
    }
    if let Some(require_all) = policy.get("require_all") {
        if !require_all.is_boolean() {
            errors.push(param_contract_message(
                "invalid_param_value",
                "Param 'policy.require_all' must be a boolean.".to_string(),
                recipe_id,
                step,
                "policy.require_all",
                json!("literal bool"),
                require_all.clone(),
            ));
        }
    }
    errors
}

fn validate_permission_when(
    recipe_id: &str,
    step: &Step,
    param_name: &str,
    value: &Value,
) -> Vec<PlannerMessage> {
    let Some(when) = value.as_object() else {
        return vec![param_contract_message(
            "invalid_param_value",
            format!("Param '{param_name}' must be an object."),
            recipe_id,
            step,
            param_name,
            json!("object"),
            value.clone(),
        )];
    };

    let mut errors = Vec::new();
    for field_name in
        sorted_unsupported_fields(when, &["rooted", "android_api_min", "android_api_max"])
    {
        errors.push(param_contract_message(
            "invalid_param_value",
            format!("Param '{param_name}' contains unsupported field '{field_name}'."),
            recipe_id,
            step,
            &format!("{param_name}.{field_name}"),
            json!(["rooted", "android_api_min", "android_api_max"]),
            when.get(&field_name).cloned().unwrap_or(Value::Null),
        ));
    }

    if let Some(rooted) = when.get("rooted") {
        if !rooted.is_boolean() {
            errors.push(param_contract_message(
                "invalid_param_value",
                format!("Param '{param_name}.rooted' must be a boolean."),
                recipe_id,
                step,
                &format!("{param_name}.rooted"),
                json!("literal bool"),
                rooted.clone(),
            ));
        }
    }

    for field_name in ["android_api_min", "android_api_max"] {
        if let Some(value) = when.get(field_name) {
            if !is_integer_value(value) {
                errors.push(param_contract_message(
                    "invalid_param_value",
                    format!("Param '{param_name}.{field_name}' must be an integer."),
                    recipe_id,
                    step,
                    &format!("{param_name}.{field_name}"),
                    json!("integer"),
                    value.clone(),
                ));
            }
        }
    }

    let api_min = when.get("android_api_min").and_then(Value::as_i64);
    let api_max = when.get("android_api_max").and_then(Value::as_i64);
    if api_min.zip(api_max).is_some_and(|(min, max)| min > max) {
        errors.push(param_contract_message(
            "invalid_param_value",
            format!("Param '{param_name}' must not set android_api_min above android_api_max."),
            recipe_id,
            step,
            param_name,
            json!("android_api_min <= android_api_max"),
            value.clone(),
        ));
    }

    errors
}

fn sorted_unsupported_fields(item: &Map<String, Value>, allowed_fields: &[&str]) -> Vec<String> {
    let allowed = allowed_fields.iter().copied().collect::<HashSet<_>>();
    let mut unsupported = item
        .keys()
        .filter(|field_name| !allowed.contains(field_name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unsupported.sort();
    unsupported
}

fn is_integer_value(value: &Value) -> bool {
    !value.is_boolean() && (value.as_i64().is_some() || value.as_u64().is_some())
}

fn require_param(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
    param_name: &str,
    expected: Value,
) -> Vec<PlannerMessage> {
    if normalized.contains_key(param_name) {
        return Vec::new();
    }
    vec![param_contract_message(
        "missing_required_param",
        format!(
            "Param '{}' is required for step type '{}'.",
            param_name, step.type_name
        ),
        recipe_id,
        step,
        param_name,
        expected,
        Value::Null,
    )]
}

fn validate_literal_mode_if_present(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
    param_name: &str,
    expected: Value,
) -> Vec<PlannerMessage> {
    let Some(value) = normalized.get(param_name) else {
        return Vec::new();
    };
    if matches!(value, ParamValue::Literal(_)) {
        return Vec::new();
    }
    vec![param_contract_message(
        "invalid_param_mode",
        format!(
            "Param '{}' must be a literal for step type '{}'.",
            param_name, step.type_name
        ),
        recipe_id,
        step,
        param_name,
        expected,
        param_value_to_json(value),
    )]
}

fn validate_literal_list_param(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
    param_name: &str,
) -> Vec<PlannerMessage> {
    let Some(value) = normalized.get(param_name) else {
        return Vec::new();
    };
    if matches!(value, ParamValue::Ref(_)) {
        return Vec::new();
    }
    if matches!(value, ParamValue::Literal(Value::Array(_))) {
        return Vec::new();
    }
    vec![param_contract_message(
        "invalid_param_value",
        format!(
            "Param '{}' must be a literal list for step type '{}'.",
            param_name, step.type_name
        ),
        recipe_id,
        step,
        param_name,
        json!("literal list"),
        param_value_to_json(value),
    )]
}

fn validate_bool_param(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
    param_name: &str,
) -> Vec<PlannerMessage> {
    let Some(value) = normalized.get(param_name) else {
        return Vec::new();
    };
    if matches!(value, ParamValue::Ref(_)) {
        return Vec::new();
    }
    if matches!(value, ParamValue::Literal(Value::Bool(_))) {
        return Vec::new();
    }
    vec![param_contract_message(
        "invalid_param_value",
        format!(
            "Param '{}' must be a literal bool for step type '{}'.",
            param_name, step.type_name
        ),
        recipe_id,
        step,
        param_name,
        json!("literal bool"),
        param_value_to_json(value),
    )]
}

fn validate_enum_param(
    recipe_id: &str,
    step: &Step,
    normalized: &OrderedMap<ParamValue>,
    param_name: &str,
    allowed: &[&str],
) -> Vec<PlannerMessage> {
    let Some(value) = normalized.get(param_name) else {
        return Vec::new();
    };
    if matches!(value, ParamValue::Ref(_)) {
        return Vec::new();
    }
    if let Some(raw_value) = literal_string(Some(value)) {
        if allowed
            .iter()
            .any(|allowed_value| *allowed_value == raw_value)
        {
            return Vec::new();
        }
    }
    vec![param_contract_message(
        "invalid_enum_value",
        format!(
            "Param '{}' must be one of {} for step type '{}'.",
            param_name,
            allowed.join(", "),
            step.type_name
        ),
        recipe_id,
        step,
        param_name,
        json!(allowed),
        param_value_to_json(value),
    )]
}

fn normalized_artifact_selection_is_empty(
    recipes: &HashMap<String, Recipe>,
    recipe_id: &str,
    normalized: &OrderedMap<ParamValue>,
) -> bool {
    let mut selected = string_list(normalized.get("artifacts"));
    if let Some(recipe) = recipes.get(recipe_id) {
        for group_id in string_list(normalized.get("artifact_groups")) {
            if let Some(group_artifacts) = recipe.artifact_groups.get(&group_id) {
                selected.extend(group_artifacts.clone());
            } else {
                selected.push(group_id);
            }
        }
    } else {
        selected.extend(string_list(normalized.get("artifact_groups")));
    }
    selected.is_empty()
}

fn literal_string(value: Option<&ParamValue>) -> Option<String> {
    let Some(ParamValue::Literal(Value::String(value))) = value else {
        return None;
    };
    Some(value.clone())
}

fn is_positive_integer(value: &Value) -> bool {
    value.as_i64().is_some_and(|value| value > 0) || value.as_u64().is_some_and(|value| value > 0)
}

fn expected_param_names(spec: &step_specs::StepSpecDto) -> Value {
    let mut params = spec.params.keys().cloned().collect::<Vec<_>>();
    params.sort();
    json!(params)
}

fn param_value_to_json(value: &ParamValue) -> Value {
    match value {
        ParamValue::Ref(ref_value) => json!({ "ref": ref_value }),
        ParamValue::Literal(value) => value.clone(),
    }
}

fn param_contract_message(
    code: &str,
    message: String,
    recipe_id: &str,
    step: &Step,
    param_name: &str,
    expected: Value,
    actual: Value,
) -> PlannerMessage {
    PlannerMessage {
        code: code.to_string(),
        message,
        details: json!({
            "recipe_ref": recipe_id,
            "step_id": step.id,
            "step_type": step.type_name,
            "param": param_name,
            "expected": expected,
            "actual": actual,
        }),
    }
}

fn validate_available_step_dependencies(
    recipe: &Recipe,
    available_step_ids: &[String],
    by_id: &HashMap<String, &Step>,
) -> Vec<PlannerMessage> {
    // Selection-time validation rejects authored typos, but known unavailable
    // dependencies are handled by recursive selection pruning.
    let mut errors = Vec::new();

    for step_id in available_step_ids {
        let Some(step) = by_id.get(step_id) else {
            continue;
        };
        let mut reported = HashSet::<(&str, &str)>::new();
        for dependency in &step.dependencies {
            if dependency == step_id {
                if reported.insert(("self_step_dependency", dependency.as_str())) {
                    errors.push(PlannerMessage {
                        code: "self_step_dependency".to_string(),
                        message: format!(
                            "Step '{}' may not depend on itself via dependency '{}'.",
                            step.id, dependency
                        ),
                        details: json!({
                            "recipe_ref": recipe.id,
                            "step_id": step.id,
                            "dependency": dependency,
                        }),
                    });
                }
                continue;
            }
            if !by_id.contains_key(dependency)
                && reported.insert(("unknown_step_dependency", dependency.as_str()))
            {
                errors.push(PlannerMessage {
                    code: "unknown_step_dependency".to_string(),
                    message: format!(
                        "Step '{}' depends on unknown authored step '{}'.",
                        step.id, dependency
                    ),
                    details: json!({
                        "recipe_ref": recipe.id,
                        "step_id": step.id,
                        "dependency": dependency,
                    }),
                });
            }
        }
    }

    if errors.is_empty() {
        let requested_steps = available_step_ids
            .iter()
            .filter_map(|step_id| {
                by_id
                    .get(step_id)
                    .map(|step| (step_id.clone(), (*step).clone()))
            })
            .collect::<Vec<_>>();
        errors.extend(validate_step_dependency_cycles(
            &recipe.id,
            &requested_steps,
        ));
    }

    errors
}

fn validate_emitted_step_dependencies(steps: &[(String, String, Step)]) -> Vec<PlannerMessage> {
    let mut grouped_steps = Vec::<(String, Vec<(String, Step)>)>::new();
    for (_, recipe_id, step) in steps {
        let local_step = (step.id.clone(), step.clone());
        if let Some((_, recipe_steps)) = grouped_steps
            .iter_mut()
            .find(|(existing_recipe_id, _)| existing_recipe_id == recipe_id)
        {
            recipe_steps.push(local_step);
        } else {
            grouped_steps.push((recipe_id.clone(), vec![local_step]));
        }
    }

    let mut errors = Vec::new();
    for (recipe_id, recipe_steps) in grouped_steps {
        errors.extend(validate_local_step_dependencies(&recipe_id, &recipe_steps));
    }
    errors
}

fn validate_local_step_dependencies(
    recipe_id: &str,
    steps: &[(String, Step)],
) -> Vec<PlannerMessage> {
    let step_ids = steps
        .iter()
        .map(|(step_id, _)| step_id.clone())
        .collect::<HashSet<_>>();
    let mut errors = Vec::new();

    for (step_id, step) in steps {
        let mut reported = HashSet::<(&str, &str)>::new();
        for dependency in &step.dependencies {
            if dependency == step_id {
                if reported.insert(("self_step_dependency", dependency.as_str())) {
                    errors.push(PlannerMessage {
                        code: "self_step_dependency".to_string(),
                        message: format!(
                            "Step '{}' may not depend on itself via dependency '{}'.",
                            step.id, dependency
                        ),
                        details: json!({
                            "recipe_ref": recipe_id,
                            "step_id": step.id,
                            "dependency": dependency,
                        }),
                    });
                }
                continue;
            }
            if !step_ids.contains(dependency)
                && reported.insert(("unknown_step_dependency", dependency.as_str()))
            {
                errors.push(PlannerMessage {
                    code: "unknown_step_dependency".to_string(),
                    message: format!(
                        "Step '{}' depends on unknown or non-emitted step '{}'.",
                        step.id, dependency
                    ),
                    details: json!({
                        "recipe_ref": recipe_id,
                        "step_id": step.id,
                        "dependency": dependency,
                    }),
                });
            }
        }
    }

    if errors.is_empty() {
        errors.extend(validate_step_dependency_cycles(recipe_id, steps));
    }
    errors
}

fn validate_step_dependency_cycles(
    recipe_id: &str,
    steps: &[(String, Step)],
) -> Vec<PlannerMessage> {
    let step_by_id = steps
        .iter()
        .map(|(step_id, step)| (step_id.as_str(), step))
        .collect::<HashMap<_, _>>();
    let mut permanent = HashSet::<String>::new();
    let mut temporary = HashSet::<String>::new();
    let mut stack = Vec::<String>::new();

    for (step_id, _) in steps {
        if let Some(cycle) = visit_step_dependency_cycle(
            step_id,
            &step_by_id,
            &mut permanent,
            &mut temporary,
            &mut stack,
        ) {
            return vec![PlannerMessage {
                code: "dependency_cycle".to_string(),
                message: format!("Step dependency cycle detected in recipe '{}'.", recipe_id),
                details: json!({
                    "recipe_ref": recipe_id,
                    "cycle": cycle,
                }),
            }];
        }
    }

    Vec::new()
}

fn visit_step_dependency_cycle(
    step_id: &str,
    step_by_id: &HashMap<&str, &Step>,
    permanent: &mut HashSet<String>,
    temporary: &mut HashSet<String>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    if permanent.contains(step_id) {
        return None;
    }
    if temporary.contains(step_id) {
        let start = stack.iter().position(|item| item == step_id).unwrap_or(0);
        let mut cycle = stack[start..].to_vec();
        cycle.push(step_id.to_string());
        return Some(cycle);
    }
    let step = step_by_id.get(step_id)?;

    temporary.insert(step_id.to_string());
    stack.push(step_id.to_string());
    for dependency in unique_dependency_ids(&step.dependencies) {
        if let Some(cycle) =
            visit_step_dependency_cycle(dependency, step_by_id, permanent, temporary, stack)
        {
            return Some(cycle);
        }
    }
    stack.pop();
    temporary.remove(step_id);
    permanent.insert(step_id.to_string());
    None
}

fn unique_dependency_ids(dependencies: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for dependency in dependencies {
        if seen.insert(dependency.as_str()) {
            result.push(dependency.as_str());
        }
    }
    result
}

fn validate_step_refs(steps: &[(String, String, Step)]) -> Vec<PlannerMessage> {
    let selected_steps_by_recipe = steps.iter().fold(
        HashMap::<String, HashMap<String, Step>>::new(),
        |mut acc, (_, recipe_id, step)| {
            acc.entry(recipe_id.clone())
                .or_default()
                .insert(step.id.clone(), step.clone());
            acc
        },
    );

    let mut errors = Vec::new();
    for (_, recipe_id, step) in steps {
        let Some(selected_steps) = selected_steps_by_recipe.get(recipe_id) else {
            continue;
        };
        for (param_name, value) in &step.params {
            let ParamValue::Ref(ref_value) = value else {
                continue;
            };
            if !ref_value.starts_with("steps.") {
                continue;
            }
            errors.extend(validate_step_ref(
                recipe_id,
                step,
                param_name,
                ref_value,
                selected_steps,
            ));
        }
    }
    errors
}

fn validate_step_ref(
    recipe_id: &str,
    step: &Step,
    param_name: &str,
    ref_value: &str,
    selected_steps: &HashMap<String, Step>,
) -> Vec<PlannerMessage> {
    match parse_reference(ref_value) {
        Ok(RuntimeRef::StepShorthand { target_id }) => {
            let Some(target_step) = selected_steps.get(&target_id) else {
                return vec![unknown_step_ref_message(
                    recipe_id, step, param_name, ref_value, &target_id,
                )];
            };
            if step_specs::step_spec_for(&target_step.type_name)
                .and_then(|spec| spec.primary_output_name)
                .is_none()
            {
                return vec![PlannerMessage {
                    code: "step_ref_has_no_primary_output".to_string(),
                    message: format!(
                        "Step '{}' uses shorthand ref '{}', but target step '{}' has no primary output.",
                        step.id, ref_value, target_id
                    ),
                    details: json!({
                        "recipe_ref": recipe_id,
                        "step_id": step.id,
                        "param": param_name,
                        "ref": ref_value,
                        "target_step_id": target_id,
                    }),
                }];
            };
            Vec::new()
        }
        Ok(RuntimeRef::StepOutput { target_id, field }) => {
            let Some(target_step) = selected_steps.get(&target_id) else {
                return vec![unknown_step_ref_message(
                    recipe_id, step, param_name, ref_value, &target_id,
                )];
            };
            let output_exists = step_specs::step_spec_for(&target_step.type_name)
                .is_some_and(|spec| spec.outputs.iter().any(|output| output.name == field));
            if output_exists {
                Vec::new()
            } else {
                vec![PlannerMessage {
                    code: "unknown_step_output".to_string(),
                    message: format!(
                        "Step '{}' references unknown step output '{}'.",
                        step.id, ref_value
                    ),
                    details: json!({
                        "recipe_ref": recipe_id,
                        "step_id": step.id,
                        "param": param_name,
                        "ref": ref_value,
                        "target_step_id": target_id,
                        "output_name": field,
                    }),
                }]
            }
        }
        Err(()) => vec![PlannerMessage {
            code: "invalid_ref_format".to_string(),
            message: format!(
                "Step '{}' param '{}' has invalid ref '{}'.",
                step.id, param_name, ref_value
            ),
            details: json!({
                "recipe_ref": recipe_id,
                "step_id": step.id,
                "param": param_name,
                "ref": ref_value,
            }),
        }],
        Ok(RuntimeRef::Input { .. }) | Ok(RuntimeRef::ArtifactField { .. }) => Vec::new(),
    }
}

fn unknown_step_ref_message(
    recipe_id: &str,
    step: &Step,
    param_name: &str,
    ref_value: &str,
    target_id: &str,
) -> PlannerMessage {
    PlannerMessage {
        code: "unknown_step_ref".to_string(),
        message: format!(
            "Step '{}' references unknown step '{}'.",
            step.id, ref_value
        ),
        details: json!({
            "recipe_ref": recipe_id,
            "step_id": step.id,
            "param": param_name,
            "ref": ref_value,
            "target_step_id": target_id,
        }),
    }
}

fn normalize_step_params_for_execution(
    recipe: &Recipe,
    step: &Step,
) -> OrderedMap<ExecutionParamValue> {
    let normalized = params_with_defaults(step);
    if matches!(
        step.type_name.as_str(),
        "resolve_artifacts" | "extract_artifacts"
    ) {
        return normalize_artifact_selection(recipe, &normalized);
    }

    let mut result = OrderedMap::new();
    let Some(spec) = step_specs::step_spec_for(&step.type_name) else {
        return result;
    };
    for param_name in spec.params.keys() {
        let Some(value) = normalized.get(param_name) else {
            continue;
        };
        match value {
            ParamValue::Ref(ref_value) => {
                result.insert(
                    param_name.clone(),
                    ExecutionParamValue::Ref {
                        ref_value: normalize_ref_for_execution(recipe, ref_value),
                    },
                );
            }
            ParamValue::Literal(value) => {
                result.insert(
                    param_name.clone(),
                    ExecutionParamValue::Literal {
                        value: value.clone(),
                    },
                );
            }
        }
    }
    result
}

fn params_with_defaults(step: &Step) -> OrderedMap<ParamValue> {
    let mut normalized = step.params.clone();
    if let Some(spec) = step_specs::step_spec_for(&step.type_name) {
        for (param_name, default_value) in spec.defaults {
            normalized
                .entry(param_name)
                .or_insert_with(|| ParamValue::Literal(default_value));
        }
    }
    normalized
}

fn normalize_artifact_selection(
    recipe: &Recipe,
    normalized: &OrderedMap<ParamValue>,
) -> OrderedMap<ExecutionParamValue> {
    let mut result = OrderedMap::new();
    let selected = expand_artifact_selection(
        recipe,
        normalized.get("artifacts"),
        normalized.get("artifact_groups"),
    );
    result.insert(
        "artifacts".to_string(),
        ExecutionParamValue::Literal {
            value: Value::Array(
                selected
                    .into_iter()
                    .map(|artifact_id| {
                        Value::String(make_execution_artifact_id(&recipe.id, &artifact_id))
                    })
                    .collect(),
            ),
        },
    );
    if let Some(ParamValue::Literal(value)) = normalized.get("extract_on") {
        result.insert(
            "extract_on".to_string(),
            ExecutionParamValue::Literal {
                value: value.clone(),
            },
        );
    }
    result
}

fn expand_artifact_selection(
    recipe: &Recipe,
    artifacts: Option<&ParamValue>,
    artifact_groups: Option<&ParamValue>,
) -> Vec<String> {
    let mut selected = Vec::new();
    for artifact_id in string_list(artifacts) {
        selected.push(artifact_id);
    }
    for group_id in string_list(artifact_groups) {
        if let Some(group_artifacts) = recipe.artifact_groups.get(&group_id) {
            selected.extend(group_artifacts.clone());
        }
    }
    selected
}

fn string_list(value: Option<&ParamValue>) -> Vec<String> {
    let Some(ParamValue::Literal(Value::Array(items))) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.as_str().map(ToString::to_string))
        .collect()
}

fn normalize_ref_for_execution(recipe: &Recipe, ref_value: &str) -> String {
    match parse_reference(ref_value) {
        Ok(RuntimeRef::Input { target_id }) => {
            format!("inputs.{}", make_execution_input_id(&recipe.id, &target_id))
        }
        Ok(RuntimeRef::ArtifactField { target_id, field }) => {
            format!(
                "artifacts.{}.{}",
                make_execution_artifact_id(&recipe.id, &target_id),
                field
            )
        }
        Ok(RuntimeRef::StepShorthand { target_id }) => {
            let primary_output = recipe
                .steps
                .iter()
                .find(|step| step.id == target_id)
                .and_then(|step| step_specs::step_spec_for(&step.type_name))
                .and_then(|spec| spec.primary_output_name)
                .unwrap_or_default();
            format!(
                "steps.{}.outputs.{}",
                make_execution_step_id(&recipe.id, &target_id),
                primary_output
            )
        }
        Ok(RuntimeRef::StepOutput { target_id, field }) => {
            format!(
                "steps.{}.outputs.{}",
                make_execution_step_id(&recipe.id, &target_id),
                field
            )
        }
        Err(()) => ref_value.to_string(),
    }
}

fn make_execution_step_id(recipe_ref: &str, step_id: &str) -> String {
    format!("{recipe_ref}/{step_id}")
}

fn make_execution_input_id(recipe_ref: &str, input_id: &str) -> String {
    format!("{recipe_ref}/{input_id}")
}

fn make_execution_artifact_id(recipe_ref: &str, artifact_id: &str) -> String {
    format!("{recipe_ref}/{artifact_id}")
}
