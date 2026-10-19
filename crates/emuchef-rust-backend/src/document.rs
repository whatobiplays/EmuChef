//! In-memory recipe document state for sidecar sessions.
//!
//! A document wraps the current authored recipe model with the editor-facing
//! lifecycle state needed for sidecar document sessions. The Rust backend keeps
//! planner behavior and executor behavior out of this document abstraction
//! slice while preserving the stored authoredRoot needed for validation refresh.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::catalog;
use crate::catalog_change::{
    snapshot_existing_file, CatalogChangeCoordinator, CatalogChangeError, FileSnapshot,
    PublicationGuard,
};
use crate::commands::{ArtifactField, InputField, OverviewField, OverviewValue, RecipeCommand};
use crate::model::{
    AppArtifactInputBinding, AppArtifactReference, InputDeclaration, InputValidation, OrderedMap,
    ParamValue, Recipe, RecipeArtifact, RemoteFileArtifact, Step, StepCondition, StepConstraints,
};
use crate::step_specs;
use crate::validation;
use crate::yaml::{self, RecipeLoadError};

#[derive(Clone, Debug)]
pub struct RecipeDocument {
    path: PathBuf,
    authored_root: Option<PathBuf>,
    persisted_snapshot: FileSnapshot,
    recipe: Recipe,
    current_yaml: String,
    saved_yaml: String,
    diagnostics: Vec<Value>,
    /// Catalog choices projected once per diagnostics refresh.
    ///
    /// Serializing the document reuses this value so DTO projections never
    /// re-scan the authored catalog.
    app_catalog: catalog::AppCatalogProjection,
    undo_stack: Vec<ContentSnapshot>,
    redo_stack: Vec<ContentSnapshot>,
}

#[derive(Clone, Debug)]
struct ContentSnapshot {
    recipe: Recipe,
    current_yaml: String,
    diagnostics: Vec<Value>,
}

fn with_destination_publication<T>(
    authored_root: Option<&Path>,
    destination: &Path,
    storage_base: Option<&Path>,
    publish: impl FnOnce(&CatalogChangeCoordinator, &PublicationGuard) -> Result<T, CatalogChangeError>,
) -> Result<T, CatalogChangeError> {
    let destination_coordinator =
        CatalogChangeCoordinator::for_destination_with_storage(None, destination, storage_base)?;
    let Some(authored_root) = authored_root else {
        return destination_coordinator
            .with_publication(|guard| publish(&destination_coordinator, guard));
    };

    let source_coordinator = match storage_base {
        Some(storage_base) => {
            CatalogChangeCoordinator::open_with_storage(authored_root, storage_base)
        }
        None => CatalogChangeCoordinator::open(authored_root),
    }?;
    CatalogChangeCoordinator::with_two_publications(
        &source_coordinator,
        &destination_coordinator,
        |_source_guard, destination_guard| publish(&destination_coordinator, destination_guard),
    )
}

fn with_recipe_publications<T>(
    authored_root: Option<&Path>,
    source_path: &Path,
    destination: &Path,
    storage_base: Option<&Path>,
    publish: impl FnOnce(
        &CatalogChangeCoordinator,
        &PublicationGuard,
        &CatalogChangeCoordinator,
        &PublicationGuard,
    ) -> Result<T, CatalogChangeError>,
) -> Result<T, CatalogChangeError> {
    let mut coordinators = Vec::with_capacity(3);
    if let Some(authored_root) = authored_root {
        coordinators.push(match storage_base {
            Some(storage_base) => {
                CatalogChangeCoordinator::open_with_storage(authored_root, storage_base)?
            }
            None => CatalogChangeCoordinator::open(authored_root)?,
        });
    }
    let source_index = coordinators.len();
    coordinators.push(CatalogChangeCoordinator::for_destination_with_storage(
        None,
        source_path,
        storage_base,
    )?);
    let destination_index = coordinators.len();
    coordinators.push(CatalogChangeCoordinator::for_destination_with_storage(
        None,
        destination,
        storage_base,
    )?);
    let coordinator_refs = coordinators.iter().collect::<Vec<_>>();

    CatalogChangeCoordinator::with_publications(&coordinator_refs, |guards| {
        publish(
            &coordinators[source_index],
            guards[source_index],
            &coordinators[destination_index],
            guards[destination_index],
        )
    })
}

/// Capture the bytes and filesystem identity that must remain current after a
/// Recipe publication completes.
fn published_snapshot(
    coordinator: &CatalogChangeCoordinator,
    path: &Path,
    expected_bytes: &[u8],
) -> Result<FileSnapshot, CatalogChangeError> {
    let snapshot = coordinator.snapshot_file(path)?.ok_or_else(|| {
        CatalogChangeError::InvalidOperation(
            "the published Recipe disappeared before its revision could be recorded".into(),
        )
    })?;
    if snapshot.bytes != expected_bytes {
        return Err(CatalogChangeError::InvalidOperation(
            "the published Recipe changed before its revision could be recorded".into(),
        ));
    }
    Ok(snapshot)
}

/// Validate a Recipe before publication and return its diagnostics.
///
/// App-bearing Recipes cannot be published without a destination catalog because
/// their App Definitions provide required context.
fn recipe_publication_diagnostics(
    recipe: &crate::model::Recipe,
    destination: &Path,
    authored_root: Option<&Path>,
) -> Result<Vec<Value>, String> {
    if authored_root.is_none() && catalog::recipe_references_any_app(recipe) {
        return Err(
            "Recipe publication is blocked; an App-aware Recipe requires a destination catalog with App Definitions."
                .to_string(),
        );
    }
    let validation_path = PathBuf::from(yaml::resolved_path_string(destination));
    let diagnostics = validation_diagnostics_for_recipe(recipe, &validation_path, authored_root);
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic["severity"] == "error")
    {
        return Err(
            "Recipe publication is blocked; review the Recipe and destination catalog diagnostics."
                .to_string(),
        );
    }
    Ok(diagnostics)
}

