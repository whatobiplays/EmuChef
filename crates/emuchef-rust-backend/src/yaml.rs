//! Authored recipe YAML loading and canonical emission.
//!
//! This module owns the recipe load and emit behavior shared by the Rust
//! runtime and editor protocol.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Value as YamlValue};

use crate::model::{
    InputDeclaration, InputOption, InputValidation, OrderedMap, ParamValue, Recipe, RecipeProvides,
    RemoteFileArtifact, Step, StepCondition, StepConstraints,
};
use crate::step_specs;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadIssue {
    pub code: &'static str,
    pub message: String,
    pub object_kind: Option<String>,
    pub object_id: Option<String>,
    pub field: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadErrorKind {
    AuthoredData,
    YamlParse,
    Io,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeLoadError {
    pub kind: LoadErrorKind,
    pub message: String,
    pub issue: Option<Box<LoadIssue>>,
}

impl RecipeLoadError {
    fn authored_data(message: impl Into<String>, issue: LoadIssue) -> Self {
        Self {
            kind: LoadErrorKind::AuthoredData,
            message: message.into(),
            issue: Some(Box::new(issue)),
        }
    }

    fn yaml_parse(message: impl Into<String>) -> Self {
        Self {
            kind: LoadErrorKind::YamlParse,
            message: message.into(),
            issue: None,
        }
    }

    fn io(message: impl Into<String>) -> Self {
        Self {
            kind: LoadErrorKind::Io,
            message: message.into(),
            issue: None,
        }
    }
}

impl fmt::Display for RecipeLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RecipeLoadError {}

pub fn emit_recipe_yaml_from_path(path: impl AsRef<Path>) -> Result<String, RecipeLoadError> {
    let recipe = load_recipe_from_path(path)?;
    emit_recipe_yaml(&recipe)
}

pub fn load_recipe_from_path(path: impl AsRef<Path>) -> Result<Recipe, RecipeLoadError> {
    let path = path.as_ref();
    let raw = load_yaml_mapping(path)?;
    let recipe = parse_recipe_mapping(&raw, path)?;
    if let Some(issue) = unsupported_step_issue(&recipe) {
        return Err(RecipeLoadError::authored_data(
            "Authored data validation failed",
            issue,
        ));
    }
    Ok(recipe)
}

pub fn load_yaml_mapping(path: &Path) -> Result<Mapping, RecipeLoadError> {
    let text = fs::read_to_string(path).map_err(|error| RecipeLoadError::io(error.to_string()))?;
    let value = serde_yaml::from_str::<YamlValue>(&text)
        .map_err(|error| RecipeLoadError::yaml_parse(error.to_string()))?;
    match value {
        YamlValue::Null => Ok(Mapping::new()),
        YamlValue::Mapping(mapping) => Ok(mapping),
        _ => {
            let issue = LoadIssue {
                code: "authored_data_invalid",
                message: format!(
                    "File {} must contain a top-level mapping.",
                    single_quote(&path_file_name(path))
                ),
                object_kind: None,
                object_id: None,
                field: None,
            };
            Err(RecipeLoadError::authored_data(
                "Authored data validation failed",
                issue,
            ))
        }
    }
}

pub fn parse_recipe_mapping(raw: &Mapping, path: &Path) -> Result<Recipe, RecipeLoadError> {
    let file_name = path_file_name(path);
    let schema_version = parse_schema_version(get_yaml(raw, "schema_version"));
    if schema_version != Some(1) {
        let issue = LoadIssue {
            code: "authored_data_invalid",
            message: format!(
                "File {} has unsupported schema_version {}.",
                single_quote(&file_name),
                stable_repr(get_yaml(raw, "schema_version"))
            ),
            object_kind: get_string(raw, "kind"),
            object_id: get_scalar_string(raw, "id"),
            field: Some("schema_version".to_string()),
        };
        return Err(RecipeLoadError::authored_data(
            "Authored data validation failed",
            issue,
        ));
    }

    if get_string(raw, "kind").as_deref() != Some("recipe") {
        let issue = LoadIssue {
            code: "authored_data_invalid",
            message: format!(
                "File {} has kind {}, expected 'recipe'.",
                single_quote(&file_name),
                stable_repr(get_yaml(raw, "kind"))
            ),
            object_kind: get_string(raw, "kind"),
            object_id: get_scalar_string(raw, "id"),
            field: Some("kind".to_string()),
        };
        return Err(RecipeLoadError::authored_data(
            "Authored data validation failed",
            issue,
        ));
    }

    if contains_yaml_key(raw, "permissions") {
        let issue = LoadIssue {
            code: "authored_data_invalid",
            message: "Recipe top-level 'permissions' is no longer supported; author permissions under grant_permissions.params.".to_string(),
            object_kind: Some("recipe".to_string()),
            object_id: get_scalar_string(raw, "id"),
            field: Some("permissions".to_string()),
        };
        return Err(RecipeLoadError::authored_data(
            "Authored data validation failed",
            issue,
        ));
    }

    parse_recipe_shape(raw, path).map_err(|message| {
        let issue = LoadIssue {
            code: "authored_data_invalid",
            message: format!(
                "File {} has an invalid schema shape: {message}.",
                single_quote(&file_name)
            ),
            object_kind: Some("recipe".to_string()),
            object_id: get_scalar_string(raw, "id"),
            field: None,
        };
        RecipeLoadError::authored_data("Authored data validation failed", issue)
    })
}

pub fn emit_recipe_yaml(recipe: &Recipe) -> Result<String, RecipeLoadError> {
    let yaml = YamlValue::Mapping(recipe_to_yaml_mapping(recipe));
    let output = serde_yaml::to_string(&yaml)
        .map_err(|error| RecipeLoadError::yaml_parse(error.to_string()))?;
    Ok(output.strip_prefix("---\n").unwrap_or(&output).to_string())
}

pub fn unsupported_step_issue(recipe: &Recipe) -> Option<LoadIssue> {
    recipe.steps.iter().find_map(|step| {
        if step_specs::is_supported_step_type(&step.type_name) {
            None
        } else {
            Some(LoadIssue {
                code: "param_contract_violation",
                message: format!("Unsupported step type {}.", single_quote(&step.type_name)),
                object_kind: Some("recipe".to_string()),
                object_id: Some(recipe.id.clone()),
                field: None,
            })
        }
    })
}

pub fn resolved_path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| absolute_path(path))
        .to_string_lossy()
        .into_owned()
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
}

