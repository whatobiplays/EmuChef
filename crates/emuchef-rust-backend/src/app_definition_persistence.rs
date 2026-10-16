//! Safe single-file persistence for typed App Definition documents.
//!
//! Opened bytes and filesystem identity stay together so a save can detect an
//! external replacement or edit before publishing a new canonical document.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::app_definition_admission::{
    verify_admission_snapshot, AdmissionPublishKind, AppDefinitionSaveAdmission,
    GeneratedAppPairSnapshot,
};
use crate::authored_models::{emit_app_definition_yaml, AppDefinitionV1};

static APP_DEFINITION_PUBLISH_LOCK: Mutex<()> = Mutex::new(());

/// A file revision contains its canonical path, original bytes, and identity.
///
/// The raw bytes are kept independently of any parsed or canonical YAML so a
/// later save can distinguish the file that was opened from an external edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedAppDefinitionFile {
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) identity: FileIdentity,
}

impl OpenedAppDefinitionFile {
    /// Return the canonical path of the file captured from disk.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the exact bytes captured when the file was opened or saved.
    pub fn raw_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the filesystem identity captured with the file bytes.
    pub fn identity(&self) -> &FileIdentity {
        &self.identity
    }

    /// Build the placeholder revision for a generated catalog file that does
    /// not exist yet.
    ///
    /// The placeholder carries no bytes or trusted identity. Publication
    /// verifies that the destination is still absent before writing, so the
    /// placeholder is never compared against captured disk state.
    pub(crate) fn absent_destination(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            bytes: Vec::new(),
            identity: FileIdentity::absent(),
        }
    }
}

/// Filesystem identity used with exact bytes to detect replacement and edits,
/// including changes to the Unix permission bits preserved during replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    length: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    permission_mode: u32,
}

impl FileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
                device: metadata.dev(),
                inode: metadata.ino(),
                permission_mode: metadata.mode() & 0o7777,
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                length: metadata.len(),
                modified: metadata.modified().ok(),
            }
        }
    }

    /// Build the placeholder identity used by a not-yet-created destination.
    fn absent() -> Self {
        #[cfg(unix)]
        {
            Self {
                length: 0,
                modified: None,
                device: 0,
                inode: 0,
                permission_mode: 0,
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                length: 0,
                modified: None,
            }
        }
    }
}

/// An actionable failure returned by App Definition admission or publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDefinitionPersistenceError {
    pub code: String,
    pub message: String,
    pub path: Option<PathBuf>,
}

impl AppDefinitionPersistenceError {
    pub(crate) fn new(code: &str, message: impl Into<String>, path: Option<&Path>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            path: path.map(Path::to_path_buf),
        }
    }

    pub(crate) fn io(
        code: &str,
        message: &str,
        path: &Path,
        error: impl std::fmt::Display,
    ) -> Self {
        Self::new(code, format!("{message}: {error}"), Some(path))
    }
}

impl std::fmt::Display for AppDefinitionPersistenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppDefinitionPersistenceError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileSnapshot {
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) identity: FileIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CatalogDirectorySnapshot {
    pub(crate) path: PathBuf,
    pub(crate) identity: DirectoryIdentity,
    pub(crate) files: Vec<FileSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirectoryIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl DirectoryIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Self {}
        }
    }
}

/// Capture the exact bytes and current identity of an opened regular file.
///
/// Symbolic links and non-file destinations are rejected so the saved path
/// cannot change meaning by following a link outside its original boundary.
pub fn capture_opened_app_definition_file(
    path: impl AsRef<Path>,
) -> Result<OpenedAppDefinitionFile, AppDefinitionPersistenceError> {
    let snapshot = capture_file_snapshot(path.as_ref())?;
    Ok(OpenedAppDefinitionFile {
        path: snapshot.path,
        bytes: snapshot.bytes,
        identity: snapshot.identity,
    })
}

/// Publish the admitted App Definition as one atomic file replacement or a
/// no-clobber catalog creation. Replacement preserves the opened Unix permission
/// bits. The caller updates document state only after this function succeeds.
pub fn persist_app_definition(
    value: &AppDefinitionV1,
    admission: &AppDefinitionSaveAdmission,
) -> Result<OpenedAppDefinitionFile, AppDefinitionPersistenceError> {
    persist_with_hook(value, admission, || {})
}

/// Run persistence with a hook immediately before the final stale-input check.
///
/// This deterministic seam lets the admission tests simulate an external edit
/// between validation and publication without timing-dependent threads.
#[cfg(test)]
pub fn persist_app_definition_with_hook<F>(
    value: &AppDefinitionV1,
    admission: &AppDefinitionSaveAdmission,
    before_final_check: F,
) -> Result<OpenedAppDefinitionFile, AppDefinitionPersistenceError>
where
    F: FnOnce(),
{
    persist_with_hook(value, admission, before_final_check)
}

