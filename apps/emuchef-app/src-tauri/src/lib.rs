mod adb;
mod catalog;
mod commands;
mod execution;
mod handles;
mod sidecar;

use std::sync::Mutex;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let sidecar = sidecar::SidecarState::new();
            sidecar.initialize();
            let catalog = catalog::CatalogDescriptor::resolve(app.handle());
            let app_data = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("Application data directory is unavailable: {error}"))?;
            app.manage(commands::AppState {
                sidecar,
                catalog,
                adb: Mutex::new(adb::AdbManager::new(app_data.join("platform-tools"))),
                handles: Mutex::new(handles::SessionHandles::default()),
                executions: Mutex::new(execution::ExecutionHandleStore::default()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime_status,
            commands::get_catalog,
            commands::get_adb_setup_status,
            commands::open_platform_tools_download_page,
            commands::import_platform_tools_zip,
            commands::remove_platform_tools,
            commands::poll_devices,
            commands::probe_device,
            commands::match_device,
            commands::describe_configuration,
            commands::create_review,
            commands::discard_review,
            commands::get_review_status,
            commands::pick_input_path,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running EmuChef");
}
