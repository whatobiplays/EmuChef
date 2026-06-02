//! Sidecar-only document session manager for the experimental Rust backend.
//!
//! Session state is intentionally in-memory and process-local. It persists
//! across JSONL requests handled by one sidecar process and is never shared with
//! one-shot CLI invocations.

use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::commands;
use crate::document::RecipeDocument;
use crate::dto;
use crate::errors::ApiError;
use crate::ref_index;

#[derive(Debug, Default)]
pub struct DocumentSessionManager {
    documents: IndexMap<String, RecipeDocument>,
    next_id: u64,
}

impl DocumentSessionManager {
    pub fn open_recipe(
        &mut self,
        path: &str,
        authored_root: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = RecipeDocument::open(path, authored_root).map_err(|error| {
            ApiError::load_failed(
                format!("Failed to load recipe: {error}"),
                json!({ "path": path }),
            )
        })?;
        let document_id = self.next_document_id();
        let document_dto = dto::document_to_dto(&document, &document_id);
        self.documents.insert(document_id, document);
        Ok(json!({ "document": document_dto }))
    }

    pub fn create_recipe_from_template(
        &mut self,
        template_path: &str,
        destination_path: &str,
        recipe_id: &str,
        authored_root: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = RecipeDocument::create_from_template(
            template_path,
            destination_path,
            recipe_id,
            authored_root,
        )
        .map_err(|error| {
            ApiError::load_failed(
                format!("Failed to create recipe from template: {error}"),
                json!({ "templatePath": template_path, "destinationPath": destination_path }),
            )
        })?;
        let document_id = self.next_document_id();
        let document_dto = dto::document_to_dto(&document, &document_id);
        self.documents.insert(document_id, document);
        Ok(json!({ "document": document_dto }))
    }

    pub fn get_document(&self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.document(document_id)?;
        Ok(json!({
            "document": dto::document_to_dto(document, document_id)
        }))
    }

    pub fn save_recipe(&mut self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.document_mut(document_id)?;
        document.save().map_err(|error| {
            ApiError::save_failed(
                format!("Failed to save recipe: {error}"),
                json!({ "documentId": document_id }),
            )
        })?;
        Ok(json!({
            "document": dto::document_to_dto(document, document_id)
        }))
    }

    pub fn save_recipe_as(&mut self, document_id: &str, path: &str) -> Result<Value, ApiError> {
        let document = self.document_mut(document_id)?;
        document.save_as(path).map_err(|error| {
            ApiError::save_failed(
                format!("Failed to save recipe as: {error}"),
                json!({ "documentId": document_id, "path": path }),
            )
        })?;
        Ok(json!({
            "document": dto::document_to_dto(document, document_id)
        }))
    }

    pub fn apply_recipe_command(
        &mut self,
        document_id: &str,
        command: &Value,
    ) -> Result<Value, ApiError> {
        let document = self.document_mut(document_id)?;
        let command = commands::decode_recipe_command(command)?;
        let changed = document.apply_command(command).map_err(|error| {
            ApiError::command_failed(
                format!("Command failed: {error}"),
                json!({ "documentId": document_id }),
            )
        })?;
        Ok(json!({
            "commandResult": {"changed": changed},
            "document": dto::document_to_dto(document, document_id),
        }))
    }

    pub fn undo(&mut self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.document_mut(document_id)?;
        let changed = document.undo();
        Ok(json!({
            "commandResult": {"changed": changed},
            "document": dto::document_to_dto(document, document_id),
        }))
    }

    pub fn redo(&mut self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.document_mut(document_id)?;
        let changed = document.redo();
        Ok(json!({
            "commandResult": {"changed": changed},
            "document": dto::document_to_dto(document, document_id),
        }))
    }

    pub fn emit_yaml(&self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.document(document_id)?;
        Ok(json!({ "yaml": document.yaml() }))
    }

    pub fn validate(&mut self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.document_mut(document_id)?;
        document.validate();
        Ok(json!({ "diagnostics": document.diagnostics() }))
    }

    pub fn get_ref_index(&self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.document(document_id)?;
        Ok(json!({
            "refIndex": ref_index::ref_index_to_dto(document.recipe())
        }))
    }

    pub fn set_document_authored_root(
        &mut self,
        document_id: &str,
        authored_root: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = self.document_mut(document_id)?;
        document.set_authored_root(authored_root);
        Ok(json!({
            "document": dto::document_to_dto(document, document_id)
        }))
    }

    pub fn close_document(&mut self, document_id: &str) -> Result<Value, ApiError> {
        if self.documents.shift_remove(document_id).is_none() {
            return Err(ApiError::unknown_document(document_id));
        }
        Ok(json!({}))
    }

    fn next_document_id(&mut self) -> String {
        self.next_id += 1;
        format!("doc-{}", self.next_id)
    }

    fn document(&self, document_id: &str) -> Result<&RecipeDocument, ApiError> {
        self.documents
            .get(document_id)
            .ok_or_else(|| ApiError::unknown_document(document_id))
    }

    fn document_mut(&mut self, document_id: &str) -> Result<&mut RecipeDocument, ApiError> {
        self.documents
            .get_mut(document_id)
            .ok_or_else(|| ApiError::unknown_document(document_id))
    }
}