fn persist_with_hook<BeforeCheck>(
    value: &AppDefinitionV1,
    admission: &AppDefinitionSaveAdmission,
    before_final_check: BeforeCheck,
) -> Result<OpenedAppDefinitionFile, AppDefinitionPersistenceError>
where
    BeforeCheck: FnOnce(),
{
    let _guard = APP_DEFINITION_PUBLISH_LOCK.lock().map_err(|_| {
        AppDefinitionPersistenceError::new(
            "app_definition_publish_unavailable",
            "App Definition saves are unavailable because the save lock was poisoned.",
            None,
        )
    })?;

    if !admission.can_save() {
        return Err(AppDefinitionPersistenceError::new(
            "app_definition_save_rejected",
            "The App Definition or one of its direct dependencies has validation errors.",
            None,
        ));
    }
    let snapshot = admission.snapshot.as_ref().ok_or_else(|| {
        AppDefinitionPersistenceError::new(
            "app_definition_save_rejected",
            "A complete App Definition admission snapshot is unavailable.",
            None,
        )
    })?;
    if value != &snapshot.candidate {
        return Err(AppDefinitionPersistenceError::new(
            "app_definition_admission_stale",
            "The App Definition draft changed after save admission; validate it again before saving.",
            Some(&snapshot.destination),
        ));
    }
    let canonical_yaml = emit_app_definition_yaml(value).map_err(|error| {
        AppDefinitionPersistenceError::new(
            error.code(),
            error.message(),
            Some(&snapshot.destination),
        )
    })?;

    verify_admission_snapshot(snapshot)?;
    validate_destination(snapshot)?;

    let parent = snapshot.destination.parent().ok_or_else(|| {
        AppDefinitionPersistenceError::new(
            "app_definition_destination_invalid",
            "The App Definition destination has no parent directory.",
            Some(&snapshot.destination),
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_publish_failed",
            "A synchronized temporary file could not be created beside the App Definition",
            parent,
            error,
        )
    })?;
    temporary
        .write_all(canonical_yaml.as_bytes())
        .map_err(|error| {
            AppDefinitionPersistenceError::io(
                "app_definition_publish_failed",
                "Canonical App Definition YAML could not be written to the temporary file",
                temporary.path(),
                error,
            )
        })?;
    #[cfg(unix)]
    if snapshot.publish_kind == AdmissionPublishKind::ReplaceOpenedFile {
        use std::os::unix::fs::PermissionsExt;
        // Apply the captured mode after writing, since writing may clear special
        // permission bits. The final snapshot check rejects intervening chmods.
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(
                snapshot.opened_file.identity.permission_mode,
            ))
            .map_err(|error| {
                AppDefinitionPersistenceError::io(
                    "app_definition_publish_failed",
                    "The opened App Definition permission bits could not be preserved",
                    temporary.path(),
                    error,
                )
            })?;
    }
    temporary.as_file().sync_all().map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_publish_failed",
            "The temporary App Definition file could not be synchronized",
            temporary.path(),
            error,
        )
    })?;
    let temporary_identity =
        FileIdentity::from_metadata(&temporary.as_file().metadata().map_err(|error| {
            AppDefinitionPersistenceError::io(
                "app_definition_publish_failed",
                "The synchronized temporary App Definition identity could not be read",
                temporary.path(),
                error,
            )
        })?);

    before_final_check();
    verify_admission_snapshot(snapshot)?;
    validate_destination(snapshot)?;

    let persistence_result = match snapshot.publish_kind {
        AdmissionPublishKind::ReplaceOpenedFile => temporary.persist(&snapshot.destination),
        AdmissionPublishKind::CreateCatalogApp => {
            temporary.persist_noclobber(&snapshot.destination)
        }
    };
    persistence_result.map_err(|error| {
        if snapshot.publish_kind == AdmissionPublishKind::CreateCatalogApp
            && error.error.kind() == std::io::ErrorKind::AlreadyExists
        {
            AppDefinitionPersistenceError::new(
                "app_definition_stale_state",
                "The requested App Definition destination appeared after save admission; validate again before saving.",
                Some(&snapshot.destination),
            )
        } else {
            AppDefinitionPersistenceError::io(
                "app_definition_publish_failed",
                "The App Definition could not be published atomically",
                &snapshot.destination,
                error.error,
            )
        }
    })?;
    sync_parent_best_effort(parent);

    Ok(OpenedAppDefinitionFile {
        path: snapshot.destination.clone(),
        bytes: canonical_yaml.into_bytes(),
        identity: temporary_identity,
    })
}

/// Outcome of one successful generated App/Recipe pair publication.
#[derive(Debug)]
pub(crate) struct GeneratedPairOutcome {
    /// Published App Definition revision, including the new file identity.
    pub(crate) app: OpenedAppDefinitionFile,
    /// Canonical destination of the Recipe created by this publication.
    pub(crate) recipe_path: PathBuf,
    /// True when the publication replaced an existing App file.
    pub(crate) app_replaced: bool,
}

