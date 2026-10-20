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
use crate::app_definition_persistence::{
    persist_app_definition_with_storage, OpenedAppDefinitionFile,
};
use crate::app_generator_review::{self, AppGeneratorReviewState};
use crate::catalog_refactor::{
    CatalogChangeBlocker, CatalogChangeIntent, CatalogChangePreview, CatalogPreviewBinding,
    CatalogRefactorSession,
};
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
    catalog_refactors: CatalogRefactorSession,
    catalog_storage_base: Option<PathBuf>,
    executions: ExecutionSessionManager,
}

fn reconcile_catalog_change(
    documents: &mut IndexMap<String, RecipeDocument>,
    app_definitions: &mut IndexMap<String, AppDefinitionDocument>,
    changed_files: &[PathBuf],
) -> Result<(), String> {
    let mut dirty_paths = documents
        .values()
        .filter(|document| document.is_dirty())
        .filter(|document| changed_files.iter().any(|path| path == document.path()))
        .map(|document| document.path().to_path_buf())
        .chain(
            app_definitions
                .values()
                .filter(|document| document.is_dirty())
                .filter(|document| changed_files.iter().any(|path| path == document.path()))
                .map(|document| document.path().to_path_buf()),
        )
        .collect::<Vec<_>>();
    dirty_paths.sort();
    dirty_paths.dedup();
    if !dirty_paths.is_empty() {
        return Err(format!(
            "Affected documents became dirty before reconciliation: {}",
            dirty_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let mut recipe_updates = Vec::new();
    for (document_id, document) in documents.iter() {
        if !changed_files.iter().any(|path| path == document.path()) {
            continue;
        }
        let root = document
            .authored_root()
            .and_then(Path::to_str)
            .map(str::to_owned);
        let reloaded = RecipeDocument::open(document.path(), root.as_deref()).map_err(|error| {
            format!(
                "Affected Recipe {} could not be reconciled: {error}",
                document.path().display()
            )
        })?;
        recipe_updates.push((document_id.clone(), reloaded));
    }

    let mut app_updates = Vec::new();
    let mut closed_apps = Vec::new();
    for (document_id, document) in app_definitions.iter() {
        if !changed_files.iter().any(|path| path == document.path()) {
            continue;
        }
        let root = document
            .authored_root()
            .and_then(Path::to_str)
            .map(str::to_owned);
        match AppDefinitionDocument::open(document.path(), root.as_deref()) {
            Ok(mut reloaded) => {
                refresh_app_definition_diagnostics(&mut reloaded);
                app_updates.push((document_id.clone(), reloaded));
            }
            Err(error) if !document.path().exists() => {
                let old_parent = document.path().parent();
                let renamed_path = changed_files.iter().find(|path| {
                    path.exists()
                        && path.as_path() != document.path()
                        && path.parent() == old_parent
                });
                if let Some(path) = renamed_path {
                    let mut reloaded =
                        AppDefinitionDocument::open(path, root.as_deref()).map_err(|error| {
                            format!(
                                "Renamed App Definition {} could not be reconciled: {error}",
                                path.display()
                            )
                        })?;
                    refresh_app_definition_diagnostics(&mut reloaded);
                    app_updates.push((document_id.clone(), reloaded));
                } else {
                    let _ = error;
                    closed_apps.push(document_id.clone());
                }
            }
            Err(error) => {
                return Err(format!(
                    "Affected App Definition {} could not be reconciled: {}",
                    document.path().display(),
                    error.message
                ));
            }
        }
    }

    for (document_id, reloaded) in recipe_updates {
        if let Some(document) = documents.get_mut(&document_id) {
            *document = reloaded;
        }
    }
    for (document_id, reloaded) in app_updates {
        if let Some(document) = app_definitions.get_mut(&document_id) {
            *document = reloaded;
        }
    }
    for document_id in closed_apps {
        app_definitions.shift_remove(&document_id);
    }
    for document in documents.values_mut() {
        let Some(root) = document.authored_root() else {
            continue;
        };
        if changed_files.iter().any(|path| path.starts_with(root)) {
            document.refresh_catalog_projection();
        }
    }
    Ok(())
}

/// Replace every complete path prefix before a catalog diagnostic crosses the
/// Rust-to-editor boundary. Both slash and backslash separators are handled so
/// absolute paths do not become platform-dependent client data.
fn redact_catalog_path(message: &str, path: &Path, replacement: &str) -> String {
    let path = path.to_string_lossy();
    message
        .replace(&format!("{path}/"), &format!("{replacement}/"))
        .replace(&format!("{path}\\"), &format!("{replacement}\\"))
        .replace(path.as_ref(), replacement)
}

/// Remove authored-root and application-storage path prefixes from a catalog
/// error while retaining the actionable filesystem-relative diagnostic.
fn sanitized_catalog_diagnostic(
    authored_root: Option<&Path>,
    storage_base: Option<&Path>,
    message: &str,
) -> String {
    let mut sanitized = message.to_owned();
    if let Some(root) = authored_root {
        sanitized = redact_catalog_path(&sanitized, root, "<authored-root>");
    }
    let storage_root = storage_base
        .map(Path::to_path_buf)
        .or_else(|| crate::catalog_change::stable_storage_directory().ok());
    if let Some(storage_root) = storage_root {
        sanitized = redact_catalog_path(&sanitized, &storage_root, "<application-data>");
    }
    sanitized
}

fn catalog_change_blockers(
    documents: &IndexMap<String, RecipeDocument>,
    app_definitions: &IndexMap<String, AppDefinitionDocument>,
    refactors: &CatalogRefactorSession,
    preview: &CatalogChangePreview,
    authored_root: &Path,
) -> Result<Vec<CatalogChangeBlocker>, String> {
    let mut blockers = Vec::new();
    for document in documents.values().filter(|document| document.is_dirty()) {
        let affected_file = preview
            .changed_files
            .iter()
            .any(|path| authored_root.join(path) == document.path());
        let affected_reference = refactors.dirty_recipe_references_identity(
            &preview.operation_id,
            document.authored_root(),
            document.path(),
            document.recipe(),
        )?;
        if affected_file || affected_reference {
            blockers.push(CatalogChangeBlocker {
                path: document
                    .path()
                    .strip_prefix(authored_root)
                    .unwrap_or_else(|_| document.path())
                    .to_path_buf(),
                reason: if affected_reference {
                    "This Recipe has unsaved typed references affected by the catalog change."
                } else {
                    "This Recipe has unsaved changes in an affected catalog file."
                }
                .into(),
            });
        }
    }
    for document in app_definitions
        .values()
        .filter(|document| document.is_dirty())
    {
        if preview
            .changed_files
            .iter()
            .any(|path| authored_root.join(path) == document.path())
        {
            blockers.push(CatalogChangeBlocker {
                path: document
                    .path()
                    .strip_prefix(authored_root)
                    .unwrap_or_else(|_| document.path())
                    .to_path_buf(),
                reason: "This App Definition has unsaved changes in an affected catalog file."
                    .into(),
            });
        }
    }
    blockers.sort();
    blockers.dedup();
    Ok(blockers)
}

impl DocumentSessionManager {
    /// Create an editor session with an explicit durable catalog-journal base.
    /// Production sessions use the stable per-user default; isolated profiles
    /// and tests can provide their own application-state directory.
    pub fn with_catalog_storage(storage_base: impl AsRef<Path>) -> Self {
        let storage_base = storage_base.as_ref().to_path_buf();
        Self {
            app_generator: AppGeneratorReviewState::with_catalog_storage(storage_base.clone()),
            catalog_refactors: CatalogRefactorSession::with_storage(storage_base.clone()),
            catalog_storage_base: Some(storage_base),
            ..Self::default()
        }
    }

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
        let review = app_generator_review::peek_review(&self.app_generator, review_id, request_id)?;
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
        let app_path = review.pair.app_snapshot.destination.clone();
        let app_definitions = &mut self.app_definitions;
        app_generator_review::publish_review(
            &mut self.app_generator,
            operation_id,
            &review,
            |outcome| {
                for document in app_definitions
                    .values_mut()
                    .filter(|document| document.path() == app_path)
                {
                    document.refresh_after_generator_publication(outcome.app.clone())?;
                }
                Ok(())
            },
        )
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

    /// Preview one explicit App identity refactor from the selected App
    /// Definition's trusted authored root.
    pub fn preview_catalog_change(
        &mut self,
        document_id: &str,
        intent: CatalogChangeIntent,
    ) -> Result<Value, ApiError> {
        let document = self.app_definition_document(document_id)?;
        let identity = document.catalog_identity().cloned().ok_or_else(|| {
            ApiError::invalid_request(
                "A persisted catalog App identity is required before previewing a catalog change.",
            )
        })?;
        let requested_app_id = match &intent {
            CatalogChangeIntent::RenameApp { app_id, .. }
            | CatalogChangeIntent::RenameArtifact { app_id, .. }
            | CatalogChangeIntent::RenameTarget { app_id, .. }
            | CatalogChangeIntent::DeleteApp { app_id } => app_id,
        };
        if requested_app_id != &identity.id {
            return Err(ApiError::invalid_request_with_details(
                "A catalog change must target the selected App Definition's persisted identity.",
                json!({
                    "documentId": document_id,
                    "selectedAppId": identity.id,
                    "requestedAppId": requested_app_id,
                }),
            ));
        }
        let root = identity.authored_root.clone();
        let storage_base = self.catalog_refactors.storage_base().map(Path::to_path_buf);
        let binding = CatalogPreviewBinding::for_app_document(
            document_id,
            &identity,
            document.yaml(),
            document.opened_bytes(),
            document.opened_file().identity(),
        );
        let mut preview = self
            .catalog_refactors
            .preview_for_document(&root, intent, binding)
            .map_err(|message| {
                ApiError::command_failed(
                    format!(
                        "Catalog change preview failed: {}",
                        sanitized_catalog_diagnostic(
                            Some(&root),
                            storage_base.as_deref(),
                            &message
                        )
                    ),
                    json!({ "documentId": document_id }),
                )
            })?;
        preview.blockers = catalog_change_blockers(
            &self.documents,
            &self.app_definitions,
            &self.catalog_refactors,
            &preview,
            &root,
        )
        .map_err(|message| {
            ApiError::command_failed(
                format!("Catalog change preview could not inspect open sessions: {message}"),
                json!({ "documentId": document_id }),
            )
        })?;
        serde_json::to_value(preview).map_err(|_| {
            ApiError::command_failed(
                "Catalog change preview could not be represented.",
                json!({ "documentId": document_id }),
            )
        })
    }

    /// Apply a single-use preview after aggregating every dirty affected
    /// document across both typed editor registries.
    pub fn apply_catalog_change(
        &mut self,
        operation_id: &str,
        preview_token: &str,
    ) -> Result<Value, ApiError> {
        let operation_root = self
            .catalog_refactors
            .authored_root_for_operation(operation_id);
        let storage_base = self.catalog_refactors.storage_base().map(Path::to_path_buf);
        if let Some(binding) = self.catalog_refactors.preview_binding(operation_id) {
            let document = self
                .app_definitions
                .get(&binding.document_id)
                .ok_or_else(|| {
                    ApiError::invalid_request_with_details(
                        "The App Definition session bound to this preview is no longer open.",
                        json!({ "operationId": operation_id }),
                    )
                })?;
            let identity = document.catalog_identity().ok_or_else(|| {
                ApiError::command_failed(
                    "The App Definition session no longer has a persisted catalog identity.",
                    json!({ "operationId": operation_id }),
                )
            })?;
            let current = CatalogPreviewBinding::for_app_document(
                &binding.document_id,
                identity,
                document.yaml(),
                document.opened_bytes(),
                document.opened_file().identity(),
            );
            if current != binding {
                return Err(ApiError::invalid_request_with_details(
                    "The catalog-change preview no longer matches the selected App Definition session. Preview again before applying.",
                    json!({ "operationId": operation_id }),
                ));
            }
        }
        let dirty_paths = self
            .documents
            .values()
            .filter(|document| document.is_dirty())
            .map(|document| document.path().to_path_buf())
            .chain(
                self.app_definitions
                    .values()
                    .filter(|document| document.is_dirty())
                    .map(|document| document.path().to_path_buf()),
            )
            .collect::<Vec<_>>();
        let mut dirty_reference_paths = Vec::new();
        for document in self
            .documents
            .values()
            .filter(|document| document.is_dirty())
        {
            let references_changed_identity = self
                .catalog_refactors
                .dirty_recipe_references_identity(
                    operation_id,
                    document.authored_root(),
                    document.path(),
                    document.recipe(),
                )
                .map_err(|message| {
                    ApiError::command_failed(
                        format!(
                            "Catalog change could not verify dirty Recipe references: {}",
                            sanitized_catalog_diagnostic(
                                operation_root.as_deref(),
                                storage_base.as_deref(),
                                &message,
                            )
                        ),
                        json!({ "operationId": operation_id }),
                    )
                })?;
            if references_changed_identity {
                dirty_reference_paths.push(document.path().to_path_buf());
            }
        }
        let (documents, app_definitions, catalog_refactors) = (
            &mut self.documents,
            &mut self.app_definitions,
            &mut self.catalog_refactors,
        );
        let status = catalog_refactors
            .apply(
                operation_id,
                preview_token,
                &dirty_paths,
                &dirty_reference_paths,
                |changed_files| reconcile_catalog_change(documents, app_definitions, changed_files),
            )
            .map_err(|message| {
                ApiError::command_failed(
                    format!(
                        "Catalog change apply failed: {}",
                        sanitized_catalog_diagnostic(
                            operation_root.as_deref(),
                            storage_base.as_deref(),
                            &message,
                        )
                    ),
                    json!({ "operationId": operation_id }),
                )
            })?;
        serde_json::to_value(status).map_err(|_| {
            ApiError::command_failed(
                "Catalog change status could not be represented.",
                json!({ "operationId": operation_id }),
            )
        })
    }

    pub fn catalog_change_status(
        &self,
        authored_root: &str,
        operation_id: &str,
    ) -> Result<Value, ApiError> {
        let storage_base = self.catalog_refactors.storage_base().map(Path::to_path_buf);
        let status = self
            .catalog_refactors
            .status(authored_root, operation_id)
            .map_err(|message| {
                ApiError::command_failed(
                    sanitized_catalog_diagnostic(
                        Some(Path::new(authored_root)),
                        storage_base.as_deref(),
                        &message,
                    ),
                    json!({ "operationId": operation_id }),
                )
            })?;
        Ok(json!({ "operation": status }))
    }

    pub fn list_catalog_change_recoveries(&self, authored_root: &str) -> Result<Value, ApiError> {
        let storage_base = self.catalog_refactors.storage_base().map(Path::to_path_buf);
        let recoveries = self
            .catalog_refactors
            .list_recoveries(authored_root)
            .map_err(|message| {
                ApiError::command_failed(
                    sanitized_catalog_diagnostic(
                        Some(Path::new(authored_root)),
                        storage_base.as_deref(),
                        &message,
                    ),
                    json!({}),
                )
            })?;
        Ok(json!({ "recoveries": recoveries }))
    }

    pub fn retry_catalog_change_recovery(
        &mut self,
        authored_root: &str,
        operation_id: &str,
    ) -> Result<Value, ApiError> {
        let storage_base = self.catalog_refactors.storage_base().map(Path::to_path_buf);
        let (documents, app_definitions, catalog_refactors) = (
            &mut self.documents,
            &mut self.app_definitions,
            &mut self.catalog_refactors,
        );
        let status = catalog_refactors
            .retry_recovery(authored_root, operation_id, |changed_files| {
                reconcile_catalog_change(documents, app_definitions, changed_files)
            })
            .map_err(|message| {
                ApiError::command_failed(
                    sanitized_catalog_diagnostic(
                        Some(Path::new(authored_root)),
                        storage_base.as_deref(),
                        &message,
                    ),
                    json!({ "operationId": operation_id }),
                )
            })?;
        serde_json::to_value(status).map_err(|_| {
            ApiError::command_failed(
                "Catalog recovery status could not be represented.",
                json!({ "operationId": operation_id }),
            )
        })
    }

    pub fn create_recipe_from_template(
        &mut self,
        template_path: &str,
        destination_path: &str,
        recipe_id: &str,
        authored_root: Option<&str>,
    ) -> Result<Value, ApiError> {
        let document = RecipeDocument::create_from_template_with_storage(
            template_path,
            destination_path,
            recipe_id,
            authored_root,
            self.catalog_storage_base.as_deref(),
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
        let catalog_storage_base = self.catalog_storage_base.clone();
        let document = self.document_mut(document_id)?;
        document
            .save_with_storage(catalog_storage_base.as_deref())
            .map_err(|error| {
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
        let catalog_storage_base = self.catalog_storage_base.clone();
        let document = self.document_mut(document_id)?;
        document
            .save_as_with_storage(path, catalog_storage_base.as_deref())
            .map_err(|error| {
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
        let catalog_storage_base = self.catalog_storage_base.clone();
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
        let opened_file = persist_app_definition_with_storage(
            document.app_definition(),
            &admission,
            catalog_storage_base.as_deref(),
        )
        .map_err(|error| {
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
        let catalog_storage_base = self.catalog_storage_base.clone();
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
        let opened_file = persist_app_definition_with_storage(
            document.app_definition(),
            &admission,
            catalog_storage_base.as_deref(),
        )
        .map_err(|error| {
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
        // Successful Recipe validation returns the complete refreshed
        // projection so the editor applies authoritative state instead of
        // merging a diagnostics-only response.
        Ok(json!({
            "diagnostics": document.diagnostics(),
            "document": dto::document_to_dto(document, document_id),
        }))
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

    fn app_definition_document(
        &self,
        document_id: &str,
    ) -> Result<&AppDefinitionDocument, ApiError> {
        if self.documents.contains_key(document_id) {
            return Err(wrong_document_kind(document_id, "app_definition"));
        }
        self.app_definitions
            .get(document_id)
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

#[cfg(test)]
mod catalog_refactor_dirty_session_tests {
    use super::*;
    use crate::catalog_refactor::{CatalogChangeIntent, CatalogRefactorSession};
    use serde_json::json;
    use std::fs;
    use std::path::Path;

    #[test]
    fn catalog_diagnostics_redact_authored_and_application_storage_paths() {
        let temporary = tempfile::TempDir::new().expect("temporary paths");
        let authored_root = temporary.path().join("authored");
        let storage_base = temporary.path().join("application-state");
        let message = format!(
            "Failed to read {}/apps/demo.yaml from {}/catalog-changes/record.json",
            authored_root.display(),
            storage_base.display()
        );

        let sanitized =
            sanitized_catalog_diagnostic(Some(&authored_root), Some(&storage_base), &message);

        assert!(!sanitized.contains(temporary.path().to_string_lossy().as_ref()));
        assert!(sanitized.contains("<authored-root>/apps/demo.yaml"));
        assert!(sanitized.contains("<application-data>/catalog-changes/record.json"));
    }

    #[test]
    fn catalog_recovery_discovery_errors_do_not_echo_the_authored_root() {
        let temporary = tempfile::TempDir::new().expect("temporary catalog");
        let missing_root = temporary.path().join("missing-authored-root");
        let sessions = DocumentSessionManager::default();

        let error = sessions
            .list_catalog_change_recoveries(missing_root.to_string_lossy().as_ref())
            .expect_err("a missing authored root must fail recovery discovery");
        let diagnostic = format!("{error:?}");

        assert!(!diagnostic.contains(missing_root.to_string_lossy().as_ref()));
        assert!(!diagnostic.contains(temporary.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn app_rename_blocks_dirty_recipe_reference_not_present_on_disk() {
        let temporary = tempfile::TempDir::new().expect("temporary catalog");
        let authored_root = temporary.path().join("authored");
        let apps = authored_root.join("apps");
        let recipes = authored_root.join("recipes");
        fs::create_dir_all(&apps).unwrap();
        fs::create_dir_all(&recipes).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/authored_root/editor_app_catalog/authored");
        let app_path = apps.join("demo_app.yaml");
        let recipe_path = recipes.join("demo_recipe.yaml");
        fs::copy(fixtures.join("apps/demo_app.yaml"), &app_path).unwrap();
        fs::copy(fixtures.join("recipes/demo_recipe.yaml"), &recipe_path).unwrap();

        let mut sessions = DocumentSessionManager {
            catalog_refactors: CatalogRefactorSession::with_storage(
                temporary.path().join("application-state"),
            ),
            ..DocumentSessionManager::default()
        };
        let root = authored_root.to_string_lossy().to_string();
        let recipe = sessions
            .open_recipe(recipe_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let recipe_id = recipe["document"]["documentId"].as_str().unwrap();
        sessions
            .apply_recipe_command(
                recipe_id,
                &json!({
                    "type": "AddStep",
                    "stepId": "launch",
                    "stepType": "launch_app",
                    "name": "Launch",
                }),
            )
            .unwrap();
        sessions
            .apply_recipe_command(
                recipe_id,
                &json!({
                    "type": "SetStepAppRef",
                    "stepId": "launch",
                    "appRef": "demo_app",
                }),
            )
            .unwrap();

        let app = sessions
            .open_app_definition(app_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let app_document_id = app["document"]["documentId"].as_str().unwrap();
        let preview = sessions
            .preview_catalog_change(
                app_document_id,
                CatalogChangeIntent::RenameApp {
                    app_id: "demo_app".into(),
                    new_id: "renamed_app".into(),
                },
            )
            .unwrap();
        let operation_id = preview["operationId"].as_str().unwrap();
        let preview_token = preview["previewToken"].as_str().unwrap();
        assert_eq!(
            preview["blockers"],
            json!([{
                "path": "recipes/demo_recipe.yaml",
                "reason": "This Recipe has unsaved typed references affected by the catalog change."
            }])
        );

        let error = sessions
            .apply_catalog_change(operation_id, preview_token)
            .expect_err("unsaved typed references must block a catalog identity change");

        assert!(format!("{error:?}").contains("demo_recipe.yaml"));
        assert!(app_path.exists());
        assert!(!apps.join("renamed_app.yaml").exists());
        assert!(!fs::read_to_string(&recipe_path)
            .unwrap()
            .contains("app_ref: demo_app"));
    }

    #[test]
    fn selected_app_document_cannot_preview_another_app_identity_change() {
        let temporary = tempfile::TempDir::new().expect("temporary catalog");
        let authored_root = temporary.path().join("authored");
        let apps = authored_root.join("apps");
        fs::create_dir_all(&apps).unwrap();
        fs::create_dir_all(authored_root.join("recipes")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/authored_root/editor_app_catalog/authored");
        let selected_path = apps.join("demo_app.yaml");
        let other_path = apps.join("second_app.yaml");
        fs::copy(fixtures.join("apps/demo_app.yaml"), &selected_path).unwrap();
        fs::copy(fixtures.join("apps/second_app.yaml"), &other_path).unwrap();

        let mut sessions = DocumentSessionManager {
            catalog_refactors: CatalogRefactorSession::with_storage(
                temporary.path().join("application-state"),
            ),
            ..DocumentSessionManager::default()
        };
        let root = authored_root.to_string_lossy().to_string();
        let selected = sessions
            .open_app_definition(selected_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let selected_id = selected["document"]["documentId"].as_str().unwrap();

        let error = sessions
            .preview_catalog_change(
                selected_id,
                CatalogChangeIntent::RenameApp {
                    app_id: "second_app".into(),
                    new_id: "renamed_other".into(),
                },
            )
            .expect_err("a document must not operate on another App identity");

        assert!(format!("{error:?}").contains("second_app"));
        let deletion_error = sessions
            .preview_catalog_change(
                selected_id,
                CatalogChangeIntent::DeleteApp {
                    app_id: "second_app".into(),
                },
            )
            .expect_err("a document must not delete another App identity");
        assert!(format!("{deletion_error:?}").contains("second_app"));
        assert!(selected_path.exists());
        assert!(other_path.exists());
        assert!(!apps.join("renamed_other.yaml").exists());
    }

    #[test]
    fn preview_binding_rejects_apply_after_selected_document_changes() {
        let temporary = tempfile::TempDir::new().expect("temporary catalog");
        let authored_root = temporary.path().join("authored");
        let apps = authored_root.join("apps");
        fs::create_dir_all(&apps).unwrap();
        fs::create_dir_all(authored_root.join("recipes")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/authored_root/editor_app_catalog/authored");
        let app_path = apps.join("demo_app.yaml");
        fs::copy(fixtures.join("apps/demo_app.yaml"), &app_path).unwrap();

        let mut sessions = DocumentSessionManager {
            catalog_refactors: CatalogRefactorSession::with_storage(
                temporary.path().join("application-state"),
            ),
            ..DocumentSessionManager::default()
        };
        let root = authored_root.to_string_lossy().to_string();
        let document = sessions
            .open_app_definition(app_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let document_id = document["document"]["documentId"].as_str().unwrap();
        let preview = sessions
            .preview_catalog_change(
                document_id,
                CatalogChangeIntent::RenameApp {
                    app_id: "demo_app".into(),
                    new_id: "renamed_app".into(),
                },
            )
            .unwrap();
        sessions
            .apply_app_definition_command(
                document_id,
                &json!({"type": "SetName", "name": "Draft name changed after preview"}),
            )
            .unwrap();

        let error = sessions
            .apply_catalog_change(
                preview["operationId"].as_str().unwrap(),
                preview["previewToken"].as_str().unwrap(),
            )
            .expect_err("changing the selected document must invalidate its preview binding");

        assert!(format!("{error:?}").contains("preview"));
        assert!(app_path.exists());
        assert!(!apps.join("renamed_app.yaml").exists());
    }

    #[test]
    fn catalog_preview_and_status_paths_are_authored_root_relative() {
        let temporary = tempfile::TempDir::new().expect("temporary catalog");
        let authored_root = temporary.path().join("authored");
        let apps = authored_root.join("apps");
        fs::create_dir_all(&apps).unwrap();
        fs::create_dir_all(authored_root.join("recipes")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/authored_root/editor_app_catalog/authored");
        let app_path = apps.join("demo_app.yaml");
        fs::copy(fixtures.join("apps/demo_app.yaml"), &app_path).unwrap();

        let mut sessions = DocumentSessionManager {
            catalog_refactors: CatalogRefactorSession::with_storage(
                temporary.path().join("application-state"),
            ),
            ..DocumentSessionManager::default()
        };

        let root = authored_root.to_string_lossy().to_string();
        let document = sessions
            .open_app_definition(app_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let document_id = document["document"]["documentId"].as_str().unwrap();
        let preview = sessions
            .preview_catalog_change(
                document_id,
                CatalogChangeIntent::RenameApp {
                    app_id: "demo_app".into(),
                    new_id: "renamed_app".into(),
                },
            )
            .unwrap();
        assert_eq!(preview["references"][0]["path"], "apps/demo_app.yaml");
        assert_eq!(
            preview["changedFiles"],
            json!(["apps/demo_app.yaml", "apps/renamed_app.yaml"])
        );

        let status = sessions
            .apply_catalog_change(
                preview["operationId"].as_str().unwrap(),
                preview["previewToken"].as_str().unwrap(),
            )
            .unwrap();
        assert_eq!(
            status["changedFiles"],
            json!(["apps/demo_app.yaml", "apps/renamed_app.yaml"])
        );
        let replay = sessions
            .catalog_change_status(&root, status["operationId"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            replay["operation"]["changedFiles"],
            json!(["apps/demo_app.yaml", "apps/renamed_app.yaml"])
        );
    }

    #[test]
    fn catalog_apply_failure_does_not_expose_absolute_authored_paths() {
        let temporary = tempfile::TempDir::new().expect("temporary catalog");
        let authored_root = temporary.path().join("authored");
        let apps = authored_root.join("apps");
        fs::create_dir_all(&apps).unwrap();
        fs::create_dir_all(authored_root.join("recipes")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/authored_root/editor_app_catalog/authored");
        let app_path = apps.join("demo_app.yaml");
        fs::copy(fixtures.join("apps/demo_app.yaml"), &app_path).unwrap();

        let mut sessions = DocumentSessionManager {
            catalog_refactors: CatalogRefactorSession::with_storage(
                temporary.path().join("application-state"),
            ),
            ..DocumentSessionManager::default()
        };
        let root = authored_root.to_string_lossy().to_string();
        let document = sessions
            .open_app_definition(app_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let document_id = document["document"]["documentId"].as_str().unwrap();
        let preview = sessions
            .preview_catalog_change(
                document_id,
                CatalogChangeIntent::RenameApp {
                    app_id: "demo_app".into(),
                    new_id: "renamed_app".into(),
                },
            )
            .unwrap();
        let external_yaml = fs::read_to_string(&app_path)
            .unwrap()
            .replace("Demo App", "Externally edited App");
        fs::write(&app_path, external_yaml).unwrap();

        let error = sessions
            .apply_catalog_change(
                preview["operationId"].as_str().unwrap(),
                preview["previewToken"].as_str().unwrap(),
            )
            .expect_err("an external edit must reject the stale preview");
        let diagnostic = format!("{error:?}");

        assert!(diagnostic.contains("apps/demo_app.yaml"));
        assert!(!diagnostic.contains(&root));
        assert!(!apps.join("renamed_app.yaml").exists());
    }

    #[test]
    fn successful_target_rename_refreshes_unrelated_open_recipe_projection() {
        let temporary = tempfile::TempDir::new().expect("temporary catalog");
        let authored_root = temporary.path().join("authored");
        let apps = authored_root.join("apps");
        let recipes = authored_root.join("recipes");
        fs::create_dir_all(&apps).unwrap();
        fs::create_dir_all(&recipes).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/authored_root/editor_app_catalog/authored");
        let app_path = apps.join("demo_app.yaml");
        fs::copy(fixtures.join("apps/demo_app.yaml"), &app_path).unwrap();
        fs::copy(
            fixtures.join("recipes/demo.target_usage.yaml"),
            recipes.join("demo.target_usage.yaml"),
        )
        .unwrap();
        let observer_path = recipes.join("observer.yaml");
        let observer_source = "schema_version: 1\nkind: recipe\nid: observer\nname: Observer\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs: {}\nartifacts: {}\nartifact_groups: {}\nsteps:\n  - id: launch\n    type: launch_app\n    name: Launch\n    user_toggleable: false\n    app_ref: demo_app\n    dependencies: []\n    constraints:\n      capabilities: []\n      conflicts_with: []\n    skip_if: []\n    params: {}\n    verify: []\n";
        fs::write(&observer_path, observer_source).unwrap();

        let mut sessions = DocumentSessionManager {
            catalog_refactors: CatalogRefactorSession::with_storage(
                temporary.path().join("application-state"),
            ),
            ..DocumentSessionManager::default()
        };
        let root = authored_root.to_string_lossy().to_string();
        let observer = sessions
            .open_recipe(observer_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let observer_id = observer["document"]["documentId"].as_str().unwrap();
        sessions
            .apply_recipe_command(
                observer_id,
                &json!({
                    "type": "SetStepUserToggleable",
                    "stepId": "launch",
                    "userToggleable": true,
                }),
            )
            .unwrap();
        let app = sessions
            .open_app_definition(app_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let app_id = app["document"]["documentId"].as_str().unwrap();
        let preview = sessions
            .preview_catalog_change(
                app_id,
                CatalogChangeIntent::RenameTarget {
                    app_id: "demo_app".into(),
                    target_id: "config_file".into(),
                    new_id: "config_file_new".into(),
                },
            )
            .unwrap();
        assert_eq!(preview["blockers"], json!([]));
        let status = sessions
            .apply_catalog_change(
                preview["operationId"].as_str().unwrap(),
                preview["previewToken"].as_str().unwrap(),
            )
            .unwrap();
        assert_eq!(status["state"], "completed");

        let refreshed = sessions.get_document(observer_id).unwrap();
        let observer = &refreshed["document"];
        let targets = observer["appCatalog"]["apps"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|app| app["targets"].as_array().unwrap())
            .filter_map(|target| target["id"].as_str())
            .collect::<Vec<_>>();
        assert!(targets.contains(&"config_file_new"));
        assert!(!targets.contains(&"config_file"));
        assert_eq!(observer["dirty"], true);
        assert_eq!(observer["canUndo"], true);
        assert!(observer["yaml"]
            .as_str()
            .unwrap()
            .contains("user_toggleable: true"));
        assert_eq!(fs::read_to_string(&observer_path).unwrap(), observer_source);
    }

    #[test]
    fn preview_reports_another_dirty_app_definition_session() {
        let temporary = tempfile::TempDir::new().expect("temporary catalog");
        let authored_root = temporary.path().join("authored");
        let apps = authored_root.join("apps");
        fs::create_dir_all(&apps).unwrap();
        fs::create_dir_all(authored_root.join("recipes")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/authored_root/editor_app_catalog/authored");
        let app_path = apps.join("demo_app.yaml");
        fs::copy(fixtures.join("apps/demo_app.yaml"), &app_path).unwrap();

        let mut sessions = DocumentSessionManager {
            catalog_refactors: CatalogRefactorSession::with_storage(
                temporary.path().join("application-state"),
            ),
            ..DocumentSessionManager::default()
        };
        let root = authored_root.to_string_lossy().to_string();
        let selected = sessions
            .open_app_definition(app_path.to_str().unwrap(), Some(&root))
            .unwrap();
        let dirty = sessions
            .open_app_definition(app_path.to_str().unwrap(), Some(&root))
            .unwrap();
        sessions
            .apply_app_definition_command(
                dirty["document"]["documentId"].as_str().unwrap(),
                &json!({"type": "SetName", "name": "Unsaved App Name"}),
            )
            .unwrap();

        let preview = sessions
            .preview_catalog_change(
                selected["document"]["documentId"].as_str().unwrap(),
                CatalogChangeIntent::RenameTarget {
                    app_id: "demo_app".into(),
                    target_id: "config_file".into(),
                    new_id: "config_file_new".into(),
                },
            )
            .unwrap();

        assert_eq!(
            preview["blockers"],
            json!([{
                "path": "apps/demo_app.yaml",
                "reason": "This App Definition has unsaved changes in an affected catalog file."
            }])
        );
    }
}