fn parse_recipe_shape(raw: &Mapping, path: &Path) -> Result<Recipe, String> {
    let id = required_scalar_string(raw, "id")?;
    let name = required_scalar_string(raw, "name")?;
    let inputs = parse_inputs(get_yaml(raw, "inputs"))?;
    let artifacts = parse_artifacts(get_yaml(raw, "artifacts"))?;
    let artifact_groups = parse_artifact_groups(get_yaml(raw, "artifact_groups"))?;
    Ok(Recipe {
        schema_version: 1,
        kind: "recipe".to_string(),
        id,
        name,
        description: optional_string(get_yaml(raw, "description")),
        recipe_dependencies: parse_string_vec(get_yaml(raw, "recipe_dependencies"))?,
        provides: parse_provides(get_yaml(raw, "provides")),
        inputs,
        artifacts,
        artifact_groups,
        steps: parse_steps(get_yaml(raw, "steps"), path)?,
    })
}

fn parse_inputs(value: Option<&YamlValue>) -> Result<OrderedMap<InputDeclaration>, String> {
    let mut inputs = OrderedMap::new();
    let Some(value) = value else {
        return Ok(inputs);
    };
    if value.is_null() {
        return Ok(inputs);
    }
    let YamlValue::Mapping(mapping) = value else {
        return Err("recipe inputs must be a mapping".to_string());
    };
    for (key, value) in mapping {
        let input_id = yaml_key_to_string(key);
        let YamlValue::Mapping(input_map) = value else {
            return Err(format!(
                "input {} must be a mapping",
                single_quote(&input_id)
            ));
        };
        let input = InputDeclaration {
            type_name: required_scalar_string(input_map, "type")?,
            role: get_scalar_string(input_map, "role").unwrap_or_else(|| "generic".to_string()),
            label: get_scalar_string(input_map, "label").unwrap_or_else(|| input_id.clone()),
            description: optional_string(get_yaml(input_map, "description")),
            required: optional_strict_bool(input_map, "required", true)?,
            multiple: optional_strict_bool(input_map, "multiple", false)?,
            validation: parse_input_validation(get_yaml(input_map, "validation"))?,
            default: yaml_to_json(get_yaml(input_map, "default").unwrap_or(&YamlValue::Null)),
            options: parse_input_options(get_yaml(input_map, "options"))?,
            sensitive: optional_strict_bool(input_map, "sensitive", false)?,
            advanced: optional_strict_bool(input_map, "advanced", false)?,
            metadata: parse_json_map(get_yaml(input_map, "metadata"))?,
        };
        validate_input_declaration(&input_id, &input)?;
        inputs.insert(input_id, input);
    }
    Ok(inputs)
}

