use serde::{Deserialize, Serialize};
use tauri::{
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Emitter, Runtime,
};

const ACTION_OPEN_RECIPE: &str = "openRecipe";
const ACTION_OPEN_USER_CONFIGURATION: &str = "openUserConfiguration";
const ACTION_GENERATE_APP_RECIPE: &str = "generateAppRecipe";
const ACTION_GENERATE_DEVICE_PROFILE: &str = "generateDeviceProfile";
const ACTION_SAVE_RECIPE: &str = "saveRecipe";
const ACTION_SAVE_RECIPE_AS: &str = "saveRecipeAs";
const ACTION_RESTART_SIDECAR: &str = "restartSidecar";
const ACTION_UNDO: &str = "undo";
const ACTION_REDO: &str = "redo";
const ACTION_VALIDATE: &str = "validate";
const ACTION_REFRESH_YAML: &str = "refreshYaml";
const ACTION_SET_AUTHORED_ROOT: &str = "setAuthoredRoot";
const ACTION_CLEAR_AUTHORED_ROOT: &str = "clearAuthoredRoot";

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorMenuState {
    has_document: bool,
    has_selected_authored_root: bool,
    has_document_authored_root: bool,
    dirty: bool,
    can_undo: bool,
    can_redo: bool,
    command_in_flight: bool,
    document_session_valid: bool,
    backend_compatible: Option<bool>,
    sidecar_running: Option<bool>,
    generator_active: bool,
}

