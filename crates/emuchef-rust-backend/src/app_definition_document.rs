//! Typed App Definition drafts and their independent editor history.
//!
//! This module keeps document editing separate from catalog admission. Parsing
//! establishes the schema-v1 shape; semantic diagnostics describe repairable
//! fields, while each command validates only the fields it changes before it
//! enters the history stack.

use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app_definition_admission::{
    admit_app_definition_save, AppDefinitionCatalogIdentity, AppDefinitionSaveContext,
    AppDefinitionSaveDestination,
};
use crate::app_definition_persistence::{
    capture_opened_app_definition_file, OpenedAppDefinitionFile,
};
use crate::authored_models::{
    self, AppArtifactV1, AppDefinitionV1, AppPermissionSet, AppPermissionSets, AppTargetV1,
    AuthoredModelDiagnostic,
};
use crate::catalog;

#[derive(Clone, Debug)]
pub struct AppDefinitionDocument {
    path: PathBuf,
    authored_root: Option<PathBuf>,
    catalog_identity: Option<CatalogIdentity>,
    app_definition: AppDefinitionV1,
    current_yaml: String,
    saved_yaml: String,
    opened_file: OpenedAppDefinitionFile,
    diagnostics: Vec<Value>,
    undo_stack: Vec<ContentSnapshot>,
    redo_stack: Vec<ContentSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogIdentity {
    pub id: String,
    pub package_id: String,
    pub path: PathBuf,
    pub authored_root: PathBuf,
}

/// A rejected typed command and any diagnostics produced for its candidate.
#[derive(Clone, Debug)]
pub struct AppDefinitionCommandError {
    pub message: String,
    pub diagnostics: Option<Vec<Value>>,
}

/// An App Definition load failure, with structured diagnostics for an
/// incompatible schema header when one can be identified safely.
#[derive(Clone, Debug)]
pub struct AppDefinitionOpenError {
    pub message: String,
    pub diagnostics: Vec<Value>,
}

impl AppDefinitionOpenError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            diagnostics: Vec::new(),
        }
    }
}

impl From<String> for AppDefinitionOpenError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl fmt::Display for AppDefinitionOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<String> for AppDefinitionCommandError {
    fn from(message: String) -> Self {
        Self {
            message,
            diagnostics: None,
        }
    }
}

#[derive(Clone, Debug)]
struct ContentSnapshot {
    app_definition: AppDefinitionV1,
    current_yaml: String,
    diagnostics: Vec<Value>,
}