/// Ownership report for a pair whose Recipe was created before the final App
/// commit failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GeneratedPairRollback {
    pub(crate) recipe_path: PathBuf,
    /// True when the new Recipe was removed by this publication.
    pub(crate) recipe_removed: bool,
    /// True when the new Recipe no longer matches this publication's own
    /// content and identity, so it was preserved for manual recovery.
    pub(crate) recipe_recovery_required: bool,
}

/// Failure of one generated App/Recipe pair publication.
#[derive(Debug)]
pub(crate) enum GeneratedPairPersistFailure {
    /// Nothing was written; the caller reports a plain refusal.
    Refused(AppDefinitionPersistenceError),
    /// The new Recipe was committed but the final App commit failed.
    RecipeCommitted {
        error: AppDefinitionPersistenceError,
        rollback: GeneratedPairRollback,
    },
}

/// Publish one admitted generated App/Recipe pair.
///
/// The App and the new Recipe are staged as synchronized temporary files,
/// then the Recipe is created with no-clobber semantics first and the App is
/// committed atomically last. Full combined verification runs before the
/// first write; after that only the two destinations owned by this operation
/// are re-checked, so the Recipe this publication creates never invalidates
/// its own captured inventory. When the final App commit fails, the new
/// Recipe is removed only while it still matches this publication's exact
/// identity and content; any other state is preserved for recovery.
pub(crate) fn persist_generated_app_pair(
    pair: &GeneratedAppPairSnapshot,
) -> Result<GeneratedPairOutcome, GeneratedPairPersistFailure> {
    persist_generated_app_pair_with(pair, || {})
}

