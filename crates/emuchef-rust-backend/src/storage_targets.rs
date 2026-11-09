//! Trusted setup-local storage target rules shared by Recipe planning and copy review.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

/// Maximum number of named Setup Locations accepted for a device plan.
pub const MAX_SETUP_LOCATIONS: usize = 32;
/// Maximum UTF-8 byte length of a Setup Location's display name.
pub const MAX_SETUP_LOCATION_NAME_BYTES: usize = 128;
/// Maximum UTF-8 byte length of a Setup Location's relative destination path.
pub const MAX_RELATIVE_LOCATION_PATH_BYTES: usize = 1024;

/// The shared-storage volume classes supported by Setup Locations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageTargetVolumeKind {
    /// The device's system-user emulated shared-storage volume.
    InternalShared,
    /// A removable shared-storage volume exposed by the device.
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
    /// Stable Setup Location identifier within the authored plan.
    pub id: String,
    /// Identifier of the authored device plan that declared this location.
    pub device_plan: String,
    /// Device-session epoch in which the trusted volume observation was made.
    pub session_epoch: u64,
    /// Sanitized name shown to the user during setup and review.
    pub name: String,
    /// Validated path below the volume root, using forward slashes.
    pub relative_path: String,
    /// Trusted identifier for the resolved volume instance.
    pub volume_handle: String,
    /// Supported shared-storage class for the resolved volume.
    pub volume_kind: StorageTargetVolumeKind,
    /// Physical volume root resolved by trusted Tauri code; never expose to UI.
    pub mount_root: String,
    /// Private identity evidence used to detect replacement of the volume.
    pub volume_identity: String,
    /// Private continuity evidence for this mounted volume observation.
    pub volume_continuity: String,
    /// Short-lived token proving that the volume observation is fresh.
    pub observation_token: String,
}

impl StorageTargetBinding {
    /// Validate the trusted binding's plan, path, root, and volume evidence.
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

    /// Resolve the validated relative path below its trusted physical root.
    pub fn destination_path(&self) -> Result<PathBuf, String> {
        self.validate()?;
        let mut destination = PathBuf::from(&self.mount_root);
        for component in self.relative_path.split('/') {
            destination.push(component);
        }
        Ok(destination)
    }
}

/// Tauri-issued authority for one real execution attempt that writes to a
/// reviewed Setup Location. This travels only over the trusted Rust sidecar
/// request path and is kept out of the reviewed plan and its digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetupLocationExecutionLease {
    /// Monotonic attempt identifier assigned by trusted Tauri code.
    pub attempt_sequence: u64,
    /// Device identity handle for the authorized execution session.
    pub device_handle: String,
    /// Device-session epoch covered by this authorization.
    pub session_epoch: u64,
    /// Runtime generation covered by this authorization.
    pub runtime_generation: u64,
    /// Reviewed plan identifier authorized for this attempt.
    pub plan_id: String,
    /// Canonical digest of the immutable reviewed plan.
    pub plan_digest: String,
}

impl SetupLocationExecutionLease {
    /// Reject incomplete identities, generations, or reviewed-plan digests.
    pub fn validate(&self) -> Result<(), String> {
        for value in [&self.device_handle, &self.plan_id] {
            if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err("setup location execution lease identity invalid".to_string());
            }
        }
        if self.attempt_sequence == 0 || self.session_epoch == 0 || self.runtime_generation == 0 {
            return Err("setup location execution lease generation invalid".to_string());
        }
        if self.plan_digest.len() != 64
            || !self
                .plan_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("setup location execution lease digest invalid".to_string());
        }
        Ok(())
    }
}

/// Backend-side gate for one Tauri-issued lease. It records only whether this
/// attempt may enter a destination mutation and how many such mutations have
/// already entered. Tauri remains the source of session validity.
#[derive(Debug)]
pub(crate) struct SetupLocationMutationLease {
    context: SetupLocationExecutionLease,
    registry_poisoned: Arc<AtomicBool>,
    state: Mutex<SetupLocationMutationState>,
    idle: Condvar,
}

#[derive(Debug, Default)]
struct SetupLocationMutationState {
    revoked: bool,
    active_mutations: usize,
    // Remote work or local cleanup may still be incomplete. Keep later
    // writes closed and lease retirement unconfirmed.
    mutation_uncertain: bool,
}

impl SetupLocationMutationLease {
    pub(crate) fn new(context: SetupLocationExecutionLease) -> Self {
        Self::new_with_registry_poison(context, Arc::new(AtomicBool::new(false)))
    }

    pub(crate) fn new_with_registry_poison(
        context: SetupLocationExecutionLease,
        registry_poisoned: Arc<AtomicBool>,
    ) -> Self {
        Self {
            context,
            registry_poisoned,
            state: Mutex::new(SetupLocationMutationState::default()),
            idle: Condvar::new(),
        }
    }

    pub(crate) fn context(&self) -> &SetupLocationExecutionLease {
        &self.context
    }