fn parse_input_validation(value: Option<&YamlValue>) -> Result<InputValidation, String> {
    let mapping = match value {
        None | Some(YamlValue::Null) => None,
        Some(YamlValue::Mapping(mapping)) => Some(mapping),
        Some(_) => return Err("input validation must be a mapping".to_string()),
    };
    Ok(InputValidation {
        must_exist: mapping
            .map(|mapping| optional_strict_bool(mapping, "must_exist", false))
            .transpose()?
            .unwrap_or(false),
        allowed_extensions: mapping
            .map(|mapping| parse_string_vec(get_yaml(mapping, "allowed_extensions")))
            .transpose()?
            .unwrap_or_default(),
        path_kind: mapping.and_then(|mapping| get_scalar_string(mapping, "path_kind")),
        allowed_prefixes: mapping
            .map(|mapping| parse_string_vec(get_yaml(mapping, "allowed_prefixes")))
            .transpose()?
            .unwrap_or_default(),
    })
}

fn parse_input_options(value: Option<&YamlValue>) -> Result<Vec<InputOption>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let YamlValue::Sequence(items) = value else {
        return Err("input options must be a list".to_string());
    };
    items
        .iter()
        .map(|item| {
            let YamlValue::Mapping(mapping) = item else {
                return Err("input option must be a mapping".to_string());
            };
            let value = get_yaml(mapping, "value")
                .map(yaml_to_json)
                .ok_or_else(|| "input option requires 'value'".to_string())?;
            let label = get_scalar_string(mapping, "label")
                .unwrap_or_else(|| deterministic_option_label(&value));
            if label.trim().is_empty() {
                return Err("input option label must not be empty".to_string());
            }
            Ok(InputOption { value, label })
        })
        .collect()
}

fn deterministic_option_label(value: &JsonValue) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
}

fn validate_input_declaration(input_id: &str, input: &InputDeclaration) -> Result<(), String> {
    const SUPPORTED_TYPES: &[&str] = &[
        "string",
        "integer",
        "boolean",
        "enum",
        "file",
        "directory",
        "path",
        "device_path",
        "string_list",
        "path_list",
        "object",
    ];
    if !SUPPORTED_TYPES.contains(&input.type_name.as_str()) {
        return Err(format!(
            "input {} has unsupported type {}",
            single_quote(input_id),
            single_quote(&input.type_name)
        ));
    }
    if let Some(path_kind) = &input.validation.path_kind {
        if !matches!(path_kind.as_str(), "file" | "directory") {
            return Err(format!(
                "input {} validation.path_kind must be 'file' or 'directory'",
                single_quote(input_id)
            ));
        }
    }
    if input
        .validation
        .allowed_prefixes
        .iter()
        .any(|prefix| !prefix.starts_with('/'))
    {
        return Err(format!(
            "input {} validation.allowed_prefixes entries must be absolute paths",
            single_quote(input_id)
        ));
    }
    if input.type_name == "enum" && input.options.is_empty() {
        return Err(format!(
            "enum input {} requires at least one option",
            single_quote(input_id)
        ));
    }
    let mut option_values = Vec::new();
    for option in &input.options {
        if option_values.contains(&option.value) {
            return Err(format!(
                "input {} has duplicate option value {}",
                single_quote(input_id),
                stable_json_repr(&option.value)
            ));
        }
        option_values.push(option.value.clone());
    }
    if !input.default.is_null() && !input.value_matches_type(&input.default) {
        return Err(format!(
            "input {} default is incompatible with type {}",
            single_quote(input_id),
            single_quote(&input.type_name)
        ));
    }
    if input.type_name == "enum"
        && !input.default.is_null()
        && !option_values.contains(&input.default)
    {
        return Err(format!(
            "input {} default {} is not an enum option",
            single_quote(input_id),
            stable_json_repr(&input.default)
        ));
    }
    Ok(())
}

fn stable_json_repr(value: &JsonValue) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn optional_strict_bool(mapping: &Mapping, key: &str, default: bool) -> Result<bool, String> {
    match get_yaml(mapping, key) {
        None => Ok(default),
        Some(YamlValue::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("'{key}' must be a boolean")),
    }
}

