//! In-memory recipe document state for Phase 6F sidecar sessions.
//!
//! A document wraps the Phase 6E authored recipe model with the editor-facing
//! lifecycle state needed for open/get/save/close. It deliberately does not
//! implement command mutation, undo/redo, catalog-context validation, or ref
//! indexing.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

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
        })
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.current_yaml =
            yaml::emit_recipe_yaml(&self.recipe).map_err(|error| error.to_string())?;
        fs::write(&self.path, &self.current_yaml).map_err(|error| error.to_string())?;
        self.saved_yaml = self.current_yaml.clone();
        self.diagnostics = validation_diagnostics(&self.path, self.authored_root.is_some());
        Ok(())
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
        false
    }

    pub fn can_redo(&self) -> bool {
        false
    }
}

fn validation_diagnostics(path: &Path, authored_root_provided: bool) -> Vec<Value> {
    validation::validate_recipe_path_result(path, authored_root_provided)
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
