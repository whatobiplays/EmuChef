//! Private storage for original configuration files before an authorized replacement.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

const BACKUP_DIRECTORY_NAME: &str = "configuration-backups";
const BACKUP_FILE_NAME: &str = "original.cfg";
const METADATA_FILE_NAME: &str = "metadata.v2.json";
const MAX_OPERATION_ID_LENGTH: usize = 128;

/// Identifies the device target and execution attempt for a replacement backup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationBackupContext<'a> {
    pub device_handle: &'a str,
    pub runtime_generation: u64,
    pub attempt_sequence: u64,
    pub package_id: &'a str,
    pub target_path: &'a str,
    pub operation: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigurationBackupMetadata<'a> {
    version: u8,
    operation_id: &'a str,
    operation: &'a str,
    device_handle: &'a str,
    runtime_generation: u64,
    attempt_sequence: u64,
    package_id: &'a str,
    target_path: &'a str,
    captured_at_unix_ms: u64,
    original_sha256: &'a str,
    original_length: u64,
}

/// Stores a verified copy of the original configuration in a private,
/// operation-specific directory. The copy and its metadata are synced before
/// the caller can begin a replacement write.
#[cfg(unix)]
/// Write, synchronize, and read back the original configuration before replacement.
pub fn store_configuration_backup(
    app_data_root: &Path,
    operation_id: &str,
    context: &ConfigurationBackupContext<'_>,
    original_config: &[u8],
) -> io::Result<ConfigurationBackup> {
    validate_operation_id(operation_id)?;
    validate_backup_context(context)?;
    require_directory(app_data_root)?;

    let backup_root = app_data_root.join(BACKUP_DIRECTORY_NAME);
    ensure_private_directory(&backup_root)?;
    sync_directory(app_data_root)?;
    sync_directory(&backup_root)?;

    let operation_directory = backup_root.join(operation_id);
    match fs::symlink_metadata(&operation_directory) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a configuration backup already exists for this operation",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let staging_directory = backup_root.join(format!(".staging-{operation_id}"));
    let (length, sha256, captured_at_unix_ms) =
        prepare_staging_directory(&backup_root, &staging_directory, |staging_directory| {
            let staged_path = staging_directory.join(BACKUP_FILE_NAME);
            let staged_metadata_path = staging_directory.join(METADATA_FILE_NAME);
            let length = u64::try_from(original_config.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "configuration is too large")
            })?;
            let sha256 = digest(original_config);

            let mut backup_file = create_private_file(&staged_path)?;
            backup_file.write_all(original_config)?;
            backup_file.sync_all()?;
            drop(backup_file);

            verify_staged_bytes(&staged_path, original_config, &sha256)?;

            let captured_at_unix_ms = current_unix_time_millis()?;
            let metadata = serde_json::to_vec(&ConfigurationBackupMetadata {
                version: 2,
                operation_id,
                operation: context.operation,
                device_handle: context.device_handle,
                runtime_generation: context.runtime_generation,
                attempt_sequence: context.attempt_sequence,
                package_id: context.package_id,
                target_path: context.target_path,
                captured_at_unix_ms,
                original_sha256: &sha256,
                original_length: length,
            })
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let mut metadata_file = create_private_file(&staged_metadata_path)?;
            metadata_file.write_all(&metadata)?;
            metadata_file.sync_all()?;
            drop(metadata_file);
            if fs::read(&staged_metadata_path)? != metadata {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stored configuration backup metadata failed verification",
                ));
            }

            sync_directory(staging_directory)?;

            match fs::symlink_metadata(&operation_directory) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "a configuration backup already exists for this operation",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            Ok((length, sha256, captured_at_unix_ms))
        })?;
    if let Err(primary_error) = fs::rename(&staging_directory, &operation_directory) {
        return Err(report_cleanup_failure(primary_error, || {
            cleanup_staging_directory(&backup_root, &staging_directory)
        }));
    }
    sync_directory(&backup_root)?;

    Ok(ConfigurationBackup {
        path: operation_directory.join(BACKUP_FILE_NAME),
        metadata_path: operation_directory.join(METADATA_FILE_NAME),
        operation_id: operation_id.to_owned(),
        sha256,
        length,
        captured_at_unix_ms,
    })
}