fn parse_artifacts(value: Option<&YamlValue>) -> Result<OrderedMap<RemoteFileArtifact>, String> {
    let mut artifacts = OrderedMap::new();
    let Some(value) = value else {
        return Ok(artifacts);
    };
    if value.is_null() {
        return Ok(artifacts);
    }
    let YamlValue::Mapping(mapping) = value else {
        return Err("recipe artifacts must be a mapping".to_string());
    };
    for (key, value) in mapping {
        let artifact_id = yaml_key_to_string(key);
        let YamlValue::Mapping(artifact_map) = value else {
            return Err(format!(
                "artifact {} must be a mapping",
                single_quote(&artifact_id)
            ));
        };
        let type_name = required_scalar_string(artifact_map, "type")?;
        if type_name != "remote_file" {
            return Err(format!(
                "Unsupported artifact type: {}",
                single_quote(&type_name)
            ));
        }
        artifacts.insert(
            artifact_id,
            RemoteFileArtifact {
                type_name,
                url: required_scalar_string(artifact_map, "url")?,
                cache: get_scalar_string(artifact_map, "cache")
                    .unwrap_or_else(|| "default".to_string()),
            },
        );
    }
    Ok(artifacts)
}

fn parse_artifact_groups(value: Option<&YamlValue>) -> Result<OrderedMap<Vec<String>>, String> {
    let mut groups = OrderedMap::new();
    let Some(value) = value else {
        return Ok(groups);
    };
    if value.is_null() {
        return Ok(groups);
    }
    let YamlValue::Mapping(mapping) = value else {
        return Err("recipe artifact_groups must be a mapping".to_string());
    };
    for (key, value) in mapping {
        groups.insert(yaml_key_to_string(key), parse_string_vec(Some(value))?);
    }
    Ok(groups)
}

fn parse_provides(value: Option<&YamlValue>) -> RecipeProvides {
    let features = value
        .and_then(YamlValue::as_mapping)
        .and_then(|mapping| parse_string_vec(get_yaml(mapping, "features")).ok())
        .unwrap_or_default();
    RecipeProvides { features }
}

fn parse_steps(value: Option<&YamlValue>, _path: &Path) -> Result<Vec<Step>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    match value {
        YamlValue::Sequence(items) => items.iter().map(parse_step).collect(),
        YamlValue::Mapping(mapping) if mapping.is_empty() => Ok(Vec::new()),
        _ => Err("recipe steps must be a list".to_string()),
    }
}

fn parse_step(value: &YamlValue) -> Result<Step, String> {
    let YamlValue::Mapping(mapping) = value else {
        return Err("step must be a mapping".to_string());
    };
    Ok(Step {
        id: required_scalar_string(mapping, "id")?,
        type_name: required_scalar_string(mapping, "type")?,
        name: required_scalar_string(mapping, "name")?,
        description: optional_string(get_yaml(mapping, "description")),
        progress_note: optional_string(get_yaml(mapping, "progress_note")),
        user_toggleable: get_bool(mapping, "user_toggleable")
            .ok_or_else(|| "'user_toggleable'".to_string())?,
        dependencies: parse_string_vec(get_yaml(mapping, "dependencies"))?,
        constraints: parse_constraints(get_yaml(mapping, "constraints"))?,
        skip_if: parse_conditions(get_yaml(mapping, "skip_if"))?,
        params: parse_params(get_yaml(mapping, "params"))?,
        verify: parse_conditions(get_yaml(mapping, "verify"))?,
    })
}

fn parse_constraints(value: Option<&YamlValue>) -> Result<StepConstraints, String> {
    let Some(value) = value else {
        return Ok(StepConstraints {
            capabilities: Vec::new(),
            conflicts_with: Vec::new(),
        });
    };
    if value.is_null() {
        return Ok(StepConstraints {
            capabilities: Vec::new(),
            conflicts_with: Vec::new(),
        });
    }
    let YamlValue::Mapping(mapping) = value else {
        return Err("step constraints must be a mapping".to_string());
    };
    Ok(StepConstraints {
        capabilities: parse_string_vec(get_yaml(mapping, "capabilities"))?,
        conflicts_with: parse_string_vec(get_yaml(mapping, "conflicts_with"))?,
    })
}

fn parse_conditions(value: Option<&YamlValue>) -> Result<Vec<StepCondition>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let YamlValue::Sequence(items) = value else {
        return Err("step conditions must be a list".to_string());
    };
    items
        .iter()
        .map(|item| {
            let YamlValue::Mapping(mapping) = item else {
                return Err("step condition must be a mapping".to_string());
            };
            Ok(StepCondition {
                type_name: required_scalar_string(mapping, "type")?,
                params: parse_json_map(get_yaml(mapping, "params"))?,
            })
        })
        .collect()
}

