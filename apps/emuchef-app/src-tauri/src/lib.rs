mod adb;
mod catalog;
mod commands;
mod execution;
mod handles;
mod qualification;
mod recovery;
mod saved_configurations;
mod sidecar;
mod support;
mod updates;

use std::sync::Mutex;

use tauri::Manager;

pub fn run() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let qualification_probe = qualification::requested(&arguments);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let app_data = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("Application data directory is unavailable: {error}"))?;
            let cache_root = app_data.join("artifact-cache");
            let sidecar = sidecar::SidecarState::new(cache_root.clone());
            sidecar.initialize();
            let catalog = catalog::CatalogDescriptor::resolve(app.handle());
            if qualification_probe {
                let (report, exit_code) = match catalog.as_ref() {
                    Ok(catalog) => match qualification::run(&sidecar, catalog) {
                        Ok(report) => (report, 0),
                        Err(code) => (qualification::failure_report(code), 1),
                    },
                    Err(_) => (qualification::failure_report("catalog_unavailable"), 1),
                };
                println!(
                    "{}",
                    serde_json::to_string(&report)
                        .unwrap_or_else(|_| "{\"kind\":\"macos_packaged_app_qualification\",\"status\":\"failed\",\"code\":\"qualification_failed\"}".to_string())
                );
                app.handle().exit(exit_code);
                return Ok(());
            }
            app.manage(commands::AppState {
                sidecar,
                catalog,
                adb: Mutex::new(adb::AdbManager::new(app_data.join("platform-tools"))),
                platform_tools_selections: Mutex::new(
                    commands::PlatformToolsSelectionStore::default(),
                ),
                handles: Mutex::new(handles::SessionHandles::default()),
                executions: Mutex::new(execution::ExecutionHandleStore::default()),
                saved_configurations: Mutex::new(
                    saved_configurations::SavedConfigurationStore::load(
                        app_data.join("recent-configurations.json"),
                    ),
                ),
                recovery: Mutex::new(recovery::RecoveryStore::load(
                    app_data.join("recovery-draft.json"),
                    app_data.join("session-active.marker"),
                )),
                support: Mutex::new(support::SupportStore::new(cache_root)),
                updates: updates::UpdateService::from_production_document()?,
                update_activity: updates::ActivityGate::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime_status,
            commands::begin_app_session,
            recovery::stage_recovery_draft,
            recovery::defer_recovery_draft,
            recovery::restore_recovery_draft,
            recovery::discard_recovery_draft,
            recovery::finish_app_session,
            commands::restart_runtime,
            commands::get_catalog,
            commands::get_adb_setup_status,
            commands::open_platform_tools_download_page,
            commands::pick_platform_tools_zip,
            commands::install_platform_tools_selection,
            commands::remove_platform_tools,
            commands::poll_devices,
            commands::probe_device,
            commands::match_device,
            commands::describe_configuration,
            commands::create_review,
            commands::discard_review,
            commands::get_review_status,
            commands::pick_input_path,
            saved_configurations::list_recent_configurations,
            saved_configurations::create_saved_configuration,
            saved_configurations::open_saved_configuration,
            saved_configurations::open_recent_configuration,
            saved_configurations::relink_recent_configuration,
            saved_configurations::remove_recent_configuration,
            saved_configurations::update_saved_configuration,
            saved_configurations::save_saved_configuration,
            saved_configurations::save_saved_configuration_as,
            saved_configurations::close_saved_configuration,
            execution::start_simulated_execution,
            execution::get_simulated_execution,
            execution::get_simulated_execution_events,
            execution::cancel_simulated_execution,
            execution::get_real_execution_availability,
            execution::start_real_execution,
            execution::get_real_execution,
            execution::get_real_execution_events,
            execution::cancel_real_execution,
            execution::export_execution_report,
            execution::launch_configured_app,
            support::get_cache_inventory,
            support::cleanup_cache,
            support::export_support_diagnostics,
            updates::get_update_status,
            updates::check_for_updates,
            updates::begin_update_interaction_session,
            updates::set_update_interaction_state,
            updates::end_update_interaction_session,
            updates::open_update_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running EmuChef");
}