impl Default for EditorMenuState {
    fn default() -> Self {
        Self {
            has_document: false,
            has_selected_authored_root: false,
            has_document_authored_root: false,
            dirty: false,
            can_undo: false,
            can_redo: false,
            command_in_flight: false,
            document_session_valid: true,
            backend_compatible: None,
            sidecar_running: None,
            generator_active: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MenuActionPayload<'a> {
    action: &'a str,
}

#[tauri::command]
pub fn update_menu_state(app: AppHandle, state: EditorMenuState) -> Result<(), String> {
    let menu = build_editor_menu(&app, state).map_err(|err| err.to_string())?;
    app.set_menu(menu).map_err(|err| err.to_string())?;
    Ok(())
}

pub fn build_editor_menu<R: Runtime>(
    app: &AppHandle<R>,
    state: EditorMenuState,
) -> tauri::Result<Menu<R>> {
    #[cfg(target_os = "macos")]
    let app_menu = build_app_menu(app)?;
    let file_menu = build_file_menu(app, state)?;
    let edit_menu = build_edit_menu(app, state)?;
    let utilities_menu = build_utilities_menu(app, state)?;

    Menu::with_items(
        app,
        &[
            #[cfg(target_os = "macos")]
            &app_menu,
            &file_menu,
            &edit_menu,
            &utilities_menu,
            #[cfg(not(target_os = "macos"))]
            &build_help_menu(app)?,
        ],
    )
}

pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, action: &str) {
    match action {
        ACTION_OPEN_RECIPE
        | ACTION_OPEN_USER_CONFIGURATION
        | ACTION_GENERATE_APP_RECIPE
        | ACTION_GENERATE_DEVICE_PROFILE
        | ACTION_SAVE_RECIPE
        | ACTION_SAVE_RECIPE_AS
        | ACTION_RESTART_SIDECAR
        | ACTION_UNDO
        | ACTION_REDO
        | ACTION_VALIDATE
        | ACTION_REFRESH_YAML
        | ACTION_SET_AUTHORED_ROOT
        | ACTION_CLEAR_AUTHORED_ROOT => {
            if let Err(err) = app.emit("menu-action", MenuActionPayload { action }) {
                eprintln!("Failed to emit menu action {action}: {err}");
            }
        }
        _ => {}
    }
}

#[cfg(target_os = "macos")]
fn build_app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    let metadata = about_metadata(app);
    Submenu::with_items(
        app,
        app.package_info().name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(metadata))?,
            &PredefinedMenuItem::separator(app)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::services(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::hide(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::hide_others(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )
}

fn build_file_menu<R: Runtime>(
    app: &AppHandle<R>,
    state: EditorMenuState,
) -> tauri::Result<Submenu<R>> {
    let command_idle = !state.command_in_flight;
    let editor_idle = command_idle && !state.generator_active;
    let backend_ready = state.backend_compatible != Some(false);
    let backend_usable =
        state.backend_compatible == Some(true) && state.sidecar_running == Some(true);
    let session_ready = editor_idle && state.document_session_valid && backend_ready;
    let open_recipe_ready =
        editor_idle && backend_ready && (state.document_session_valid || backend_usable);
    let generator_ready = editor_idle && backend_usable;
    let has_root_to_clear = state.has_selected_authored_root
        || (state.has_document && state.has_document_authored_root);
    Submenu::with_items(
        app,
        "File",
        true,
        &[
            &MenuItem::with_id(
                app,
                ACTION_OPEN_RECIPE,
                "Open Recipe",
                open_recipe_ready,
                Some("CmdOrCtrl+O"),
            )?,
            &MenuItem::with_id(
                app,
                ACTION_OPEN_USER_CONFIGURATION,
                "Open User Configuration...",
                open_recipe_ready,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                ACTION_GENERATE_APP_RECIPE,
                "Generate App and Recipe...",
                generator_ready,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                ACTION_GENERATE_DEVICE_PROFILE,
                "Generate Device Profile...",
                generator_ready,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                ACTION_SET_AUTHORED_ROOT,
                "Set Authored Root...",
                session_ready,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                ACTION_CLEAR_AUTHORED_ROOT,
                "Clear Authored Root",
                session_ready && has_root_to_clear,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                ACTION_SAVE_RECIPE,
                "Save",
                session_ready && state.has_document && state.dirty,
                Some("CmdOrCtrl+S"),
            )?,
            &MenuItem::with_id(
                app,
                ACTION_SAVE_RECIPE_AS,
                "Save As…",
                session_ready && state.has_document,
                None::<&str>,
            )?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::separator(app)?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )
}

fn build_edit_menu<R: Runtime>(
    app: &AppHandle<R>,
    state: EditorMenuState,
) -> tauri::Result<Submenu<R>> {
    let document_ready = !state.command_in_flight
        && !state.generator_active
        && state.document_session_valid
        && state.backend_compatible != Some(false)
        && state.has_document;
    Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &MenuItem::with_id(
                app,
                ACTION_UNDO,
                "Undo",
                document_ready && state.can_undo,
                Some("CmdOrCtrl+Z"),
            )?,
            &MenuItem::with_id(
                app,
                ACTION_REDO,
                "Redo",
                document_ready && state.can_redo,
                Some("CmdOrCtrl+Shift+Z"),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )
}

fn build_utilities_menu<R: Runtime>(
    app: &AppHandle<R>,
    state: EditorMenuState,
) -> tauri::Result<Submenu<R>> {
    let restart_ready = !state.command_in_flight;
    let document_ready = !state.command_in_flight
        && !state.generator_active
        && state.document_session_valid
        && state.backend_compatible != Some(false)
        && state.has_document;
    Submenu::with_items(
        app,
        "Utilities",
        true,
        &[
            &MenuItem::with_id(
                app,
                ACTION_VALIDATE,
                "Validate",
                document_ready,
                Some("CmdOrCtrl+Shift+V"),
            )?,
            &MenuItem::with_id(
                app,
                ACTION_REFRESH_YAML,
                "Refresh YAML",
                document_ready,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                ACTION_RESTART_SIDECAR,
                "Restart Sidecar",
                restart_ready,
                None::<&str>,
            )?,
        ],
    )
}

#[cfg(not(target_os = "macos"))]
fn build_help_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "Help",
        true,
        &[&PredefinedMenuItem::about(
            app,
            None,
            Some(about_metadata(app)),
        )?],
    )
}

fn about_metadata<R: Runtime>(app: &AppHandle<R>) -> AboutMetadata<'static> {
    let package_info = app.package_info();
    let config = app.config();
    AboutMetadata {
        name: Some(package_info.name.clone()),
        version: Some(package_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config
            .bundle
            .publisher
            .clone()
            .map(|publisher| vec![publisher]),
        ..Default::default()
    }
}
