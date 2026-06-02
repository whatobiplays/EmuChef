//! Declarative execution-plan emission for Phase 6M/6N planner fixtures.
//!
//! Python remains the reference planner. This module intentionally models only
//! the fixture-covered `PlanningResult`/`ExecutionPlan` emission shape and does
//! not expose protocol requests, run executor operations, inspect devices, or
//! mutate authored files.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use crate::model::{OrderedMap, ParamValue, Recipe, Step, StepCondition, StepConstraints};
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

#[derive(Debug)]
pub struct PlannerLoadError {
    message: String,
}

impl PlannerInput {
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
    let artifact_selection_errors = validate_artifact_selections(&recipes, &authored_steps);
    if !artifact_selection_errors.is_empty() {
        return error_result(artifact_selection_errors);
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
    let entries = fs::read_dir(&recipe_root).map_err(|error| PlannerLoadError {
        message: format!(
            "Failed to read recipe directory {}: {error}",
            recipe_root.display()
        ),
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
            yaml::load_recipe_from_path(&path).map_err(|error| PlannerLoadError {
                message: format!("Failed to load planner recipe {}: {error}", path.display()),
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
        errors.push(PlannerMessage {
            code: "recipe_not_found".to_string(),
            message: format!("Recipe '{recipe_ref}' was not found."),
            details: json!({ "recipe_ref": recipe_ref }),
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
        indegree.insert(step_id.clone(), step.dependencies.len());
        reverse_graph.entry(step_id.clone()).or_default();
        for dependency in &step.dependencies {
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
        if let RuntimeRef::Input { target_id } = parse_reference(ref_value) {
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

fn normalize_step_params_for_execution(
    recipe: &Recipe,
    step: &Step,
) -> OrderedMap<ExecutionParamValue> {
    let normalized = params_with_defaults(&step);
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
        RuntimeRef::Input { target_id } => {
            format!("inputs.{}", make_execution_input_id(&recipe.id, &target_id))
        }
        RuntimeRef::ArtifactField { target_id, field } => {
            format!(
                "artifacts.{}.{}",
                make_execution_artifact_id(&recipe.id, &target_id),
                field
            )
        }
        RuntimeRef::StepShorthand { target_id } => {
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
        RuntimeRef::StepOutput { target_id, field } => {
            format!(
                "steps.{}.outputs.{}",
                make_execution_step_id(&recipe.id, &target_id),
                field
            )
        }
        RuntimeRef::Invalid => ref_value.to_string(),
    }
}

enum RuntimeRef {
    Input { target_id: String },
    ArtifactField { target_id: String, field: String },
    StepShorthand { target_id: String },
    StepOutput { target_id: String, field: String },
    Invalid,
}

fn parse_reference(value: &str) -> RuntimeRef {
    if let Some(target_id) = value.strip_prefix("inputs.") {
        if !target_id.is_empty() {
            return RuntimeRef::Input {
                target_id: target_id.to_string(),
            };
        }
        return RuntimeRef::Invalid;
    }

    if let Some(step_body) = value.strip_prefix("steps.") {
        if let Some((step_id, output_name)) = step_body.split_once(".outputs.") {
            if !step_id.is_empty() && !output_name.is_empty() {
                return RuntimeRef::StepOutput {
                    target_id: step_id.to_string(),
                    field: output_name.to_string(),
                };
            }
            return RuntimeRef::Invalid;
        }
        if !step_body.is_empty() {
            return RuntimeRef::StepShorthand {
                target_id: step_body.to_string(),
            };
        }
        return RuntimeRef::Invalid;
    }

    if let Some(body) = value.strip_prefix("artifacts.") {
        if let Some((artifact_id, field)) = body.rsplit_once('.') {
            if !artifact_id.is_empty() && !field.is_empty() {
                return RuntimeRef::ArtifactField {
                    target_id: artifact_id.to_string(),
                    field: field.to_string(),
                };
            }
        }
    }

    RuntimeRef::Invalid
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
