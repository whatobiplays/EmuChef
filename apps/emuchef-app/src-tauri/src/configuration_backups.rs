use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

/// Open the private backup directory for one completed configuration operation.
#[tauri::command]
pub fn open_configuration_backup_folder(
    app: AppHandle,
    operation_id: String,
) -> Result<(), String> {
    let app_data_root = app
        .path()
        .app_data_dir()
        .map_err(|_| "The private configuration backup is unavailable.".to_string())?;
    let backup_directory =
        emuchef_rust_backend::configuration_backups::configuration_backup_directory(
            &app_data_root,
            &operation_id,
        )
        .map_err(|_| "The private configuration backup is unavailable.".to_string())?;
    let original_file = std::fs::symlink_metadata(backup_directory.join("original.cfg"))
        .map_err(|_| "The private configuration backup is unavailable.".to_string())?;
    if !original_file.file_type().is_file() {
        return Err("The private configuration backup is unavailable.".to_string());
    }
    let path = backup_directory
        .to_str()
        .ok_or_else(|| "The private configuration backup path is unavailable.".to_string())?;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|_| "The private configuration backup folder could not be opened.".to_string())
}