/// Run pair publication with a hook immediately before the final App commit.
///
/// The hook runs after the Recipe commit is durable and before the App commit,
/// so tests can deterministically simulate an external writer claiming a
/// destination in that window without timing-dependent threads.
pub(crate) fn persist_generated_app_pair_with<BeforeCheck>(
    pair: &GeneratedAppPairSnapshot,
    before_final_check: BeforeCheck,
) -> Result<GeneratedPairOutcome, GeneratedPairPersistFailure>
where
    BeforeCheck: FnOnce(),
{
    let refused = GeneratedPairPersistFailure::Refused;
    let _guard = APP_DEFINITION_PUBLISH_LOCK
        .lock()
        .map_err(|_| {
            refused(AppDefinitionPersistenceError::new(
                "app_definition_publish_unavailable",
                "App Definition saves are unavailable because the save lock was poisoned.",
                None,
            ))
        })?;

    let app_snapshot = &pair.app_snapshot;
    let canonical_app_yaml = emit_app_definition_yaml(&app_snapshot.candidate).map_err(|error| {
        refused(AppDefinitionPersistenceError::new(
            error.code(),
            error.message(),
            Some(&app_snapshot.destination),
        ))
    })?;

    verify_admission_snapshot(app_snapshot).map_err(refused)?;
    validate_destination(app_snapshot).map_err(refused)?;
    validate_generated_recipe_destination(pair).map_err(refused)?;

    let app_parent = app_snapshot.destination.parent().ok_or_else(|| {
        refused(AppDefinitionPersistenceError::new(
            "app_definition_destination_invalid",
            "The App Definition destination has no parent directory.",
            Some(&app_snapshot.destination),
        ))
    })?;
    let recipe_parent = pair.recipe_destination.parent().ok_or_else(|| {
        refused(AppDefinitionPersistenceError::new(
            "recipe_destination_invalid",
            "The generated Recipe destination has no parent directory.",
            Some(&pair.recipe_destination),
        ))
    })?;

    let mut app_temporary = tempfile::NamedTempFile::new_in(app_parent).map_err(|error| {
        refused(AppDefinitionPersistenceError::io(
            "app_definition_publish_failed",
            "A synchronized temporary file could not be created beside the App Definition",
            app_parent,
            error,
        ))
    })?;
    app_temporary
        .write_all(canonical_app_yaml.as_bytes())
        .map_err(|error| {
            refused(AppDefinitionPersistenceError::io(
                "app_definition_publish_failed",
                "Canonical App Definition YAML could not be written to the temporary file",
                app_temporary.path(),
                error,
            ))
        })?;
    #[cfg(unix)]
    if app_snapshot.publish_kind == AdmissionPublishKind::ReplaceOpenedFile {
        use std::os::unix::fs::PermissionsExt;
        // Apply the captured mode after writing, since writing may clear
        // special permission bits.
        app_temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(
                app_snapshot.opened_file.identity.permission_mode,
            ))
            .map_err(|error| {
                refused(AppDefinitionPersistenceError::io(
                    "app_definition_publish_failed",
                    "The opened App Definition permission bits could not be preserved",
                    app_temporary.path(),
                    error,
                ))
            })?;
    }
    app_temporary.as_file().sync_all().map_err(|error| {
        refused(AppDefinitionPersistenceError::io(
            "app_definition_publish_failed",
            "The temporary App Definition file could not be synchronized",
            app_temporary.path(),
            error,
        ))
    })?;
    let app_temporary_identity =
        FileIdentity::from_metadata(&app_temporary.as_file().metadata().map_err(|error| {
            refused(AppDefinitionPersistenceError::io(
                "app_definition_publish_failed",
                "The synchronized temporary App Definition identity could not be read",
                app_temporary.path(),
                error,
            ))
        })?);

    let mut recipe_temporary = tempfile::NamedTempFile::new_in(recipe_parent).map_err(|error| {
        refused(AppDefinitionPersistenceError::io(
            "recipe_publish_failed",
            "A synchronized temporary file could not be created beside the generated Recipe",
            recipe_parent,
            error,
        ))
    })?;
    recipe_temporary
        .write_all(pair.recipe_canonical_yaml.as_bytes())
        .map_err(|error| {
            refused(AppDefinitionPersistenceError::io(
                "recipe_publish_failed",
                "Canonical Recipe YAML could not be written to the temporary file",
                recipe_temporary.path(),
                error,
            ))
        })?;
    recipe_temporary.as_file().sync_all().map_err(|error| {
        refused(AppDefinitionPersistenceError::io(
            "recipe_publish_failed",
            "The temporary Recipe file could not be synchronized",
            recipe_temporary.path(),
            error,
        ))
    })?;
    let recipe_temporary_identity =
        FileIdentity::from_metadata(&recipe_temporary.as_file().metadata().map_err(|error| {
            refused(AppDefinitionPersistenceError::io(
                "recipe_publish_failed",
                "The synchronized temporary Recipe identity could not be read",
                recipe_temporary.path(),
                error,
            ))
        })?);

    // Re-check only the destinations this publication owns. The complete
    // catalog verification above is authoritative for everything else;
    // recapturing it here would reject the Recipe this operation creates.
    verify_app_destination_identity(app_snapshot).map_err(refused)?;
    validate_generated_recipe_destination(pair).map_err(refused)?;

    if let Err(error) = recipe_temporary.persist_noclobber(&pair.recipe_destination) {
        return Err(refused(if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            AppDefinitionPersistenceError::new(
                "recipe_destination_exists",
                "A file appeared at the generated Recipe destination before publication.",
                Some(&pair.recipe_destination),
            )
        } else {
            AppDefinitionPersistenceError::io(
                "recipe_publish_failed",
                "The generated Recipe could not be published without overwriting",
                &pair.recipe_destination,
                error.error,
            )
        }));
    }
    sync_parent_best_effort(recipe_parent);

    before_final_check();
    verify_catalog_state_before_final_commit(pair, &recipe_temporary_identity).map_err(|error| {
        let rollback = rollback_created_recipe(pair, &recipe_temporary_identity);
        GeneratedPairPersistFailure::RecipeCommitted { error, rollback }
    })?;
    let app_commit = match app_snapshot.publish_kind {
        AdmissionPublishKind::ReplaceOpenedFile => {
            app_temporary.persist(&app_snapshot.destination).map(|_| ())
        }
        AdmissionPublishKind::CreateCatalogApp => app_temporary
            .persist_noclobber(&app_snapshot.destination)
            .map(|_| ()),
    };
    if let Err(error) = app_commit {
        let persistence_error = if app_snapshot.publish_kind == AdmissionPublishKind::CreateCatalogApp
            && error.error.kind() == std::io::ErrorKind::AlreadyExists
        {
            AppDefinitionPersistenceError::new(
                "app_definition_stale_state",
                "The requested App Definition destination appeared after save admission; validate again before saving.",
                Some(&app_snapshot.destination),
            )
        } else {
            AppDefinitionPersistenceError::io(
                "app_definition_publish_failed",
                "The App Definition could not be published atomically",
                &app_snapshot.destination,
                error.error,
            )
        };
        let rollback = rollback_created_recipe(pair, &recipe_temporary_identity);
        return Err(GeneratedPairPersistFailure::RecipeCommitted {
            error: persistence_error,
            rollback,
        });
    }
    sync_parent_best_effort(app_parent);

    Ok(GeneratedPairOutcome {
        app: OpenedAppDefinitionFile {
            path: app_snapshot.destination.clone(),
            bytes: canonical_app_yaml.into_bytes(),
            identity: app_temporary_identity,
        },
        recipe_path: pair.recipe_destination.clone(),
        app_replaced: app_snapshot.publish_kind == AdmissionPublishKind::ReplaceOpenedFile,
    })
}

