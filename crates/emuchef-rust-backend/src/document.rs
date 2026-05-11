//! In-memory recipe document state for Phase 6G sidecar sessions.
//!
//! A document wraps the Phase 6E authored recipe model with the editor-facing
//! lifecycle state needed for sidecar document sessions. Phase 6G adds a narrow
//! command slice and snapshot undo/redo while still omitting catalog-context
//! validation, step command parity, and real ref indexing.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::commands::{OverviewField, OverviewValue, RecipeCommand};
use crate::model::Recipe;
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
        let authored_root =
            authored_root.map(|root| PathBuf::from(yaml::resolved_path_string(Path::new(root))));
        let diagnostics = validation_diagnostics(&path, authored_root.is_some());

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
        self.redo_stack.push(current);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let current = self.content_snapshot();
        self.restore_content(next);
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
            self.authored_root.is_some(),
        );
    }
}

fn validation_diagnostics(path: &Path, authored_root_provided: bool) -> Vec<Value> {
    validation::validate_recipe_path_result(path, authored_root_provided)
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn validation_diagnostics_for_recipe(
    recipe: &Recipe,
    path: &Path,
    authored_root_provided: bool,
) -> Vec<Value> {
    validation::validate_loaded_recipe_result(recipe, path, authored_root_provided)
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
