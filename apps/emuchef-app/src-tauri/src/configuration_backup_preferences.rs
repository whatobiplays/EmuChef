use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use uuid::Uuid;

const PREFERENCE_FILE_NAME: &str = "configuration-backup-preference.v1";
const PERSISTENT_BACKUP_OPT_OUT_MESSAGE: &str = "Disable private backups for future RetroArch replacements?\n\nWithout a verified private backup, EmuChef may not be able to restore custom settings or comments if a replacement fails or is interrupted. This setting persists for future reviews. No configuration will be written now.\n\nContinue?";
const PREFERENCE_VERSION: u8 = 1;
static PREFERENCE_ADMISSION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PreferenceRecord {
    version: u8,
    enabled: bool,
}

#[tauri::command]
/// Read the saved setting that controls private backups before configuration replacement.
pub fn get_configuration_backup_preference(app: AppHandle) -> Result<bool, String> {
    let app_data_root = app
        .path()
        .app_data_dir()
        .map_err(|_| "Application settings are unavailable.".to_string())?;
    read_preference(&app_data_root)
}

#[tauri::command]
/// Save whether configuration replacement should create a private local backup.
pub async fn set_configuration_backup_preference(
    app: AppHandle,
    enabled: bool,
) -> Result<bool, String> {
    let app_data_root = app
        .path()
        .app_data_dir()
        .map_err(|_| "Application settings are unavailable.".to_string())?;
    if !enabled && !confirm_persistent_backup_opt_out(&app).await? {
        return Err(
            "The backup preference was not changed because confirmation was cancelled.".to_string(),
        );
    }
    write_preference(&app_data_root, enabled)?;
    Ok(enabled)
}

async fn confirm_persistent_backup_opt_out(app: &AppHandle) -> Result<bool, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    app.dialog()
        .message(PERSISTENT_BACKUP_OPT_OUT_MESSAGE)
        .title("Disable private configuration backups")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancel)
        .show(move |confirmed| {
            let _ = sender.send(confirmed);
        });
    tauri::async_runtime::spawn_blocking(move || receiver.recv())
        .await
        .map_err(|_| "Backup preference confirmation failed.".to_string())?
        .map_err(|_| "Backup preference confirmation was unavailable.".to_string())
}

pub(crate) fn read_preference(app_data_root: &Path) -> Result<bool, String> {
    ensure_app_data_directory(app_data_root).map_err(|_| {
        "Application settings could not be checked safely. Backup preference is unavailable."
            .to_string()
    })?;
    let path = preference_path(app_data_root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(_) => {
            return Err(
                "Application settings could not be read. Backup preference is unavailable."
                    .to_string(),
            )
        }
    };
    if !metadata.file_type().is_file() {
        return Err("Application settings file is not a regular file.".to_string());
    }
    validate_private_mode(&metadata).map_err(|_| {
        "Application settings file permissions are unsafe. Backup preference is unavailable."
            .to_string()
    })?;
    if metadata.len() > 128 {
        return Err("Application settings file is invalid.".to_string());
    }
    let bytes = fs::read(path).map_err(|_| {
        "Application settings could not be read. Backup preference is unavailable.".to_string()
    })?;
    let record: PreferenceRecord = serde_json::from_slice(&bytes).map_err(|_| {
        "Application settings are invalid. Backup preference is unavailable.".to_string()
    })?;
    if record.version != PREFERENCE_VERSION {
        return Err("Application settings use an unsupported version.".to_string());
    }
    Ok(record.enabled)
}

/// Identifies whether admission failed before or during the real start path.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ReviewedPreferenceError<E> {
    PreferenceUnavailableOrStale,
    ExecutionStart(E),
}