/// Re-check the captured catalog state and the already committed Recipe
/// immediately before the final App commit.
///
/// Publication is the only operation allowed to change the catalog between
/// admission and this point, and the sole change it owns is the new Recipe at
/// the publication's Recipe destination. Every other entry must still match the
/// identity and bytes captured at admission: an App edited after the Recipe
/// commit, or a dependent Recipe edited in the same window, refuses the final
/// commit so the App is never published on state admission did not validate.
/// The owned Recipe itself must still be present with its committed bytes and
/// filesystem identity; the directory comparison tolerates that one addition,
/// so it cannot notice the Recipe's deletion on its own.
fn verify_catalog_state_before_final_commit(
    pair: &GeneratedAppPairSnapshot,
    committed_recipe_identity: &FileIdentity,
) -> Result<(), AppDefinitionPersistenceError> {
    verify_app_destination_identity(&pair.app_snapshot)?;
    verify_owned_recipe_committed(pair, committed_recipe_identity)?;
    let prepared_recipe_path = pair.recipe_destination.as_path();
    for directory in &pair.app_snapshot.directories {
        if !catalog_directory_is_unchanged_except_owned_recipe(
            directory,
            prepared_recipe_path,
            pair.recipe_canonical_yaml.as_bytes(),
        )? {
            return Err(stale_file(&directory.path));
        }
    }
    Ok(())
}

/// Require the Recipe this publication committed to still be present with the
/// committed bytes and filesystem identity.
///
/// An external removal or replacement in the commit window refuses the final
/// App commit: the App is never published without the Recipe saved alongside
/// it, and a foreign file left at the destination is preserved for recovery
/// instead of being adopted or removed.
fn verify_owned_recipe_committed(
    pair: &GeneratedAppPairSnapshot,
    committed_identity: &FileIdentity,
) -> Result<(), AppDefinitionPersistenceError> {
    match fs::symlink_metadata(&pair.recipe_destination) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(stale_file(&pair.recipe_destination));
        }
        Err(error) => {
            return Err(AppDefinitionPersistenceError::io(
                "recipe_destination_invalid",
                "The generated Recipe destination could not be inspected",
                &pair.recipe_destination,
                error,
            ));
        }
    }
    let current = capture_file_snapshot(&pair.recipe_destination)?;
    if current.bytes != pair.recipe_canonical_yaml.as_bytes()
        || &current.identity != committed_identity
    {
        return Err(stale_file(&pair.recipe_destination));
    }
    Ok(())
}

/// Compare one captured catalog directory against the current disk state.
///
/// Returns true when the directory still holds every captured file with its
/// captured identity and bytes. The only permitted addition is the Recipe this
/// publication created at `prepared_recipe_path` with `prepared_bytes`.
fn catalog_directory_is_unchanged_except_owned_recipe(
    captured: &CatalogDirectorySnapshot,
    prepared_recipe_path: &Path,
    prepared_bytes: &[u8],
) -> Result<bool, AppDefinitionPersistenceError> {
    let parent = captured.path.parent().unwrap_or(&captured.path);
    let current = capture_catalog_directory(&captured.path, parent)?;
    if current.path != captured.path || current.identity != captured.identity {
        return Ok(false);
    }
    let mut matched = vec![false; captured.files.len()];
    for file in &current.files {
        if file.path == prepared_recipe_path {
            if file.bytes != prepared_bytes {
                return Ok(false);
            }
            continue;
        }
        let Some(index) = captured
            .files
            .iter()
            .position(|expected| expected.path == file.path)
        else {
            return Ok(false);
        };
        if matched[index]
            || captured.files[index].bytes != file.bytes
            || captured.files[index].identity != file.identity
        {
            return Ok(false);
        }
        matched[index] = true;
    }
    Ok(matched.iter().all(|seen| *seen))
}

/// Re-check only the App destination owned by this publication.
fn verify_app_destination_identity(
    snapshot: &crate::app_definition_admission::AdmissionSnapshot,
) -> Result<(), AppDefinitionPersistenceError> {
    if snapshot.expect_new_destination {
        return match fs::symlink_metadata(&snapshot.destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(stale_file(&snapshot.destination)),
            Err(error) => Err(AppDefinitionPersistenceError::io(
                "app_definition_destination_invalid",
                "The App Definition destination could not be inspected",
                &snapshot.destination,
                error,
            )),
        };
    }
    let current = capture_file_snapshot(&snapshot.opened_file.path)?;
    if current.path != snapshot.opened_file.path
        || current.bytes != snapshot.opened_file.bytes
        || current.identity != snapshot.opened_file.identity
    {
        return Err(stale_file(&snapshot.opened_file.path));
    }
    Ok(())
}