impl RecipeDocument {
    /// Open the exact Recipe file revision that the document will later protect
    /// from overwriting if another process changes it first.
    pub fn open(
        path: impl AsRef<Path>,
        authored_root: Option<&str>,
    ) -> Result<Self, RecipeLoadError> {
        let input_path = path.as_ref();
        let persisted_snapshot = snapshot_existing_file(input_path)
            .map_err(|error| RecipeLoadError::io(error.to_string()))?;
        let recipe = yaml::load_recipe_from_bytes(&persisted_snapshot.bytes, input_path)?;
        let current_yaml = yaml::emit_recipe_yaml(&recipe)?;
        let path = PathBuf::from(yaml::resolved_path_string(input_path));
        let authored_root = catalog::normalize_authored_root(authored_root, &path);
        let diagnostics =
            validation_diagnostics_for_recipe(&recipe, &path, authored_root.as_deref());
        let app_catalog = catalog::project_app_catalog(authored_root.as_deref());

        Ok(Self {
            path,
            authored_root,
            persisted_snapshot,
            recipe,
            saved_yaml: current_yaml.clone(),
            current_yaml,
            diagnostics,
            app_catalog,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    /// Create a clean document by copying authored recipe content from a template.
    ///
    /// Template creation is a backend document operation: the template is parsed
    /// with the normal authored recipe loader, only the recipe id is replaced,
    /// and the canonical YAML is written to the requested destination. The
    /// document starts clean because the saved baseline is the newly written
    /// destination content.
    pub fn create_from_template(
        template_path: impl AsRef<Path>,
        destination_path: impl AsRef<Path>,
        recipe_id: &str,
        authored_root: Option<&str>,
    ) -> Result<Self, String> {
        Self::create_from_template_with_storage(
            template_path,
            destination_path,
            recipe_id,
            authored_root,
            None,
        )
    }

    pub(crate) fn create_from_template_with_storage(
        template_path: impl AsRef<Path>,
        destination_path: impl AsRef<Path>,
        recipe_id: &str,
        authored_root: Option<&str>,
        storage_base: Option<&Path>,
    ) -> Result<Self, String> {
        let template_path = template_path.as_ref();
        let destination_path = destination_path.as_ref();
        with_destination_publication(
            authored_root.map(Path::new),
            destination_path,
            storage_base,
            |coordinator, guard| {
                coordinator.validate_destination(destination_path)?;
                Self::create_from_template_uncoordinated(
                    template_path,
                    destination_path,
                    recipe_id,
                    coordinator,
                    guard,
                )
                .map_err(CatalogChangeError::InvalidOperation)
            },
        )
        .map_err(|error| error.to_string())
    }

    fn create_from_template_uncoordinated(
        template_path: &Path,
        destination_path: &Path,
        recipe_id: &str,
        coordinator: &CatalogChangeCoordinator,
        guard: &PublicationGuard,
    ) -> Result<Self, String> {
        let mut recipe =
            yaml::load_recipe_from_path(template_path).map_err(|error| error.to_string())?;
        recipe.id = recipe_id.to_string();
        let destination_authored_root = coordinator.authored_catalog_root().map(Path::to_path_buf);
        let diagnostics = recipe_publication_diagnostics(
            &recipe,
            destination_path,
            destination_authored_root.as_deref(),
        )?;
        let current_yaml = yaml::emit_recipe_yaml(&recipe).map_err(|error| error.to_string())?;
        Self::publish_recipe_bytes(
            coordinator,
            guard,
            destination_path,
            current_yaml.as_bytes(),
            None,
        )
        .map_err(|error| error.to_string())?;
        let persisted_snapshot =
            published_snapshot(coordinator, destination_path, current_yaml.as_bytes())
                .map_err(|error| error.to_string())?;
        let path = PathBuf::from(yaml::resolved_path_string(destination_path));
        let app_catalog = catalog::project_app_catalog(destination_authored_root.as_deref());

        Ok(Self {
            path,
            authored_root: destination_authored_root,
            persisted_snapshot,
            recipe,
            saved_yaml: current_yaml.clone(),
            current_yaml,
            diagnostics,
            app_catalog,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    /// Save this document only if its opened file revision is still current.
    pub fn save(&mut self) -> Result<(), String> {
        self.save_with_storage(None)
    }

    pub(crate) fn save_with_storage(&mut self, storage_base: Option<&Path>) -> Result<(), String> {
        let authored_root = self.authored_root.clone();
        let path = self.path.clone();
        with_recipe_publications(
            authored_root.as_deref(),
            &path,
            &path,
            storage_base,
            |source, source_guard, destination, destination_guard| {
                source.publish_with_guard(source_guard, |_| {
                    self.ensure_persisted_snapshot_current(source)
                })?;
                self.save_with_guard(destination, destination_guard)
            },
        )
        .map_err(|error| error.to_string())
    }

    /// Save while a caller already owns the authored-root publication lock.
    pub(crate) fn save_with_guard(
        &mut self,
        coordinator: &CatalogChangeCoordinator,
        guard: &PublicationGuard,
    ) -> Result<(), CatalogChangeError> {
        coordinator.publish_with_guard(guard, |_| {
            coordinator.validate_destination(&self.path)?;
            self.save_uncoordinated(coordinator, guard)
                .map_err(CatalogChangeError::InvalidOperation)
        })
    }

    fn save_uncoordinated(
        &mut self,
        coordinator: &CatalogChangeCoordinator,
        guard: &PublicationGuard,
    ) -> Result<(), String> {
        recipe_publication_diagnostics(
            &self.recipe,
            &self.path,
            coordinator.authored_catalog_root(),
        )?;
        let current_yaml =
            yaml::emit_recipe_yaml(&self.recipe).map_err(|error| error.to_string())?;
        Self::publish_recipe_bytes(
            coordinator,
            guard,
            &self.path,
            current_yaml.as_bytes(),
            Some(&self.persisted_snapshot),
        )
        .map_err(|error| error.to_string())?;
        self.persisted_snapshot =
            published_snapshot(coordinator, &self.path, current_yaml.as_bytes())
                .map_err(|error| error.to_string())?;
        self.current_yaml = current_yaml;
        self.saved_yaml = self.current_yaml.clone();
        self.refresh_diagnostics();
        Ok(())
    }

    /// Save a current Recipe revision to a new path while coordinating both
    /// the source and destination roots.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        self.save_as_with_storage(path, None)
    }

    pub(crate) fn save_as_with_storage(
        &mut self,
        path: impl AsRef<Path>,
        storage_base: Option<&Path>,
    ) -> Result<(), String> {
        let path = path.as_ref();
        let authored_root = self.authored_root.clone();
        let source_path = self.path.clone();
        with_recipe_publications(
            authored_root.as_deref(),
            &source_path,
            path,
            storage_base,
            |source, source_guard, destination, destination_guard| {
                source.publish_with_guard(source_guard, |_| {
                    self.ensure_persisted_snapshot_current(source)
                })?;
                self.save_as_with_guard(destination, destination_guard, path)
            },
        )
        .map_err(|error| error.to_string())
    }

    /// Save As while a caller already owns the authored-root publication lock.
    pub(crate) fn save_as_with_guard(
        &mut self,
        coordinator: &CatalogChangeCoordinator,
        guard: &PublicationGuard,
        path: &Path,
    ) -> Result<(), CatalogChangeError> {
        coordinator.publish_with_guard(guard, |_| {
            coordinator.validate_destination(path)?;
            self.save_as_uncoordinated(coordinator, guard, path)
                .map_err(CatalogChangeError::InvalidOperation)
        })
    }

    fn save_as_uncoordinated(
        &mut self,
        coordinator: &CatalogChangeCoordinator,
        guard: &PublicationGuard,
        path: &Path,
    ) -> Result<(), String> {
        recipe_publication_diagnostics(&self.recipe, path, coordinator.authored_catalog_root())?;
        let destination_authored_root = coordinator.authored_catalog_root().map(Path::to_path_buf);
        let current_yaml =
            yaml::emit_recipe_yaml(&self.recipe).map_err(|error| error.to_string())?;
        Self::publish_recipe_bytes(coordinator, guard, path, current_yaml.as_bytes(), None)
            .map_err(|error| error.to_string())?;
        let persisted_snapshot = published_snapshot(coordinator, path, current_yaml.as_bytes())
            .map_err(|error| error.to_string())?;
        self.path = PathBuf::from(yaml::resolved_path_string(path));
        self.authored_root = destination_authored_root;
        self.persisted_snapshot = persisted_snapshot;
        self.current_yaml = current_yaml;
        self.saved_yaml = self.current_yaml.clone();
        self.refresh_diagnostics();
        Ok(())
    }

    fn publish_recipe_bytes(
        coordinator: &CatalogChangeCoordinator,
        guard: &PublicationGuard,
        destination: &Path,
        bytes: &[u8],
        expected: Option<&FileSnapshot>,
    ) -> Result<(), CatalogChangeError> {
        let operation_id = match expected {
            Some(expected) => coordinator.stage_file_write_with_expected_snapshot(
                guard,
                destination,
                expected,
                bytes,
            )?,
            None => coordinator.stage_file_write(guard, destination, bytes)?,
        };
        coordinator.publish_staged_file_write(guard, &operation_id)
    }

    fn ensure_persisted_snapshot_current(
        &self,
        coordinator: &CatalogChangeCoordinator,
    ) -> Result<(), CatalogChangeError> {
        if coordinator.snapshot_file(&self.path)?.as_ref() != Some(&self.persisted_snapshot) {
            return Err(CatalogChangeError::InvalidOperation(
                "the Recipe changed on disk since it was opened or last saved; reload before saving"
                    .into(),
            ));
        }
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

    /// Replace only the catalog-validation context for the open document.
    ///
    /// Authored root is session metadata, not authored YAML. Clearing it leaves
    /// the in-memory recipe, YAML text, dirty state, path, and history stacks
    /// unchanged while forcing diagnostics to reflect the new catalog context.
    pub fn set_authored_root(&mut self, authored_root: Option<&str>) {
        self.authored_root = match authored_root {
            Some(root) => catalog::normalize_authored_root(Some(root), &self.path),
            None => None,
        };
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

    /// Catalog choices projected for the current authored root.
    pub fn app_catalog(&self) -> &catalog::AppCatalogProjection {
        &self.app_catalog
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
            RecipeCommand::AddAppArtifact {
                artifact_id,
                app_ref,
                artifact,
                input_id,
            } => self.updated_add_app_artifact_recipe(artifact_id, app_ref, artifact, input_id),
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
            RecipeCommand::SetStepAppRef { step_id, app_ref } => {
                self.updated_step_app_ref_recipe(step_id, app_ref)
            }
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
                    allowed_prefixes: Vec::new(),
                },
                default: Value::Null,
                options: Vec::new(),
                sensitive: false,
                advanced: false,
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
        // Renaming rewrites supported structured usages only.
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
            InputField::Label => updated.label = json_value_to_string(&value),
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
        // Deletion removes supported structured references only.
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
            RecipeArtifact::RemoteFile(RemoteFileArtifact {
                url: required_text(&Value::String(url), "artifact url")?,
                cache: "default".to_string(),
            }),
        );
        Ok(recipe)
    }

    /// Add one Recipe App Artifact that references an App Definition artifact.
    ///
    /// Only the Recipe-owned reference is authored. An optional bound input id
    /// must already exist in the Recipe: the command never creates inputs,
    /// resolvers, or dependencies as a side effect.
    fn updated_add_app_artifact_recipe(
        &self,
        artifact_id: String,
        app_ref: String,
        artifact: String,
        input_id: Option<String>,
    ) -> Result<Recipe, String> {
        let artifact_id = normalize_catalog_identifier(&artifact_id, "artifact id")?;
        let app_ref = normalize_catalog_identifier(&app_ref, "app ref")?;
        let artifact = normalize_catalog_identifier(&artifact, "App Definition artifact id")?;
        let input = match input_id {
            Some(input_id) => Some(AppArtifactInputBinding {
                input_id: normalize_catalog_identifier(&input_id, "input id")?,
            }),
            None => None,
        };
        let mut recipe = self.recipe.clone();
        if recipe.artifacts.contains_key(&artifact_id) {
            return Err(format!("Artifact {artifact_id:?} already exists."));
        }
        recipe.artifacts.insert(
            artifact_id,
            RecipeArtifact::AppArtifact(AppArtifactReference {
                app_ref,
                artifact,
                input,
                cache: "default".to_string(),
            }),
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
        // Renaming rewrites supported structured usages only.
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

    /// Updates one editable field of an existing Recipe artifact.
    ///
    /// Remote file artifacts own their URL and cache policy. App Artifacts own
    /// only the Recipe-side reference (app ref, artifact name, and optional
    /// user-provided input binding) plus cache policy: source and package facts
    /// stay with the App Definition, so `url` updates are rejected for them.
    /// Updating the app ref or artifact name never rewrites dependent values;
    /// stale values stay visible and are reported by static validation.
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
        match (artifact, field) {
            (RecipeArtifact::RemoteFile(artifact), ArtifactField::Url) => {
                artifact.url = required_text(&value, "artifact url")?;
            }
            (RecipeArtifact::RemoteFile(artifact), ArtifactField::Cache) => {
                artifact.cache = coerce_artifact_cache(&value)?;
            }
            (RecipeArtifact::AppArtifact(artifact), ArtifactField::Cache) => {
                artifact.cache = coerce_artifact_cache(&value)?;
            }
            (RecipeArtifact::AppArtifact(artifact), ArtifactField::AppRef) => {
                artifact.app_ref =
                    normalize_catalog_identifier(&required_text(&value, "app ref")?, "app ref")?;
            }
            (RecipeArtifact::AppArtifact(artifact), ArtifactField::AppArtifact) => {
                artifact.artifact = normalize_catalog_identifier(
                    &required_text(&value, "App Definition artifact id")?,
                    "App Definition artifact id",
                )?;
            }
            (RecipeArtifact::AppArtifact(artifact), ArtifactField::Input) => {
                artifact.input = match &value {
                    Value::Null => None,
                    Value::String(input_id) => Some(AppArtifactInputBinding {
                        input_id: normalize_catalog_identifier(input_id, "input id")?,
                    }),
                    _ => {
                        return Err(
                            "App Artifact input binding must be a Recipe input id or null.".into(),
                        );
                    }
                };
            }
            (RecipeArtifact::AppArtifact(_), ArtifactField::Url) => {
                return Err(
                    "App Definition artifact source policy is owned by the App Definition.".into(),
                );
            }
            (RecipeArtifact::RemoteFile(_), ArtifactField::AppRef)
            | (RecipeArtifact::RemoteFile(_), ArtifactField::AppArtifact)
            | (RecipeArtifact::RemoteFile(_), ArtifactField::Input) => {
                return Err(
                    "App Definition artifact references apply only to App Artifacts.".into(),
                );
            }
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
        // Deletion cleans group membership, step selection, and parameter
        // references in one command.
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
        // Renaming rewrites supported structured usages only.
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
        // Deletion removes supported structured references only.
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
                progress_note: None,
                user_toggleable: false,
                app_ref: None,
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
        // Step deletion removes supported dependencies, conflicts, and
        // structured references.
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

    /// Change only the step's App Definition reference. Existing parameters
    /// remain authored so the editor can show and clear them explicitly.
    fn updated_step_app_ref_recipe(
        &self,
        step_id: String,
        app_ref: Option<String>,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        recipe.steps[index].app_ref = app_ref.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        Ok(recipe)
    }

    fn updated_step_dependencies_recipe(
        &self,
        step_id: String,
        dependencies: Vec<String>,
    ) -> Result<Recipe, String> {
        let mut recipe = self.recipe.clone();
        let index = step_index(&recipe.steps, &step_id)?;
        // Command application normalizes and deduplicates dependency ids;
        // planner/catalog existence checks are intentionally out of scope.
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
        self.app_catalog = catalog::project_app_catalog(self.authored_root.as_deref());
    }

    /// Refresh catalog-backed choices and diagnostics without replacing the
    /// recipe model, YAML buffer, dirty baseline, or undo/redo history.
    pub(crate) fn refresh_catalog_projection(&mut self) {
        self.refresh_diagnostics();
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
    step_specs::authored_step_spec_for(step)
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
/// Normalizes a value that names an authored catalog object.
///
/// App-aware editor commands accept identifiers in the same lowercase
/// syntax the authored catalog loader enforces, so every accepted value can
/// later be resolved against a catalog entry. Values are trimmed before the
/// syntax check.
fn normalize_catalog_identifier(value: &str, label: &str) -> Result<String, String> {
    let normalized = normalize_identifier(value, label)?;
    if !crate::authored_models::is_valid_identifier(&normalized) {
        return Err(format!(
            "{label} {normalized:?} must use authored identifier syntax."
        ));
    }
    Ok(normalized)
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
            .is_some_and(|value| param_value_equals_default(value, &default));
        if should_remove {
            normalized.shift_remove(&param_name);
        }
    }
    normalized
}

fn param_value_equals_default(value: &ParamValue, default: &Value) -> bool {
    match value {
        ParamValue::Ref(_) | ParamValue::AppTarget(_) => false,
        ParamValue::Literal(value) => json_values_equal(value, default),
    }
}

fn json_values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Bool(left), Value::Number(right)) => bool_equals_number(*left, right),
        (Value::Number(left), Value::Bool(right)) => bool_equals_number(*right, left),
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
                    .all(|(left, right)| json_values_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left_value)| {
                    right
                        .get(key)
                        .is_some_and(|right_value| json_values_equal(left_value, right_value))
                })
        }
        _ => false,
    }
}

fn bool_equals_number(bool_value: bool, number: &serde_json::Number) -> bool {
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
    let text = json_value_to_string(value);
    normalize_identifier(&text, label)
}

fn optional_text(value: &Value) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let text = json_value_to_string(value);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => json_value_repr(value),
    }
}

fn json_value_repr(value: &Value) -> String {
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
                .map(json_value_repr)
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
                    json_value_repr(value)
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
    let value = json_value_to_string(value);
    match value.as_str() {
        "string" | "integer" | "boolean" | "enum" | "file" | "directory" | "path"
        | "device_path" | "string_list" | "path_list" | "object" => Ok(value),
        _ => Err(format!("Invalid input type {value:?}.")),
    }
}

fn coerce_optional_input_type(value: &Value) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let value = json_value_to_string(value);
    if value.is_empty() {
        return Ok(None);
    }
    match value.as_str() {
        "file" | "directory" => Ok(Some(value)),
        _ => Err(format!("Invalid input path kind {value:?}.")),
    }
}

fn coerce_input_role(value: &Value) -> Result<String, String> {
    let value = json_value_to_string(value);
    match value.as_str() {
        "apk" | "bios" | "roms" | "config_bundle" | "generic" => Ok(value),
        _ => Err(format!("Invalid input role {value:?}.")),
    }
}

fn coerce_artifact_cache(value: &Value) -> Result<String, String> {
    let value = json_value_to_string(value);
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
                let value = json_value_to_string(item);
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

#[cfg(test)]
mod catalog_coordination_tests {
    use super::*;
    use crate::catalog_change::{CatalogChangeCoordinator, OperationPhase, OperationRecord};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn save_as_rebinds_a_destination_catalog_and_uses_it_for_later_saves() {
        let source_app = include_str!(
            "../tests/fixtures/authored_root/editor_app_catalog/authored/apps/demo_app.yaml"
        );
        let source = app_aware_catalog_root(source_app, false);
        let source_path = source.path().join("recipes/source.yaml");
        let source_yaml = include_str!("../../../templates/authored/recipe.blank.template.yaml");
        fs::write(&source_path, source_yaml).expect("write source Recipe");
        let destination_app = source_app
            .replace("demo_app", "second_app")
            .replace("Demo App", "Second App")
            .replace("com.example.demo", "com.example.second");
        let destination_root = app_aware_catalog_root(&destination_app, false);
        let destination = destination_root.path().join("recipes/destination.yaml");
        let mut document = RecipeDocument::open(&source_path, source.path().to_str())
            .expect("open catalog-backed Recipe");

        document.save_as(&destination).unwrap();

        let expected_root = fs::canonicalize(destination_root.path()).unwrap();
        assert_eq!(document.authored_root(), Some(expected_root.as_path()));
        assert_eq!(
            fs::canonicalize(document.path()).unwrap(),
            fs::canonicalize(&destination).unwrap()
        );
        let app_catalog = document.app_catalog().to_dto();
        assert_eq!(app_catalog["status"], "ready", "{app_catalog:#}");
        assert_eq!(
            app_catalog["apps"]
                .as_array()
                .expect("app projection")
                .iter()
                .map(|app| app["id"].as_str().expect("App id"))
                .collect::<Vec<_>>(),
            vec!["second_app"]
        );
        assert_eq!(
            fs::read_to_string(&source_path).expect("source Recipe bytes"),
            source_yaml
        );

        document.recipe.name = "Destination save".to_string();
        document
            .save()
            .expect("a later ordinary Save uses the destination catalog");
        let saved = yaml::load_recipe_from_path(&destination).expect("load destination Recipe");
        assert_eq!(saved.name, "Destination save");
        assert!(destination.is_file());
    }

    #[test]
    fn template_can_publish_to_another_catalog() {
        let source = authored_root();
        let other = authored_root();
        let destination = other.path().join("escaped.yaml");
        RecipeDocument::create_from_template(
            source.path().join("recipe.yaml"),
            &destination,
            "new.recipe",
            source.path().to_str(),
        )
        .unwrap();
        assert!(destination.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn save_as_rejects_dangling_destination_symlink() {
        let root = authored_root();
        let target = root.path().join("missing.yaml");
        let destination = root.path().join("symlink.yaml");
        std::os::unix::fs::symlink(&target, &destination).unwrap();
        let mut document =
            RecipeDocument::open(root.path().join("recipe.yaml"), root.path().to_str()).unwrap();
        assert!(document.save_as(&destination).is_err());
        assert!(!target.exists());
    }

    fn app_aware_catalog_root(app_definition: &str, include_recipe: bool) -> TempDir {
        let temp = TempDir::new().expect("app-aware catalog root");
        let apps = temp.path().join("apps");
        let recipes = temp.path().join("recipes");
        fs::create_dir_all(&apps).expect("create App Definition directory");
        fs::create_dir_all(&recipes).expect("create Recipe directory");
        fs::write(apps.join("retroarch.yaml"), app_definition)
            .expect("write App Definition fixture");
        if include_recipe {
            fs::write(
                recipes.join("app.retroarch.provision.yaml"),
                include_str!("../../../authored/recipes/app.retroarch.provision.yaml"),
            )
            .expect("write App-aware Recipe fixture");
        }
        temp
    }

    fn catalog_root_without_app_definitions() -> TempDir {
        let temp = TempDir::new().expect("catalog root without App Definitions");
        fs::create_dir_all(temp.path().join("apps"))
            .expect("create empty App Definition directory");
        fs::create_dir_all(temp.path().join("recipes")).expect("create Recipe directory");
        temp
    }

    fn authored_root() -> TempDir {
        let temp = TempDir::new().expect("authored root");
        fs::write(
            temp.path().join("recipe.yaml"),
            include_str!("../../../templates/authored/recipe.blank.template.yaml"),
        )
        .expect("write recipe fixture");
        temp
    }

    fn condition_only_app_target_recipe_yaml() -> String {
        include_str!("../tests/fixtures/recipes/phase6k_nested_ref_literal.yaml")
            .replace("ref: inputs.missing", "app_target: system")
    }

    fn literal_condition_recipe_yaml() -> String {
        include_str!("../tests/fixtures/recipes/phase6k_nested_ref_literal.yaml")
            .replace(
                "          path:\n            ref: inputs.missing",
                "          path: /sdcard/EmuChef/condition",
            )
            .replacen("- type: path_exists", "- type: file_exists", 1)
    }

    fn leave_unresolved(root: &Path) -> CatalogChangeCoordinator {
        let coordinator = CatalogChangeCoordinator::open(root).expect("coordinator");
        let guard = coordinator.acquire_publication().expect("publication lock");
        coordinator
            .begin_operation(
                &guard,
                OperationRecord::new("writer-gate", "test", "stable-request"),
            )
            .expect("journal operation");
        coordinator
            .transition(&guard, "writer-gate", OperationPhase::Staged)
            .expect("stage operation");
        drop(guard);
        coordinator
    }

    fn clear_test_recovery(coordinator: &CatalogChangeCoordinator) {
        coordinator
            .retry_recovery("writer-gate", |guard, _record| {
                coordinator.transition(guard, "writer-gate", OperationPhase::Rejected)
            })
            .expect("terminalize test recovery");
        let _guard = coordinator
            .acquire_publication()
            .expect("clean terminal test record");
    }

    #[test]
    fn ordinary_recipe_save_is_rejected_when_recovery_is_unresolved() {
        let root = authored_root();
        let mut document = RecipeDocument::open(
            root.path().join("recipe.yaml"),
            Some(root.path().to_str().expect("utf8 temp path")),
        )
        .expect("open recipe");
        let recovery = leave_unresolved(root.path());
        let before = fs::read(root.path().join("recipe.yaml")).expect("original bytes");
        assert!(document.save().is_err());
        assert_eq!(
            fs::read(root.path().join("recipe.yaml")).expect("unchanged bytes"),
            before
        );
        clear_test_recovery(&recovery);
    }

    #[test]
    fn recipe_save_rejects_external_changes_since_open_without_overwriting_them() {
        let root = authored_root();
        let path = root.path().join("recipe.yaml");
        let mut document =
            RecipeDocument::open(&path, Some(root.path().to_str().expect("utf8 temp path")))
                .expect("open recipe");
        document.recipe.name = "stale local edit".to_string();
        let external_bytes = b"external catalog rewrite\n";
        fs::write(&path, external_bytes).expect("simulate another process rewrite");

        let error = document
            .save()
            .expect_err("a stale Recipe buffer must not overwrite a concurrent rewrite");
        assert!(error.contains("changed on disk"), "{error}");
        assert_eq!(
            fs::read(path).expect("external bytes remain"),
            external_bytes
        );
    }

    #[test]
    fn recipe_save_as_is_rejected_when_recovery_is_unresolved() {
        let root = authored_root();
        let mut document = RecipeDocument::open(
            root.path().join("recipe.yaml"),
            Some(root.path().to_str().expect("utf8 temp path")),
        )
        .expect("open recipe");
        let recovery = leave_unresolved(root.path());
        let destination = root.path().join("save-as.yaml");
        assert!(document.save_as(&destination).is_err());
        assert!(!destination.exists());
        clear_test_recovery(&recovery);
    }

    #[test]
    fn recipe_template_creation_is_rejected_when_recovery_is_unresolved() {
        let root = authored_root();
        let recovery = leave_unresolved(root.path());
        let destination = root.path().join("from-template.yaml");
        assert!(RecipeDocument::create_from_template(
            root.path().join("recipe.yaml"),
            &destination,
            "new.recipe",
            Some(root.path().to_str().expect("utf8 temp path")),
        )
        .is_err());
        assert!(!destination.exists());
        clear_test_recovery(&recovery);
    }

    #[test]
    fn save_as_outside_source_root_is_blocked_by_source_recovery() {
        let source = authored_root();
        let destination_dir = TempDir::new().expect("destination directory");
        let destination = destination_dir.path().join("saved.yaml");
        let mut document = RecipeDocument::open(
            source.path().join("recipe.yaml"),
            Some(source.path().to_str().expect("utf8 source root")),
        )
        .expect("open recipe");
        let recovery = leave_unresolved(source.path());

        let error = document
            .save_as(&destination)
            .expect_err("source recovery must block Save As");
        assert!(error.to_lowercase().contains("recovery"), "{error}");
        assert!(!destination.exists());
        clear_test_recovery(&recovery);
    }

    #[test]
    fn save_as_into_another_catalog_is_blocked_by_destination_recovery() {
        let source = authored_root();
        let target = authored_root();
        let destination = target.path().join("saved.yaml");
        let mut document = RecipeDocument::open(
            source.path().join("recipe.yaml"),
            Some(source.path().to_str().expect("utf8 source root")),
        )
        .expect("open recipe");
        let recovery = leave_unresolved(target.path());

        let error = document
            .save_as(&destination)
            .expect_err("destination recovery must block Save As");
        assert!(error.to_lowercase().contains("recovery"), "{error}");
        assert!(!destination.exists());
        clear_test_recovery(&recovery);
    }

    #[test]
    fn template_outside_source_root_is_blocked_by_source_recovery() {
        let source = authored_root();
        let destination_dir = TempDir::new().expect("destination directory");
        let destination = destination_dir.path().join("from-template.yaml");
        let recovery = leave_unresolved(source.path());

        let error = RecipeDocument::create_from_template(
            source.path().join("recipe.yaml"),
            &destination,
            "new.recipe",
            Some(source.path().to_str().expect("utf8 source root")),
        )
        .expect_err("source recovery must block template publication");
        assert!(error.to_lowercase().contains("recovery"), "{error}");
        assert!(!destination.exists());
        clear_test_recovery(&recovery);
    }

    #[test]
    fn template_creation_rejects_an_invalid_but_parseable_recipe() {
        let destination_root =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), false);
        let template_root = TempDir::new().expect("template directory");
        let template = template_root.path().join("invalid.yaml");
        let source = include_str!("../../../authored/recipes/app.retroarch.provision.yaml");
        let invalid = source.replacen(
            "  dependencies: []",
            "  dependencies:\n    - missing-step",
            1,
        );
        assert_ne!(
            source, invalid,
            "template fixture must exercise a dependency"
        );
        fs::write(&template, invalid).expect("write invalid template");
        let destination = destination_root.path().join("recipes/from-template.yaml");

        let error =
            RecipeDocument::create_from_template(&template, &destination, "new.invalid", None)
                .expect_err("semantic template errors must block publication");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn template_creation_rejects_a_missing_destination_app_definition() {
        let template_root = TempDir::new().expect("template directory");
        let template = template_root.path().join("template.yaml");
        fs::write(
            &template,
            include_str!("../../../authored/recipes/app.retroarch.provision.yaml"),
        )
        .expect("write App-aware template");
        let destination_root = catalog_root_without_app_definitions();
        let destination = destination_root.path().join("recipes/from-template.yaml");

        let error =
            RecipeDocument::create_from_template(&template, &destination, "new.missing_app", None)
                .expect_err("a missing destination App Definition must block publication");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn template_creation_rejects_an_app_aware_template_without_destination_apps() {
        let source =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), true);
        let destination_root = TempDir::new().expect("destination catalog without apps");
        fs::create_dir(destination_root.path().join("recipes"))
            .expect("create destination Recipe directory");
        let template = source.path().join("recipes/app.retroarch.provision.yaml");
        let destination = destination_root.path().join("recipes/from-template.yaml");
        let recipe = yaml::load_recipe_from_path(&template).expect("load template Recipe");
        let diagnostics =
            validation_diagnostics_for_recipe(&recipe, &destination, Some(destination_root.path()));
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic["severity"] == "error" && diagnostic["code"] == "app_definition_unknown"
            }),
            "a catalog without an apps directory must have no App Definitions: {diagnostics:#?}"
        );

