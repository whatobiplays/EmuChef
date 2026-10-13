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
