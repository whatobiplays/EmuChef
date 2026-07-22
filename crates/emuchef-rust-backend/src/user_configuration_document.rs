//! Editable document state for persisted user configurations.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::user_configuration::{
    build_compatibility_baseline, compatibility_baseline_state, emit_user_configuration_yaml,
    load_user_configuration, prepare_configuration_for_v2_write, validate_binding_key,
    validate_configuration_identifier, validate_user_configuration_with_catalog,
    CompatibilityBaselineState, UserConfiguration, UserConfigurationLoadError,
};
use crate::yaml;

#[derive(Clone, Debug)]
pub struct UserConfigurationDocument {
    path: PathBuf,
    authored_root: Option<PathBuf>,
    configuration: UserConfiguration,
    saved_yaml: String,
    current_yaml: String,
    diagnostics: Vec<Value>,
}

impl UserConfigurationDocument {
    pub fn open(
        path: impl AsRef<Path>,
        authored_root: Option<&str>,
    ) -> Result<Self, UserConfigurationLoadError> {
        let input_path = path.as_ref();
        let configuration = load_user_configuration(input_path)?;
        let current_yaml = emit_user_configuration_yaml(&configuration)?;
        let path = PathBuf::from(yaml::resolved_path_string(input_path));
        let authored_root = authored_root.map(PathBuf::from);
        let diagnostics = diagnostics_for(&configuration, &path, authored_root.as_deref());
        Ok(Self {
            path,
            authored_root,
            configuration,
            saved_yaml: current_yaml.clone(),
            current_yaml,
            diagnostics,
        })
    }

    pub fn create(
        path: impl AsRef<Path>,
        id: &str,
        name: &str,
        device_plan: &str,
        selected_recipes: Vec<String>,
        bindings: IndexMap<String, Value>,
        authored_root: Option<&str>,
    ) -> Result<Self, UserConfigurationLoadError> {
        validate_configuration_identifier(id, "user-configuration id")?;
        if name.trim().is_empty() {
            return Err(structural_error(
                "User-configuration name must not be empty.",
            ));
        }
        if device_plan.trim().is_empty() {
            return Err(structural_error("Device plan must not be empty."));
        }
        for recipe_id in &selected_recipes {
            validate_configuration_identifier(recipe_id, "selected recipe id")?;
        }
        for key in bindings.keys() {
            validate_binding_key(key)?;
        }
        let path = path.as_ref().to_path_buf();
        let mut configuration = UserConfiguration {
            schema_version: 1,
            kind: "user_configuration".to_string(),
            id: id.to_string(),
            name: name.to_string(),
            device_plan: device_plan.to_string(),
            selected_recipes,
            bindings,
            compatibility: None,
            extensions: BTreeMap::new(),
            unsupported_extensions: Vec::new(),
        };
        if let Some(root) = authored_root.map(Path::new) {
            prepare_configuration_for_v2_write(&mut configuration, &path, root)?;
        }
        let current_yaml = emit_user_configuration_yaml(&configuration)?;
        atomic_write_new(&path, current_yaml.as_bytes())?;
        Self::open(path, authored_root)
    }

    pub fn set_binding(
        &mut self,
        key: &str,
        value: Option<Value>,
    ) -> Result<bool, UserConfigurationLoadError> {
        validate_binding_key(key)?;
        let changed = match value {
            Some(value) if self.configuration.bindings.get(key) != Some(&value) => {
                self.configuration.bindings.insert(key.to_string(), value);
                true
            }
            Some(_) => false,
            None => self.configuration.bindings.shift_remove(key).is_some(),
        };
        self.refresh()?;
        Ok(changed)
    }

    pub fn set_selected_recipes(
        &mut self,
        selected_recipes: Vec<String>,
    ) -> Result<bool, UserConfigurationLoadError> {
        for recipe_id in &selected_recipes {
            validate_configuration_identifier(recipe_id, "selected recipe id")?;
        }
        let changed = self.configuration.selected_recipes != selected_recipes;
        self.configuration.selected_recipes = selected_recipes;
        self.refresh()?;
        Ok(changed)
    }