        let error =
            RecipeDocument::create_from_template(&template, &destination, "new.missing_apps", None)
                .expect_err("an app-aware template must not bypass destination catalog validation");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[cfg(unix)]
    #[test]
    fn template_creation_rejects_a_dangling_destination_apps_directory() {
        let source =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), true);
        let destination_root = TempDir::new().expect("destination catalog with dangling apps");
        fs::create_dir(destination_root.path().join("recipes"))
            .expect("create destination Recipe directory");
        std::os::unix::fs::symlink("missing-apps", destination_root.path().join("apps"))
            .expect("create dangling App Definition directory");
        let template = source.path().join("recipes/app.retroarch.provision.yaml");
        let destination = destination_root.path().join("recipes/from-template.yaml");
        let recipe = yaml::load_recipe_from_path(&template).expect("load template Recipe");
        let diagnostics =
            validation_diagnostics_for_recipe(&recipe, &destination, Some(destination_root.path()));
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic["severity"] == "error"
                    && diagnostic["code"] == "app_definition_catalog_unreadable"
            }),
            "a dangling apps directory must fail closed: {diagnostics:#?}"
        );

        let error = RecipeDocument::create_from_template(
            &template,
            &destination,
            "new.dangling_apps",
            None,
        )
        .expect_err("a dangling destination apps directory must block publication");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn template_creation_rejects_a_standalone_app_aware_recipe() {
        let template_root = TempDir::new().expect("template directory");
        let template = template_root.path().join("app-aware.yaml");
        fs::write(
            &template,
            include_str!("../../../authored/recipes/app.retroarch.provision.yaml"),
        )
        .expect("write App-aware template");
        let destination_root = TempDir::new().expect("standalone destination directory");
        let destination = destination_root.path().join("standalone.yaml");

        let error =
            RecipeDocument::create_from_template(&template, &destination, "standalone.app", None)
                .expect_err("an App-aware Recipe requires destination App Definitions");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn template_creation_rejects_a_standalone_condition_only_app_target() {
        let template_root = TempDir::new().expect("template directory");
        let template = template_root.path().join("condition-only.yaml");
        fs::write(&template, condition_only_app_target_recipe_yaml())
            .expect("write condition-only template");
        let destination_root = TempDir::new().expect("standalone destination directory");
        let destination = destination_root.path().join("condition-only.yaml");

        let error = RecipeDocument::create_from_template(
            &template,
            &destination,
            "standalone.condition_only",
            None,
        )
        .expect_err("a condition-only App Target requires destination App Definitions");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn standalone_save_rejects_a_condition_only_app_target_without_modifying_the_file() {
        let root = TempDir::new().expect("standalone Recipe directory");
        let path = root.path().join("recipe.yaml");
        fs::write(&path, condition_only_app_target_recipe_yaml())
            .expect("write condition-only Recipe");
        let before = fs::read(&path).expect("read original Recipe");
        let mut document =
            RecipeDocument::open(&path, None).expect("open standalone condition-only Recipe");
        document.recipe.name = "changed without publication".to_string();

        let error = document
            .save()
            .expect_err("standalone Save must reject a condition-only App Target");

        assert!(error.contains("blocked"), "{error}");
        assert_eq!(
            fs::read(&path).expect("unchanged Recipe bytes"),
            before,
            "a rejected Save must not modify the existing Recipe"
        );
    }

    #[test]
    fn standalone_save_as_rejects_a_condition_only_app_target_without_publishing() {
        let source = TempDir::new().expect("source directory");
        let source_path = source.path().join("recipe.yaml");
        fs::write(&source_path, condition_only_app_target_recipe_yaml())
            .expect("write source condition-only Recipe");
        let before = fs::read(&source_path).expect("read source Recipe");
        let destination_root = TempDir::new().expect("destination directory");
        let destination = destination_root.path().join("saved.yaml");
        let mut document = RecipeDocument::open(&source_path, None)
            .expect("open standalone condition-only Recipe");

        let error = document
            .save_as(&destination)
            .expect_err("standalone Save As must reject a condition-only App Target");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
        assert_eq!(
            fs::read(&source_path).expect("unchanged source bytes"),
            before,
            "a rejected Save As must not modify the source Recipe"
        );
    }

    #[test]
    fn template_creation_allows_standalone_literal_condition_paths() {
        let template_root = TempDir::new().expect("template directory");
        let template = template_root.path().join("literal-conditions.yaml");
        fs::write(&template, literal_condition_recipe_yaml())
            .expect("write literal-condition template");
        let destination_root = TempDir::new().expect("standalone destination directory");
        let destination = destination_root.path().join("literal-conditions.yaml");

        let document = RecipeDocument::create_from_template(
            &template,
            &destination,
            "standalone.literal_conditions",
            None,
        )
        .expect("literal device paths do not require App Definitions");

        assert!(destination.is_file());
        assert_eq!(document.authored_root(), None);
        assert_eq!(document.recipe.steps[0].skip_if[0].type_name, "file_exists");
        assert_eq!(
            document.recipe.steps[0].skip_if[0].params["path"],
            "/sdcard/EmuChef/condition"
        );
        assert_eq!(document.recipe.steps[0].verify[0].type_name, "path_exists");
        assert_eq!(
            document.recipe.steps[0].verify[0].params["path"],
            "/sdcard/EmuChef/condition"
        );
    }

    #[test]
    fn template_creation_keeps_a_sibling_of_recipes_standalone() {
        let root = TempDir::new().expect("project root");
        fs::create_dir(root.path().join("recipes")).expect("create sibling Recipe directory");
        let standalone = root.path().join("standalone");
        fs::create_dir(&standalone).expect("create standalone directory");
        let template = root.path().join("generic.yaml");
        fs::write(
            &template,
            include_str!("../../../templates/authored/recipe.blank.template.yaml"),
        )
        .expect("write generic template");
        let destination = standalone.join("generic.yaml");

        let document = RecipeDocument::create_from_template(
            &template,
            &destination,
            "standalone.generic",
            None,
        )
        .expect("a sibling of recipes is a standalone destination");

        assert!(destination.is_file());
        assert_eq!(document.authored_root(), None);
        assert_eq!(
            fs::canonicalize(document.path()).unwrap(),
            fs::canonicalize(&destination).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn template_creation_rejects_a_symlinked_destination_recipes_directory() {
        let catalog = TempDir::new().expect("catalog root");
        let outside = TempDir::new().expect("outside root");
        let real_recipes = outside.path().join("real");
        fs::create_dir_all(real_recipes.join("sub")).expect("create outside Recipe directory");
        std::os::unix::fs::symlink(&real_recipes, catalog.path().join("recipes"))
            .expect("create symlinked Recipe directory");
        let template_root = TempDir::new().expect("template directory");
        let template = template_root.path().join("generic.yaml");
        fs::write(
            &template,
            include_str!("../../../templates/authored/recipe.blank.template.yaml"),
        )
        .expect("write generic template");
        let destination = catalog.path().join("recipes/sub/generic.yaml");
        let published_outside = real_recipes.join("sub/generic.yaml");

        let error =
            RecipeDocument::create_from_template(&template, &destination, "symlink.recipe", None)
                .expect_err("a symlinked recipes directory must fail closed");

        assert!(error.to_lowercase().contains("symlink"), "{error}");
        assert!(!destination.exists());
        assert!(!published_outside.exists());
    }

    #[test]
    fn template_creation_rejects_an_invalid_destination_app_definition() {
        let template_root = TempDir::new().expect("template directory");
        let template = template_root.path().join("template.yaml");
        fs::write(
            &template,
            include_str!("../../../authored/recipes/app.retroarch.provision.yaml"),
        )
        .expect("write App-aware template");
        let invalid_app = include_str!("../../../authored/apps/retroarch.yaml").replacen(
            "package_id: com.retroarch.aarch64",
            "package_id: invalid package",
            1,
        );
        let destination_root = app_aware_catalog_root(&invalid_app, false);
        let destination = destination_root.path().join("recipes/from-template.yaml");

        let error =
            RecipeDocument::create_from_template(&template, &destination, "new.invalid_app", None)
                .expect_err("an invalid destination App Definition must block publication");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn template_creation_rejects_a_missing_destination_app_artifact() {
        let source =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), true);
        let incompatible_app = include_str!("../../../authored/apps/retroarch.yaml").replacen(
            "  apk:\n",
            "  unavailable_apk:\n",
            1,
        );
        assert!(incompatible_app.contains("  unavailable_apk:\n"));
        let destination_root = app_aware_catalog_root(&incompatible_app, false);
        let template = source.path().join("recipes/app.retroarch.provision.yaml");
        let destination = destination_root.path().join("recipes/from-template.yaml");
        let recipe = yaml::load_recipe_from_path(&template).expect("load template Recipe");
        let diagnostics =
            validation_diagnostics_for_recipe(&recipe, &destination, Some(destination_root.path()));
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == "app_artifact_unknown"),
            "the destination must reject the missing App Artifact: {diagnostics:#?}"
        );

        let error = RecipeDocument::create_from_template(
            &template,
            &destination,
            "new.missing_artifact",
            Some(source.path().to_str().expect("utf8 source root")),
        )
        .expect_err("a missing destination App Artifact must block publication");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn template_creation_rejects_cross_app_artifact_provenance() {
        let source =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), true);
        let destination_root =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), false);
        let other_app = include_str!("../../../authored/apps/retroarch.yaml")
            .replacen("id: retroarch\n", "id: other_app\n", 1)
            .replacen(
                "package_id: com.retroarch.aarch64",
                "package_id: com.example.other",
                1,
            );
        fs::write(
            destination_root.path().join("apps/other_app.yaml"),
            other_app,
        )
        .expect("write second destination App Definition");
        let template_root = TempDir::new().expect("template directory");
        let template = template_root.path().join("cross-app.yaml");
        let source_recipe = include_str!("../../../authored/recipes/app.retroarch.provision.yaml");
        let recipe = source_recipe.replacen(
            "  type: install_apk\n  app_ref: retroarch",
            "  type: install_apk\n  app_ref: other_app",
            1,
        );
        assert_ne!(source_recipe, recipe);
        fs::write(&template, recipe).expect("write cross-App template");
        let destination = destination_root.path().join("recipes/from-template.yaml");

        let error = RecipeDocument::create_from_template(
            &template,
            &destination,
            "new.cross_app",
            Some(source.path().to_str().expect("utf8 source root")),
        )
        .expect_err("cross-App artifact provenance must block publication");

        assert!(error.contains("blocked"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn template_creation_into_another_catalog_is_blocked_by_destination_recovery() {
        let source =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), true);
        let destination_root =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), false);
        let destination = destination_root.path().join("recipes/from-template.yaml");
        let recovery = leave_unresolved(destination_root.path());

        let error = RecipeDocument::create_from_template(
            source.path().join("recipes/app.retroarch.provision.yaml"),
            &destination,
            "new.retroarch",
            Some(source.path().to_str().expect("utf8 source root")),
        )
        .expect_err("destination recovery must block template publication");

        assert!(error.to_lowercase().contains("recovery"), "{error}");
        assert!(!destination.exists());
        clear_test_recovery(&recovery);
    }

    #[test]
    fn template_creation_binds_the_destination_catalog_and_publishes_canonical_yaml() {
        let source =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), true);
        let destination_root =
            app_aware_catalog_root(include_str!("../../../authored/apps/retroarch.yaml"), false);
        let destination = destination_root.path().join("recipes/from-template.yaml");

        let document = RecipeDocument::create_from_template(
            source.path().join("recipes/app.retroarch.provision.yaml"),
            &destination,
            "new.retroarch",
            Some(source.path().to_str().expect("utf8 source root")),
        )
        .expect("a valid canonical template should publish");

        let expected_root = fs::canonicalize(destination_root.path()).unwrap();
        assert!(destination.is_file());
        assert_eq!(document.recipe.id, "new.retroarch");
        assert_eq!(document.authored_root(), Some(expected_root.as_path()));
        assert_eq!(
            fs::canonicalize(document.path()).unwrap(),
            fs::canonicalize(&destination).unwrap()
        );
        assert!(
            document
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic["severity"] != "error"),
            "published document must remain valid: {:#?}",
            document.diagnostics
        );
        let published = yaml::load_recipe_from_path(&destination).expect("published Recipe");
        assert_eq!(published.id, "new.retroarch");
        assert_eq!(published.steps.len(), document.recipe.steps.len());
        assert!(
            document.recipe.steps.iter().any(|step| {
                step.app_ref.is_some()
                    && step.verify.iter().any(|condition| {
                        condition
                            .params
                            .values()
                            .any(crate::app_definition_admission::contains_app_target_key)
                    })
            }),
            "the canonical fixture must exercise an app_ref with an App Target condition"
        );
        assert_eq!(
            published
                .steps
                .iter()
                .map(|step| (&step.skip_if, &step.verify))
                .collect::<Vec<_>>(),
            document
                .recipe
                .steps
                .iter()
                .map(|step| (&step.skip_if, &step.verify))
                .collect::<Vec<_>>(),
            "canonical App Target conditions must keep their shape after publication"
        );
    }
}