/// Refuses to create backups on platforms where private durable storage is not
/// implemented. Replacement remains blocked unless the user has an applicable
/// file-specific backup waiver.
#[cfg(not(unix))]
pub fn store_configuration_backup(
    _app_data_root: &Path,
    _operation_id: &str,
    _context: &ConfigurationBackupContext<'_>,
    _original_config: &[u8],
) -> io::Result<ConfigurationBackup> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private durable configuration backups are unavailable on this platform",
    ))
}

/// Paths and integrity details for a verified configuration backup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationBackup {
    pub path: PathBuf,
    pub metadata_path: PathBuf,
    pub operation_id: String,
    pub sha256: String,
    pub length: u64,
    pub captured_at_unix_ms: u64,
}

/// Return the verified operation directory for opening an existing backup.
pub fn configuration_backup_directory(
    app_data_root: &Path,
    operation_id: &str,
) -> io::Result<PathBuf> {
    validate_operation_id(operation_id)?;
    let backup_root = app_data_root.join(BACKUP_DIRECTORY_NAME);
    require_directory(&backup_root)?;
    let operation_directory = backup_root.join(operation_id);
    require_directory(&operation_directory)?;
    Ok(operation_directory)
}

/// Resolve app data from the artifact cache root configured by the native app.
pub fn app_data_root_from_cache_root(cache_root: &Path) -> io::Result<PathBuf> {
    if cache_root.file_name().and_then(|name| name.to_str()) != Some("artifact-cache") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native artifact cache root is invalid",
        ));
    }
    let app_data_root = cache_root.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "native app data root is unavailable",
        )
    })?;
    require_directory(app_data_root)?;
    Ok(app_data_root.to_path_buf())
}

fn validate_operation_id(operation_id: &str) -> io::Result<()> {
    let mut bytes = operation_id.bytes();
    let valid_start = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric());
    if !valid_start
        || operation_id.len() > MAX_OPERATION_ID_LENGTH
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "operation ID must start with an ASCII letter or digit and contain only letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn prepare_staging_directory<Result>(
    backup_root: &Path,
    staging_directory: &Path,
    prepare: impl FnOnce(&Path) -> io::Result<Result>,
) -> io::Result<Result> {
    create_private_directory(staging_directory)?;
    let result = sync_directory(backup_root).and_then(|()| prepare(staging_directory));
    match result {
        Ok(value) => Ok(value),
        Err(primary_error) => Err(report_cleanup_failure(primary_error, || {
            cleanup_staging_directory(backup_root, staging_directory)
        })),
    }
}

#[cfg(unix)]
fn cleanup_staging_directory(backup_root: &Path, staging_directory: &Path) -> io::Result<()> {
    match fs::remove_dir_all(staging_directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    sync_directory(backup_root)
}

fn report_cleanup_failure(
    primary_error: io::Error,
    cleanup: impl FnOnce() -> io::Result<()>,
) -> io::Error {
    match cleanup() {
        Ok(()) => primary_error,
        Err(cleanup_error) => io::Error::new(
            primary_error.kind(),
            format!("{primary_error}; cleanup failed: {cleanup_error}"),
        ),
    }
}

fn verify_staged_bytes(path: &Path, expected: &[u8], expected_sha256: &str) -> io::Result<()> {
    let readback = fs::read(path)?;
    if readback.len() != expected.len() || digest(&readback) != expected_sha256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "stored configuration backup failed integrity verification",
        ));
    }
    Ok(())
}