    pub fn set_device_plan(
        &mut self,
        device_plan: String,
    ) -> Result<bool, UserConfigurationLoadError> {
        if device_plan.trim().is_empty() {
            return Err(structural_error("Device plan must not be empty."));
        }
        let changed = self.configuration.device_plan != device_plan;
        self.configuration.device_plan = device_plan;
        self.refresh()?;
        Ok(changed)
    }

    pub fn set_authored_root(&mut self, authored_root: Option<&str>) {
        self.authored_root = authored_root.map(PathBuf::from);
        self.validate();
    }

    pub fn validate(&mut self) {
        self.diagnostics = diagnostics_for(
            &self.configuration,
            &self.path,
            self.authored_root.as_deref(),
        );
    }

    pub fn save(&mut self) -> Result<(), UserConfigurationLoadError> {
        let mut proposed = self.configuration.clone();
        if let Some(root) = self.authored_root.as_deref() {
            prepare_configuration_for_v2_write(&mut proposed, &self.path, root)?;
        }
        let yaml = emit_user_configuration_yaml(&proposed)?;
        atomic_write_replace(&self.path, yaml.as_bytes())?;
        self.configuration = proposed;
        self.current_yaml = yaml.clone();
        self.saved_yaml = yaml;
        self.validate();
        Ok(())
    }

    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<(), UserConfigurationLoadError> {
        self.save_as_with_identity(path, None, None)
    }