fn parse_params(value: Option<&YamlValue>) -> Result<OrderedMap<ParamValue>, String> {
    let mut params = OrderedMap::new();
    let Some(value) = value else {
        return Ok(params);
    };
    if value.is_null() {
        return Ok(params);
    }
    let YamlValue::Mapping(mapping) = value else {
        return Err("step params must be a mapping".to_string());
    };
    for (key, value) in mapping {
        let param_name = yaml_key_to_string(key);
        params.insert(param_name, parse_param_value(value)?);
    }
    Ok(params)
}

fn parse_param_value(value: &YamlValue) -> Result<ParamValue, String> {
    if let YamlValue::Mapping(mapping) = value {
        if mapping.len() == 1 {
            if let Some(ref_value) = get_yaml(mapping, "ref").and_then(YamlValue::as_str) {
                validate_ref_syntax(ref_value)?;
                return Ok(ParamValue::Ref(ref_value.to_string()));
            }
        }
    }
    Ok(ParamValue::Literal(yaml_to_json(value)))
}

fn validate_ref_syntax(ref_value: &str) -> Result<(), String> {
    if let Some(input_id) = ref_value.strip_prefix("inputs.") {
        if !input_id.is_empty() {
            return Ok(());
        }
    }
    if let Some(rest) = ref_value.strip_prefix("artifacts.") {
        if let Some((artifact_id, field)) = rest.rsplit_once('.') {
            if !artifact_id.is_empty() && !field.is_empty() {
                return Ok(());
            }
        }
    }
    if let Some(rest) = ref_value.strip_prefix("steps.") {
        if let Some((step_id, output_name)) = rest.split_once(".outputs.") {
            if !step_id.is_empty() && !output_name.is_empty() {
                return Ok(());
            }
        } else if !rest.is_empty() {
            return Ok(());
        }
    }
    Err(format!("Invalid authored ref: {}", single_quote(ref_value)))
}

fn parse_json_map(value: Option<&YamlValue>) -> Result<OrderedMap<JsonValue>, String> {
    let mut result = OrderedMap::new();
    let Some(value) = value else {
        return Ok(result);
    };
    if value.is_null() {
        return Ok(result);
    }
    let YamlValue::Mapping(mapping) = value else {
        return Err("value must be a mapping".to_string());
    };
    for (key, value) in mapping {
        result.insert(yaml_key_to_string(key), yaml_to_json(value));
    }
    Ok(result)
}

fn parse_string_vec(value: Option<&YamlValue>) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let YamlValue::Sequence(items) = value else {
        return Err("value must be a list".to_string());
    };
    Ok(items.iter().map(yaml_value_to_string).collect())
}

fn recipe_to_yaml_mapping(recipe: &Recipe) -> Mapping {
    let mut mapping = Mapping::new();
    insert(
        &mut mapping,
        "schema_version",
        YamlValue::Number(recipe.schema_version.into()),
    );
    insert(&mut mapping, "kind", YamlValue::String(recipe.kind.clone()));
    insert(&mut mapping, "id", YamlValue::String(recipe.id.clone()));
    insert(&mut mapping, "name", YamlValue::String(recipe.name.clone()));
    if let Some(description) = &recipe.description {
        insert(
            &mut mapping,
            "description",
            YamlValue::String(description.clone()),
        );
    }
    insert(
        &mut mapping,
        "recipe_dependencies",
        string_sequence(&recipe.recipe_dependencies),
    );
    let mut provides = Mapping::new();
    insert(
        &mut provides,
        "features",
        string_sequence(&recipe.provides.features),
    );
    insert(&mut mapping, "provides", YamlValue::Mapping(provides));
    insert(&mut mapping, "inputs", inputs_to_yaml(&recipe.inputs));
    insert(
        &mut mapping,
        "artifacts",
        artifacts_to_yaml(&recipe.artifacts),
    );
    insert(
        &mut mapping,
        "artifact_groups",
        artifact_groups_to_yaml(&recipe.artifact_groups),
    );
    insert(
        &mut mapping,
        "steps",
        YamlValue::Sequence(recipe.steps.iter().map(step_to_yaml).collect()),
    );
    mapping
}

