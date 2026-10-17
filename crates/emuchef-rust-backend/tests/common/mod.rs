//! Shared deterministic test state for backend integration binaries.

use std::fs;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use emuchef_rust_backend::session::DocumentSessionManager;

static CATALOG_STORAGE: OnceLock<std::path::PathBuf> = OnceLock::new();
static APPLICATION_DATA: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Configure an isolated application-data directory for tests that exercise
/// production entry points whose journal location defaults from the process
/// environment. The process-wide value is initialized once and shared by all
/// tests in the binary.
pub fn test_application_data() -> &'static std::path::Path {
    APPLICATION_DATA.get_or_init(|| {
        let executable = std::env::current_exe().expect("test executable path");
        let executable_name = executable
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("integration-tests");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "emuchef-catalog-integration-data-{executable_name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("isolated application-data directory");
        std::env::set_var("HOME", &path);
        std::env::set_var("XDG_DATA_HOME", path.join("xdg-data"));
        #[cfg(target_os = "windows")]
        std::env::set_var("LOCALAPPDATA", &path);
        path
    })
}

/// Create a session manager whose durable catalog state is isolated from the
/// developer's configured application-data directory.
pub fn test_sessions() -> DocumentSessionManager {
    test_application_data();
    let storage = CATALOG_STORAGE.get_or_init(|| {
        let executable = std::env::current_exe().expect("test executable path");
        let executable_name = executable
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("integration-tests");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        executable
            .parent()
            .expect("test executable directory")
            .join(format!(
                "catalog-integration-state-{executable_name}-{}-{nonce}",
                std::process::id()
            ))
    });
    DocumentSessionManager::with_catalog_storage(storage)
}
