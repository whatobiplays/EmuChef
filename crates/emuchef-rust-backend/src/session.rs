//! Sidecar-only document session manager for Phase 6F.
//!
//! Session state is intentionally in-memory and process-local. It persists
//! across JSONL requests handled by one sidecar process and is never shared with
//! one-shot CLI invocations.

use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::document::RecipeDocument;
use crate::dto;
use crate::errors::ApiError;

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