fn inputs_to_yaml(inputs: &OrderedMap<InputDeclaration>) -> YamlValue {
    let mut mapping = Mapping::new();
    for (id, input) in inputs {
        let mut payload = Mapping::new();
        insert(
            &mut payload,
            "type",
            YamlValue::String(input.type_name.clone()),
        );
        insert(&mut payload, "role", YamlValue::String(input.role.clone()));
        insert(
            &mut payload,
            "label",
            YamlValue::String(input.label.clone()),
        );
        if let Some(description) = &input.description {
            insert(
                &mut payload,
                "description",
                YamlValue::String(description.clone()),
            );
        }
        insert(&mut payload, "required", YamlValue::Bool(input.required));
        insert(&mut payload, "multiple", YamlValue::Bool(input.multiple));
        let mut validation = Mapping::new();
        insert(
            &mut validation,
            "must_exist",
            YamlValue::Bool(input.validation.must_exist),
        );
        insert(
            &mut validation,
            "allowed_extensions",
            string_sequence(&input.validation.allowed_extensions),
        );
        if let Some(path_kind) = &input.validation.path_kind {
            insert(
                &mut validation,
                "path_kind",
                YamlValue::String(path_kind.clone()),
            );
        }
        if !input.validation.allowed_prefixes.is_empty() {
            insert(
                &mut validation,
                "allowed_prefixes",
                string_sequence(&input.validation.allowed_prefixes),
            );
        }
        insert(&mut payload, "validation", YamlValue::Mapping(validation));
        insert(&mut payload, "default", json_to_yaml_sorted(&input.default));
        if !input.options.is_empty() {
            insert(
                &mut payload,
                "options",
                YamlValue::Sequence(
                    input
                        .options
                        .iter()
                        .map(|option| {
                            let mut option_mapping = Mapping::new();
                            insert(
                                &mut option_mapping,
                                "value",
                                json_to_yaml_sorted(&option.value),
                            );
                            insert(
                                &mut option_mapping,
                                "label",
                                YamlValue::String(option.label.clone()),
                            );
                            YamlValue::Mapping(option_mapping)
                        })
                        .collect(),
                ),
            );
        }
        if input.sensitive {
            insert(&mut payload, "sensitive", YamlValue::Bool(true));
        }
        if input.advanced {
            insert(&mut payload, "advanced", YamlValue::Bool(true));
        }
        if !input.metadata.is_empty() {
            insert(
                &mut payload,
                "metadata",
                ordered_json_map_to_yaml_sorted(&input.metadata),
            );
        }
        insert(&mut mapping, id, YamlValue::Mapping(payload));
    }
    YamlValue::Mapping(mapping)
}

fn artifacts_to_yaml(artifacts: &OrderedMap<RemoteFileArtifact>) -> YamlValue {
    let mut mapping = Mapping::new();
    for (id, artifact) in artifacts {
        let mut payload = Mapping::new();
        insert(
            &mut payload,
            "type",
            YamlValue::String(artifact.type_name.clone()),
        );
        insert(&mut payload, "url", YamlValue::String(artifact.url.clone()));
        insert(
            &mut payload,
            "cache",
            YamlValue::String(artifact.cache.clone()),
        );
        insert(&mut mapping, id, YamlValue::Mapping(payload));
    }
    YamlValue::Mapping(mapping)
}

fn artifact_groups_to_yaml(groups: &OrderedMap<Vec<String>>) -> YamlValue {
    let mut mapping = Mapping::new();
    for (id, members) in groups {
        insert(&mut mapping, id, string_sequence(members));
    }
    YamlValue::Mapping(mapping)
}

fn step_to_yaml(step: &Step) -> YamlValue {
    let mut mapping = Mapping::new();
    insert(&mut mapping, "id", YamlValue::String(step.id.clone()));
    insert(
        &mut mapping,
        "type",
        YamlValue::String(step.type_name.clone()),
    );
    insert(&mut mapping, "name", YamlValue::String(step.name.clone()));
    if let Some(description) = &step.description {
        insert(
            &mut mapping,
            "description",
            YamlValue::String(description.clone()),
        );
    }
    if let Some(progress_note) = &step.progress_note {
        insert(
            &mut mapping,
            "progress_note",
            YamlValue::String(progress_note.clone()),
        );
    }
    insert(
        &mut mapping,
        "user_toggleable",
        YamlValue::Bool(step.user_toggleable),
    );
    insert(
        &mut mapping,
        "dependencies",
        string_sequence(&step.dependencies),
    );
    let mut constraints = Mapping::new();
    insert(
        &mut constraints,
        "capabilities",
        string_sequence(&step.constraints.capabilities),
    );
    insert(
        &mut constraints,
        "conflicts_with",
        string_sequence(&step.constraints.conflicts_with),
    );
    insert(&mut mapping, "constraints", YamlValue::Mapping(constraints));
    insert(
        &mut mapping,
        "skip_if",
        YamlValue::Sequence(step.skip_if.iter().map(condition_to_yaml).collect()),
    );
    insert(&mut mapping, "params", params_to_yaml(step));
    insert(
        &mut mapping,
        "verify",
        YamlValue::Sequence(step.verify.iter().map(condition_to_yaml).collect()),
    );
    YamlValue::Mapping(mapping)
}

