//! In-memory recipe document state for sidecar sessions.
//!
//! A document wraps the current authored recipe model with the editor-facing
//! lifecycle state needed for sidecar document sessions. The Rust backend keeps
//! planner behavior and executor behavior out of this crate-local migration
//! slice while preserving the stored authoredRoot needed for validation refresh.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::catalog;
use crate::commands::{ArtifactField, InputField, OverviewField, OverviewValue, RecipeCommand};
use crate::model::{
    InputDeclaration, InputValidation, OrderedMap, ParamValue, Recipe, RemoteFileArtifact, Step,
    StepCondition, StepConstraints,
};
use crate::step_specs;
use crate::validation;
use crate::yaml::{self, RecipeLoadError};

#[derive(Clone, Debug)]
pub struct RecipeDocument {
    path: PathBuf,
    authored_root: Option<PathBuf>,
    recipe: Recipe,
    current_yaml: String,
    saved_yaml: String,
    diagnostics: Vec<Value>,
    undo_stack: Vec<ContentSnapshot>,
    redo_stack: Vec<ContentSnapshot>,
}

#[derive(Clone, Debug)]
struct ContentSnapshot {
    recipe: Recipe,
    current_yaml: String,
    diagnostics: Vec<Value>,
}

impl RecipeDocument {
    pub fn open(
        path: impl AsRef<Path>,
        authored_root: Option<&str>,
    ) -> Result<Self, RecipeLoadError> {
        let input_path = path.as_ref();
        let recipe = yaml::load_recipe_from_path(input_path)?;
        let current_yaml = yaml::emit_recipe_yaml(&recipe)?;
        let path = PathBuf::from(yaml::resolved_path_string(input_path));
        let authored_root = catalog::normalize_authored_root(authored_root, &path);
        let diagnostics =
            validation_diagnostics_for_recipe(&recipe, &path, authored_root.as_deref());

        Ok(Self {
            path,
            authored_root,
            recipe,
            saved_yaml: current_yaml.clone(),
            current_yaml,
            diagnostics,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.current_yaml =
            yaml::emit_recipe_yaml(&self.recipe).map_err(|error| error.to_string())?;
        fs::write(&self.path, &self.current_yaml).map_err(|error| error.to_string())?;
        self.saved_yaml = self.current_yaml.clone();
        self.refresh_diagnostics();
        Ok(())
    }

    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        let current_yaml =
            yaml::emit_recipe_yaml(&self.recipe).map_err(|error| error.to_string())?;
        fs::write(path.as_ref(), &current_yaml).map_err(|error| error.to_string())?;
        self.path = PathBuf::from(yaml::resolved_path_string(path.as_ref()));
        self.current_yaml = current_yaml;
        self.saved_yaml = self.current_yaml.clone();
        self.refresh_diagnostics();
        Ok(())
    }

    pub fn apply_command(&mut self, command: RecipeCommand) -> Result<bool, String> {
        let updated_recipe = self.updated_recipe(command)?;
        let updated_yaml =
            yaml::emit_recipe_yaml(&updated_recipe).map_err(|error| error.to_string())?;
        if updated_yaml == self.current_yaml {
            return Ok(false);
        }

        let previous = self.content_snapshot();
        self.recipe = updated_recipe;
        self.current_yaml = updated_yaml;
        self.refresh_diagnostics();
        self.undo_stack.push(previous);
        self.redo_stack.clear();
        Ok(true)
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo_stack.pop() else {
            return false;
        };
        let current = self.content_snapshot();
        self.restore_content(previous);
        self.refresh_diagnostics();
        self.redo_stack.push(current);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let current = self.content_snapshot();
        self.restore_content(next);
        self.refresh_diagnostics();
        self.undo_stack.push(current);
        true
    }

    pub fn validate(&mut self) {
        self.refresh_diagnostics();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn authored_root(&self) -> Option<&Path> {
        self.authored_root.as_deref()
    }

    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    pub fn yaml(&self) -> &str {
        &self.current_yaml
    }

    pub fn diagnostics(&self) -> &[Value] {
        &self.diagnostics
    }

    pub fn is_dirty(&self) -> bool {
        self.current_yaml != self.saved_yaml
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    fn updated_recipe(&self, command: RecipeCommand) -> Result<Recipe, String> {
        match command {
            RecipeCommand::SetOverviewField { field, value } => {
                self.updated_overview_recipe(field, value)
            }
            RecipeCommand::AddInput { input_id } => self.updated_add_input_recipe(input_id),
            RecipeCommand::RenameInput {
                input_id,
                new_input_id,
            } => self.updated_rename_input_recipe(input_id, new_input_id),
            RecipeCommand::UpdateInputField {
                input_id,
                field,
                value,
            } => self.updated_input_field_recipe(input_id, field, value),
            RecipeCommand::DeleteInput { input_id } => self.updated_delete_input_recipe(input_id),
            RecipeCommand::DuplicateInput {
                source_input_id,
                new_input_id,
            } => self.updated_duplicate_input_recipe(source_input_id, new_input_id),
            RecipeCommand::AddArtifact { artifact_id, url } => {
                self.updated_add_artifact_recipe(artifact_id, url)
            }
            RecipeCommand::RenameArtifact {
                artifact_id,
                new_artifact_id,
            } => self.updated_rename_artifact_recipe(artifact_id, new_artifact_id),
            RecipeCommand::UpdateArtifactField {
                artifact_id,
                field,
                value,
            } => self.updated_artifact_field_recipe(artifact_id, field, value),
            RecipeCommand::DeleteArtifact { artifact_id } => {
                self.updated_delete_artifact_recipe(artifact_id)
            }
            RecipeCommand::DuplicateArtifact {
                source_artifact_id,
                new_artifact_id,
            } => self.updated_duplicate_artifact_recipe(source_artifact_id, new_artifact_id),
            RecipeCommand::AddArtifactGroup { group_id } => {
                self.updated_add_artifact_group_recipe(group_id)
            }
            RecipeCommand::RenameArtifactGroup {
                group_id,
                new_group_id,
            } => self.updated_rename_artifact_group_recipe(group_id, new_group_id),
            RecipeCommand::DeleteArtifactGroup { group_id } => {
                self.updated_delete_artifact_group_recipe(group_id)
            }
            RecipeCommand::DuplicateArtifactGroup {
                source_group_id,
                new_group_id,
            } => self.updated_duplicate_artifact_group_recipe(source_group_id, new_group_id),
            RecipeCommand::ReorderArtifactGroup { group_id, to_index } => {
                self.updated_reorder_artifact_group_recipe(group_id, to_index)
            }
            RecipeCommand::AddArtifactGroupMember {
                group_id,
                artifact_id,
                index,
            } => self.updated_add_artifact_group_member_recipe(group_id, artifact_id, index),
            RecipeCommand::RemoveArtifactGroupMember { group_id, index } => {
                self.updated_remove_artifact_group_member_recipe(group_id, index)
            }
            RecipeCommand::ReorderArtifactGroupMember {
                group_id,
                index,
                to_index,
            } => self.updated_reorder_artifact_group_member_recipe(group_id, index, to_index),
            RecipeCommand::AddStep {
                step_id,
                step_type,
                name,
                index,
            } => self.updated_add_step_recipe(step_id, step_type, name, index),
            RecipeCommand::DeleteStep { step_id } => self.updated_delete_step_recipe(step_id),
            RecipeCommand::DuplicateStep {
                source_step_id,
                new_step_id,
            } => self.updated_duplicate_step_recipe(source_step_id, new_step_id),
            RecipeCommand::ReorderStep { step_id, to_index } => {
                self.updated_reorder_step_recipe(step_id, to_index)
            }
            RecipeCommand::UpdateStepBasics {
                step_id,
                name,
                description,
            } => self.updated_step_basics_recipe(step_id, name, description),
            RecipeCommand::SetStepUserToggleable {
                step_id,
                user_toggleable,
            } => self.updated_step_user_toggleable_recipe(step_id, user_toggleable),
            RecipeCommand::UpdateStepDependencies {
                step_id,
                dependencies,
            } => self.updated_step_dependencies_recipe(step_id, dependencies),
            RecipeCommand::UpdateStepParams { step_id, params } => {
                self.updated_step_params_recipe(step_id, params)
            }
            RecipeCommand::UpdateStepConstraints {
                step_id,
                constraints,
            } => self.updated_step_constraints_recipe(step_id, constraints),
            RecipeCommand::UpdateStepSkipIf { step_id, skip_if } => {
                self.updated_step_skip_if_recipe(step_id, skip_if)
            }
            RecipeCommand::UpdateStepVerify { step_id, verify } => {
                self.updated_step_verify_recipe(step_id, verify)
            }
        }
    }

    fn updated_overview_recipe(
        &self,
        field: OverviewField,
        value: OverviewValue,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        match field {
            OverviewField::Name => {
                let OverviewValue::Text(value) = value else {
                    return Err("recipe name must not be null.".to_string());
                };
                let name = value.trim();
                if name.is_empty() {
                    return Err("recipe name must not be empty.".to_string());
                }
                recipe.name = name.to_string();
            }
            OverviewField::Description => {
                recipe.description = match value {
                    OverviewValue::Null => None,
                    OverviewValue::Text(value) if value.trim().is_empty() => None,
                    OverviewValue::Text(value) => Some(value),
                };
            }
        }
        Ok(recipe)
    }

    fn updated_add_input_recipe(&self, input_id: String) -> Result<Recipe, String> {
        let input_id = normalize_identifier(&input_id, "input id")?;
        let mut recipe = self.recipe.clone();
        if recipe.inputs.contains_key(&input_id) {
            return Err(format!("Input {input_id:?} already exists."));
        }
        recipe.inputs.insert(
            input_id.clone(),
            InputDeclaration {
                type_name: "file".to_string(),
                role: "generic".to_string(),
                label: input_id,
                description: None,
                required: false,
                multiple: false,
                validation: InputValidation {
                    must_exist: false,
                    allowed_extensions: Vec::new(),
                    path_kind: Some("file".to_string()),
                },
                default: Value::Null,
                metadata: OrderedMap::new(),
            },
        );
        Ok(recipe)
    }

    fn updated_rename_input_recipe(
        &self,
        input_id: String,
        new_input_id: String,
    ) -> Result<Recipe, String> {
        let new_input_id = normalize_identifier(&new_input_id, "input id")?;
        let mut recipe = self.recipe.clone();
        let input = recipe
            .inputs
            .get(&input_id)
            .cloned()
            .ok_or_else(|| format!("Unknown input id {input_id:?}."))?;
        if new_input_id != input_id && recipe.inputs.contains_key(&new_input_id) {
            return Err(format!("Input {new_input_id:?} already exists."));
        }
        recipe.inputs = rename_ordered_map_key(&recipe.inputs, &input_id, &new_input_id, input)?;
        // Mirrors Python commands.py _rename_input / tests/test_editor_core.py
        // test_rename_commands_rewrite_supported_structured_usages.
        recipe.steps = recipe
            .steps
            .iter()
            .map(|step| rewrite_step_refs(step, RefTargetKind::Input, &input_id, &new_input_id))
            .collect();
        Ok(recipe)
    }

    fn updated_input_field_recipe(
        &self,
        input_id: String,
        field: InputField,
        value: Value,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let input = recipe
            .inputs
            .get(&input_id)
            .cloned()
            .ok_or_else(|| format!("Unknown input id {input_id:?}."))?;
        let mut updated = input;
        match field {
            InputField::Type => updated.type_name = coerce_input_type(&value)?,
            InputField::Role => updated.role = coerce_input_role(&value)?,
            InputField::Label => updated.label = json_to_python_string(&value),
            InputField::Description => updated.description = optional_text(&value),
            InputField::Required => updated.required = json_truthy(&value),
            InputField::Multiple => updated.multiple = json_truthy(&value),
            InputField::ValidationMustExist => updated.validation.must_exist = json_truthy(&value),
            InputField::ValidationAllowedExtensions => {
                updated.validation.allowed_extensions = coerce_allowed_extensions(&value)?
            }
            InputField::ValidationPathKind => {
                updated.validation.path_kind = coerce_optional_input_type(&value)?
            }
        }
        *recipe
            .inputs
            .get_mut(&input_id)
            .ok_or_else(|| format!("Unknown input id {input_id:?}."))? = updated;
        Ok(recipe)
    }

    fn updated_delete_input_recipe(&self, input_id: String) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        if recipe.inputs.shift_remove(&input_id).is_none() {
            return Err(format!("Unknown input {input_id:?}."));
        }
        // Mirrors Python commands.py _delete_input / tests/test_editor_core.py
        // test_delete_input_and_artifact_group_remove_supported_structured_refs_only.
        recipe.steps = recipe
            .steps
            .iter()
            .map(|step| remove_step_refs(step, RefTargetKind::Input, &input_id))
            .collect();
        Ok(recipe)
    }

    fn updated_duplicate_input_recipe(
        &self,
        source_input_id: String,
        new_input_id: String,
    ) -> Result<Recipe, String> {
        let new_input_id = normalize_identifier(&new_input_id, "input id")?;
        let mut recipe = self.recipe.clone();
        let input = recipe
            .inputs
            .get(&source_input_id)
            .cloned()
            .ok_or_else(|| format!("Unknown input id {source_input_id:?}."))?;
        if recipe.inputs.contains_key(&new_input_id) {
            return Err(format!("Input {new_input_id:?} already exists."));
        }
        recipe.inputs.insert(new_input_id, input);
        Ok(recipe)
    }

    fn updated_add_artifact_recipe(
        &self,
        artifact_id: String,
        url: String,
    ) -> Result<Recipe, String> {
        let artifact_id = normalize_identifier(&artifact_id, "artifact id")?;
        let mut recipe = self.recipe.clone();
        if recipe.artifacts.contains_key(&artifact_id) {
            return Err(format!("Artifact {artifact_id:?} already exists."));
        }
        recipe.artifacts.insert(
            artifact_id,
            RemoteFileArtifact {
                type_name: "remote_file".to_string(),
                url: required_text(&Value::String(url), "artifact url")?,
                cache: "default".to_string(),
            },
        );
        Ok(recipe)
    }

    fn updated_rename_artifact_recipe(
        &self,
        artifact_id: String,
        new_artifact_id: String,
    ) -> Result<Recipe, String> {
        let new_artifact_id = normalize_identifier(&new_artifact_id, "artifact id")?;
        let mut recipe = self.recipe.clone();
        let artifact = recipe
            .artifacts
            .get(&artifact_id)
            .cloned()
            .ok_or_else(|| format!("Unknown artifact id {artifact_id:?}."))?;
        if new_artifact_id != artifact_id && recipe.artifacts.contains_key(&new_artifact_id) {
            return Err(format!("Artifact {new_artifact_id:?} already exists."));
        }
        recipe.artifacts =
            rename_ordered_map_key(&recipe.artifacts, &artifact_id, &new_artifact_id, artifact)?;
        for members in recipe.artifact_groups.values_mut() {
            for member in members {
                if member == &artifact_id {
                    *member = new_artifact_id.clone();
                }
            }
        }
        // Mirrors Python commands.py _rename_artifact / tests/test_editor_core.py
        // test_rename_commands_rewrite_supported_structured_usages.
        recipe.steps = recipe
            .steps
            .iter()
            .map(|step| {
                rewrite_step_refs(
                    step,
                    RefTargetKind::Artifact,
                    &artifact_id,
                    &new_artifact_id,
                )
            })
            .collect();
        Ok(recipe)
    }

    fn updated_artifact_field_recipe(
        &self,
        artifact_id: String,
        field: ArtifactField,
        value: Value,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let artifact = recipe
            .artifacts
            .get_mut(&artifact_id)
            .ok_or_else(|| format!("Unknown artifact id {artifact_id:?}."))?;
        match field {
            ArtifactField::Url => artifact.url = required_text(&value, "artifact url")?,
            ArtifactField::Cache => artifact.cache = coerce_artifact_cache(&value)?,
        }
        Ok(recipe)
    }

    fn updated_delete_artifact_recipe(&self, artifact_id: String) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        if recipe.artifacts.shift_remove(&artifact_id).is_none() {
            return Err(format!("Unknown artifact {artifact_id:?}."));
        }
        for members in recipe.artifact_groups.values_mut() {
            members.retain(|member| member != &artifact_id);
        }
        // Mirrors Python commands.py _delete_artifact / tests/test_editor_core.py
        // test_delete_artifact_cleans_group_membership_step_selection_and_param_ref_in_one_command.
        recipe.steps = recipe
            .steps
            .iter()
            .map(|step| remove_step_refs(step, RefTargetKind::Artifact, &artifact_id))
            .collect();
        Ok(recipe)
    }

    fn updated_duplicate_artifact_recipe(
        &self,
        source_artifact_id: String,
        new_artifact_id: String,
    ) -> Result<Recipe, String> {
        let new_artifact_id = normalize_identifier(&new_artifact_id, "artifact id")?;
        let mut recipe = self.recipe.clone();
        let artifact = recipe
            .artifacts
            .get(&source_artifact_id)
            .cloned()
            .ok_or_else(|| format!("Unknown artifact id {source_artifact_id:?}."))?;
        if recipe.artifacts.contains_key(&new_artifact_id) {
            return Err(format!("Artifact {new_artifact_id:?} already exists."));
        }
        recipe.artifacts.insert(new_artifact_id, artifact);
        Ok(recipe)
    }

    fn updated_add_artifact_group_recipe(&self, group_id: String) -> Result<Recipe, String> {
        let group_id = normalize_identifier(&group_id, "artifact group id")?;
        let mut recipe = self.recipe.clone();
        if recipe.artifact_groups.contains_key(&group_id) {
            return Err(format!("Artifact group {group_id:?} already exists."));
        }
        recipe.artifact_groups.insert(group_id, Vec::new());
        Ok(recipe)
    }

    fn updated_rename_artifact_group_recipe(
        &self,
        group_id: String,
        new_group_id: String,
    ) -> Result<Recipe, String> {
        let new_group_id = normalize_identifier(&new_group_id, "artifact group id")?;
        let mut recipe = self.recipe.clone();
        let members = recipe
            .artifact_groups
            .get(&group_id)
            .cloned()
            .ok_or_else(|| format!("Unknown artifact group {group_id:?}."))?;
        if new_group_id != group_id && recipe.artifact_groups.contains_key(&new_group_id) {
            return Err(format!("Artifact group {new_group_id:?} already exists."));
        }
        recipe.artifact_groups =
            rename_ordered_map_key(&recipe.artifact_groups, &group_id, &new_group_id, members)?;
        // Mirrors Python commands.py _rename_artifact_group / tests/test_editor_core.py
        // test_rename_commands_rewrite_supported_structured_usages.
        recipe.steps = recipe
            .steps
            .iter()
            .map(|step| rewrite_artifact_group_selection(step, &group_id, &new_group_id))
            .collect();
        Ok(recipe)
    }

    fn updated_delete_artifact_group_recipe(&self, group_id: String) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        if recipe.artifact_groups.shift_remove(&group_id).is_none() {
            return Err(format!("Unknown artifact group {group_id:?}."));
        }
        // Mirrors Python commands.py _delete_artifact_group / tests/test_editor_core.py
        // test_delete_input_and_artifact_group_remove_supported_structured_refs_only.
        recipe.steps = recipe
            .steps
            .iter()
            .map(|step| remove_artifact_group_selection(step, &group_id))
            .collect();
        Ok(recipe)
    }