    /// Linearizes mutation permission with revocation. A permit remains held
    /// through the device operation, so revocation cannot complete while an
    /// already-authorized destination mutation is still running.
    pub(crate) fn acquire(&self) -> Result<SetupLocationMutationPermit<'_>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "setup location session authority unavailable".to_string())?;
        if self.registry_poisoned.load(Ordering::Acquire)
            || state.revoked
            || state.mutation_uncertain
        {
            return Err("setup location session authority expired".to_string());
        }
        state.active_mutations = state
            .active_mutations
            .checked_add(1)
            .ok_or_else(|| "setup location session authority unavailable".to_string())?;
        Ok(SetupLocationMutationPermit {
            lease: self,
            terminal_confirmed: false,
        })
    }

    /// Close the gate before waiting for a mutation already holding a permit.
    pub(crate) fn revoke(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "setup location session authority unavailable".to_string())?;
        state.revoked = true;
        Ok(())
    }

    pub(crate) fn wait_for_idle(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "setup location session authority unavailable".to_string())?;
        while state.active_mutations != 0 {
            state = self
                .idle
                .wait(state)
                .map_err(|_| "setup location session authority unavailable".to_string())?;
        }
        if state.mutation_uncertain {
            return Err("setup location mutation completion is uncertain".to_string());
        }
        Ok(())
    }

    /// Permanently blocks lease reuse when device cleanup may still be active.
    pub(crate) fn mark_uncertain(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.mutation_uncertain = true;
        self.registry_poisoned.store(true, Ordering::Release);
    }
}

pub(crate) struct SetupLocationMutationPermit<'a> {
    lease: &'a SetupLocationMutationLease,
    terminal_confirmed: bool,
}

impl SetupLocationMutationPermit<'_> {
    /// Records that the destination command returned with a known terminal result.
    pub(crate) fn confirm_terminal(&mut self) {
        self.terminal_confirmed = true;
    }

    /// Permanently blocks this runtime registry when mutation completion is
    /// not proven, even if the local child process was reaped.
    pub(crate) fn mark_uncertain(&mut self) {
        self.lease.mark_uncertain();
        self.terminal_confirmed = true;
    }
}

impl Drop for SetupLocationMutationPermit<'_> {
    fn drop(&mut self) {
        let mut state: MutexGuard<'_, SetupLocationMutationState> = match self.lease.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                self.lease.registry_poisoned.store(true, Ordering::Release);
                poisoned.into_inner()
            }
        };
        if !self.terminal_confirmed {
            state.mutation_uncertain = true;
            self.lease.registry_poisoned.store(true, Ordering::Release);
        }
        state.active_mutations = state.active_mutations.saturating_sub(1);
        if state.active_mutations == 0 {
            self.lease.idle.notify_all();
        }
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

/// Trim and validate a printable Setup Location name within the configured limit.
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

/// Validate a bounded relative path with safe, unambiguous path components.
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

/// Validate a physical root against the supported shared-storage volume class.
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

#[cfg(test)]
mod setup_location_mutation_lease_tests {
    use std::sync::Arc;

    use super::{SetupLocationExecutionLease, SetupLocationMutationLease};

    fn context() -> SetupLocationExecutionLease {
        SetupLocationExecutionLease {
            attempt_sequence: 1,
            device_handle: "device-test".to_string(),
            session_epoch: 4,
            runtime_generation: 7,
            plan_id: "plan-test".to_string(),
            plan_digest: "a".repeat(64),
        }
    }

    #[test]
    fn dropping_permit_does_not_restore_poisoned_mutation_authority() {
        let lease = Arc::new(SetupLocationMutationLease::new(context()));
        let permit = lease.acquire().unwrap();
        let poison = Arc::clone(&lease);
        let poison_result = std::thread::spawn(move || {
            let _guard = poison.state.lock().unwrap();
            panic!("injected lease-authority failure while permit is active");
        })
        .join();
        assert!(poison_result.is_err());

        drop(permit);

        assert!(lease.acquire().is_err());
        assert!(lease.revoke().is_err());
    }

    #[test]
    fn permit_drop_during_unwind_poisoned_mutation_authority() {
        let lease = SetupLocationMutationLease::new(context());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _permit = lease.acquire().unwrap();
            panic!("injected panic after destination mutation permit acquisition");
        }));

        assert!(result.is_err());
        assert!(lease.acquire().is_err());
        assert!(lease.wait_for_idle().is_err());
    }

    #[test]
    fn unavailable_lease_state_denies_mutation_and_revocation() {
        let lease = Arc::new(SetupLocationMutationLease::new(context()));
        let poison = Arc::clone(&lease);
        let _ = std::thread::spawn(move || {
            let _guard = poison.state.lock().unwrap();
            panic!("injected lease-authority failure");
        })
        .join();

        assert!(lease.acquire().is_err());
        assert!(lease.revoke().is_err());
        assert!(lease.wait_for_idle().is_err());
    }
}