/// Validate the Recipe destination owned by this publication.
fn validate_generated_recipe_destination(
    pair: &GeneratedAppPairSnapshot,
) -> Result<(), AppDefinitionPersistenceError> {
    let destination = &pair.recipe_destination;
    if !is_yaml_catalog_file(destination) {
        return Err(AppDefinitionPersistenceError::new(
            "recipe_destination_invalid",
            "The generated Recipe destination must be a YAML catalog file.",
            Some(destination),
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        AppDefinitionPersistenceError::new(
            "recipe_destination_invalid",
            "The generated Recipe destination has no parent directory.",
            Some(destination),
        )
    })?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "recipe_destination_invalid",
            "The generated Recipe destination directory is unavailable",
            parent,
            error,
        )
    })?;
    if canonical_parent != parent {
        return Err(AppDefinitionPersistenceError::new(
            "recipe_destination_invalid",
            "The generated Recipe destination must use its canonical directory path.",
            Some(parent),
        ));
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(AppDefinitionPersistenceError::new(
            "recipe_destination_exists",
            "A file already exists at the generated Recipe destination.",
            Some(destination),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppDefinitionPersistenceError::io(
            "recipe_destination_invalid",
            "The generated Recipe destination could not be inspected",
            destination,
            error,
        )),
    }
}

/// Remove the Recipe this publication created when it still matches the
/// written content and identity; otherwise report the state truthfully.
///
/// An already-absent destination means the committed Recipe is gone without
/// this publication removing it, so nothing of this publication remains to
/// recover. Any other mismatch means a foreign file occupies the destination:
/// it is preserved and recovery is reported.
fn rollback_created_recipe(
    pair: &GeneratedAppPairSnapshot,
    created_identity: &FileIdentity,
) -> GeneratedPairRollback {
    let recipe_path = pair.recipe_destination.clone();
    if let Err(error) = fs::symlink_metadata(&recipe_path) {
        if error.kind() == std::io::ErrorKind::NotFound {
            return GeneratedPairRollback {
                recipe_path,
                recipe_removed: false,
                recipe_recovery_required: false,
            };
        }
    }
    let matches_creation = capture_file_snapshot(&recipe_path).is_ok_and(|current| {
        current.bytes == pair.recipe_canonical_yaml.as_bytes()
            && &current.identity == created_identity
    });
    if !matches_creation {
        return GeneratedPairRollback {
            recipe_path,
            recipe_removed: false,
            recipe_recovery_required: true,
        };
    }
    match fs::remove_file(&recipe_path) {
        Ok(()) => GeneratedPairRollback {
            recipe_path,
            recipe_removed: true,
            recipe_recovery_required: false,
        },
        Err(_) => GeneratedPairRollback {
            recipe_path,
            recipe_removed: false,
            recipe_recovery_required: true,
        },
    }
}

fn validate_destination(
    snapshot: &crate::app_definition_admission::AdmissionSnapshot,
) -> Result<(), AppDefinitionPersistenceError> {
    let parent = snapshot.destination.parent().ok_or_else(|| {
        AppDefinitionPersistenceError::new(
            "app_definition_destination_invalid",
            "The App Definition destination has no parent directory.",
            Some(&snapshot.destination),
        )
    })?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_destination_invalid",
            "The App Definition destination directory is unavailable",
            parent,
            error,
        )
    })?;
    if canonical_parent != parent {
        return Err(AppDefinitionPersistenceError::new(
            "app_definition_destination_invalid",
            "The App Definition destination must use its canonical directory path.",
            Some(parent),
        ));
    }

    match fs::symlink_metadata(&snapshot.destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(AppDefinitionPersistenceError::new(
                "app_definition_destination_invalid",
                "The App Definition destination is a symbolic link.",
                Some(&snapshot.destination),
            ))
        }
        Ok(_metadata) if snapshot.publish_kind == AdmissionPublishKind::CreateCatalogApp => {
            Err(AppDefinitionPersistenceError::new(
                "app_definition_destination_conflict",
                "An App Definition already exists at the requested catalog path.",
                Some(&snapshot.destination),
            ))
        }
        Ok(metadata) if !metadata.is_file() => Err(AppDefinitionPersistenceError::new(
            "app_definition_destination_invalid",
            "The App Definition destination is not a regular file.",
            Some(&snapshot.destination),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if snapshot.publish_kind == AdmissionPublishKind::CreateCatalogApp {
                Ok(())
            } else {
                Err(AppDefinitionPersistenceError::new(
                    "app_definition_stale_state",
                    "The opened App Definition was removed before it could be saved.",
                    Some(&snapshot.destination),
                ))
            }
        }
        Err(error) => Err(AppDefinitionPersistenceError::io(
            "app_definition_destination_invalid",
            "The App Definition destination could not be inspected",
            &snapshot.destination,
            error,
        )),
    }
}