    fn updated_duplicate_artifact_group_recipe(
        &self,
        source_group_id: String,
        new_group_id: String,
    ) -> Result<Recipe, String> {
        let new_group_id = normalize_identifier(&new_group_id, "artifact group id")?;
        let mut recipe = self.recipe.clone();
        let members = recipe
            .artifact_groups
            .get(&source_group_id)
            .cloned()
            .ok_or_else(|| format!("Unknown artifact group {source_group_id:?}."))?;
        if recipe.artifact_groups.contains_key(&new_group_id) {
            return Err(format!("Artifact group {new_group_id:?} already exists."));
        }
        recipe.artifact_groups.insert(new_group_id, members);
        Ok(recipe)
    }

    fn updated_reorder_artifact_group_recipe(
        &self,
        group_id: String,
        to_index: i64,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let from_index = recipe
            .artifact_groups
            .get_index_of(&group_id)
            .ok_or_else(|| format!("Unknown artifact group {group_id:?}."))?;
        recipe.artifact_groups =
            move_ordered_map_item(&recipe.artifact_groups, from_index, to_index)?;
        Ok(recipe)
    }

    fn updated_add_artifact_group_member_recipe(
        &self,
        group_id: String,
        artifact_id: String,
        index: Option<i64>,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        if !recipe.artifacts.contains_key(&artifact_id) {
            return Err(format!("Unknown artifact {artifact_id:?}."));
        }
        let members = recipe
            .artifact_groups
            .get_mut(&group_id)
            .ok_or_else(|| format!("Unknown artifact group {group_id:?}."))?;
        if members.contains(&artifact_id) {
            return Err(format!(
                "Artifact {artifact_id:?} is already in group {group_id:?}."
            ));
        }
        let insertion_index = match index {
            Some(index) => {
                validate_insert_index(index, members.len(), "artifact group membership")?
            }
            None => members.len(),
        };
        members.insert(insertion_index, artifact_id);
        Ok(recipe)
    }