fn params_to_yaml(step: &Step) -> YamlValue {
    let mut mapping = Mapping::new();
    let mut ordered_names = Vec::new();
    if let Some(spec) = step_specs::step_spec_for(&step.type_name) {
        for name in spec.param_order {
            push_unique(&mut ordered_names, name);
        }
        for name in spec.params.keys() {
            push_unique(&mut ordered_names, name.clone());
        }
    }
    let mut extras: Vec<String> = step
        .params
        .keys()
        .filter(|name| !ordered_names.contains(name))
        .cloned()
        .collect();
    extras.sort();
    ordered_names.extend(extras);
    for name in ordered_names {
        if let Some(value) = step.params.get(&name) {
            insert(&mut mapping, &name, param_to_yaml(value));
        }
    }
    YamlValue::Mapping(mapping)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn param_to_yaml(value: &ParamValue) -> YamlValue {
    match value {
        ParamValue::Ref(ref_value) => {
            let mut mapping = Mapping::new();
            insert(&mut mapping, "ref", YamlValue::String(ref_value.clone()));
            YamlValue::Mapping(mapping)
        }
        ParamValue::Literal(value) => json_to_yaml_sorted(value),
    }
}

fn condition_to_yaml(condition: &StepCondition) -> YamlValue {
    let mut mapping = Mapping::new();
    insert(
        &mut mapping,
        "type",
        YamlValue::String(condition.type_name.clone()),
    );
    insert(
        &mut mapping,
        "params",
        ordered_json_map_to_yaml_sorted(&condition.params),
    );
    YamlValue::Mapping(mapping)
}

fn ordered_json_map_to_yaml_sorted(values: &OrderedMap<JsonValue>) -> YamlValue {
    let mut keys: Vec<&String> = values.keys().collect();
    keys.sort();
    let mut mapping = Mapping::new();
    for key in keys {
        insert(&mut mapping, key, json_to_yaml_sorted(&values[key]));
    }
    YamlValue::Mapping(mapping)
}

fn json_to_yaml_sorted(value: &JsonValue) -> YamlValue {
    match value {
        JsonValue::Null => YamlValue::Null,
        JsonValue::Bool(value) => YamlValue::Bool(*value),
        JsonValue::Number(_) | JsonValue::String(_) => {
            serde_yaml::to_value(value).expect("JSON scalar should convert to YAML")
        }
        JsonValue::Array(items) => {
            YamlValue::Sequence(items.iter().map(json_to_yaml_sorted).collect())
        }
        JsonValue::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            let mut mapping = Mapping::new();
            for key in keys {
                insert(&mut mapping, key, json_to_yaml_sorted(&object[key]));
            }
            YamlValue::Mapping(mapping)
        }
    }
}

fn string_sequence(values: &[String]) -> YamlValue {
    YamlValue::Sequence(
        values
            .iter()
            .map(|item| YamlValue::String(item.clone()))
            .collect(),
    )
}

fn yaml_to_json(value: &YamlValue) -> JsonValue {
    match value {
        YamlValue::Null => JsonValue::Null,
        YamlValue::Bool(value) => JsonValue::Bool(*value),
        YamlValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                JsonValue::from(value)
            } else if let Some(value) = number.as_u64() {
                JsonValue::from(value)
            } else if let Some(value) = number.as_f64() {
                JsonValue::from(value)
            } else {
                JsonValue::Null
            }
        }
        YamlValue::String(value) => JsonValue::String(value.clone()),
        YamlValue::Sequence(items) => JsonValue::Array(items.iter().map(yaml_to_json).collect()),
        YamlValue::Mapping(mapping) => {
            let mut object = serde_json::Map::new();
            for (key, value) in mapping {
                object.insert(yaml_key_to_string(key), yaml_to_json(value));
            }
            JsonValue::Object(object)
        }
        YamlValue::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