/// Commands exposed to the Config Editor for schema-v1 App Definition drafts.
///
/// Registry entry identifiers are command fields rather than mutable fields
/// inside the nested values. This prevents a replace operation from acting as
/// an implicit rename of a referenced artifact or target.
#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "PascalCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AppDefinitionCommand {
    SetId {
        id: String,
    },
    SetName {
        name: String,
    },
    SetDescription {
        description: Option<String>,
    },
    SetCategory {
        category: Option<String>,
    },
    SetPackageId {
        package_id: String,
    },
    SetLauncherActivity {
        launcher_activity: Option<String>,
    },
    AddArtifact {
        artifact_id: String,
        artifact: AppArtifactV1,
    },
    ReplaceArtifact {
        artifact_id: String,
        artifact: AppArtifactV1,
    },
    RemoveArtifact {
        artifact_id: String,
    },
    ReorderArtifact {
        artifact_id: String,
        to_index: usize,
    },
    SetPermissionSet {
        permission_set: PermissionSetName,
        value: Option<AppPermissionSet>,
    },
    AddTarget {
        target_id: String,
        target: AppTargetV1,
    },
    ReplaceTarget {
        target_id: String,
        target: AppTargetV1,
    },
    RemoveTarget {
        target_id: String,
    },
    ReorderTarget {
        target_id: String,
        to_index: usize,
    },
    SetMetadataEntry {
        key: String,
        value: Value,
    },
    RemoveMetadataEntry {
        key: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionSetName {
    Baseline,
    Elevated,
}

enum ValidationScope {
    None,
    Root(&'static str),
    Artifact(String),
    Target(String),
    PermissionSet(PermissionSetName),
    Metadata(String),
}

impl AppDefinitionDocument {
    pub fn open(
        path: impl AsRef<Path>,
        authored_root: Option<&str>,
    ) -> Result<Self, AppDefinitionOpenError> {
        let requested_path = path.as_ref();
        reject_symlinked_catalog_membership(requested_path, authored_root)?;
        let opened_file = capture_opened_app_definition_file(requested_path).map_err(|error| {
            AppDefinitionOpenError::new(format!(
                "App Definition could not be opened ({}): {}",
                error.code, error.message
            ))
        })?;
        let path = opened_file.path().to_path_buf();
        let source = std::str::from_utf8(opened_file.raw_bytes())
            .map_err(|_| AppDefinitionOpenError::new("App Definition must be UTF-8 YAML."))?;
        if let Ok(raw_document) = serde_yaml::from_str::<serde_yaml::Value>(source) {
            let incompatible_header = incompatible_header_diagnostics(&raw_document, &path);
            if !incompatible_header.is_empty() {
                return Err(AppDefinitionOpenError {
                    message: "This editor opens only App Definitions with kind 'app_definition' and schema_version 1.".to_string(),
                    diagnostics: incompatible_header,
                });
            }
        }
        let app_definition =
            authored_models::parse_app_definition_yaml(source).map_err(|error| {
                AppDefinitionOpenError::new(format!("App Definition could not be parsed: {error}"))
            })?;
        let physical_catalog_root = catalog_root_for_app_path(&path);
        let authored_root = authored_root
            .and_then(|root| catalog::normalize_authored_root(Some(root), &path))
            .or_else(|| physical_catalog_root.clone());
        let catalog_identity = physical_catalog_root.map(|authored_root| CatalogIdentity {
            id: app_definition.id.clone(),
            package_id: app_definition.package_id.clone(),
            path: path.clone(),
            authored_root,
        });
        let current_yaml = emit_app_definition_draft_yaml(&app_definition)?;
        let diagnostics = app_definition_diagnostics(&app_definition);

        Ok(Self {
            path,
            authored_root,
            catalog_identity,
            app_definition,
            saved_yaml: current_yaml.clone(),
            current_yaml,
            opened_file,
            diagnostics,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    pub fn apply_command(&mut self, command_value: &Value) -> Result<bool, String> {
        self.apply_command_with_diagnostics(command_value)
            .map_err(|error| error.message)
    }

    pub fn apply_command_with_diagnostics(
        &mut self,
        command_value: &Value,
    ) -> Result<bool, AppDefinitionCommandError> {
        let command = decode_app_definition_command(command_value)?;
        if self.catalog_identity.is_some()
            && matches!(
                command,
                AppDefinitionCommand::SetId { .. } | AppDefinitionCommand::SetPackageId { .. }
            )
        {
            return Err("Catalog App Definition identity is fixed; create a new App to change its id or package."
                .to_string()
                .into());
        }

        let mut candidate = self.app_definition.clone();
        let deletion = match &command {
            AppDefinitionCommand::RemoveArtifact { artifact_id } => {
                Some(DeletedEntry::Artifact(artifact_id.clone()))
            }
            AppDefinitionCommand::RemoveTarget { target_id } => {
                Some(DeletedEntry::Target(target_id.clone()))
            }
            _ => None,
        };
        let validation_scope = apply_app_definition_command(&mut candidate, command)?;
        let candidate_yaml = emit_app_definition_draft_yaml(&candidate)?;
        if candidate_yaml == self.current_yaml {
            return Ok(false);
        }

        let candidate_diagnostics = app_definition_diagnostics(&candidate);
        validate_changed_field(&candidate, &validation_scope)?;

        if self.catalog_identity.is_some() {
            if let Some(deletion) = deletion {
                let admission = admit_app_definition_save(
                    &candidate,
                    &self.save_context(AppDefinitionSaveDestination::OpenedFile),
                );
                let candidate_diagnostics = admission.diagnostics_as_values();
                if admission.snapshot.is_none() {
                    return Err(AppDefinitionCommandError {
                        message: "App Definition references could not be classified reliably, so this deletion was rejected. Review the document diagnostics and repair the catalog inputs before retrying.".to_string(),
                        diagnostics: Some(candidate_diagnostics),
                    });
                }
                let referenced = match &deletion {
                    DeletedEntry::Artifact(id) => admission.has_dependent_artifact_reference(id),
                    DeletedEntry::Target(id) => admission.has_dependent_target_reference(id),
                };
                if referenced {
                    return Err(AppDefinitionCommandError {
                        message: format!(
                            "Cannot remove this referenced {}; repair the affected Recipe references first. See the rejection diagnostics for their locations.",
                            deletion.kind_name(),
                        ),
                        diagnostics: Some(candidate_diagnostics),
                    });
                }
            }
        }

        let previous = self.content_snapshot();
        self.app_definition = candidate;
        self.current_yaml = candidate_yaml;
        self.diagnostics = candidate_diagnostics;
        self.undo_stack.push(previous);
        self.redo_stack.clear();
        Ok(true)
    }

    pub fn undo(&mut self) -> bool {
        while let Some(previous) = self.undo_stack.pop() {
            if self
                .catalog_identity
                .as_ref()
                .is_some_and(|identity| !snapshot_matches_catalog_identity(&previous, identity))
            {
                continue;
            }
            let current = self.content_snapshot();
            self.restore_content(previous);
            self.redo_stack.push(current);
            return true;
        }
        false
    }

    pub fn redo(&mut self) -> bool {
        while let Some(next) = self.redo_stack.pop() {
            if self
                .catalog_identity
                .as_ref()
                .is_some_and(|identity| !snapshot_matches_catalog_identity(&next, identity))
            {
                continue;
            }
            let current = self.content_snapshot();
            self.restore_content(next);
            self.undo_stack.push(current);
            return true;
        }
        false
    }

    pub fn validate(&mut self) {
        self.diagnostics = app_definition_diagnostics(&self.app_definition);
    }

    pub fn set_authored_root(&mut self, authored_root: Option<&str>) {
        self.authored_root =
            authored_root.and_then(|root| catalog::normalize_authored_root(Some(root), &self.path));
        self.validate();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn authored_root(&self) -> Option<&Path> {
        self.authored_root.as_deref()
    }

    /// Return the catalog that owns this document's persisted identity.
    /// Catalog selection is independent of ownership; standalone documents
    /// acquire an owner only after successful canonical Save As publication.
    pub fn owning_authored_root(&self) -> Option<&Path> {
        self.catalog_identity
            .as_ref()
            .map(|identity| identity.authored_root.as_path())
    }

    /// Report whether a standalone document has a usable publication root.
    /// This is a UI availability signal, not Save admission: destination,
    /// identity, dependent Recipe, and no-clobber checks still run at Save As.
    pub fn can_save_as(&self) -> bool {
        self.catalog_identity.is_none()
            && self.authored_root.as_deref().is_some_and(|root| {
                root.is_absolute()
                    && fs::canonicalize(root).is_ok_and(|canonical| canonical == root)
                    && ensure_real_catalog_directory(root, "authored root")
                        .is_ok_and(|exists| exists)
                    && ensure_real_catalog_directory(&root.join("apps"), "apps directory")
                        .is_ok_and(|exists| exists)
                    && ensure_real_catalog_directory(&root.join("recipes"), "recipes directory")
                        .is_ok_and(|exists| exists)
            })
    }

    pub fn catalog_identity(&self) -> Option<&CatalogIdentity> {
        self.catalog_identity.as_ref()
    }

    pub fn app_definition(&self) -> &AppDefinitionV1 {
        &self.app_definition
    }

    pub fn yaml(&self) -> &str {
        &self.current_yaml
    }

    pub fn diagnostics(&self) -> &[Value] {
        &self.diagnostics
    }

    pub(crate) fn set_diagnostics(&mut self, diagnostics: Vec<Value>) {
        self.diagnostics = diagnostics;
    }

    pub(crate) fn save_context(
        &self,
        destination: AppDefinitionSaveDestination,
    ) -> AppDefinitionSaveContext {
        AppDefinitionSaveContext {
            opened_file: self.opened_file.clone(),
            authored_root: self
                .catalog_identity
                .as_ref()
                .map(|identity| identity.authored_root.clone())
                .or_else(|| self.authored_root.clone()),
            opened_catalog_identity: self.catalog_identity.as_ref().map(|identity| {
                AppDefinitionCatalogIdentity {
                    id: identity.id.clone(),
                    package_id: identity.package_id.clone(),
                }
            }),
            destination,
        }
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

    /// Original disk bytes are retained for stale-state admission at save time.
    pub fn opened_bytes(&self) -> &[u8] {
        self.opened_file.raw_bytes()
    }

    pub fn opened_file(&self) -> &OpenedAppDefinitionFile {
        &self.opened_file
    }

    /// Called only after the save-admission and atomic-publication layer succeeds.
    pub fn mark_saved(&mut self, opened_file: OpenedAppDefinitionFile, yaml: String) {
        self.path = opened_file.path().to_path_buf();
        self.opened_file = opened_file;
        self.current_yaml = yaml.clone();
        self.saved_yaml = yaml;
        self.validate();
    }

    pub fn set_saved_path(&mut self, path: PathBuf, authored_root: PathBuf) {
        self.path = path;
        self.authored_root = Some(authored_root.clone());
        self.catalog_identity = Some(CatalogIdentity {
            id: self.app_definition.id.clone(),
            package_id: self.app_definition.package_id.clone(),
            path: self.path.clone(),
            authored_root,
        });
        let identity = self
            .catalog_identity
            .as_ref()
            .expect("Save As establishes a catalog App identity");
        self.undo_stack
            .retain(|snapshot| snapshot_matches_catalog_identity(snapshot, identity));
        self.redo_stack
            .retain(|snapshot| snapshot_matches_catalog_identity(snapshot, identity));
    }

    fn content_snapshot(&self) -> ContentSnapshot {
        ContentSnapshot {
            app_definition: self.app_definition.clone(),
            current_yaml: self.current_yaml.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    fn restore_content(&mut self, snapshot: ContentSnapshot) {
        self.app_definition = snapshot.app_definition;
        self.current_yaml = snapshot.current_yaml;
        self.diagnostics = snapshot.diagnostics;
    }
}

fn snapshot_matches_catalog_identity(
    snapshot: &ContentSnapshot,
    identity: &CatalogIdentity,
) -> bool {
    snapshot.app_definition.id == identity.id
        && snapshot.app_definition.package_id == identity.package_id
}

#[derive(Clone, Debug)]
enum DeletedEntry {
    Artifact(String),
    Target(String),
}

impl DeletedEntry {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Artifact(_) => "App Artifact",
            Self::Target(_) => "App Target",
        }
    }
}

fn decode_app_definition_command(value: &Value) -> Result<AppDefinitionCommand, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "App Definition command must be an object.".to_string())?;
    let command_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "App Definition command must include a string 'type'.".to_string())?;
    let required_fields: &[&str] = match command_type {
        "SetId" => &["id"],
        "SetName" => &["name"],
        "SetDescription" => &["description"],
        "SetCategory" => &["category"],
        "SetPackageId" => &["packageId"],
        "SetLauncherActivity" => &["launcherActivity"],
        "AddArtifact" | "ReplaceArtifact" => &["artifactId", "artifact"],
        "RemoveArtifact" => &["artifactId"],
        "ReorderArtifact" => &["artifactId", "toIndex"],
        "SetPermissionSet" => &["permissionSet", "value"],
        "AddTarget" | "ReplaceTarget" => &["targetId", "target"],
        "RemoveTarget" => &["targetId"],
        "ReorderTarget" => &["targetId", "toIndex"],
        "SetMetadataEntry" => &["key", "value"],
        "RemoveMetadataEntry" => &["key"],
        _ => {
            return Err(format!(
                "Unknown App Definition command type: {command_type}"
            ))
        }
    };
    if let Some(field) = required_fields
        .iter()
        .find(|field| !object.contains_key(**field))
    {
        return Err(format!(
            "App Definition command {command_type} is missing required field {field:?}."
        ));
    }
    serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid App Definition command: {error}"))
}

/// Serialize a structurally typed draft without applying strict Save admission.
pub fn emit_app_definition_draft_yaml(value: &AppDefinitionV1) -> Result<String, String> {
    serde_yaml::to_string(value)
        .map_err(|error| format!("App Definition draft could not be serialized: {error}"))
}

pub fn app_definition_diagnostics(value: &AppDefinitionV1) -> Vec<Value> {
    authored_models::validate_app_definition(value)
        .into_iter()
        .map(diagnostic_to_value)
        .collect()
}

fn reject_symlinked_catalog_membership(
    requested_path: &Path,
    authored_root: Option<&str>,
) -> Result<(), AppDefinitionOpenError> {
    let mut roots = Vec::new();
    if let Some(authored_root) = authored_root {
        roots.push(PathBuf::from(authored_root));
    }
    let mut prefix = PathBuf::new();
    for component in requested_path.components() {
        if matches!(component, Component::Normal(name) if name.to_str() == Some("apps")) {
            roots.push(if prefix.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                prefix.clone()
            });
        }
        prefix.push(component.as_os_str());
    }
    roots.sort();
    roots.dedup();

    for root in roots {
        if !ensure_real_catalog_directory(&root, "authored root")? {
            continue;
        }
        ensure_real_catalog_directory(&root.join("apps"), "apps directory")?;
    }
    Ok(())
}

fn ensure_real_catalog_directory(path: &Path, label: &str) -> Result<bool, AppDefinitionOpenError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(catalog_path_open_error(
            path,
            format!(
                "The authored {label} at '{}' is a symbolic link; catalog paths must use real directories.",
                path.display()
            ),
        )),
        Ok(metadata) if !metadata.is_dir() => Err(catalog_path_open_error(
            path,
            format!(
                "The authored {label} at '{}' is not a directory.",
                path.display()
            ),
        )),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(catalog_path_open_error(
            path,
            format!(
                "The authored {label} at '{}' could not be inspected: {error}.",
                path.display()
            ),
        )),
    }
}

fn catalog_path_open_error(path: &Path, message: String) -> AppDefinitionOpenError {
    AppDefinitionOpenError {
        diagnostics: vec![json!({
            "severity": "error",
            "code": "app_catalog_path_invalid",
            "message": message,
            "file": path.to_string_lossy(),
            "object_kind": authored_models::APP_DEFINITION_KIND,
            "object_id": null,
            "field": "path",
        })],
        message: "App Definition catalog membership could not be established safely.".to_string(),
    }
}

fn incompatible_header_diagnostics(raw: &serde_yaml::Value, path: &Path) -> Vec<Value> {
    let Some(mapping) = raw.as_mapping() else {
        return Vec::new();
    };
    let header_value = |field: &str| mapping.get(&serde_yaml::Value::String(field.to_string()));
    let kind = header_value("kind").and_then(serde_yaml::Value::as_str);
    let schema_version = header_value("schema_version").and_then(serde_yaml::Value::as_i64);
    let object_id = header_value("id").and_then(serde_yaml::Value::as_str);
    let mut diagnostics = Vec::new();

    if let Some(version) =
        schema_version.filter(|version| *version != authored_models::SCHEMA_VERSION_V1)
    {
        diagnostics.push(json!({
            "severity": "error",
            "code": "schema_version_unsupported",
            "message": format!(
                "Unsupported App Definition schema_version '{version}'; this editor supports version 1 only."
            ),
            "file": path.to_string_lossy(),
            "object_kind": authored_models::APP_DEFINITION_KIND,
            "object_id": object_id,
            "field": "schema_version",
        }));
    }
    if let Some(kind) = kind.filter(|kind| *kind != authored_models::APP_DEFINITION_KIND) {
        diagnostics.push(json!({
            "severity": "error",
            "code": "authored_kind_invalid",
            "message": format!(
                "Expected kind 'app_definition' but found '{kind}'. Open the file with its matching document editor."
            ),
            "file": path.to_string_lossy(),
            "object_kind": authored_models::APP_DEFINITION_KIND,
            "object_id": object_id,
            "field": "kind",
        }));
    }
    diagnostics
}

fn diagnostic_to_value(diagnostic: AuthoredModelDiagnostic) -> Value {
    json!({
        "code": diagnostic.code,
        "message": diagnostic.message,
        "field": diagnostic.field,
    })
}

fn apply_app_definition_command(
    app: &mut AppDefinitionV1,
    command: AppDefinitionCommand,
) -> Result<ValidationScope, String> {
    match command {
        AppDefinitionCommand::SetId { id } => {
            app.id = id;
            Ok(ValidationScope::Root("id"))
        }
        AppDefinitionCommand::SetName { name } => {
            app.name = name;
            Ok(ValidationScope::Root("name"))
        }
        AppDefinitionCommand::SetDescription { description } => {
            app.description = description;
            Ok(ValidationScope::Root("description"))
        }
        AppDefinitionCommand::SetCategory { category } => {
            app.category = category;
            Ok(ValidationScope::Root("category"))
        }
        AppDefinitionCommand::SetPackageId { package_id } => {
            app.package_id = package_id;
            Ok(ValidationScope::Root("package_id"))
        }
        AppDefinitionCommand::SetLauncherActivity { launcher_activity } => {
            app.launcher_activity = launcher_activity;
            Ok(ValidationScope::Root("launcher_activity"))
        }
        AppDefinitionCommand::AddArtifact {
            artifact_id,
            artifact,
        } => {
            if app.artifacts.contains_key(&artifact_id) {
                return Err(format!("Artifact {artifact_id:?} already exists."));
            }
            app.artifacts.insert(artifact_id.clone(), artifact);
            Ok(ValidationScope::Artifact(artifact_id))
        }
        AppDefinitionCommand::ReplaceArtifact {
            artifact_id,
            artifact,
        } => {
            let slot = app.artifacts.get_mut(&artifact_id).ok_or_else(|| {
                format!("Artifact {artifact_id:?} does not exist and cannot be replaced.")
            })?;
            *slot = artifact;
            Ok(ValidationScope::Artifact(artifact_id))
        }
        AppDefinitionCommand::RemoveArtifact { artifact_id } => {
            if app.artifacts.shift_remove(&artifact_id).is_none() {
                return Err(format!("Artifact {artifact_id:?} does not exist."));
            }
            Ok(ValidationScope::None)
        }
        AppDefinitionCommand::ReorderArtifact {
            artifact_id,
            to_index,
        } => {
            reorder_registry(&mut app.artifacts, &artifact_id, to_index, "artifact")?;
            Ok(ValidationScope::None)
        }
        AppDefinitionCommand::SetPermissionSet {
            permission_set,
            value,
        } => {
            let sets = app
                .permission_sets
                .get_or_insert_with(AppPermissionSets::default);
            match permission_set {
                PermissionSetName::Baseline => {
                    sets.baseline = value;
                }
                PermissionSetName::Elevated => {
                    sets.elevated = value;
                }
            }
            if sets.baseline.is_none() && sets.elevated.is_none() {
                app.permission_sets = None;
            }
            Ok(ValidationScope::PermissionSet(permission_set))
        }
        AppDefinitionCommand::AddTarget { target_id, target } => {
            if app.targets.contains_key(&target_id) {
                return Err(format!("Target {target_id:?} already exists."));
            }
            app.targets.insert(target_id.clone(), target);
            Ok(ValidationScope::Target(target_id))
        }
        AppDefinitionCommand::ReplaceTarget { target_id, target } => {
            let slot = app.targets.get_mut(&target_id).ok_or_else(|| {
                format!("Target {target_id:?} does not exist and cannot be replaced.")
            })?;
            *slot = target;
            Ok(ValidationScope::Target(target_id))
        }
        AppDefinitionCommand::RemoveTarget { target_id } => {
            if app.targets.shift_remove(&target_id).is_none() {
                return Err(format!("Target {target_id:?} does not exist."));
            }
            Ok(ValidationScope::None)
        }
        AppDefinitionCommand::ReorderTarget {
            target_id,
            to_index,
        } => {
            reorder_registry(&mut app.targets, &target_id, to_index, "target")?;
            Ok(ValidationScope::None)
        }
        AppDefinitionCommand::SetMetadataEntry { key, value } => {
            app.metadata.insert(key.clone(), value);
            Ok(ValidationScope::Metadata(key))
        }
        AppDefinitionCommand::RemoveMetadataEntry { key } => {
            if app.metadata.shift_remove(&key).is_none() {
                return Err(format!("Metadata entry {key:?} does not exist."));
            }
            Ok(ValidationScope::None)
        }
    }
}

fn reorder_registry<T>(
    registry: &mut IndexMap<String, T>,
    id: &str,
    to_index: usize,
    kind: &str,
) -> Result<(), String> {
    let Some(from_index) = registry.get_index_of(id) else {
        return Err(format!("{} {id:?} does not exist.", title_case(kind)));
    };
    if to_index >= registry.len() {
        return Err(format!(
            "Destination index for {kind} {id:?} must be less than {}.",
            registry.len()
        ));
    }
    if from_index != to_index {
        let (key, value) = registry
            .shift_remove_entry(id)
            .ok_or_else(|| format!("{} {id:?} does not exist.", title_case(kind)))?;
        registry.shift_insert(to_index, key, value);
    }
    Ok(())
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn validate_changed_field(app: &AppDefinitionV1, scope: &ValidationScope) -> Result<(), String> {
    match scope {
        ValidationScope::None => Ok(()),
        ValidationScope::Root(field) => {
            let diagnostic = authored_models::validate_app_definition(app)
                .into_iter()
                .find(|diagnostic| diagnostic.field == *field);
            diagnostic.map_or(Ok(()), |diagnostic| Err(diagnostic.message))
        }
        ValidationScope::Artifact(artifact_id) => {
            let Some(artifact) = app.artifacts.get(artifact_id) else {
                return Ok(());
            };
            let mut probe = app.clone();
            probe.artifacts = IndexMap::from([(artifact_id.clone(), artifact.clone())]);
            reject_entry_diagnostics(
                authored_models::validate_app_definition(&probe),
                &format!("artifacts[{artifact_id}]"),
            )
        }
        ValidationScope::Target(target_id) => {
            let Some(target) = app.targets.get(target_id) else {
                return Ok(());
            };
            let mut probe = app.clone();
            probe.targets = IndexMap::from([(target_id.clone(), target.clone())]);
            reject_entry_diagnostics(
                authored_models::validate_app_definition(&probe),
                &format!("targets[{target_id}]"),
            )
        }
        ValidationScope::PermissionSet(permission_set) => {
            validate_permission_set_change(app, *permission_set)
        }
        ValidationScope::Metadata(key) => {
            let Some(value) = app.metadata.get(key) else {
                return Ok(());
            };
            let mut probe = app.clone();
            probe.metadata.clear();
            probe.metadata.insert(key.clone(), value.clone());
            let diagnostic = authored_models::validate_app_definition(&probe)
                .into_iter()
                .find(|diagnostic| diagnostic.field == "metadata");
            diagnostic.map_or(Ok(()), |diagnostic| Err(diagnostic.message))
        }
    }
}

fn reject_entry_diagnostics(
    diagnostics: Vec<AuthoredModelDiagnostic>,
    field_prefix: &str,
) -> Result<(), String> {
    diagnostics
        .into_iter()
        .find(|diagnostic| {
            diagnostic.field == field_prefix
                || diagnostic
                    .field
                    .strip_prefix(field_prefix)
                    .is_some_and(|suffix| suffix.starts_with('.'))
        })
        .map_or(Ok(()), |diagnostic| Err(diagnostic.message))
}

fn validate_permission_set_change(
    app: &AppDefinitionV1,
    edited_set: PermissionSetName,
) -> Result<(), String> {
    let Some(sets) = app.permission_sets.as_ref() else {
        return Ok(());
    };
    let (selected, other) = match edited_set {
        PermissionSetName::Baseline => (sets.baseline.as_ref(), sets.elevated.as_ref()),
        PermissionSetName::Elevated => (sets.elevated.as_ref(), sets.baseline.as_ref()),
    };
    let Some(selected) = selected else {
        return Ok(());
    };

    let mut probe = app.clone();
    probe.permission_sets = Some(AppPermissionSets {
        baseline: Some(selected.clone()),
        elevated: None,
    });
    let diagnostic = authored_models::validate_app_definition(&probe)
        .into_iter()
        .find(|diagnostic| diagnostic.field.starts_with("permission_sets.baseline"));
    if let Some(diagnostic) = diagnostic {
        return Err(diagnostic.message);
    }

    if other.is_some_and(|other| {
        permission_action_names(selected).iter().any(|name| {
            permission_action_names(other)
                .iter()
                .any(|other_name| name == other_name)
        })
    }) {
        return Err(
            "Permission automation actions must be unique within and across permission sets."
                .to_string(),
        );
    }
    Ok(())
}

fn permission_action_names(set: &AppPermissionSet) -> Vec<String> {
    set.runtime
        .iter()
        .map(|action| format!("runtime:{}", action.permission))
        .chain(
            set.app_ops
                .iter()
                .map(|action| format!("app_op:{}", action.op)),
        )
        .collect()
}

fn catalog_root_for_app_path(path: &Path) -> Option<PathBuf> {
    let apps = path.parent()?;
    if apps.file_name().and_then(|name| name.to_str()) != Some("apps") {
        return None;
    }
    let root = fs::canonicalize(apps.parent()?).ok()?;
    is_catalog_app_path(&root, path).then_some(root)
}

fn is_catalog_app_path(root: &Path, path: &Path) -> bool {
    let apps = root.join("apps");
    let Ok(canonical_apps) = fs::canonicalize(apps) else {
        return false;
    };
    let Some(parent) = path.parent() else {
        return false;
    };
    parent == canonical_apps
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "yaml" | "yml"))
}