    fn updated_remove_artifact_group_member_recipe(
        &self,
        group_id: String,
        index: i64,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let members = recipe
            .artifact_groups
            .get_mut(&group_id)
            .ok_or_else(|| format!("Unknown artifact group {group_id:?}."))?;
        let index = validate_index(index, members.len(), "artifact group membership")?;
        members.remove(index);
        Ok(recipe)
    }

    fn updated_reorder_artifact_group_member_recipe(
        &self,
        group_id: String,
        index: i64,
        to_index: i64,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let members = recipe
            .artifact_groups
            .get_mut(&group_id)
            .ok_or_else(|| format!("Unknown artifact group {group_id:?}."))?;
        let from_index = validate_index(index, members.len(), "reorder source")?;
        let to_index = validate_index(to_index, members.len(), "reorder target")?;
        let member = members.remove(from_index);
        members.insert(to_index, member);
        Ok(recipe)
    }

    fn updated_add_step_recipe(
        &self,
        step_id: String,
        step_type: String,
        name: String,
        index: Option<i64>,
    ) -> Result<Recipe, String> {
        let step_id = normalize_identifier(&step_id, "step id")?;
        let mut recipe = self.recipe.clone();
        if recipe.steps.iter().any(|step| step.id == step_id) {
            return Err(format!("Step {step_id:?} already exists."));
        }
        if step_specs::step_spec_for(&step_type).is_none() {
            return Err(format!("Unsupported step type {step_type:?}."));
        }
        let insertion_index = match index {
            Some(index) => validate_insert_index(index, recipe.steps.len(), "step")?,
            None => recipe.steps.len(),
        };
        recipe.steps.insert(
            insertion_index,
            Step {
                id: step_id,
                type_name: step_type,
                name: required_text(&Value::String(name), "step name")?,
                description: None,
                user_toggleable: false,
                dependencies: Vec::new(),
                constraints: StepConstraints {
                    capabilities: Vec::new(),
                    conflicts_with: Vec::new(),
                },
                skip_if: Vec::new(),
                params: OrderedMap::new(),
                verify: Vec::new(),
            },
        );
        Ok(recipe)
    }

