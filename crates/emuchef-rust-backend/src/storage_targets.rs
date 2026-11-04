//! Trusted setup-local storage target rules shared by Recipe planning and copy review.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_SETUP_LOCATIONS: usize = 32;
pub const MAX_SETUP_LOCATION_NAME_BYTES: usize = 128;
pub const MAX_RELATIVE_LOCATION_PATH_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageTargetVolumeKind {
    InternalShared,
    RemovableSd,
}

/// Tauri-resolved evidence for one session-local Setup Location.
///
/// This value crosses only the trusted Rust/Tauri sidecar boundary. The UI
/// receives the location ID and sanitized presentation fields, never this
/// physical binding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageTargetBinding {
    pub id: String,
    pub device_plan: String,
    pub session_epoch: u64,
    pub name: String,
    pub relative_path: String,
    pub volume_handle: String,
    pub volume_kind: StorageTargetVolumeKind,
    pub mount_root: String,
    pub volume_identity: String,
    pub volume_continuity: String,
    pub observation_token: String,
}

impl StorageTargetBinding {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() || self.id.len() > 128 || self.id.chars().any(char::is_control) {
            return Err("setup location identity is invalid".to_string());
        }
        if self.device_plan.is_empty() || self.device_plan.chars().any(char::is_control) {
            return Err("setup location setup identity is invalid".to_string());
        }
        validate_setup_location_name(&self.name)?;
        validate_relative_location_path(&self.relative_path)?;
        validate_volume_root(self.volume_kind, &self.mount_root)?;
        for token in [
            &self.volume_handle,
            &self.volume_identity,
            &self.volume_continuity,
            &self.observation_token,
        ] {
            if !is_opaque_token(token) {
                return Err("setup location volume evidence is invalid".to_string());
            }
        }
        Ok(())
    }

    pub fn destination_path(&self) -> Result<PathBuf, String> {
        self.validate()?;
        let mut destination = PathBuf::from(&self.mount_root);
        for component in self.relative_path.split('/') {
            destination.push(component);
        }
        Ok(destination)
    }
}

/// Exact path state reviewed before a Setup Location copy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SetupDestinationPrecondition {
    /// Canonical destination scope expected after any reviewed creation.
    pub(crate) canonical_scope: String,
    /// Existing directory whose identity anchors the reviewed path.
    pub(crate) existing_anchor_path: String,
    pub(crate) existing_anchor_binding: String,
    /// Missing path components created one at a time after revalidation.
    pub(crate) missing_components: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WriterEffect {
    EnsureDirectory(PathBuf),
    WriteFile(PathBuf),
    DeleteFile(PathBuf),
    DeleteDirectory(PathBuf),
}

pub fn validate_setup_location_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty()
        || name.len() > MAX_SETUP_LOCATION_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        return Err(
            "setup location name must be non-empty, printable, and at most 128 bytes".into(),
        );
    }
    Ok(name.to_string())
}

pub fn validate_relative_location_path(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > MAX_RELATIVE_LOCATION_PATH_BYTES
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return Err(
            "setup location path must be a bounded relative path with unambiguous separators"
                .into(),
        );
    }
    for component in value.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with('.')
            || component.ends_with(' ')
        {
            return Err("setup location path contains an invalid component".into());
        }
    }
    Ok(value.to_string())
}

pub fn validate_volume_root(kind: StorageTargetVolumeKind, root: &str) -> Result<&str, String> {
    match kind {
        StorageTargetVolumeKind::InternalShared if root == "/storage/emulated/0" => Ok(root),
        StorageTargetVolumeKind::InternalShared => {
            Err("internal storage root is not the reported system-user emulated root".into())
        }
        StorageTargetVolumeKind::RemovableSd => {
            let Some(component) = root.strip_prefix("/storage/") else {
                return Err("removable storage root is not app-visible shared storage".into());
            };
            if component.is_empty()
                || component.contains('/')
                || component == "."
                || component == ".."
                || !component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err("removable storage root does not name one supported volume".into());
            }
            Ok(root)
        }
    }
}

pub(crate) fn is_opaque_token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Check whether two active writers have effects that cannot safely coexist.
///
/// Parent-directory creation is idempotent and may be shared. A delete or file
/// write conflicts at the same path, and destructive or file effects conflict
/// with descendant effects. Callers pass only writers used by selected Recipe
/// steps, so unused named locations do not participate.
pub(crate) fn writer_effects_conflict(left: &[WriterEffect], right: &[WriterEffect]) -> bool {
    left.iter().any(|left| {
        right.iter().any(|right| match (left, right) {
            (WriterEffect::EnsureDirectory(a), WriterEffect::EnsureDirectory(b)) if a == b => false,
            _ => effects_conflict(left, right),
        })
    })
}

fn effects_conflict(left: &WriterEffect, right: &WriterEffect) -> bool {
    let (left_path, left_descendants_conflict) = effect_path(left);
    let (right_path, right_descendants_conflict) = effect_path(right);
    if left_path == right_path {
        return true;
    }
    (left_descendants_conflict && right_path.starts_with(left_path))
        || (right_descendants_conflict && left_path.starts_with(right_path))
}

fn effect_path(effect: &WriterEffect) -> (&Path, bool) {
    match effect {
        WriterEffect::EnsureDirectory(path) => (path, false),
        WriterEffect::WriteFile(path)
        | WriterEffect::DeleteFile(path)
        | WriterEffect::DeleteDirectory(path) => (path, true),
    }
}