fn validate_backup_context(context: &ConfigurationBackupContext<'_>) -> io::Result<()> {
    let expected_suffix = format!("/{}/files/retroarch.cfg", context.package_id);
    if context.operation != "replace"
        || context.device_handle.is_empty()
        || context.package_id.is_empty()
        || context.package_id.contains('/')
        || context.package_id.contains('\n')
        || !context.target_path.starts_with('/')
        || !context.target_path.ends_with(&expected_suffix)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration backup identity does not match a replacement target",
        ));
    }
    Ok(())
}

fn current_unix_time_millis() -> io::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .as_millis();
    u64::try_from(millis).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "backup timestamp is out of range",
        )
    })
}

fn require_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "app-data root must be a directory",
        ))
    }
}

fn ensure_private_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "configuration backup path must be a directory",
                ));
            }
            set_private_directory_permissions(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_private_directory(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(path)?;
                    if !metadata.file_type().is_dir() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "configuration backup path must be a directory",
                        ));
                    }
                    set_private_directory_permissions(path)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)?;
    if let Err(primary_error) = set_private_directory_permissions(path) {
        return Err(report_cleanup_failure(primary_error, || {
            fs::remove_dir(path)?;
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }));
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn create_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(all(test, unix))]
mod tests {
    use super::{
        app_data_root_from_cache_root, prepare_staging_directory, report_cleanup_failure,
        store_configuration_backup, verify_staged_bytes, ConfigurationBackupContext,
    };
    use std::fs;
    use std::io::{self, ErrorKind, Write};
    use tempfile::tempdir;