    fn updated_delete_step_recipe(&self, step_id: String) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        recipe.steps.remove(index);
        // Mirrors Python commands.py _delete_step/_remove_step_refs and
        // tests/test_editor_core.py test_step_delete_removes_supported_dependencies_conflicts_and_refs.
        recipe.steps = recipe
            .steps
            .iter()
            .map(|step| remove_step_refs(step, RefTargetKind::Step, &step_id))
            .collect();
        Ok(recipe)
    }

    fn updated_duplicate_step_recipe(
        &self,
        source_step_id: String,
        new_step_id: String,
    ) -> Result<Recipe, String> {
        let new_step_id = normalize_identifier(&new_step_id, "step id")?;
        let mut recipe = self.recipe.clone();
        if recipe.steps.iter().any(|step| step.id == new_step_id) {
            return Err(format!("Step {new_step_id:?} already exists."));
        }
        let source_index = step_index(&recipe.steps, &source_step_id)?;
        let mut copied = recipe.steps[source_index].clone();
        copied.id = new_step_id;
        recipe.steps.insert(source_index + 1, copied);
        Ok(recipe)
    }

    fn updated_reorder_step_recipe(
        &self,
        step_id: String,
        to_index: i64,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let from_index = step_index(&recipe.steps, &step_id)?;
        let to_index = validate_index(to_index, recipe.steps.len(), "reorder target")?;
        let step = recipe.steps.remove(from_index);
        recipe.steps.insert(to_index, step);
        Ok(recipe)
    }

    fn updated_step_basics_recipe(
        &self,
        step_id: String,
        name: String,
        description: Value,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        let step = &mut recipe.steps[index];
        step.name = required_text(&Value::String(name), "step name")?;
        step.description = optional_text(&description);
        Ok(recipe)
    }

    fn updated_step_user_toggleable_recipe(
        &self,
        step_id: String,
        user_toggleable: bool,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        recipe.steps[index].user_toggleable = user_toggleable;
        Ok(recipe)
    }

    fn updated_step_dependencies_recipe(
        &self,
        step_id: String,
        dependencies: Vec<String>,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        // Python command application only normalizes and deduplicates dependency
        // ids; planner/catalog existence checks are intentionally out of scope.
        recipe.steps[index].dependencies =
            normalize_identifier_list(dependencies, "step dependency id")?;
        Ok(recipe)
    }

    fn updated_step_params_recipe(
        &self,
        step_id: String,
        params: OrderedMap<ParamValue>,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        let step_type = recipe.steps[index].type_name.clone();
        recipe.steps[index].params = normalize_step_params(&step_type, params);
        Ok(recipe)
    }

    fn updated_step_constraints_recipe(
        &self,
        step_id: String,
        constraints: StepConstraints,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        recipe.steps[index].constraints = StepConstraints {
            capabilities: normalize_identifier_list(constraints.capabilities, "step capability")?,
            conflicts_with: normalize_identifier_list(constraints.conflicts_with, "step conflict")?,
        };
        Ok(recipe)
    }

    fn updated_step_skip_if_recipe(
        &self,
        step_id: String,
        skip_if: Vec<StepCondition>,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        recipe.steps[index].skip_if = skip_if;
        Ok(recipe)
    }

    fn updated_step_verify_recipe(
        &self,
        step_id: String,
        verify: Vec<StepCondition>,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        recipe.steps[index].verify = verify;
        Ok(recipe)
    }

    fn content_snapshot(&self) -> ContentSnapshot {
        ContentSnapshot {
            recipe: self.recipe.clone(),
            current_yaml: self.current_yaml.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    fn restore_content(&mut self, snapshot: ContentSnapshot) {
        self.recipe = snapshot.recipe;
        self.current_yaml = snapshot.current_yaml;
        self.diagnostics = snapshot.diagnostics;
    }

    fn refresh_diagnostics(&mut self) {
        self.diagnostics = validation_diagnostics_for_recipe(
            &self.recipe,
            &self.path,
            self.authored_root.as_deref(),
        );
    }
}

fn validation_diagnostics_for_recipe(
    recipe: &Recipe,
    path: &Path,
    authored_root: Option<&Path>,
) -> Vec<Value> {
    validation::validate_loaded_recipe_result(recipe, path, authored_root)
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[derive(Copy, Clone)]
enum RefTargetKind {
    Input,
    Artifact,
    Step,
}

fn rename_ordered_map_key<T: Clone>(
    mapping: &OrderedMap<T>,
    old_key: &str,
    new_key: &str,
    new_value: T,
) -> Result<OrderedMap<T>, String> {
    if !mapping.contains_key(old_key) {
        return Err(format!("Unknown mapping entry {old_key:?}."));
    }
    let mut updated = OrderedMap::new();
    for (key, value) in mapping {
        if key == old_key {
            updated.insert(new_key.to_string(), new_value.clone());
        } else {
            updated.insert(key.clone(), value.clone());
        }
    }
    Ok(updated)
}

fn move_ordered_map_item<T: Clone>(
    mapping: &OrderedMap<T>,
    from_index: usize,
    to_index: i64,
) -> Result<OrderedMap<T>, String> {
    let to_index = validate_index(to_index, mapping.len(), "reorder target")?;
    let mut items = mapping
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();
    let item = items.remove(from_index);
    items.insert(to_index, item);
    Ok(items.into_iter().collect())
}

fn rewrite_step_refs(step: &Step, target_kind: RefTargetKind, old_id: &str, new_id: &str) -> Step {
    let mut updated = step.clone();
    let mut params = step.params.clone();
    let mut changed = false;
    for (param_name, value) in &step.params {
        if !is_supported_step_param(step, param_name) {
            continue;
        }
        match value {
            ParamValue::Ref(ref_value) => {
                if let Some(rewritten) = rewrite_ref(ref_value, target_kind, old_id, new_id) {
                    params.insert(param_name.clone(), ParamValue::Ref(rewritten));
                    changed = true;
                }
            }
            ParamValue::Literal(value)
                if matches!(target_kind, RefTargetKind::Artifact) && param_name == "artifacts" =>
            {
                if let Some(rewritten) = rewrite_string_sequence_value(value, old_id, new_id) {
                    params.insert(param_name.clone(), ParamValue::Literal(rewritten));
                    changed = true;
                }
            }
            _ => {}
        }
    }
    if changed {
        updated.params = params;
    }
    if matches!(target_kind, RefTargetKind::Step) {
        let mut changed = false;
        let dependencies = updated
            .dependencies
            .iter()
            .map(|dependency| {
                if dependency == old_id {
                    changed = true;
                    new_id.to_string()
                } else {
                    dependency.clone()
                }
            })
            .collect::<Vec<_>>();
        let conflicts_with = updated
            .constraints
            .conflicts_with
            .iter()
            .map(|conflict| {
                if conflict == old_id {
                    changed = true;
                    new_id.to_string()
                } else {
                    conflict.clone()
                }
            })
            .collect::<Vec<_>>();
        if changed {
            updated.dependencies = dependencies;
            updated.constraints.conflicts_with = conflicts_with;
        }
    }
    updated
}

fn remove_step_refs(step: &Step, target_kind: RefTargetKind, target_id: &str) -> Step {
    let mut updated = step.clone();
    let mut params = step.params.clone();
    let mut changed = false;
    for (param_name, value) in &step.params {
        if !is_supported_step_param(step, param_name) {
            continue;
        }
        match value {
            ParamValue::Ref(ref_value) if ref_matches(ref_value, target_kind, target_id) => {
                params.shift_remove(param_name);
                changed = true;
            }
            ParamValue::Literal(value)
                if matches!(target_kind, RefTargetKind::Artifact) && param_name == "artifacts" =>
            {
                if let Some(rewritten) = remove_string_sequence_value(value, target_id) {
                    params.insert(param_name.clone(), ParamValue::Literal(rewritten));
                    changed = true;
                }
            }
            _ => {}
        }
    }
    if changed {
        updated.params = params;
    }
    if matches!(target_kind, RefTargetKind::Step) {
        let dependencies = updated
            .dependencies
            .iter()
            .filter(|dependency| dependency.as_str() != target_id)
            .cloned()
            .collect::<Vec<_>>();
        let conflicts_with = updated
            .constraints
            .conflicts_with
            .iter()
            .filter(|conflict| conflict.as_str() != target_id)
            .cloned()
            .collect::<Vec<_>>();
        if dependencies != updated.dependencies
            || conflicts_with != updated.constraints.conflicts_with
        {
            updated.dependencies = dependencies;
            updated.constraints.conflicts_with = conflicts_with;
        }
    }
    updated
}

fn rewrite_artifact_group_selection(step: &Step, group_id: &str, new_group_id: &str) -> Step {
    if !is_supported_step_param(step, "artifact_groups") {
        return step.clone();
    }
    let Some(ParamValue::Literal(value)) = step.params.get("artifact_groups") else {
        return step.clone();
    };
    let Some(rewritten) = rewrite_string_sequence_value(value, group_id, new_group_id) else {
        return step.clone();
    };
    let mut updated = step.clone();
    updated.params.insert(
        "artifact_groups".to_string(),
        ParamValue::Literal(rewritten),
    );
    updated
}

fn remove_artifact_group_selection(step: &Step, group_id: &str) -> Step {
    if !is_supported_step_param(step, "artifact_groups") {
        return step.clone();
    }
    let Some(ParamValue::Literal(value)) = step.params.get("artifact_groups") else {
        return step.clone();
    };
    let Some(rewritten) = remove_string_sequence_value(value, group_id) else {
        return step.clone();
    };
    let mut updated = step.clone();
    updated.params.insert(
        "artifact_groups".to_string(),
        ParamValue::Literal(rewritten),
    );
    updated
}

fn rewrite_ref(
    ref_value: &str,
    target_kind: RefTargetKind,
    old_id: &str,
    new_id: &str,
) -> Option<String> {
    match target_kind {
        RefTargetKind::Input => {
            (ref_value == format!("inputs.{old_id}")).then(|| format!("inputs.{new_id}"))
        }
        RefTargetKind::Artifact => {
            let rest = ref_value.strip_prefix("artifacts.")?;
            let (artifact_id, field) = rest.rsplit_once('.')?;
            (artifact_id == old_id).then(|| format!("artifacts.{new_id}.{field}"))
        }
        RefTargetKind::Step => {
            if ref_value == format!("steps.{old_id}") {
                return Some(format!("steps.{new_id}"));
            }
            let output_prefix = format!("steps.{old_id}.outputs.");
            let output_name = ref_value.strip_prefix(&output_prefix)?;
            (!output_name.is_empty()).then(|| format!("steps.{new_id}.outputs.{output_name}"))
        }
    }
}

fn ref_matches(ref_value: &str, target_kind: RefTargetKind, target_id: &str) -> bool {
    rewrite_ref(ref_value, target_kind, target_id, target_id).is_some()
}

fn is_supported_step_param(step: &Step, param_name: &str) -> bool {
    step_specs::step_spec_for(&step.type_name)
        .is_some_and(|spec| spec.params.contains_key(param_name))
}

fn rewrite_string_sequence_value(value: &Value, old_id: &str, new_id: &str) -> Option<Value> {
    let Value::Array(items) = value else {
        return None;
    };
    let mut changed = false;
    let rewritten = items
        .iter()
        .map(|item| {
            if item.as_str() == Some(old_id) {
                changed = true;
                Value::String(new_id.to_string())
            } else {
                item.clone()
            }
        })
        .collect::<Vec<_>>();
    changed.then_some(Value::Array(rewritten))
}

fn remove_string_sequence_value(value: &Value, target_id: &str) -> Option<Value> {
    let Value::Array(items) = value else {
        return None;
    };
    let mut changed = false;
    let filtered = items
        .iter()
        .filter_map(|item| {
            if item.as_str() == Some(target_id) {
                changed = true;
                None
            } else {
                Some(item.clone())
            }
        })
        .collect::<Vec<_>>();
    changed.then_some(Value::Array(filtered))
}

fn validate_index(index: i64, size: usize, label: &str) -> Result<usize, String> {
    if index < 0 || index as usize >= size {
        return Err(format!("{label} index {index} is out of range."));
    }
    Ok(index as usize)
}

fn validate_insert_index(index: i64, size: usize, label: &str) -> Result<usize, String> {
    if index < 0 || index as usize > size {
        return Err(format!("{label} index {index} is out of range."));
    }
    Ok(index as usize)
}

fn normalize_identifier(value: &str, label: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(format!("{label} must not be empty."));
    }
    Ok(normalized.to_string())
}

fn normalize_identifier_list(values: Vec<String>, label: &str) -> Result<Vec<String>, String> {
    let normalized = values
        .iter()
        .map(|value| normalize_identifier(value, label))
        .collect::<Result<Vec<_>, _>>()?;
    let mut unique = normalized.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != normalized.len() {
        return Err(format!("{label}s must be unique."));
    }
    Ok(normalized)
}

fn normalize_step_params(
    step_type: &str,
    params: OrderedMap<ParamValue>,
) -> OrderedMap<ParamValue> {
    let mut normalized = params;
    let Some(spec) = step_specs::step_spec_for(step_type) else {
        return normalized;
    };
    for (param_name, default) in spec.defaults {
        let should_remove = normalized
            .get(&param_name)
            .is_some_and(|value| param_value_equals_python_default(value, &default));
        if should_remove {
            normalized.shift_remove(&param_name);
        }
    }
    normalized
}

fn param_value_equals_python_default(value: &ParamValue, default: &Value) -> bool {
    match value {
        ParamValue::Ref(_) => false,
        ParamValue::Literal(value) => python_json_equal(value, default),
    }
}

fn python_json_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Bool(left), Value::Number(right)) => bool_equals_python_number(*left, right),
        (Value::Number(left), Value::Bool(right)) => bool_equals_python_number(*right, left),
        (Value::Number(left), Value::Number(right)) => match (left.as_f64(), right.as_f64()) {
            (Some(left), Some(right)) => left == right,
            _ => left == right,
        },
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| python_json_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left_value)| {
                    right
                        .get(key)
                        .is_some_and(|right_value| python_json_equal(left_value, right_value))
                })
        }
        _ => false,
    }
}

