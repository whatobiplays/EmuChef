pub mod commands;
pub mod menu;
pub mod sidecar_client;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(sidecar_client::SidecarState::new(
            sidecar_client::SidecarRuntime::for_current_process(),
        ))
        .menu(|app| menu::build_editor_menu(app, menu::EditorMenuState::default()))
        .on_menu_event(|app, event| {
            menu::handle_menu_event(app, event.id().as_ref());
        })
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
            commands::sidecar_get_ref_index,
            commands::sidecar_set_document_authored_root,
            menu::update_menu_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running EmuChef Config Editor");
}

#[cfg(test)]
mod capability_tests {
    use std::fs;
    use std::path::Path;

    #[test]
    fn default_capability_grants_only_required_window_close_permissions() {
        let capability_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json");
        let capability_json = fs::read_to_string(capability_path)
            .expect("default Tauri capability should be readable");
        let capability: serde_json::Value = serde_json::from_str(&capability_json)
            .expect("default Tauri capability should be JSON");

        let permissions = capability["permissions"]
            .as_array()
            .expect("default Tauri capability should list permissions");

        assert!(
            permissions
                .iter()
                .any(|permission| permission == "core:window:allow-destroy"),
            "Tauri onCloseRequested closes allowed windows through its internal destroy path"
        );
        assert!(
            permissions
                .iter()
                .any(|permission| permission == "core:window:allow-close"),
            "confirmed dirty-close handling reissues window.close through a guarded frontend path"
        );

        for broad_permission in [
            "core:window:allow-create",
            "core:window:allow-get-all-windows",
            "core:window:allow-hide",
            "core:window:allow-show",
            "core:window:allow-maximize",
            "core:window:allow-minimize",
            "core:window:allow-set-title",
            "core:window:default",
        ] {
            assert!(
                !permissions
                    .iter()
                    .any(|permission| permission == broad_permission),
                "default capability should not add unrelated broad window permission {broad_permission}"
            );
        }
    }
}