    /// Save the current portable intent under a new path and optional identity.
    ///
    /// The destination is written before the live document is replaced, so a
    /// failed Save As never changes the current path, identity, or dirty state.
    pub fn save_as_with_identity(
        &mut self,
        path: impl AsRef<Path>,
        id: Option<&str>,
        name: Option<&str>,
    ) -> Result<(), UserConfigurationLoadError> {
        let path = path.as_ref().to_path_buf();
        let mut configuration = self.configuration.clone();
        if let Some(id) = id {
            validate_configuration_identifier(id, "user-configuration id")?;
            configuration.id = id.to_string();
        }
        if let Some(name) = name {
            if name.trim().is_empty() {
                return Err(structural_error(
                    "User-configuration name must not be empty.",
                ));
            }
            configuration.name = name.to_string();
        }
        if let Some(root) = self.authored_root.as_deref() {
            prepare_configuration_for_v2_write(&mut configuration, &path, root)?;
        }
        let yaml = emit_user_configuration_yaml(&configuration)?;
        atomic_write_new(&path, yaml.as_bytes())?;
        self.path = path;
        self.configuration = configuration;
        self.current_yaml = yaml.clone();
        self.saved_yaml = yaml;
        self.validate();
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn authored_root(&self) -> Option<&Path> {
        self.authored_root.as_deref()
    }

    pub fn configuration(&self) -> &UserConfiguration {
        &self.configuration
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

    fn refresh(&mut self) -> Result<(), UserConfigurationLoadError> {
        self.current_yaml = emit_user_configuration_yaml(&self.configuration)?;
        self.validate();
        Ok(())
    }
}

pub fn document_to_dto(document: &UserConfigurationDocument, document_id: &str) -> Value {
    let configuration = document.configuration();
    let baseline_state = document.compatibility_baseline_state();
    json!({
        "documentId": document_id,
        "path": document.path().to_string_lossy(),
        "authoredRoot": document.authored_root().map(|path| path.to_string_lossy().to_string()),
        "dirty": document.is_dirty(),
        "configuration": {
            "schemaVersion": configuration.schema_version,
            "kind": configuration.kind,
            "id": configuration.id,
            "name": configuration.name,
            "devicePlan": configuration.device_plan,
            "selectedRecipes": configuration.selected_recipes,
            "bindings": configuration.bindings,
            "extensions": configuration.extensions,
            "compatibility": configuration.compatibility.as_ref().map(compatibility_to_dto),
        },
        "compatibilityStatus": {
            "baselineState": baseline_state,
            "sourceSchemaVersion": configuration.schema_version,
            "pendingSanitationCount": configuration.unsupported_extensions.len(),
        },
        "yaml": document.yaml(),
        "diagnostics": document.diagnostics(),
    })
}

impl UserConfigurationDocument {
    fn compatibility_baseline_state(&self) -> &'static str {
        if self.configuration.compatibility.is_none() {
            return "pending_first_v2_save";
        }
        let Some(root) = self.authored_root.as_deref() else {
            return "unavailable";
        };
        let Ok(current) = build_compatibility_baseline(&self.configuration, &self.path, root)
        else {
            return "repair_required";
        };
        match compatibility_baseline_state(&self.configuration, &current) {
            CompatibilityBaselineState::PendingFirstV2Save => "pending_first_v2_save",
            CompatibilityBaselineState::Unchanged => "unchanged",
            CompatibilityBaselineState::MateriallyChanged => "materially_changed",
        }
    }
}

fn compatibility_to_dto(
    compatibility: &crate::user_configuration::SavedCompatibilityBaseline,
) -> Value {
    json!({
        "devicePlan": {
            "id": compatibility.device_plan.id,
            "label": compatibility.device_plan.label,
            "fingerprint": compatibility.device_plan.fingerprint,
        },
        "recipes": compatibility.recipes.iter().map(|recipe| json!({
            "id": recipe.id,
            "label": recipe.label,
            "selected": recipe.selected,
            "fingerprint": recipe.fingerprint,
            "inputs": recipe.inputs.iter().map(|input| json!({
                "key": input.key,
                "label": input.label,
                "fingerprint": input.fingerprint,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn atomic_write_new(path: &Path, bytes: &[u8]) -> Result<(), UserConfigurationLoadError> {
    let parent = path
        .parent()
        .ok_or_else(|| io_error("Destination directory is unavailable."))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        io_error(format!(
            "Temporary configuration file could not be created: {error}"
        ))
    })?;
    write_and_sync(&mut temporary, bytes)?;
    temporary.persist_noclobber(path).map_err(|error| {
        io_error(format!(
            "Destination already exists or could not be activated safely: {}",
            error.error
        ))
    })?;
    sync_parent(parent)?;
    Ok(())
}

fn atomic_write_replace(path: &Path, bytes: &[u8]) -> Result<(), UserConfigurationLoadError> {
    let parent = path
        .parent()
        .ok_or_else(|| io_error("Destination directory is unavailable."))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        io_error(format!(
            "Temporary configuration file could not be created: {error}"
        ))
    })?;
    write_and_sync(&mut temporary, bytes)?;
    temporary.persist(path).map_err(|error| {
        io_error(format!(
            "Configuration file could not be replaced safely: {}",
            error.error
        ))
    })?;
    sync_parent(parent)?;
    Ok(())
}

fn write_and_sync(
    temporary: &mut tempfile::NamedTempFile,
    bytes: &[u8],
) -> Result<(), UserConfigurationLoadError> {
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.flush())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| {
            io_error(format!(
                "Configuration data could not be synchronized: {error}"
            ))
        })
}

fn sync_parent(parent: &Path) -> Result<(), UserConfigurationLoadError> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            io_error(format!(
                "Configuration directory could not be synchronized: {error}"
            ))
        })
}

fn io_error(message: impl Into<String>) -> UserConfigurationLoadError {
    UserConfigurationLoadError {
        kind: crate::user_configuration::UserConfigurationLoadErrorKind::Io,
        code: "user_configuration_io",
        message: message.into(),
    }
}

fn diagnostics_for(
    configuration: &UserConfiguration,
    path: &Path,
    authored_root: Option<&Path>,
) -> Vec<Value> {
    authored_root.map_or_else(Vec::new, |root| {
        validate_user_configuration_with_catalog(configuration, path, root)
    })
}

fn structural_error(message: &str) -> UserConfigurationLoadError {
    UserConfigurationLoadError {
        kind: crate::user_configuration::UserConfigurationLoadErrorKind::Structural,
        code: "user_configuration_structural_invalid",
        message: message.to_string(),
    }
}
