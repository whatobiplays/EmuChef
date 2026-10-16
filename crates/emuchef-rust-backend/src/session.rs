//! Sidecar-only document and product execution session state.
//!
//! Session state is intentionally in-memory and process-local. It persists
//! across JSONL requests handled by one sidecar process and is never shared with
//! one-shot CLI invocations. Product execution workers retain their own shared
//! report/event state after the request dispatcher returns.

use std::fs;
use std::path::{Component, Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::app_definition_admission::{admit_app_definition_save, AppDefinitionSaveDestination};
use crate::app_definition_document::AppDefinitionDocument;
use crate::app_definition_persistence::{persist_app_definition, OpenedAppDefinitionFile};
use crate::app_generator_review::{self, AppGeneratorReviewState};
use crate::commands;
use crate::document::RecipeDocument;
use crate::dto;
use crate::errors::ApiError;
use crate::execution_session::{ExecutionSessionManager, SidecarRuntimeConfig};
use crate::ref_index;
use crate::user_configuration_document::{self, UserConfigurationDocument};

#[derive(Debug, Default)]
pub struct DocumentSessionManager {
    documents: IndexMap<String, RecipeDocument>,
    app_definitions: IndexMap<String, AppDefinitionDocument>,
    user_configurations: IndexMap<String, UserConfigurationDocument>,
    next_id: u64,
    app_generator: AppGeneratorReviewState,
    executions: ExecutionSessionManager,
}

impl DocumentSessionManager {
    /// Build process-local document and product execution state using roots
    /// fixed by sidecar startup arguments.
    pub fn with_runtime_config(config: SidecarRuntimeConfig) -> Self {
        Self {
            executions: ExecutionSessionManager::new(config),
            ..Self::default()
        }
    }

    pub(crate) fn executions(&self) -> &ExecutionSessionManager {
        &self.executions
    }
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

    pub fn open_app_definition(
        &mut self,
        path: &str,
        authored_root: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = AppDefinitionDocument::open(path, authored_root).map_err(|error| {
            let mut details = json!({ "path": path });
            if !error.diagnostics.is_empty() {
                details["diagnostics"] = json!(dto::app_diagnostics_to_dto(&error.diagnostics));
            }
            ApiError::load_failed(
                format!("Failed to load App Definition: {}", error.message),
                details,
            )
        })?;
        let document_id = self.next_document_id();
        let mut document = document;
        refresh_app_definition_diagnostics(&mut document);
        let document_dto = dto::app_definition_document_to_dto(&document, &document_id);
        self.app_definitions.insert(document_id, document);
        Ok(json!({ "document": document_dto }))
    }

    /// Return the retained App Generator handle state.
    pub(crate) fn app_generator(&self) -> &AppGeneratorReviewState {
        &self.app_generator
    }

    /// Return the mutable App Generator handle state.
    pub(crate) fn app_generator_mut(&mut self) -> &mut AppGeneratorReviewState {
        &mut self.app_generator
    }

    /// Publish one reviewed generated App/Recipe pair for the native layer.
    ///
    /// The dirty-target guard runs at publication time across every active App
    /// Definition document whose path matches the target catalog path. A
    /// rejection mutates no draft and does not invalidate unsaved App intent.
    pub fn save_generated_app_review(
        &mut self,
        operation_id: &str,
        review_id: &str,
        request_id: i64,
    ) -> Result<Value, ApiError> {
        if let Some(recorded) =
            app_generator_review::operation_replay(&self.app_generator, operation_id)
        {
            return Ok(recorded);
        }
        // Pre-publication guards inspect the retained review without consuming
        // it, so a refusal (blocking diagnostics or an unsaved App Definition
        // document on the target path) leaves the reviewed candidate available
        // for a later publication attempt.
        let review =
            app_generator_review::peek_review(&self.app_generator, review_id, request_id)?;
        if review.blocking {
            return Err(app_generator_review::review_blocking_error(review));
        }
        let dirty = self.app_definitions.values().any(|document| {
            document.path() == review.pair.app_snapshot.destination.as_path() && document.is_dirty()
        });
        if dirty {
            return Err(app_generator_review::dirty_app_document_error(review));
        }
        // A publication attempt consumes the review handle; recovery from a
        // refused or unknown attempt goes through a fresh review.
        let review =
            app_generator_review::take_review(&mut self.app_generator, review_id, request_id)?;
        app_generator_review::publish_review(&mut self.app_generator, operation_id, &review)
    }

    pub fn apply_app_definition_command(
        &mut self,
        document_id: &str,
        command: &Value,
    ) -> Result<Value, ApiError> {
        let document = self.app_definition_document_mut(document_id)?;
        let changed = match document.apply_command_with_diagnostics(command) {
            Ok(changed) => changed,
            Err(error) => {
                let raw_diagnostics = error
                    .diagnostics
                    .unwrap_or_else(|| document.diagnostics().to_vec());
                let diagnostics = dto::app_diagnostics_to_dto(&raw_diagnostics);
                return Err(ApiError::invalid_command_with_details(
                    format!("App Definition command failed: {}", error.message),
                    json!({
                        "documentId": document_id,
                        "diagnostics": diagnostics,
                    }),
                ));
            }
        };
        refresh_app_definition_diagnostics(document);
        Ok(json!({
            "commandResult": { "changed": changed },
            "document": dto::app_definition_document_to_dto(document, document_id),
        }))
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
        if let Some(document) = self.app_definitions.get(document_id) {
            return Ok(json!({
                "document": dto::app_definition_document_to_dto(document, document_id)
            }));
        }
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

    /// Save an App Definition only after its complete save admission succeeds.
    pub fn save_app_definition(&mut self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.app_definition_document_mut(document_id)?;
        let admission = admit_app_definition_save(
            document.app_definition(),
            &document.save_context(AppDefinitionSaveDestination::OpenedFile),
        );
        let raw_diagnostics = admission.diagnostics_as_values();
        document.set_diagnostics(raw_diagnostics.clone());
        let diagnostics = dto::app_diagnostics_to_dto(&raw_diagnostics);
        if !admission.can_save() {
            return Err(ApiError::save_failed(
                "App Definition save is blocked; review the document diagnostics.",
                json!({ "documentId": document_id, "diagnostics": diagnostics }),
            ));
        }
        let opened_file =
            persist_app_definition(document.app_definition(), &admission).map_err(|error| {
                let diagnostic = persistence_error_to_diagnostic(&error);
                document.set_diagnostics(vec![diagnostic.clone()]);
                ApiError::save_failed(
                    format!("App Definition save failed: {}", error.message),
                    json!({
                        "documentId": document_id,
                        "code": error.code,
                        "path": error.path,
                        "diagnostics": [diagnostic],
                    }),
                )
            })?;
        let yaml = saved_yaml(&opened_file);
        document.mark_saved(opened_file, yaml);
        refresh_app_definition_diagnostics(document);
        Ok(json!({
            "document": dto::app_definition_document_to_dto(document, document_id)
        }))
    }

    /// Save a standalone App Definition as a new canonical authored catalog App.
    pub fn save_app_definition_as(
        &mut self,
        document_id: &str,
        path: &str,
    ) -> Result<Value, ApiError> {
        let document = self.app_definition_document_mut(document_id)?;
        let Some(selected_root) = document.authored_root().map(Path::to_path_buf) else {
            let message =
                "Choose an authored root before saving this App Definition as a catalog App.";
            let diagnostic = save_request_diagnostic(
                "app_authored_root_required",
                message,
                Some(path),
                Some("authoredRoot"),
            );
            document.set_diagnostics(vec![diagnostic.clone()]);
            return Err(ApiError::save_failed(
                message,
                json!({ "documentId": document_id, "path": path, "diagnostics": [diagnostic] }),
            ));
        };
        let authored_root = match authored_root_for_app_path(
            path,
            &document.app_definition().id,
            &selected_root,
        ) {
            Ok(root) => root,
            Err(message) => {
                let diagnostic = save_request_diagnostic(
                    "app_definition_path_invalid",
                    &message,
                    Some(path),
                    Some("file"),
                );
                document.set_diagnostics(vec![diagnostic.clone()]);
                return Err(ApiError::save_failed(
                    message,
                    json!({ "documentId": document_id, "path": path, "diagnostics": [diagnostic] }),
                ));
            }
        };
        let mut context = document.save_context(AppDefinitionSaveDestination::NewCatalogApp);
        context.authored_root = Some(authored_root);
        let admission = admit_app_definition_save(document.app_definition(), &context);
        let raw_diagnostics = admission.diagnostics_as_values();
        document.set_diagnostics(raw_diagnostics.clone());
        let diagnostics = dto::app_diagnostics_to_dto(&raw_diagnostics);
        if !admission.can_save() {
            return Err(ApiError::save_failed(
                "App Definition Save As is blocked; review the document diagnostics.",
                json!({ "documentId": document_id, "path": path, "diagnostics": diagnostics }),
            ));
        }
        let opened_file =
            persist_app_definition(document.app_definition(), &admission).map_err(|error| {
                let diagnostic = persistence_error_to_diagnostic(&error);
                document.set_diagnostics(vec![diagnostic.clone()]);
                ApiError::save_failed(
                    format!("App Definition Save As failed: {}", error.message),
                    json!({
                        "documentId": document_id,
                        "path": path,
                        "code": error.code,
                        "diagnostics": [diagnostic],
                    }),
                )
            })?;
        let saved_root = opened_file
            .path()
            .parent()
            .and_then(Path::parent)
            .expect("an admitted App Definition destination has an apps parent")
            .to_path_buf();
        let saved_path = opened_file.path().to_path_buf();
        let yaml = saved_yaml(&opened_file);
        document.set_saved_path(saved_path, saved_root);
        document.mark_saved(opened_file, yaml);
        refresh_app_definition_diagnostics(document);
        Ok(json!({
            "document": dto::app_definition_document_to_dto(document, document_id)
        }))
    }

    /// Route generic Save to the document kind registered in this session.
    pub fn save_document(&mut self, document_id: &str) -> Result<Value, ApiError> {
        if self.app_definitions.contains_key(document_id) {
            self.save_app_definition(document_id)
        } else {
            self.save_recipe(document_id)
        }
    }

    /// Route generic Save As while preserving each document kind's restrictions.
    pub fn save_document_as(&mut self, document_id: &str, path: &str) -> Result<Value, ApiError> {
        if self.app_definitions.contains_key(document_id) {
            self.save_app_definition_as(document_id, path)
        } else {
            self.save_recipe_as(document_id, path)
        }
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
        if let Some(document) = self.app_definitions.get_mut(document_id) {
            let changed = document.undo();
            refresh_app_definition_diagnostics(document);
            return Ok(json!({
                "commandResult": { "changed": changed },
                "document": dto::app_definition_document_to_dto(document, document_id),
            }));
        }
        let document = self.document_mut(document_id)?;
        let changed = document.undo();
        Ok(json!({
            "commandResult": {"changed": changed},
            "document": dto::document_to_dto(document, document_id),
        }))
    }

    pub fn redo(&mut self, document_id: &str) -> Result<Value, ApiError> {
        if let Some(document) = self.app_definitions.get_mut(document_id) {
            let changed = document.redo();
            refresh_app_definition_diagnostics(document);
            return Ok(json!({
                "commandResult": { "changed": changed },
                "document": dto::app_definition_document_to_dto(document, document_id),
            }));
        }
        let document = self.document_mut(document_id)?;
        let changed = document.redo();
        Ok(json!({
            "commandResult": {"changed": changed},
            "document": dto::document_to_dto(document, document_id),
        }))
    }

    pub fn emit_yaml(&self, document_id: &str) -> Result<Value, ApiError> {
        if let Some(document) = self.app_definitions.get(document_id) {
            return Ok(json!({ "yaml": document.yaml() }));
        }
        let document = self.document(document_id)?;
        Ok(json!({ "yaml": document.yaml() }))
    }

    pub fn validate(&mut self, document_id: &str) -> Result<Value, ApiError> {
        if let Some(document) = self.app_definitions.get_mut(document_id) {
            document.validate();
            refresh_app_definition_diagnostics(document);
            let diagnostics = dto::app_diagnostics_to_dto(document.diagnostics());
            return Ok(json!({ "diagnostics": diagnostics }));
        }
        let document = self.document_mut(document_id)?;
        document.validate();
        Ok(json!({ "diagnostics": document.diagnostics() }))
    }

    pub fn get_ref_index(&self, document_id: &str) -> Result<Value, ApiError> {
        if self.app_definitions.contains_key(document_id) {
            return Err(wrong_document_kind(document_id, "recipe"));
        }
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
        if let Some(document) = self.app_definitions.get_mut(document_id) {
            document.set_authored_root(authored_root);
            refresh_app_definition_diagnostics(document);
            return Ok(json!({
                "document": dto::app_definition_document_to_dto(document, document_id)
            }));
        }
        let document = self.document_mut(document_id)?;
        document.set_authored_root(authored_root);
        Ok(json!({
            "document": dto::document_to_dto(document, document_id)
        }))
    }

    pub fn close_document(&mut self, document_id: &str) -> Result<Value, ApiError> {
        if self.app_definitions.shift_remove(document_id).is_some() {
            return Ok(json!({}));
        }
        if self.documents.shift_remove(document_id).is_none() {
            return Err(ApiError::unknown_document(document_id));
        }
        Ok(json!({}))
    }

    pub fn open_user_configuration(
        &mut self,
        path: &str,
        authored_root: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = UserConfigurationDocument::open(path, authored_root).map_err(|error| {
            user_configuration_load_error("load", error, json!({ "path": path }))
        })?;
        let document_id = self.next_document_id();
        let document_dto = user_configuration_document::document_to_dto(&document, &document_id);
        self.user_configurations.insert(document_id, document);
        Ok(json!({ "document": document_dto }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_user_configuration(
        &mut self,
        path: &str,
        configuration_id: &str,
        name: &str,
        device_plan: &str,
        selected_recipes: Vec<String>,
        bindings: IndexMap<String, Value>,
        authored_root: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = UserConfigurationDocument::create(
            path,
            configuration_id,
            name,
            device_plan,
            selected_recipes,
            bindings,
            authored_root,
        )
        .map_err(|error| user_configuration_load_error("create", error, json!({ "path": path })))?;
        let document_id = self.next_document_id();
        let document_dto = user_configuration_document::document_to_dto(&document, &document_id);
        self.user_configurations.insert(document_id, document);
        Ok(json!({ "document": document_dto }))
    }

    pub fn get_user_configuration_document(&self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.user_configuration_document(document_id)?;
        Ok(json!({
            "document": user_configuration_document::document_to_dto(document, document_id)
        }))
    }

    pub fn save_user_configuration(&mut self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.user_configuration_document_mut(document_id)?;
        document.save().map_err(|error| {
            ApiError::save_failed(
                format!("Failed to save user configuration: {error}"),
                json!({ "documentId": document_id }),
            )
        })?;
        Ok(json!({
            "document": user_configuration_document::document_to_dto(document, document_id)
        }))
    }

    pub fn save_user_configuration_as(
        &mut self,
        document_id: &str,
        path: &str,
        configuration_id: Option<&str>,
        name: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = self.user_configuration_document_mut(document_id)?;
        document
            .save_as_with_identity(path, configuration_id, name)
            .map_err(|error| {
                ApiError::save_failed(
                    format!("Failed to save user configuration as: {error}"),
                    json!({ "documentId": document_id, "path": path }),
                )
            })?;
        Ok(json!({
            "document": user_configuration_document::document_to_dto(document, document_id)
        }))
    }

    pub fn set_user_configuration_binding(
        &mut self,
        document_id: &str,
        key: &str,
        value: Option<Value>,
    ) -> Result<Value, ApiError> {
        let document = self.user_configuration_document_mut(document_id)?;
        let changed = document.set_binding(key, value).map_err(|error| {
            ApiError::command_failed(
                format!("User-configuration binding command failed: {error}"),
                json!({ "documentId": document_id, "key": key }),
            )
        })?;
        Ok(json!({
            "commandResult": { "changed": changed },
            "document": user_configuration_document::document_to_dto(document, document_id),
        }))
    }

    pub fn set_user_configuration_selected_recipes(
        &mut self,
        document_id: &str,
        selected_recipes: Vec<String>,
    ) -> Result<Value, ApiError> {
        let document = self.user_configuration_document_mut(document_id)?;
        let changed = document
            .set_selected_recipes(selected_recipes)
            .map_err(|error| {
                ApiError::command_failed(
                    format!("User-configuration recipe command failed: {error}"),
                    json!({ "documentId": document_id }),
                )
            })?;
        Ok(json!({
            "commandResult": { "changed": changed },
            "document": user_configuration_document::document_to_dto(document, document_id),
        }))
    }

    pub fn set_user_configuration_device_plan(
        &mut self,
        document_id: &str,
        device_plan: String,
    ) -> Result<Value, ApiError> {
        let document = self.user_configuration_document_mut(document_id)?;
        let changed = document.set_device_plan(device_plan).map_err(|error| {
            ApiError::command_failed(
                format!("User-configuration device-plan command failed: {error}"),
                json!({ "documentId": document_id }),
            )
        })?;
        Ok(json!({
            "commandResult": { "changed": changed },
            "document": user_configuration_document::document_to_dto(document, document_id),
        }))
    }

    pub fn validate_user_configuration(&mut self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.user_configuration_document_mut(document_id)?;
        document.validate();
        Ok(json!({ "diagnostics": document.diagnostics() }))
    }

    pub fn emit_user_configuration_yaml(&self, document_id: &str) -> Result<Value, ApiError> {
        let document = self.user_configuration_document(document_id)?;
        Ok(json!({ "yaml": document.yaml() }))
    }

    pub fn set_user_configuration_authored_root(
        &mut self,
        document_id: &str,
        authored_root: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = self.user_configuration_document_mut(document_id)?;
        document.set_authored_root(authored_root);
        Ok(json!({
            "document": user_configuration_document::document_to_dto(document, document_id)
        }))
    }

    pub fn close_user_configuration(&mut self, document_id: &str) -> Result<Value, ApiError> {
        if self.user_configurations.shift_remove(document_id).is_none() {
            return Err(ApiError::unknown_document(document_id));
        }
        Ok(json!({}))
    }

    fn next_document_id(&mut self) -> String {
        self.next_id += 1;
        format!("doc-{}", self.next_id)
    }

    fn document(&self, document_id: &str) -> Result<&RecipeDocument, ApiError> {
        self.documents.get(document_id).ok_or_else(|| {
            if self.app_definitions.contains_key(document_id) {
                wrong_document_kind(document_id, "recipe")
            } else {
                ApiError::unknown_document(document_id)
            }
        })
    }

    fn document_mut(&mut self, document_id: &str) -> Result<&mut RecipeDocument, ApiError> {
        if self.app_definitions.contains_key(document_id) {
            return Err(wrong_document_kind(document_id, "recipe"));
        }
        self.documents
            .get_mut(document_id)
            .ok_or_else(|| ApiError::unknown_document(document_id))
    }

    fn app_definition_document_mut(
        &mut self,
        document_id: &str,
    ) -> Result<&mut AppDefinitionDocument, ApiError> {
        if self.documents.contains_key(document_id) {
            return Err(wrong_document_kind(document_id, "app_definition"));
        }
        self.app_definitions
            .get_mut(document_id)
            .ok_or_else(|| ApiError::unknown_document(document_id))
    }

    fn user_configuration_document(
        &self,
        document_id: &str,
    ) -> Result<&UserConfigurationDocument, ApiError> {
        self.user_configurations
            .get(document_id)
            .ok_or_else(|| ApiError::unknown_document(document_id))
    }

    fn user_configuration_document_mut(
        &mut self,
        document_id: &str,
    ) -> Result<&mut UserConfigurationDocument, ApiError> {
        self.user_configurations
            .get_mut(document_id)
            .ok_or_else(|| ApiError::unknown_document(document_id))
    }
}

fn refresh_app_definition_diagnostics(document: &mut AppDefinitionDocument) {
    let admission = admit_app_definition_save(
        document.app_definition(),
        &document.save_context(AppDefinitionSaveDestination::OpenedFile),
    );
    document.set_diagnostics(admission.diagnostics_as_values());
}

fn saved_yaml(opened_file: &OpenedAppDefinitionFile) -> String {
    String::from_utf8_lossy(opened_file.raw_bytes()).into_owned()
}

fn persistence_error_to_diagnostic(
    error: &crate::app_definition_persistence::AppDefinitionPersistenceError,
) -> Value {
    save_request_diagnostic(
        &error.code,
        &error.message,
        error
            .path
            .as_deref()
            .map(|path| path.to_string_lossy())
            .as_deref(),
        None,
    )
}

fn save_request_diagnostic(
    code: &str,
    message: &str,
    path: Option<&str>,
    field: Option<&str>,
) -> Value {
    json!({
        "severity": "error",
        "code": code,
        "message": message,
        "file": path,
        "field": field,
    })
}

fn authored_root_for_app_path(
    path: &str,
    app_id: &str,
    selected_root: &Path,
) -> Result<PathBuf, String> {
    let requested = PathBuf::from(path);
    if !requested.is_absolute() {
        return Err(
            "App Definition Save As requires an absolute canonical apps/<id>.yaml path."
                .to_string(),
        );
    }
    if requested
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(
            "App Definition Save As requires a canonical path without dot segments.".to_string(),
        );
    }
    let expected_filename = format!("{app_id}.yaml");
    if requested.file_name().and_then(|name| name.to_str()) != Some(expected_filename.as_str()) {
        return Err(format!(
            "App Definition Save As must target the canonical filename apps/{expected_filename}."
        ));
    }
    let apps = requested
        .parent()
        .ok_or_else(|| "App Definition Save As path has no parent directory.".to_string())?;
    if apps.file_name().and_then(|name| name.to_str()) != Some("apps") {
        return Err("App Definition Save As must target the authored apps directory.".to_string());
    }
    let root = apps
        .parent()
        .ok_or_else(|| "App Definition Save As path has no authored root.".to_string())?;
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("Authored root could not be canonicalized: {error}"))?;
    let canonical_apps = fs::canonicalize(canonical_root.join("apps"))
        .map_err(|error| format!("Authored apps directory could not be canonicalized: {error}"))?;
    if canonical_apps != canonical_root.join("apps") {
        return Err("App Definition Save As cannot use a symlinked apps directory.".to_string());
    }
    let requested_apps = fs::canonicalize(apps).map_err(|error| {
        format!("App Definition Save As apps directory could not be resolved: {error}")
    })?;
    if requested_apps != canonical_apps {
        return Err(
            "App Definition Save As path is not canonical; use the selected root's apps directory."
                .to_string(),
        );
    }
    let canonical_selected_root = fs::canonicalize(selected_root)
        .map_err(|error| format!("Selected authored root could not be canonicalized: {error}"))?;
    if canonical_root != canonical_selected_root {
        return Err(
            "App Definition Save As must target the currently selected authored root.".to_string(),
        );
    }
    Ok(canonical_root)
}

fn wrong_document_kind(document_id: &str, expected_kind: &str) -> ApiError {
    ApiError::invalid_request_with_details(
        format!("Document {document_id:?} is not a {expected_kind} document."),
        json!({ "documentId": document_id, "expectedKind": expected_kind }),
    )
}

fn user_configuration_load_error(
    action: &str,
    error: crate::user_configuration::UserConfigurationLoadError,
    details: Value,
) -> ApiError {
    ApiError::load_failed(
        format!("Failed to {action} user configuration: {error}"),
        details,
    )
}