fn bool_equals_python_number(bool_value: bool, number: &serde_json::Number) -> bool {
    let expected = if bool_value { 1.0 } else { 0.0 };
    number.as_f64() == Some(expected)
}

fn step_index(steps: &[Step], step_id: &str) -> Result<usize, String> {
    steps
        .iter()
        .position(|step| step.id == step_id)
        .ok_or_else(|| format!("Unknown step {step_id:?}."))
}

fn required_text(value: &Value, label: &str) -> Result<String, String> {
    let text = json_to_python_string(value);
    normalize_identifier(&text, label)
}

fn optional_text(value: &Value) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let text = json_to_python_string(value);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn json_to_python_string(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => json_to_python_repr(value),
    }
}

fn json_to_python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(value) => format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(json_to_python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(object) => format!(
            "{{{}}}",
            object
                .iter()
                .map(|(key, value)| format!(
                    "'{}': {}",
                    key.replace('\\', "\\\\").replace('\'', "\\'"),
                    json_to_python_repr(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => {
            number.as_i64().is_some_and(|value| value != 0)
                || number.as_u64().is_some_and(|value| value != 0)
                || number.as_f64().is_some_and(|value| value != 0.0)
        }
        Value::String(value) => !value.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
    }
}

fn coerce_input_type(value: &Value) -> Result<String, String> {
    let value = json_to_python_string(value);
    match value.as_str() {
        "file" | "directory" => Ok(value),
        _ => Err(format!("Invalid input type {value:?}.")),
    }
}

fn coerce_optional_input_type(value: &Value) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let value = json_to_python_string(value);
    if value.is_empty() {
        return Ok(None);
    }
    coerce_input_type(&Value::String(value)).map(Some)
}

fn coerce_input_role(value: &Value) -> Result<String, String> {
    let value = json_to_python_string(value);
    match value.as_str() {
        "apk" | "bios" | "roms" | "config_bundle" | "generic" => Ok(value),
        _ => Err(format!("Invalid input role {value:?}.")),
    }
}

fn coerce_artifact_cache(value: &Value) -> Result<String, String> {
    let value = json_to_python_string(value);
    match value.as_str() {
        "default" | "none" => Ok(value),
        _ => Err(format!("Invalid artifact cache mode {value:?}.")),
    }
}

fn coerce_allowed_extensions(value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(value) => Ok(value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect()),
        Value::Array(items) => {
            let mut extensions = Vec::new();
            for item in items {
                let value = json_to_python_string(item);
                if value.trim().is_empty() {
                    continue;
                }
                extensions.push(normalize_identifier(&value, "extension")?);
            }
            Ok(extensions)
        }
        _ => Err("allowed_extensions must be a string, list, or tuple.".to_string()),
    }
}
