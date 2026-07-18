pub mod app_generator;
pub mod app_sources;
pub mod commands;
pub mod device_profile_generator;
pub mod menu;
pub mod sidecar_client;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(sidecar_client::SidecarState::new(
            sidecar_client::SidecarRuntime::for_current_process(),
        ))
        .manage(device_profile_generator::DeviceProfileGeneratorState::default())
        .manage(app_generator::AppGeneratorState::default())
        .menu(|app| menu::build_editor_menu(app, menu::EditorMenuState::default()))
        .on_menu_event(|app, event| {
            menu::handle_menu_event(app, event.id().as_ref());
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_step_specs,
            commands::open_recipe,
            commands::validate_recipe_path,
            commands::emit_recipe_yaml_from_path,
            commands::open_user_configuration,
            commands::create_user_configuration,
            commands::get_user_configuration_document,
            commands::save_user_configuration,
            commands::save_user_configuration_as,
            commands::set_user_configuration_binding,
            commands::remove_user_configuration_binding,
            commands::set_user_configuration_selected_recipes,
            commands::set_user_configuration_device_plan,
            commands::validate_user_configuration,
            commands::emit_user_configuration_yaml,
            commands::set_user_configuration_authored_root,
            commands::close_user_configuration,
            commands::describe_configuration,
            commands::plan_configuration,
            commands::sidecar_status,
            commands::sidecar_ping,
            commands::sidecar_restart,
            app_generator::get_config_editor_authored_root,
            app_generator::set_config_editor_authored_root,
            app_generator::begin_app_generator,
            app_generator::choose_app_generator_apk,
            app_generator::choose_app_generator_authored_root,
            app_generator::analyze_app_generator_source,
            app_generator::download_app_generator_remote_apk,
            app_generator::set_app_generator_authored_root,
            app_generator::inspect_app_generator_apk,
            app_generator::generate_app_recipe_draft,
            app_generator::generate_remote_app_recipe_draft,
            app_generator::check_app_recipe_collisions,
            app_generator::save_generated_app_recipe,
            app_generator::save_generated_remote_app_recipe,
            app_generator::cancel_app_generator,
            device_profile_generator::begin_device_profile_generator,
            device_profile_generator::choose_device_profile_authored_root,
            device_profile_generator::set_device_profile_authored_root,
            device_profile_generator::list_device_profile_generator_devices,
            device_profile_generator::probe_device_profile_generator_device,
            device_profile_generator::generate_device_profile_draft,
            device_profile_generator::check_device_profile_collisions,
            device_profile_generator::save_generated_device_profile,
            device_profile_generator::cancel_device_profile_generator,
            commands::get_document,
            commands::apply_recipe_command,
            commands::undo,
            commands::redo,
            commands::save_recipe,
            commands::save_recipe_as,
            commands::validate,
            commands::emit_yaml,
            commands::get_ref_index,
            commands::set_document_authored_root,
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