/// Check the reviewed preference and record a short admission transition while
/// updates are excluded. The closure must not perform sidecar, device, or other
/// long-running work; execution starts only after this function releases the
/// preference lock.
pub(crate) fn with_reviewed_preference<T>(
    app_data_root: &Path,
    reviewed_enabled: bool,
    start_invocation: impl FnOnce() -> Result<T, String>,
) -> Result<T, ReviewedPreferenceError<String>> {
    let admission = PREFERENCE_ADMISSION_LOCK
        .lock()
        .map_err(|_| ReviewedPreferenceError::PreferenceUnavailableOrStale)?;
    let current_enabled = read_preference(app_data_root)
        .map_err(|_| ReviewedPreferenceError::PreferenceUnavailableOrStale)?;
    if current_enabled != reviewed_enabled {
        return Err(ReviewedPreferenceError::PreferenceUnavailableOrStale);
    }
    let result = start_invocation().map_err(ReviewedPreferenceError::ExecutionStart);
    drop(admission);
    result
}

fn write_preference(app_data_root: &Path, enabled: bool) -> Result<(), String> {
    let _admission = PREFERENCE_ADMISSION_LOCK
        .lock()
        .map_err(|_| "Backup preference update is unavailable.".to_string())?;
    ensure_app_data_directory(app_data_root)
        .map_err(|_| "Application settings could not be stored safely.".to_string())?;
    let path = preference_path(app_data_root);
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.file_type().is_file() {
            return Err("Application settings file is not a regular file.".to_string());
        }
        validate_private_mode(&metadata)
            .map_err(|_| "Application settings file permissions are unsafe.".to_string())?;
    }

    let staging_path = app_data_root.join(format!(
        ".{PREFERENCE_FILE_NAME}.{}.tmp",
        Uuid::new_v4().simple()
    ));
    let record = PreferenceRecord {
        version: PREFERENCE_VERSION,
        enabled,
    };
    let bytes = serde_json::to_vec(&record)
        .map_err(|_| "Application settings could not be encoded.".to_string())?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&staging_path)
            .map_err(|_| "Application settings could not be stored safely.".to_string())?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| "Application settings could not be synchronized.".to_string())?;
        drop(file);
        fs::rename(&staging_path, &path)
            .map_err(|_| "Application settings could not be updated atomically.".to_string())?;
        sync_directory(app_data_root)
            .map_err(|_| "Application settings could not be synchronized.".to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(staging_path);
    }
    result
}

fn preference_path(app_data_root: &Path) -> PathBuf {
    app_data_root.join(PREFERENCE_FILE_NAME)
}

#[cfg(test)]
pub(crate) fn set_preference_for_test(app_data_root: &Path, enabled: bool) -> Result<(), String> {
    write_preference(app_data_root, enabled)
}

pub(crate) fn bind_to_plan_payload(
    payload: &mut serde_json::Value,
    enabled: bool,
) -> Result<(), String> {
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "Configuration plan request is invalid.".to_string())?;
    object.insert(
        "configurationBackupEnabled".to_string(),
        serde_json::Value::Bool(enabled),
    );
    Ok(())
}

pub(crate) fn bind_backup_waiver_to_plan_payload(
    payload: &mut serde_json::Value,
    digest: &str,
) -> Result<(), String> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Configuration backup waiver is invalid.".to_string());
    }
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "Configuration plan request is invalid.".to_string())?;
    object.insert(
        "configurationBackupWaiverDigest".to_string(),
        serde_json::Value::String(digest.to_string()),
    );
    Ok(())
}

fn ensure_app_data_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => return Ok(()),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "application data path is not a directory",
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "application data path is not a directory",
        ));
    }
    Ok(())
}