    const ORIGINAL_CONFIG: &[u8] = b"abc";
    const ORIGINAL_SHA256: &str =
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn backup_context() -> ConfigurationBackupContext<'static> {
        ConfigurationBackupContext {
            device_handle: "device-123",
            runtime_generation: 7,
            attempt_sequence: 2,
            package_id: "com.retroarch.aarch64",
            target_path:
                "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg",
            operation: "replace",
        }
    }

    #[test]
    fn app_data_root_is_derived_only_from_the_native_artifact_cache() {
        let root = tempdir().unwrap();
        let cache_root = root.path().join("artifact-cache");
        fs::create_dir(&cache_root).unwrap();

        assert_eq!(
            app_data_root_from_cache_root(&cache_root).unwrap(),
            root.path()
        );

        let other_cache = root.path().join("cache");
        fs::create_dir(&other_cache).unwrap();
        assert_eq!(
            app_data_root_from_cache_root(&other_cache)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn stores_original_bytes_with_operation_metadata() {
        let root = tempdir().unwrap();
        let context = ConfigurationBackupContext {
            device_handle: "device-123",
            runtime_generation: 7,
            attempt_sequence: 2,
            package_id: "com.retroarch.aarch64",
            target_path:
                "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg",
            operation: "replace",
        };
        let backup =
            store_configuration_backup(root.path(), "op-001", &context, ORIGINAL_CONFIG).unwrap();

        assert_eq!(fs::read(&backup.path).unwrap(), ORIGINAL_CONFIG);
        assert_eq!(backup.operation_id, "op-001");
        assert_eq!(backup.sha256, ORIGINAL_SHA256);
        assert_eq!(backup.length, 3);
        assert_eq!(backup.path.parent(), backup.metadata_path.parent());
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(&backup.metadata_path).unwrap()).unwrap();
        assert_eq!(metadata["version"], 2);
        assert_eq!(metadata["operationId"], "op-001");
        assert_eq!(metadata["operation"], "replace");
        assert_eq!(metadata["deviceHandle"], "device-123");
        assert_eq!(metadata["runtimeGeneration"], 7);
        assert_eq!(metadata["attemptSequence"], 2);
        assert_eq!(metadata["packageId"], "com.retroarch.aarch64");
        assert_eq!(
            metadata["targetPath"],
            "/storage/emulated/0/Android/data/com.retroarch.aarch64/files/retroarch.cfg"
        );
        assert!(metadata["capturedAtUnixMs"].as_u64().unwrap() > 0);
        assert_eq!(metadata["originalSha256"], ORIGINAL_SHA256);
        assert_eq!(metadata["originalLength"], 3);
    }

    #[test]
    fn rejects_operation_ids_that_are_not_safe_path_components() {
        let root = tempdir().unwrap();

        for operation_id in ["", "../escape", "nested/id", ".hidden", "bad id", "ümlaut"] {
            let error = store_configuration_backup(
                root.path(),
                operation_id,
                &backup_context(),
                ORIGINAL_CONFIG,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidInput, "{operation_id:?}");
        }
    }

    #[test]
    fn refuses_to_overwrite_an_existing_operation_backup() {
        let root = tempdir().unwrap();
        let first =
            store_configuration_backup(root.path(), "op-001", &backup_context(), ORIGINAL_CONFIG)
                .unwrap();
        let original_metadata = fs::read(&first.metadata_path).unwrap();

        let error =
            store_configuration_backup(root.path(), "op-001", &backup_context(), b"replacement")
                .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&first.path).unwrap(), ORIGINAL_CONFIG);
        assert_eq!(fs::read(&first.metadata_path).unwrap(), original_metadata);
    }

    #[test]
    fn removes_staging_files_after_a_partial_original_write_failure() {
        let root = tempdir().unwrap();
        let backup_root = root.path().join("configuration-backups");
        fs::create_dir(&backup_root).unwrap();
        let staging_directory = backup_root.join(".staging-op-partial");

        let error =
            prepare_staging_directory(&backup_root, &staging_directory, |staging_directory| {
                let mut file = super::create_private_file(&staging_directory.join("original.cfg"))?;
                file.write_all(b"partial")?;
                Err::<(), _>(io::Error::new(
                    ErrorKind::WriteZero,
                    "injected partial original write failure",
                ))
            })
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::WriteZero);
        assert!(error
            .to_string()
            .contains("injected partial original write failure"));
        assert!(!staging_directory.exists());
        assert!(!backup_root.join("op-partial").exists());
    }

    #[test]
    fn removes_staging_files_after_readback_verification_failure() {
        let root = tempdir().unwrap();
        let backup_root = root.path().join("configuration-backups");
        fs::create_dir(&backup_root).unwrap();
        let staging_directory = backup_root.join(".staging-op-readback");

        let error =
            prepare_staging_directory(&backup_root, &staging_directory, |staging_directory| {
                let staged_path = staging_directory.join("original.cfg");
                fs::write(&staged_path, b"corrupted")?;
                verify_staged_bytes(&staged_path, ORIGINAL_CONFIG, ORIGINAL_SHA256)
            })
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("failed integrity verification"));
        assert!(!staging_directory.exists());
        assert!(!backup_root.join("op-readback").exists());
    }

    #[test]
    fn preserves_the_primary_error_and_reports_failed_staging_cleanup() {
        let error =
            report_cleanup_failure(io::Error::other("primary backup write failure"), || {
                Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "injected staging removal failure",
                ))
            });

        assert_eq!(error.kind(), ErrorKind::Other);
        assert!(error.to_string().contains("primary backup write failure"));
        assert!(error.to_string().contains("cleanup failed"));
        assert!(error
            .to_string()
            .contains("injected staging removal failure"));
    }

    #[cfg(unix)]
    #[test]
    fn creates_private_backup_directories_and_files() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempdir().unwrap();
        let backup =
            store_configuration_backup(root.path(), "op-001", &backup_context(), ORIGINAL_CONFIG)
                .unwrap();
        let operation_directory = backup.path.parent().unwrap();
        let backup_root = operation_directory.parent().unwrap();

        assert_eq!(
            fs::metadata(backup_root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(operation_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&backup.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&backup.metadata_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
