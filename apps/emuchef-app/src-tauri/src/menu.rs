//! Native document menu for saved end-user setups.

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Emitter, Manager, Runtime,
};

use crate::commands::AppState;
use crate::saved_configurations::recent_menu_entries;

const ACTION_NEW: &str = "newConfiguration";
const ACTION_OPEN: &str = "openConfiguration";
const ACTION_SAVE: &str = "saveConfiguration";
const ACTION_SAVE_AS: &str = "saveConfigurationAs";
const ACTION_IMPORT: &str = "importConfiguration";
const ACTION_EXPORT: &str = "exportConfiguration";
const ACTION_MANAGE: &str = "manageConfigurations";
const RECENT_PREFIX: &str = "openRecent:";

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedMenuState {
    runtime_ready: bool,
    command_blocked: bool,
    has_document: bool,
    dirty: bool,
    has_portable_intent: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MenuActionPayload {
    action: String,
    recent_handle: Option<String>,
}

#[tauri::command]
pub fn update_saved_configuration_menu(
    app: AppHandle,
    state: SavedMenuState,
) -> Result<(), String> {
    let menu = build_menu(&app, state).map_err(|error| error.to_string())?;
    app.set_menu(menu)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn build_menu<R: Runtime>(app: &AppHandle<R>, state: SavedMenuState) -> tauri::Result<Menu<R>> {
    #[cfg(target_os = "macos")]
    let app_menu = build_app_menu(app)?;
    let file_menu = build_file_menu(app, state)?;
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    Menu::with_items(
        app,
        &[
            #[cfg(target_os = "macos")]
            &app_menu,
            &file_menu,
            &edit_menu,
        ],
    )
}

pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let payload = if let Some(handle) = id.strip_prefix(RECENT_PREFIX) {
        MenuActionPayload {
            action: "openRecentConfiguration".to_string(),
            recent_handle: Some(handle.to_string()),
        }
    } else if matches!(
        id,
        ACTION_NEW
            | ACTION_OPEN
            | ACTION_SAVE
            | ACTION_SAVE_AS
            | ACTION_IMPORT
            | ACTION_EXPORT
            | ACTION_MANAGE
    ) {
        MenuActionPayload {
            action: id.to_string(),
            recent_handle: None,
        }
    } else {
        return;
    };
    let _ = app.emit("saved-configuration-menu-action", payload);
}

#[cfg(target_os = "macos")]
fn build_app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let package = app.package_info();
    Submenu::with_items(
        app,
        package.name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(
                app,
                None,
                Some(AboutMetadata {
                    name: Some(package.name.clone()),
                    version: Some(package.version.to_string()),
                    credits: Some(
                        "EmuChef helps prepare supported Android handhelds.\n\nLicensed under the GNU General Public License v3.0."
                            .to_string(),
                    ),
                    ..Default::default()
                }),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )
}

fn build_file_menu<R: Runtime>(
    app: &AppHandle<R>,
    state: SavedMenuState,
) -> tauri::Result<Submenu<R>> {
    let ready = state.runtime_ready && !state.command_blocked;
    let recent_items = app
        .try_state::<AppState>()
        .and_then(|state| recent_menu_entries(&state).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|entry| {
            MenuItem::with_id(
                app,
                format!("{RECENT_PREFIX}{}", entry.recent_handle),
                entry.label,
                ready && entry.available,
                None::<&str>,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut recent_refs = recent_items
        .iter()
        .map(|item| item as &dyn tauri::menu::IsMenuItem<R>)
        .collect::<Vec<_>>();
    let empty_recent;
    if recent_refs.is_empty() {
        empty_recent = MenuItem::with_id(
            app,
            "noRecentConfigurations",
            "No Recent Setups",
            false,
            None::<&str>,
        )?;
        recent_refs.push(&empty_recent);
    }
    let open_recent = Submenu::with_items(app, "Open Recent", true, &recent_refs)?;
    Submenu::with_items(
        app,
        "File",
        true,
        &[
            &MenuItem::with_id(app, ACTION_NEW, "New", ready, Some("CmdOrCtrl+N"))?,
            &MenuItem::with_id(app, ACTION_OPEN, "Open…", ready, Some("CmdOrCtrl+O"))?,
            &open_recent,
            &MenuItem::with_id(
                app,
                ACTION_MANAGE,
                "Manage Saved Setups…",
                ready,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                ACTION_SAVE,
                "Save",
                ready && state.has_portable_intent && (!state.has_document || state.dirty),
                Some("CmdOrCtrl+S"),
            )?,
            &MenuItem::with_id(
                app,
                ACTION_SAVE_AS,
                "Save As…",
                ready && state.has_portable_intent,
                Some("CmdOrCtrl+Shift+S"),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, ACTION_IMPORT, "Import…", ready, None::<&str>)?,
            &MenuItem::with_id(
                app,
                ACTION_EXPORT,
                "Export…",
                ready && state.has_document,
                None::<&str>,
            )?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::separator(app)?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )
}
