mod adb;
mod catalog;
mod commands;
mod device_qualification;
mod execution;
mod handles;
mod menu;
mod phase6d6_ui_smoke;
mod qualification;
// The gate API is introduced before the later qualification orchestration
// layer consumes it, so it is intentionally unused by the ordinary app flow.
#[allow(dead_code)]
pub(crate) mod qualification_build;
#[allow(dead_code)]
pub(crate) mod qualification_repository;
mod recovery;
mod saved_configurations;
mod sidecar;
mod support;
mod support_codes;
mod updates;

use std::sync::Mutex;

use tauri::Manager;

pub fn run() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let qualification_probe = qualification::requested(&arguments);
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .menu(|app| menu::build_menu(app, menu::SavedMenuState::default()))
        .on_menu_event(|app, event| menu::handle_menu_event(app, event.id().as_ref()))
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
                input_contracts: Mutex::new(commands::InputContractSnapshot::default()),
                handles: Mutex::new(handles::SessionHandles::default()),
                root_qualification: Mutex::new(
                    device_qualification::RootQualificationStore::default(),
                ),
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
            app.manage(phase6d6_ui_smoke::Phase6d6UiSmokeStore::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime_status,
            commands::begin_app_session,
            recovery::stage_recovery_draft,
            recovery::defer_recovery_draft,
            recovery::restore_recovery_draft,
            recovery::discard_recovery_draft,
            commands::restart_runtime,
            commands::get_catalog,
            commands::get_adb_setup_status,
            commands::open_platform_tools_download_page,
            commands::pick_platform_tools_zip,
            commands::install_platform_tools_selection,
            commands::remove_platform_tools,
            commands::poll_devices,
            commands::probe_device,
            device_qualification::check_device_root,
            commands::match_device,
            commands::describe_configuration,
            commands::create_review,
            commands::discard_review,
            commands::get_review_status,
            commands::pick_input_path,
            saved_configurations::list_recent_configurations,
            saved_configurations::create_saved_configuration,
            saved_configurations::open_saved_configuration,
            saved_configurations::preview_saved_configuration,
            saved_configurations::preview_recent_configuration,
            saved_configurations::confirm_saved_configuration_preview,
            saved_configurations::cancel_saved_configuration_preview,
            saved_configurations::compare_saved_configuration_preview,
            saved_configurations::apply_saved_configuration_preview_repair,
            saved_configurations::open_recent_configuration,
            saved_configurations::relink_recent_configuration,
            saved_configurations::remove_recent_configuration,
            saved_configurations::update_saved_configuration,
            saved_configurations::save_saved_configuration,
            saved_configurations::save_saved_configuration_as,
            saved_configurations::rename_saved_configuration,
            saved_configurations::duplicate_saved_configuration,
            saved_configurations::import_saved_configuration,
            saved_configurations::export_saved_configuration,
            saved_configurations::close_saved_configuration,
            execution::start_simulated_execution,
            execution::get_simulated_execution,
            execution::get_simulated_execution_events,
            execution::cancel_simulated_execution,
            execution::get_execution_capabilities,
            execution::start_real_execution,
            execution::get_real_execution,
            execution::get_real_execution_events,
            execution::cancel_real_execution,
            execution::export_execution_report,
            device_qualification::get_device_qualification,
            execution::launch_configured_app,
            support::get_cache_inventory,
            support::get_support_snapshot,
            support::cleanup_cache,
            support::reset_local_app_state,
            support::export_support_diagnostics,
            updates::get_update_status,
            updates::check_for_updates,
            updates::begin_update_interaction_session,
            updates::set_update_interaction_state,
            updates::end_update_interaction_session,
            updates::open_update_download,
            menu::update_saved_configuration_menu,
            phase6d6_ui_smoke::phase6d6_ui_smoke_status,
            phase6d6_ui_smoke::phase6d6_ui_smoke_load_projection,
            phase6d6_ui_smoke::phase6d6_ui_smoke_capture,
        ])
        .build(tauri::generate_context!())
        .expect("error while building EmuChef");

    app.run(|app_handle, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            if !finish_recovery_process_session(app_handle) {
                api.prevent_exit();
            }
        }
        tauri::RunEvent::Exit => {
            let _ = finish_recovery_process_session(app_handle);
        }
        _ => {}
    });
}

// A clean process shutdown may arrive through different Tauri lifecycle
// paths depending on platform and quit mechanism. Perform recovery cleanup
// from both ExitRequested and the final Exit event. The operation is
// idempotent, so repeated calls are safe.
fn finish_recovery_process_session(app_handle: &tauri::AppHandle) -> bool {
    let Some(state) = app_handle.try_state::<commands::AppState>() else {
        return true;
    };
    state
        .recovery
        .lock()
        .map_err(|_| ())
        .and_then(|mut recovery| recovery.finish_process_termination().map_err(|_| ()))
        .is_ok()
}
