pub mod commands;
pub mod python_bridge;
pub mod sidecar_client;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(sidecar_client::SidecarState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_step_specs,
            commands::open_recipe,
            commands::validate_recipe_path,
            commands::emit_recipe_yaml_from_path,
            commands::sidecar_status,
            commands::sidecar_list_step_specs,
            commands::sidecar_open_recipe,
            commands::sidecar_get_document,
            commands::sidecar_apply_recipe_command,
            commands::sidecar_undo,
            commands::sidecar_redo,
            commands::sidecar_save_recipe,
            commands::sidecar_save_recipe_as,
            commands::sidecar_validate,
            commands::sidecar_emit_yaml,
            commands::sidecar_get_ref_index
        ])
        .run(tauri::generate_context!())
        .expect("error while running EmuChef Config Editor");
}
