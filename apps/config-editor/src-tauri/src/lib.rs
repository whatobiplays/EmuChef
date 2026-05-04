pub mod commands;
pub mod python_bridge;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_step_specs,
            commands::open_recipe,
            commands::validate_recipe_path,
            commands::emit_recipe_yaml_from_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running EmuChef Config Editor");
}