fn validate_private_mode(metadata: &fs::Metadata) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "application settings file is not private",
            ));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private settings storage is unavailable on this platform",
        ));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{
        bind_to_plan_payload, read_preference, with_reviewed_preference, write_preference,
        ReviewedPreferenceError, PREFERENCE_ADMISSION_LOCK,
    };

    static PREFERENCE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn native_preference_overwrites_request_supplied_backup_mode() {
        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        let mut payload = serde_json::json!({ "configurationBackupEnabled": true });

        bind_to_plan_payload(&mut payload, false).unwrap();

        assert!(!payload["configurationBackupEnabled"].as_bool().unwrap());
    }

    #[test]
    fn missing_preference_defaults_to_private_backup_enabled() {
        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        let root = TempDir::new().unwrap();

        assert!(read_preference(root.path()).unwrap());
    }

    #[test]
    fn preference_survives_a_new_read_and_can_be_reenabled() {
        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        let root = TempDir::new().unwrap();

        write_preference(root.path(), false).unwrap();
        assert!(!read_preference(root.path()).unwrap());

        write_preference(root.path(), true).unwrap();
        assert!(read_preference(root.path()).unwrap());
    }

    #[test]
    fn preference_change_before_final_admission_transition_rejects_stale_review() {
        use std::{sync::mpsc, thread};

        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        let root = TempDir::new().unwrap();
        write_preference(root.path(), true).unwrap();
        let path = root.path().to_path_buf();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let admission = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            continue_rx.recv().unwrap();
            with_reviewed_preference(&path, true, || Ok("admitted"))
        });
        ready_rx.recv().unwrap();

        write_preference(root.path(), false).unwrap();
        continue_tx.send(()).unwrap();
        assert_eq!(
            admission.join().unwrap(),
            Err(ReviewedPreferenceError::PreferenceUnavailableOrStale)
        );
    }

    #[test]
    fn final_admission_transition_is_atomic_with_preference_check() {
        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        let root = TempDir::new().unwrap();
        write_preference(root.path(), true).unwrap();

        let admitted_mode = with_reviewed_preference(root.path(), true, || {
            assert!(
                PREFERENCE_ADMISSION_LOCK.try_lock().is_err(),
                "the final admission transition must run under the preference gate"
            );
            Ok(true)
        })
        .unwrap();

        write_preference(root.path(), false).unwrap();
        assert!(!read_preference(root.path()).unwrap());
        assert!(
            admitted_mode,
            "later preference updates apply to later reviews"
        );
    }

    #[test]
    fn admission_failure_keeps_its_execution_classification() {
        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        let root = TempDir::new().unwrap();
        write_preference(root.path(), true).unwrap();
        let start_error =
            "execution_dispatch_uncertain: sidecar acknowledgment missing".to_string();

        let result = with_reviewed_preference::<()>(root.path(), true, || {
            assert!(PREFERENCE_ADMISSION_LOCK.try_lock().is_err());
            Err(start_error.clone())
        });

        assert_eq!(
            result,
            Err(ReviewedPreferenceError::ExecutionStart(start_error))
        );
    }

    #[test]
    fn persistent_opt_out_confirmation_discloses_recovery_consequences() {
        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        for expected in [
            "future RetroArch replacements",
            "Without a verified private backup",
            "restore custom settings or comments",
            "replacement fails or is interrupted",
            "persists for future reviews",
            "No configuration will be written now",
        ] {
            assert!(
                super::PERSISTENT_BACKUP_OPT_OUT_MESSAGE.contains(expected),
                "missing {expected:?} from {:?}",
                super::PERSISTENT_BACKUP_OPT_OUT_MESSAGE
            );
        }
    }

    #[test]
    fn malformed_preference_fails_closed_instead_of_disabling_backup() {
        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        let root = TempDir::new().unwrap();
        fs::write(
            root.path().join("configuration-backup-preference.v1"),
            b"{not-json",
        )
        .unwrap();

        assert!(read_preference(root.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn preference_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = PREFERENCE_TEST_LOCK.lock().unwrap();
        let root = TempDir::new().unwrap();
        write_preference(root.path(), false).unwrap();
        let mode = fs::metadata(root.path().join("configuration-backup-preference.v1"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(mode, 0o600);
    }
}