pub(crate) fn capture_catalog_directory(
    directory: &Path,
    canonical_root: &Path,
) -> Result<CatalogDirectorySnapshot, AppDefinitionPersistenceError> {
    let before = fs::symlink_metadata(directory).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_catalog_inventory_incomplete",
            "A required authored catalog directory could not be inspected",
            directory,
            error,
        )
    })?;
    if before.file_type().is_symlink() || !before.is_dir() {
        return Err(AppDefinitionPersistenceError::new(
            "app_catalog_path_invalid",
            "A required authored catalog directory must be a real directory inside the authored root.",
            Some(directory),
        ));
    }
    let canonical_directory = fs::canonicalize(directory).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_catalog_path_invalid",
            "A required authored catalog directory could not be canonicalized",
            directory,
            error,
        )
    })?;
    if canonical_directory != directory || !canonical_directory.starts_with(canonical_root) {
        return Err(AppDefinitionPersistenceError::new(
            "app_catalog_path_invalid",
            "A required authored catalog directory resolves outside its canonical root.",
            Some(directory),
        ));
    }

    let identity = DirectoryIdentity::from_metadata(&before);
    let entries = fs::read_dir(&canonical_directory).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_catalog_inventory_incomplete",
            "A required authored catalog directory could not be read",
            &canonical_directory,
            error,
        )
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            AppDefinitionPersistenceError::io(
                "app_catalog_inventory_incomplete",
                "An authored catalog directory entry could not be read",
                &canonical_directory,
                error,
            )
        })?;
        let entry_path = entry.path();
        if !is_yaml_catalog_file(&entry_path) {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| {
            AppDefinitionPersistenceError::io(
                "app_catalog_inventory_incomplete",
                "An authored YAML catalog entry could not be inspected",
                &entry_path,
                error,
            )
        })?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(AppDefinitionPersistenceError::new(
                "app_catalog_path_invalid",
                "Every top-level authored YAML catalog entry must be a regular file, not a symbolic link or directory.",
                Some(&entry_path),
            ));
        }
        let snapshot = capture_file_snapshot(&entry_path)?;
        if snapshot.path.parent() != Some(canonical_directory.as_path()) {
            return Err(AppDefinitionPersistenceError::new(
                "app_catalog_path_invalid",
                "An authored YAML file resolves outside its catalog directory.",
                Some(&entry_path),
            ));
        }
        files.push(snapshot);
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let after = fs::symlink_metadata(directory).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_catalog_inventory_incomplete",
            "An authored catalog directory changed while it was being read",
            directory,
            error,
        )
    })?;
    let canonical_after = fs::canonicalize(directory).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_catalog_inventory_incomplete",
            "An authored catalog directory changed while it was being read",
            directory,
            error,
        )
    })?;
    if after.file_type().is_symlink()
        || !after.is_dir()
        || canonical_after != canonical_directory
        || DirectoryIdentity::from_metadata(&after) != identity
    {
        return Err(AppDefinitionPersistenceError::new(
            "app_definition_stale_state",
            "An authored catalog directory changed while its save snapshot was being collected.",
            Some(&canonical_directory),
        ));
    }

    Ok(CatalogDirectorySnapshot {
        path: canonical_directory,
        identity,
        files,
    })
}

pub(crate) fn capture_file_snapshot(
    path: &Path,
) -> Result<FileSnapshot, AppDefinitionPersistenceError> {
    let before = fs::symlink_metadata(path).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_file_unavailable",
            "An App Definition or catalog file could not be inspected",
            path,
            error,
        )
    })?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(AppDefinitionPersistenceError::new(
            "app_definition_path_invalid",
            "An App Definition file path must identify a regular file and cannot follow a symbolic link.",
            Some(path),
        ));
    }
    let canonical_path = fs::canonicalize(path).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_file_unavailable",
            "An App Definition or catalog file could not be canonicalized",
            path,
            error,
        )
    })?;
    let before_identity = FileIdentity::from_metadata(&before);
    let mut file = File::open(&canonical_path).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_file_unavailable",
            "An App Definition or catalog file could not be opened",
            &canonical_path,
            error,
        )
    })?;
    let handle_before = file.metadata().map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_file_unavailable",
            "An App Definition or catalog file identity could not be read",
            &canonical_path,
            error,
        )
    })?;
    if !handle_before.is_file() || FileIdentity::from_metadata(&handle_before) != before_identity {
        return Err(stale_file(&canonical_path));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_file_unavailable",
            "An App Definition or catalog file could not be read",
            &canonical_path,
            error,
        )
    })?;
    let handle_after = file.metadata().map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_file_unavailable",
            "An App Definition or catalog file identity could not be rechecked",
            &canonical_path,
            error,
        )
    })?;
    let path_after = fs::symlink_metadata(&canonical_path).map_err(|error| {
        AppDefinitionPersistenceError::io(
            "app_definition_file_unavailable",
            "An App Definition or catalog file changed while it was read",
            &canonical_path,
            error,
        )
    })?;
    if !path_after.is_file()
        || FileIdentity::from_metadata(&handle_after) != before_identity
        || FileIdentity::from_metadata(&path_after) != before_identity
    {
        return Err(stale_file(&canonical_path));
    }
    Ok(FileSnapshot {
        path: canonical_path,
        bytes,
        identity: before_identity,
    })
}