fn get_bool(mapping: &Mapping, key: &str) -> Option<bool> {
    get_yaml(mapping, key).map(value_to_bool)
}

fn value_to_bool(value: &YamlValue) -> bool {
    match value {
        YamlValue::Null => false,
        YamlValue::Bool(value) => *value,
        YamlValue::Number(number) => {
            number.as_i64().is_some_and(|value| value != 0)
                || number.as_u64().is_some_and(|value| value != 0)
                || number.as_f64().is_some_and(|value| value != 0.0)
        }
        YamlValue::String(value) => !value.is_empty(),
        YamlValue::Sequence(values) => !values.is_empty(),
        YamlValue::Mapping(values) => !values.is_empty(),
        YamlValue::Tagged(tagged) => value_to_bool(&tagged.value),
    }
}

fn get_string(mapping: &Mapping, key: &str) -> Option<String> {
    get_yaml(mapping, key)
        .and_then(YamlValue::as_str)
        .map(ToOwned::to_owned)
}

fn get_scalar_string(mapping: &Mapping, key: &str) -> Option<String> {
    get_yaml(mapping, key).map(yaml_value_to_string)
}

fn required_scalar_string(mapping: &Mapping, key: &str) -> Result<String, String> {
    get_scalar_string(mapping, key).ok_or_else(|| format!("'{key}'"))
}

fn optional_string(value: Option<&YamlValue>) -> Option<String> {
    match value {
        None | Some(YamlValue::Null) => None,
        Some(value) => Some(yaml_value_to_string(value)),
    }
}

fn yaml_value_to_string(value: &YamlValue) -> String {
    match value {
        YamlValue::Null => "None".to_string(),
        YamlValue::Bool(true) => "True".to_string(),
        YamlValue::Bool(false) => "False".to_string(),
        YamlValue::Number(number) => number.to_string(),
        YamlValue::String(value) => value.clone(),
        YamlValue::Sequence(_) | YamlValue::Mapping(_) => serde_yaml::to_string(value)
            .unwrap_or_else(|_| String::new())
            .trim()
            .to_string(),
        YamlValue::Tagged(tagged) => yaml_value_to_string(&tagged.value),
    }
}

fn parse_schema_version(value: Option<&YamlValue>) -> Option<i64> {
    match value {
        Some(YamlValue::Number(number)) => number.as_i64(),
        Some(YamlValue::String(value)) => value.parse::<i64>().ok(),
        Some(YamlValue::Tagged(tagged)) => parse_schema_version(Some(&tagged.value)),
        _ => None,
    }
}

fn stable_repr(value: Option<&YamlValue>) -> String {
    match value {
        None | Some(YamlValue::Null) => "None".to_string(),
        Some(YamlValue::String(value)) => format!("{value:?}").replace('"', "'"),
        Some(YamlValue::Bool(true)) => "True".to_string(),
        Some(YamlValue::Bool(false)) => "False".to_string(),
        Some(YamlValue::Number(number)) => number.to_string(),
        Some(value) => yaml_value_to_string(value),
    }
}

fn get_yaml<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

fn contains_yaml_key(mapping: &Mapping, key: &str) -> bool {
    mapping.contains_key(YamlValue::String(key.to_string()))
}

fn insert(mapping: &mut Mapping, key: &str, value: YamlValue) {
    mapping.insert(YamlValue::String(key.to_string()), value);
}

fn yaml_key_to_string(key: &YamlValue) -> String {
    match key {
        YamlValue::String(value) => value.clone(),
        _ => yaml_value_to_string(key),
    }
}

fn path_file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn yaml_value(text: &str) -> YamlValue {
        serde_yaml::from_str(text).expect("test YAML value should parse")
    }

    #[test]
    fn exact_single_key_ref_mapping_becomes_ref_param() {
        assert_eq!(
            parse_param_value(&yaml_value("ref: steps.extract")).unwrap(),
            ParamValue::Ref("steps.extract".to_string())
        );

        assert_eq!(
            parse_param_value(&yaml_value("{ref: steps.extract, label: literal}")).unwrap(),
            ParamValue::Literal(json!({"label": "literal", "ref": "steps.extract"}))
        );

        assert_eq!(
            parse_param_value(&yaml_value("wrapper:\n  ref: steps.extract")).unwrap(),
            ParamValue::Literal(json!({"wrapper": {"ref": "steps.extract"}}))
        );
    }
}