pub(crate) fn file_snapshots_equal(left: &FileSnapshot, right: &FileSnapshot) -> bool {
    left.path == right.path && left.bytes == right.bytes && left.identity == right.identity
}

pub(crate) fn stale_file(path: &Path) -> AppDefinitionPersistenceError {
    AppDefinitionPersistenceError::new(
        "app_definition_stale_state",
        "An opened App Definition or relevant catalog file changed after save admission; validate again before saving.",
        Some(path),
    )
}

fn is_yaml_catalog_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    )
}

fn sync_parent_best_effort(parent: &Path) {
    #[cfg(unix)]
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
    #[cfg(not(unix))]
    let _ = parent;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opened_file_snapshot_keeps_the_original_bytes_and_canonical_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.yaml");
        fs::write(&path, b"raw: bytes\n").unwrap();

        let snapshot = capture_opened_app_definition_file(&path).unwrap();

        assert_eq!(snapshot.path(), path.canonicalize().unwrap());
        assert_eq!(snapshot.raw_bytes(), b"raw: bytes\n");
    }

    #[cfg(unix)]
    #[test]
    fn opened_file_capture_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.yaml");
        let link = directory.path().join("link.yaml");
        fs::write(&target, b"raw: bytes\n").unwrap();
        symlink(&target, &link).unwrap();

        let error = capture_opened_app_definition_file(&link).unwrap_err();

        assert_eq!(error.code, "app_definition_path_invalid");
    }
    use crate::app_definition_admission::{
        admit_app_definition_save, AppDefinitionSaveContext, AppDefinitionSaveDestination,
    };
    use crate::authored_models::AppDefinitionV1;

    fn app(id: &str, package_id: &str) -> AppDefinitionV1 {
        crate::authored_models::parse_app_definition_yaml(&format!(
            "schema_version: 1\nkind: app_definition\nid: {id}\nname: Example App\npackage_id: {package_id}\n"
        )).unwrap()
    }

    fn write_app(path: &Path, app: &AppDefinitionV1) {
        fs::write(
            path,
            crate::authored_models::emit_app_definition_yaml(app).unwrap(),
        )
        .unwrap();
    }

    fn opened_context(path: &Path) -> AppDefinitionSaveContext {
        AppDefinitionSaveContext {
            opened_file: capture_opened_app_definition_file(path).unwrap(),
            authored_root: None,
            opened_catalog_identity: None,
            destination: AppDefinitionSaveDestination::OpenedFile,
        }
    }

    fn catalog() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        root
    }

    fn save_as_context(root: &Path, path: &Path) -> AppDefinitionSaveContext {
        let mut context = opened_context(path);
        context.authored_root = Some(root.to_path_buf());
        context.destination = AppDefinitionSaveDestination::NewCatalogApp;
        context
    }

    #[test]
    #[cfg(unix)]
    fn unix_permission_change_before_final_check_rejects_replacement_and_preserves_existing_file() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.yaml");
        let opened = app("sample", "com.example.sample");
        write_app(&path, &opened);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let original_bytes = fs::read(&path).unwrap();
        let original_inode = fs::metadata(&path).unwrap().ino();
        let mut edited = opened.clone();
        edited.description = Some("Rejected update".to_string());
        let admission = admit_app_definition_save(&edited, &opened_context(&path));
        assert!(admission.can_save(), "{:?}", admission.diagnostics);

        let error = persist_app_definition_with_hook(&edited, &admission, || {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        })
        .unwrap_err();

        assert_eq!(error.code, "app_definition_stale_state");
        assert_eq!(fs::read(&path).unwrap(), original_bytes);
        let current = fs::metadata(&path).unwrap();
        assert_eq!(current.ino(), original_inode);
        assert_eq!(current.permissions().mode() & 0o7777, 0o640);
    }

    #[test]
    fn save_as_rejects_a_new_destination_present_before_the_final_snapshot_check() {
        let root = catalog();
        let external = tempfile::tempdir().unwrap();
        let opened_path = external.path().join("draft.yaml");
        let opened = app("sample", "com.example.sample");
        write_app(&opened_path, &opened);
        let admission =
            admit_app_definition_save(&opened, &save_as_context(root.path(), &opened_path));
        assert!(admission.can_save(), "{:?}", admission.diagnostics);
        let destination = root.path().join("apps/sample.yaml");
        let conflicting_bytes = b"owner-created this destination after admission".to_vec();

        let error = persist_app_definition_with_hook(&opened, &admission, || {
            fs::write(&destination, &conflicting_bytes).unwrap();
        })
        .unwrap_err();

        assert_eq!(error.code, "app_definition_stale_state");
        assert_eq!(fs::read(destination).unwrap(), conflicting_bytes);
        assert!(!fs::read(opened_path).unwrap().is_empty());
    }
}
