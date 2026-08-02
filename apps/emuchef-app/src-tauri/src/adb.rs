//! Security boundary for user-imported Android SDK Platform-Tools.
//!
//! The application never downloads Platform-Tools. A user-selected macOS ZIP
//! is treated as untrusted input, inspected before extraction, reduced to three
//! retained files, authenticated, hashed, and only then installed under the
//! application data directory.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{self, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use async_io::{block_on, Timer};
use async_process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use futures_lite::future::{self, poll_fn};
use futures_lite::io::{AsyncRead, AsyncReadExt};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::ZipArchive;

pub const PLATFORM_TOOLS_URL: &str = "https://developer.android.com/tools/releases/platform-tools";
pub const GOOGLE_TEAM_IDENTIFIER: &str = "EQHXZ8M8AV";
pub const GOOGLE_SIGNER_AUTHORITY: &str = "Developer ID Application: Google LLC (EQHXZ8M8AV)";
const MINIMUM_SUPPORTED_VERSION: &str = "35.0.0";
const TESTED_THROUGH_VERSION: &str = "37.0.0";

const MAX_ZIP_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_TOTAL_UNCOMPRESSED: u64 = 256 * 1024 * 1024;
const MAX_ENTRY_UNCOMPRESSED: u64 = 128 * 1024 * 1024;
const MAX_MEMBER_PATH_BYTES: usize = 512;
const MAX_PROCESS_OUTPUT: usize = 64 * 1024;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const PROCESS_STREAM_BUFFER: usize = 16 * 1024;
const RETAINED_FILES: [&str; 3] = ["adb", "NOTICE.txt", "source.properties"];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbSetupStatusDto {
    pub status: &'static str,
    pub version: Option<String>,
    pub warning: Option<String>,
    pub error: Option<ActionableErrorDto>,
    pub can_import: bool,
    pub can_replace: bool,
    pub can_remove: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionableErrorDto {
    code: String,
    message: String,
    actions: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedSettings {
    schema_version: u32,
    install_relative_path: String,
    version: String,
    architecture: String,
    signer_team_identifier: String,
    files: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct ResolvedAdb {
    path: PathBuf,
    version: String,
    warning: Option<String>,
    managed_relative_path: Option<String>,
}

/// Trusted identity of the Platform-Tools installation associated with a review.
/// Paths remain Tauri-private and are never serialized to the frontend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdbInstallationIdentity {
    path: PathBuf,
    version: String,
    managed_relative_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Stable classification for trusted Platform-Tools revalidation failures.
pub enum AdbRevalidationError {
    /// The retained installation cannot currently be validated or used.
    Unavailable,
    /// Cached review identity no longer names the current installation.
    Changed,
}

/// Sanitized result of checking whether retained Platform-Tools can satisfy
/// the host-side prerequisites for a real-execution attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlatformToolsReadiness {
    Ready,
    NotFound,
    Invalid,
    CheckFailed,
}

/// Immutable trusted state used for a bounded readiness check after the live
/// manager mutex has been released.
#[derive(Clone, Debug)]
pub(crate) struct AdbReadinessSnapshot {
    adb_revision: u64,
    root: PathBuf,
    current: Option<ResolvedAdb>,
    last_error: Option<ActionableErrorDto>,
}

impl ResolvedAdb {
    fn identity(&self) -> AdbInstallationIdentity {
        AdbInstallationIdentity {
            path: self.path.clone(),
            version: self.version.clone(),
            managed_relative_path: self.managed_relative_path.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AdbManager {
    root: PathBuf,
    current: Option<ResolvedAdb>,
    last_error: Option<ActionableErrorDto>,
    revision: u64,
}

impl AdbManager {
    pub fn new(root: PathBuf) -> Self {
        let mut manager = Self {
            root,
            current: None,
            last_error: None,
            revision: 0,
        };
        if let Err(error) = manager.initialize() {
            manager.last_error = Some(error);
        }
        manager
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_app_managed(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.managed_relative_path.is_some())
    }

    pub fn status(&self) -> AdbSetupStatusDto {
        match &self.current {
            Some(current) => AdbSetupStatusDto {
                status: "ready",
                version: Some(current.version.clone()),
                warning: current.warning.clone(),
                error: None,
                can_import: true,
                can_replace: true,
                can_remove: current.managed_relative_path.is_some(),
            },
            None if self.last_error.is_some() => AdbSetupStatusDto {
                status: "invalid",
                version: None,
                warning: None,
                error: self.last_error.clone(),
                can_import: true,
                can_replace: false,
                can_remove: self.settings_path().is_file(),
            },
            None => AdbSetupStatusDto {
                status: "missing",
                version: None,
                warning: None,
                error: None,
                can_import: true,
                can_replace: false,
                can_remove: false,
            },
        }
    }

    pub fn adb_path(&self) -> Result<&Path, String> {
        self.current
            .as_ref()
            .map(|current| current.path.as_path())
            .ok_or_else(|| {
                actionable_json(
                    "adb_setup_required",
                    "Import Android SDK Platform-Tools before connecting a device.",
                )
            })
    }

    pub fn installation_identity(&self) -> Result<AdbInstallationIdentity, String> {
        self.current
            .as_ref()
            .map(ResolvedAdb::identity)
            .ok_or_else(|| {
                actionable_json(
                    "adb_setup_required",
                    "Import Android SDK Platform-Tools before connecting a device.",
                )
            })
    }

    /// Clone only process-local trusted state needed for an informational
    /// readiness check. Filesystem paths remain inside Rust.
    pub(crate) fn readiness_snapshot(&self) -> AdbReadinessSnapshot {
        AdbReadinessSnapshot {
            adb_revision: self.revision,
            root: self.root.clone(),
            current: self.current.clone(),
            last_error: self.last_error.clone(),
        }
    }

    /// Revalidates the exact installation retained by a review and returns its
    /// trusted executable path. Validation repeats the existing managed or
    /// development checks instead of trusting cached startup state.
    pub fn revalidate_for_execution(
        &self,
        expected: &AdbInstallationIdentity,
    ) -> Result<PathBuf, AdbRevalidationError> {
        self.revalidate_for_execution_with_executor(expected, &RealProcessExecutor)
    }

    fn revalidate_for_execution_with_executor(
        &self,
        expected: &AdbInstallationIdentity,
        executor: &impl ProcessExecutor,
    ) -> Result<PathBuf, AdbRevalidationError> {
        let current = self
            .current
            .as_ref()
            .ok_or(AdbRevalidationError::Unavailable)?;
        if current.identity() != *expected {
            return Err(AdbRevalidationError::Changed);
        }

        validate_current_install(&self.root, current, executor).map_err(|error| match error {
            CurrentInstallValidationError::Changed => AdbRevalidationError::Changed,
            CurrentInstallValidationError::Unavailable(_) => AdbRevalidationError::Unavailable,
        })
    }

    pub fn import_zip(&mut self, source: &Path) -> Result<AdbSetupStatusDto, String> {
        let previous = self.current.clone();
        match self.import_zip_inner(source) {
            Ok(current) => {
                self.current = Some(current);
                self.last_error = None;
                self.revision = self.revision.saturating_add(1).max(1);
                Ok(self.status())
            }
            Err(error) => {
                self.current = previous;
                Err(serde_json::to_string(&error).unwrap_or_else(|_| error.message.clone()))
            }
        }
    }

    pub fn remove(&mut self) -> Result<AdbSetupStatusDto, String> {
        let managed_relative = self
            .current
            .as_ref()
            .and_then(|current| current.managed_relative_path.clone())
            .or_else(|| {
                read_settings(&self.settings_path())
                    .ok()
                    .map(|settings| settings.install_relative_path)
            });
        remove_settings_file(&self.settings_path())?;
        self.current = None;
        self.last_error = None;
        self.revision = self.revision.saturating_add(1).max(1);
        if let Some(relative) = managed_relative {
            let path = checked_install_path(&self.root, &relative)?;
            if path.exists() {
                fs::remove_dir_all(path).map_err(|_| actionable_json(
                    "adb_remove_failed",
                    "Platform-Tools settings were removed, but managed files could not be cleaned up.",
                ))?;
            }
        }
        self.cleanup_orphans();
        Ok(self.status())
    }

    fn initialize(&mut self) -> Result<(), ActionableErrorDto> {
        fs::create_dir_all(self.root.join("installs")).map_err(|_| {
            setup_error(
                "adb_storage_unavailable",
                "EmuChef could not create its managed Platform-Tools directory.",
                vec!["retry"],
            )
        })?;
        fs::create_dir_all(self.root.join("staging")).map_err(|_| {
            setup_error(
                "adb_storage_unavailable",
                "EmuChef could not create its temporary Platform-Tools directory.",
                vec!["retry"],
            )
        })?;
        set_directory_mode(&self.root, 0o700).ok();
        self.cleanup_staging();

        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("EMUCHEF_ADB_PATH").map(PathBuf::from) {
            let validated = validate_development_adb(&path, &RealProcessExecutor)?;
            self.current = Some(validated);
            return Ok(());
        }

        if self.settings_path().is_file() {
            let settings = read_settings(&self.settings_path()).map_err(|_| setup_error(
                "managed_adb_settings_invalid",
                "The managed Platform-Tools settings are invalid. Import Platform-Tools again or remove the installation.",
                vec!["replace", "remove"],
            ))?;
            let install = checked_install_path(&self.root, &settings.install_relative_path)
                .map_err(|_| {
                    setup_error(
                    "managed_adb_containment_invalid",
                    "The managed Platform-Tools location is invalid. Remove it and import again.",
                    vec!["replace", "remove"],
                )
                })?;
            let validated = validate_managed_install(&install, &settings, &RealProcessExecutor)?;
            self.current = Some(validated);
            self.cleanup_orphans();
            return Ok(());
        }

        #[cfg(debug_assertions)]
        if std::env::var("EMUCHEF_ALLOW_SYSTEM_ADB").ok().as_deref() == Some("1") {
            if let Some(path) = find_system_adb() {
                let validated = validate_development_adb(&path, &RealProcessExecutor)?;
                self.current = Some(validated);
                return Ok(());
            }
        }
        self.cleanup_orphans();
        Ok(())
    }

    fn import_zip_inner(&self, source: &Path) -> Result<ResolvedAdb, ActionableErrorDto> {
        self.import_zip_inner_with_executor(source, &RealProcessExecutor)
    }

    fn import_zip_inner_with_executor(
        &self,
        source: &Path,
        executor: &impl ProcessExecutor,
    ) -> Result<ResolvedAdb, ActionableErrorDto> {
        let mut source = secure_open_zip(source)?;
        let members = inspect_archive(&mut source)?;
        let staging = self
            .root
            .join("staging")
            .join(Uuid::new_v4().simple().to_string());
        fs::create_dir(&staging).map_err(|_| {
            setup_error(
                "adb_staging_failed",
                "EmuChef could not create a temporary validation directory.",
                vec!["retry"],
            )
        })?;
        set_directory_mode(&staging, 0o700).ok();
        let mut guard = StagingGuard::new(staging.clone());
        source.rewind().map_err(|_| {
            setup_error(
                "archive_unreadable",
                "The selected Platform-Tools ZIP could not be reread.",
                vec!["import"],
            )
        })?;
        let mut archive = ZipArchive::new(source).map_err(|_| {
            archive_error("archive_unreadable", "The selected ZIP is not readable.")
        })?;
        let mut hashes = HashMap::new();
        for retained in RETAINED_FILES {
            let index = *members
                .get(retained)
                .expect("required member was inspected");
            let mut entry = archive.by_index(index).map_err(|_| {
                archive_error(
                    "archive_unreadable",
                    "A required Platform-Tools file could not be read.",
                )
            })?;
            let destination = staging.join(retained);
            let hash = extract_fixed_file(&mut entry, &destination, retained == "adb")?;
            hashes.insert(retained.to_string(), hash);
        }

        let validation = validate_candidate(&staging, &hashes, true, executor)?;
        let install_id = Uuid::new_v4().simple().to_string();
        let relative = format!("installs/{install_id}");
        let destination = self.root.join(&relative);
        fs::rename(&staging, &destination).map_err(|_| {
            setup_error(
                "adb_install_failed",
                "Validated Platform-Tools could not be installed in application data.",
                vec!["retry"],
            )
        })?;
        guard.commit();
        let settings = ManagedSettings {
            schema_version: 1,
            install_relative_path: relative.clone(),
            version: validation.version.clone(),
            architecture: validation.architecture,
            signer_team_identifier: GOOGLE_TEAM_IDENTIFIER.to_string(),
            files: hashes,
        };
        let old_settings = read_settings(&self.settings_path()).ok();
        if let Err(error) = write_settings_atomic(&self.settings_path(), &settings) {
            let _ = fs::remove_dir_all(&destination);
            return Err(setup_error(
                "failed_replacement_recovery",
                &format!("The validated replacement could not be activated. The existing installation was preserved. {error}"),
                vec!["retry"],
            ));
        }
        if let Some(old) = old_settings {
            if old.install_relative_path != relative {
                if let Ok(old_path) = checked_install_path(&self.root, &old.install_relative_path) {
                    let _ = fs::remove_dir_all(old_path);
                }
            }
        }
        Ok(ResolvedAdb {
            path: destination.join("adb"),
            version: validation.version.clone(),
            warning: version_warning(&validation.version),
            managed_relative_path: Some(relative),
        })
    }

    fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    fn cleanup_staging(&self) {
        let staging = self.root.join("staging");
        if let Ok(entries) = fs::read_dir(staging) {
            for entry in entries.flatten() {
                if entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
    }

    fn cleanup_orphans(&self) {
        let active = read_settings(&self.settings_path())
            .ok()
            .map(|settings| settings.install_relative_path);
        if let Ok(entries) = fs::read_dir(self.root.join("installs")) {
            for entry in entries.flatten() {
                let relative = format!("installs/{}", entry.file_name().to_string_lossy());
                if active.as_deref() != Some(&relative)
                    && entry
                        .file_type()
                        .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
    }
}

impl AdbReadinessSnapshot {
    pub(crate) fn adb_revision(&self) -> u64 {
        self.adb_revision
    }

    pub(crate) fn evaluate(&self) -> PlatformToolsReadiness {
        self.evaluate_with_executor(&RealProcessExecutor)
    }

    fn evaluate_with_executor(&self, executor: &impl ProcessExecutor) -> PlatformToolsReadiness {
        let Some(current) = self.current.as_ref() else {
            return self
                .last_error
                .as_ref()
                .map(classify_readiness_error)
                .unwrap_or(PlatformToolsReadiness::NotFound);
        };
        match validate_current_install(&self.root, current, executor) {
            Ok(_) => PlatformToolsReadiness::Ready,
            Err(CurrentInstallValidationError::Changed) => PlatformToolsReadiness::Invalid,
            Err(CurrentInstallValidationError::Unavailable(error)) => {
                classify_readiness_error(&error)
            }
        }
    }
}

#[derive(Debug)]
enum CurrentInstallValidationError {
    Changed,
    Unavailable(ActionableErrorDto),
}

/// Shared orchestration over the existing managed and development validators.
/// Execution maps detailed failures to its established fail-closed contract;
/// readiness maps the same failures to sanitized informational states.
fn validate_current_install(
    root: &Path,
    current: &ResolvedAdb,
    executor: &impl ProcessExecutor,
) -> Result<PathBuf, CurrentInstallValidationError> {
    let expected = current.identity();
    let validated = if let Some(relative) = &current.managed_relative_path {
        let settings = read_settings(&root.join("settings.json")).map_err(|_| {
            CurrentInstallValidationError::Unavailable(setup_error(
                "managed_adb_settings_invalid",
                "The managed Platform-Tools settings are invalid.",
                vec!["replace", "remove"],
            ))
        })?;
        if settings.install_relative_path != *relative {
            return Err(CurrentInstallValidationError::Changed);
        }
        let install = checked_install_path(root, relative).map_err(|_| {
            CurrentInstallValidationError::Unavailable(setup_error(
                "managed_adb_containment_invalid",
                "The managed Platform-Tools location is invalid.",
                vec!["replace", "remove"],
            ))
        })?;
        validate_managed_install(&install, &settings, executor)
            .map_err(CurrentInstallValidationError::Unavailable)?
    } else {
        #[cfg(debug_assertions)]
        {
            validate_development_adb(&current.path, executor)
                .map_err(CurrentInstallValidationError::Unavailable)?
        }
        #[cfg(not(debug_assertions))]
        {
            return Err(CurrentInstallValidationError::Unavailable(setup_error(
                "development_adb_invalid",
                "Development Platform-Tools are unavailable in this build.",
                vec!["retry"],
            )));
        }
    };
    let validated_identity = validated.identity();
    let same_path = validated_identity
        .path
        .canonicalize()
        .ok()
        .zip(expected.path.canonicalize().ok())
        .is_some_and(|(validated, retained)| validated == retained);
    if validated_identity.version != expected.version
        || validated_identity.managed_relative_path != expected.managed_relative_path
        || !same_path
    {
        return Err(CurrentInstallValidationError::Changed);
    }
    Ok(validated.path)
}

fn classify_readiness_error(error: &ActionableErrorDto) -> PlatformToolsReadiness {
    match error.code.as_str() {
        "managed_adb_file_missing" | "development_adb_unavailable" => {
            PlatformToolsReadiness::NotFound
        }
        "managed_adb_settings_invalid"
        | "managed_adb_containment_invalid"
        | "managed_adb_file_invalid"
        | "managed_adb_hash_failed"
        | "managed_adb_hash_mismatch"
        | "managed_adb_metadata_mismatch"
        | "architecture_unreadable"
        | "architecture_invalid"
        | "architecture_incompatible"
        | "signature_invalid"
        | "signer_not_google"
        | "source_properties_invalid"
        | "platform_tools_too_old"
        | "adb_version_failed"
        | "adb_version_invalid"
        | "adb_version_mismatch"
        | "development_adb_invalid" => PlatformToolsReadiness::Invalid,
        _ => PlatformToolsReadiness::CheckFailed,
    }
}

fn secure_open_zip(path: &Path) -> Result<File, ActionableErrorDto> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| {
        archive_error(
            "archive_unreadable",
            "The selected ZIP could not be opened as a regular file.",
        )
    })?;
    let metadata = file.metadata().map_err(|_| {
        archive_error(
            "archive_unreadable",
            "The selected ZIP metadata could not be read.",
        )
    })?;
    if !metadata.is_file() || metadata.len() > MAX_ZIP_BYTES {
        return Err(archive_error(
            "archive_too_large",
            "The selected ZIP is not a regular file or exceeds the 128 MiB limit.",
        ));
    }
    Ok(file)
}

fn inspect_archive(source: &mut File) -> Result<HashMap<&'static str, usize>, ActionableErrorDto> {
    if archive_has_encryption_flag(source)? {
        return Err(archive_error(
            "archive_encrypted",
            "Encrypted ZIP entries are not supported.",
        ));
    }
    let mut archive = ZipArchive::new(source).map_err(|_| {
        archive_error(
            "archive_unreadable",
            "The selected file is not a readable ZIP archive.",
        )
    })?;
    if archive.len() > MAX_ENTRIES {
        return Err(archive_error(
            "archive_too_many_entries",
            "The selected ZIP contains too many entries.",
        ));
    }
    let mut total = 0u64;
    let mut names = HashSet::new();
    let mut retained = HashMap::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| {
            archive_error("archive_unreadable", "A ZIP entry could not be inspected.")
        })?;
        let raw_name = std::str::from_utf8(entry.name_raw()).map_err(|_| {
            archive_error(
                "archive_structure_invalid",
                "The ZIP contains a non-UTF-8 member name.",
            )
        })?;
        validate_member_name(raw_name)?;
        let folded = raw_name.to_lowercase();
        if !names.insert(folded) {
            return Err(archive_error(
                "archive_structure_invalid",
                "The ZIP contains duplicate or case-colliding member names.",
            ));
        }
        add_archive_size(&mut total, entry.size())?;
        validate_zip_entry_type(&entry)?;
        for retained_name in RETAINED_FILES {
            if raw_name == format!("platform-tools/{retained_name}") {
                if entry.is_dir() {
                    return Err(archive_error(
                        "archive_structure_invalid",
                        "A required Platform-Tools member is not a regular file.",
                    ));
                }
                retained.insert(retained_name, index);
            }
        }
    }
    for required in RETAINED_FILES {
        if !retained.contains_key(required) {
            return Err(archive_error(
                "archive_structure_invalid",
                &format!("The ZIP is missing platform-tools/{required}."),
            ));
        }
    }
    Ok(retained)
}

fn add_archive_size(total: &mut u64, entry_size: u64) -> Result<(), ActionableErrorDto> {
    if entry_size > MAX_ENTRY_UNCOMPRESSED {
        return Err(archive_error(
            "archive_too_large",
            "A ZIP entry exceeds the uncompressed size limit.",
        ));
    }
    *total = total.checked_add(entry_size).ok_or_else(|| {
        archive_error("archive_too_large", "The ZIP uncompressed size is invalid.")
    })?;
    if *total > MAX_TOTAL_UNCOMPRESSED {
        return Err(archive_error(
            "archive_too_large",
            "The ZIP exceeds the total uncompressed size limit.",
        ));
    }
    Ok(())
}

fn archive_has_encryption_flag(source: &mut File) -> Result<bool, ActionableErrorDto> {
    source.rewind().map_err(|_| {
        archive_error(
            "archive_unreadable",
            "The selected ZIP could not be inspected.",
        )
    })?;
    let mut bytes = Vec::new();
    source
        .take(MAX_ZIP_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            archive_error(
                "archive_unreadable",
                "The selected ZIP could not be inspected.",
            )
        })?;
    source.rewind().map_err(|_| {
        archive_error(
            "archive_unreadable",
            "The selected ZIP could not be inspected.",
        )
    })?;
    for index in 0..bytes.len().saturating_sub(10) {
        if bytes[index..].starts_with(&[0x50, 0x4b, 0x03, 0x04])
            && u16::from_le_bytes([bytes[index + 6], bytes[index + 7]]) & 1 == 1
        {
            return Ok(true);
        }
        if bytes[index..].starts_with(&[0x50, 0x4b, 0x01, 0x02])
            && u16::from_le_bytes([bytes[index + 8], bytes[index + 9]]) & 1 == 1
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_member_name(name: &str) -> Result<(), ActionableErrorDto> {
    if name.is_empty()
        || name.len() > MAX_MEMBER_PATH_BYTES
        || name.contains('\0')
        || name.contains('\\')
        || name.starts_with('/')
    {
        return Err(archive_error(
            "archive_structure_invalid",
            "The ZIP contains an unsafe member path.",
        ));
    }
    let path = Path::new(name);
    let mut components = path.components();
    if components.next() != Some(Component::Normal("platform-tools".as_ref())) {
        return Err(archive_error(
            "archive_structure_invalid",
            "The ZIP must contain exactly one top-level platform-tools directory.",
        ));
    }
    if components.any(|component| !matches!(component, Component::Normal(_))) {
        return Err(archive_error(
            "archive_structure_invalid",
            "The ZIP contains path traversal or a non-normal member path.",
        ));
    }
    Ok(())
}

fn validate_zip_entry_type(entry: &zip::read::ZipFile<'_>) -> Result<(), ActionableErrorDto> {
    if entry.encrypted() {
        return Err(archive_error(
            "archive_encrypted",
            "Encrypted ZIP entries are not supported.",
        ));
    }
    if let Some(mode) = entry.unix_mode() {
        let kind = mode & u32::from(libc::S_IFMT);
        if kind != 0 && kind != u32::from(libc::S_IFREG) && kind != u32::from(libc::S_IFDIR) {
            return Err(archive_error(
                "archive_structure_invalid",
                "The ZIP contains a link or unsupported special file.",
            ));
        }
    }
    Ok(())
}

fn extract_fixed_file<R: Read>(
    source: &mut R,
    destination: &Path,
    executable: bool,
) -> Result<String, ActionableErrorDto> {
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| {
            setup_error(
                "adb_staging_failed",
                "A required Platform-Tools file could not be created safely.",
                vec!["retry"],
            )
        })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer).map_err(|_| {
            archive_error(
                "archive_unreadable",
                "A required Platform-Tools file could not be extracted.",
            )
        })?;
        if read == 0 {
            break;
        }
        destination_file.write_all(&buffer[..read]).map_err(|_| {
            setup_error(
                "adb_staging_failed",
                "A required Platform-Tools file could not be written.",
                vec!["retry"],
            )
        })?;
        hasher.update(&buffer[..read]);
    }
    destination_file.sync_all().map_err(|_| {
        setup_error(
            "adb_staging_failed",
            "A required Platform-Tools file could not be synchronized.",
            vec!["retry"],
        )
    })?;
    set_file_mode(destination, if executable { 0o755 } else { 0o644 }).map_err(|_| {
        setup_error(
            "adb_staging_failed",
            "A required Platform-Tools file could not be secured.",
            vec!["retry"],
        )
    })?;
    Ok(hex::encode(hasher.finalize()))
}

#[derive(Debug)]
struct CandidateValidation {
    version: String,
    architecture: String,
}

fn validate_candidate(
    directory: &Path,
    expected_hashes: &HashMap<String, String>,
    require_google_signature: bool,
    executor: &impl ProcessExecutor,
) -> Result<CandidateValidation, ActionableErrorDto> {
    for retained in RETAINED_FILES {
        let path = directory.join(retained);
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            setup_error(
                "managed_adb_file_missing",
                "A required managed Platform-Tools file is missing.",
                vec!["replace", "remove"],
            )
        })?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(setup_error(
                "managed_adb_file_invalid",
                "A required managed Platform-Tools file is not a regular file.",
                vec!["replace", "remove"],
            ));
        }
        let actual = sha256_file(&path).map_err(|_| {
            setup_error(
                "managed_adb_hash_failed",
                "A managed Platform-Tools file could not be verified.",
                vec!["replace", "remove"],
            )
        })?;
        if expected_hashes.get(retained) != Some(&actual) {
            return Err(setup_error(
                "managed_adb_hash_mismatch",
                "Managed Platform-Tools files changed after validation. Import them again.",
                vec!["replace", "remove"],
            ));
        }
    }
    let adb = directory.join("adb");
    let architectures = macho_architectures(&adb)?;
    let host = std::env::consts::ARCH;
    if !architectures
        .iter()
        .any(|architecture| architecture == host)
    {
        return Err(setup_error(
            "architecture_incompatible",
            "The imported adb binary does not contain a native slice for this Mac.",
            vec!["import"],
        ));
    }
    if require_google_signature {
        verify_google_signature(&adb, directory, executor)?;
    }
    let source_revision = source_revision(&directory.join("source.properties"))?;
    enforce_version_policy(&source_revision)?;
    let output = executor.run(&adb, &["version"], directory, PROCESS_TIMEOUT)?;
    if !output.success {
        return Err(setup_error(
            "adb_version_failed",
            "The validated adb candidate did not complete 'adb version' successfully.",
            vec!["import"],
        ));
    }
    let reported = parse_adb_version(&output.stdout).ok_or_else(|| {
        setup_error(
            "adb_version_invalid",
            "The adb version output was not recognized.",
            vec!["import"],
        )
    })?;
    if reported != source_revision {
        return Err(setup_error(
            "adb_version_mismatch",
            "The adb binary version does not match source.properties.",
            vec!["import"],
        ));
    }
    Ok(CandidateValidation {
        version: source_revision.to_string(),
        architecture: architectures.join(","),
    })
}

fn validate_managed_install(
    install: &Path,
    settings: &ManagedSettings,
    executor: &impl ProcessExecutor,
) -> Result<ResolvedAdb, ActionableErrorDto> {
    if settings.schema_version != 1 || settings.signer_team_identifier != GOOGLE_TEAM_IDENTIFIER {
        return Err(setup_error(
            "managed_adb_settings_invalid",
            "Managed Platform-Tools settings are unsupported or invalid.",
            vec!["replace", "remove"],
        ));
    }
    let validation = validate_candidate(install, &settings.files, true, executor)?;
    if validation.version != settings.version || validation.architecture != settings.architecture {
        return Err(setup_error(
            "managed_adb_metadata_mismatch",
            "Managed Platform-Tools metadata changed after validation.",
            vec!["replace", "remove"],
        ));
    }
    Ok(ResolvedAdb {
        path: install.join("adb"),
        version: validation.version.clone(),
        warning: version_warning(&validation.version),
        managed_relative_path: Some(settings.install_relative_path.clone()),
    })
}

#[cfg(debug_assertions)]
fn validate_development_adb(
    path: &Path,
    executor: &impl ProcessExecutor,
) -> Result<ResolvedAdb, ActionableErrorDto> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        setup_error(
            "development_adb_unavailable",
            "The explicitly configured development ADB is unavailable.",
            vec!["retry"],
        )
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(setup_error(
            "development_adb_invalid",
            "The explicitly configured development ADB must be a regular file.",
            vec!["retry"],
        ));
    }
    let architectures = macho_architectures(path)?;
    if !architectures
        .iter()
        .any(|architecture| architecture == std::env::consts::ARCH)
    {
        return Err(setup_error(
            "architecture_incompatible",
            "The development ADB is incompatible with this Mac.",
            vec!["retry"],
        ));
    }
    let cwd = path.parent().unwrap_or_else(|| Path::new("/"));
    let output = executor.run(path, &["version"], cwd, PROCESS_TIMEOUT)?;
    if !output.success {
        return Err(setup_error(
            "adb_version_failed",
            "The development ADB did not complete 'adb version' successfully.",
            vec!["retry"],
        ));
    }
    let version = parse_adb_version(&output.stdout).ok_or_else(|| {
        setup_error(
            "adb_version_invalid",
            "The development ADB version output was not recognized.",
            vec!["retry"],
        )
    })?;
    enforce_version_policy(&version)?;
    Ok(ResolvedAdb {
        path: path.to_path_buf(),
        version: version.to_string(),
        warning: version_warning(&version.to_string()),
        managed_relative_path: None,
    })
}

fn verify_google_signature(
    adb: &Path,
    cwd: &Path,
    executor: &impl ProcessExecutor,
) -> Result<(), ActionableErrorDto> {
    let verify = executor.run(
        Path::new("/usr/bin/codesign"),
        &[
            "--verify",
            "--strict",
            "--verbose=2",
            adb.to_string_lossy().as_ref(),
        ],
        cwd,
        PROCESS_TIMEOUT,
    )?;
    if !verify.success {
        return Err(setup_error(
            "signature_invalid",
            "The adb signature is invalid.",
            vec!["import"],
        ));
    }
    let details = executor.run(
        Path::new("/usr/bin/codesign"),
        &["-d", "--verbose=4", adb.to_string_lossy().as_ref()],
        cwd,
        PROCESS_TIMEOUT,
    )?;
    let combined = format!("{}\n{}", details.stdout, details.stderr);
    if !details.success
        || !combined
            .lines()
            .any(|line| line.trim() == format!("TeamIdentifier={GOOGLE_TEAM_IDENTIFIER}"))
        || !combined
            .lines()
            .any(|line| line.trim() == format!("Authority={GOOGLE_SIGNER_AUTHORITY}"))
    {
        return Err(setup_error(
            "signer_not_google",
            "The adb binary is not signed by the verified Google LLC Developer ID.",
            vec!["import"],
        ));
    }
    Ok(())
}

fn source_revision(path: &Path) -> Result<Version, ActionableErrorDto> {
    let text = fs::read_to_string(path).map_err(|_| {
        archive_error(
            "source_properties_invalid",
            "source.properties could not be read.",
        )
    })?;
    let raw = text
        .lines()
        .find_map(|line| line.strip_prefix("Pkg.Revision="))
        .ok_or_else(|| {
            archive_error(
                "source_properties_invalid",
                "source.properties does not contain Pkg.Revision.",
            )
        })?;
    Version::parse(raw.trim()).map_err(|_| {
        archive_error(
            "source_properties_invalid",
            "source.properties contains an invalid Platform-Tools version.",
        )
    })
}

fn parse_adb_version(stdout: &str) -> Option<Version> {
    let raw = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Version "))?;
    let semver = raw.split('-').next()?.trim();
    Version::parse(semver).ok()
}

fn enforce_version_policy(version: &Version) -> Result<(), ActionableErrorDto> {
    let minimum = Version::parse(MINIMUM_SUPPORTED_VERSION).expect("minimum version constant");
    if version < &minimum {
        return Err(setup_error(
            "platform_tools_too_old",
            "Platform-Tools 35.0.0 or newer is required.",
            vec!["import"],
        ));
    }
    Ok(())
}

fn version_warning(version: &str) -> Option<String> {
    let version = Version::parse(version).ok()?;
    let tested = Version::parse(TESTED_THROUGH_VERSION).ok()?;
    (version > tested).then(|| format!(
        "Platform-Tools {version} is newer than the latest release tested with EmuChef ({tested}). It passed all local validation checks."
    ))
}

fn macho_architectures(path: &Path) -> Result<Vec<String>, ActionableErrorDto> {
    let mut file = File::open(path).map_err(|_| {
        setup_error(
            "architecture_unreadable",
            "The adb binary architecture could not be read.",
            vec!["import"],
        )
    })?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(4096)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            setup_error(
                "architecture_unreadable",
                "The adb binary architecture could not be read.",
                vec!["import"],
            )
        })?;
    parse_macho_architectures(&bytes).ok_or_else(|| {
        setup_error(
            "architecture_invalid",
            "The imported adb is not a supported 64-bit macOS executable.",
            vec!["import"],
        )
    })
}

fn parse_macho_architectures(bytes: &[u8]) -> Option<Vec<String>> {
    const CPU_X86_64: u32 = 0x0100_0007;
    const CPU_ARM64: u32 = 0x0100_000c;
    if bytes.len() < 8 {
        return None;
    }
    let magic = &bytes[..4];
    if magic == [0xcf, 0xfa, 0xed, 0xfe] {
        return macho_cpu_name(u32::from_le_bytes(bytes[4..8].try_into().ok()?))
            .map(|name| vec![name]);
    }
    if magic == [0xfe, 0xed, 0xfa, 0xcf] {
        return macho_cpu_name(u32::from_be_bytes(bytes[4..8].try_into().ok()?))
            .map(|name| vec![name]);
    }
    let (big_endian, entry_size) = match magic {
        [0xca, 0xfe, 0xba, 0xbe] => (true, 20usize),
        [0xbe, 0xba, 0xfe, 0xca] => (false, 20usize),
        [0xca, 0xfe, 0xba, 0xbf] => (true, 32usize),
        [0xbf, 0xba, 0xfe, 0xca] => (false, 32usize),
        _ => return None,
    };
    let read_u32 = |slice: &[u8]| {
        let bytes: [u8; 4] = slice.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    let count = read_u32(&bytes[4..8])? as usize;
    if count == 0 || count > 16 || bytes.len() < 8 + count * entry_size {
        return None;
    }
    let mut result = Vec::new();
    for index in 0..count {
        let offset = 8 + index * entry_size;
        let cpu = read_u32(&bytes[offset..offset + 4])?;
        let name = match cpu {
            CPU_X86_64 => "x86_64",
            CPU_ARM64 => "aarch64",
            _ => continue,
        };
        if !result.iter().any(|value| value == name) {
            result.push(name.to_string());
        }
    }
    (!result.is_empty()).then_some(result)
}

fn macho_cpu_name(cpu: u32) -> Option<String> {
    match cpu {
        0x0100_0007 => Some("x86_64".to_string()),
        0x0100_000c => Some("aarch64".to_string()),
        _ => None,
    }
}

trait ProcessExecutor {
    fn run(
        &self,
        program: &Path,
        args: &[&str],
        cwd: &Path,
        timeout: Duration,
    ) -> Result<ControlledOutput, ActionableErrorDto>;
}

struct RealProcessExecutor;

#[derive(Debug)]
struct ControlledOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

impl ProcessExecutor for RealProcessExecutor {
    fn run(
        &self,
        program: &Path,
        args: &[&str],
        cwd: &Path,
        timeout: Duration,
    ) -> Result<ControlledOutput, ActionableErrorDto> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| {
                setup_error(
                    "validation_process_unavailable",
                    "A required local validation process could not be started.",
                    vec!["retry"],
                )
            })?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                cleanup_process(&mut child);
                return Err(setup_error(
                    "validation_process_failed",
                    "A local validation process could not be monitored.",
                    vec!["retry"],
                ));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                cleanup_process(&mut child);
                return Err(setup_error(
                    "validation_process_failed",
                    "A local validation process could not be monitored.",
                    vec!["retry"],
                ));
            }
        };
        match block_on(run_process(child, stdout, stderr, timeout)) {
            Ok(output) => Ok(ControlledOutput {
                success: output.status_success,
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
            Err(ProcessFailure::TimedOut | ProcessFailure::TimedOutCleanupUncertain) => {
                Err(setup_error(
                    "validation_process_timeout",
                    "A local validation process timed out and was terminated.",
                    vec!["retry"],
                ))
            }
            Err(
                ProcessFailure::OutputTooLarge | ProcessFailure::OutputTooLargeCleanupUncertain,
            ) => Err(setup_error(
                "validation_output_too_large",
                "Local validation produced too much output.",
                vec!["retry"],
            )),
            Err(ProcessFailure::OutputRead | ProcessFailure::OutputReadCleanupUncertain) => {
                Err(setup_error(
                    "validation_output_failed",
                    "Local validation output could not be read.",
                    vec!["retry"],
                ))
            }
            Err(
                ProcessFailure::Wait
                | ProcessFailure::WaitCleanupUncertain
                | ProcessFailure::CleanupUncertain,
            ) => Err(setup_error(
                "validation_process_failed",
                "A local validation process could not be monitored.",
                vec!["retry"],
            )),
        }
    }
}

#[derive(Debug)]
struct ProcessOutput {
    status_success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessFailure {
    TimedOut,
    TimedOutCleanupUncertain,
    OutputTooLarge,
    OutputTooLargeCleanupUncertain,
    OutputRead,
    OutputReadCleanupUncertain,
    Wait,
    WaitCleanupUncertain,
    CleanupUncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessStream {
    Stdout,
    Stderr,
}

impl ProcessStream {
    fn read_failure(self) -> ProcessFailure {
        let _ = self;
        ProcessFailure::OutputRead
    }

    fn overflow_failure(self) -> ProcessFailure {
        let _ = self;
        ProcessFailure::OutputTooLarge
    }
}

async fn capture_process_stream<R: AsyncRead + Unpin>(
    mut reader: R,
    stream: ProcessStream,
) -> Result<Vec<u8>, ProcessFailure> {
    let mut retained = Vec::with_capacity(MAX_PROCESS_OUTPUT.min(16 * 1024));
    let mut buffer = [0_u8; PROCESS_STREAM_BUFFER];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => return Ok(retained),
            Ok(count) => {
                let remaining = MAX_PROCESS_OUTPUT.saturating_sub(retained.len());
                let retained_count = count.min(remaining);
                retained.extend_from_slice(&buffer[..retained_count]);
                if count > remaining {
                    return Err(stream.overflow_failure());
                }
            }
            Err(_) => return Err(stream.read_failure()),
        }
    }
}

async fn read_process_streams(
    stdout: ChildStdout,
    stderr: ChildStderr,
) -> Result<(Vec<u8>, Vec<u8>), ProcessFailure> {
    let mut stdout_future = Box::pin(capture_process_stream(stdout, ProcessStream::Stdout));
    let mut stderr_future = Box::pin(capture_process_stream(stderr, ProcessStream::Stderr));
    let mut stdout_result = None;
    let mut stderr_result = None;
    poll_fn(|context| {
        if stdout_result.is_none() {
            if let std::task::Poll::Ready(result) = stdout_future.as_mut().poll(context) {
                match result {
                    Ok(bytes) => stdout_result = Some(bytes),
                    Err(error) => return std::task::Poll::Ready(Err(error)),
                }
            }
        }
        if stderr_result.is_none() {
            if let std::task::Poll::Ready(result) = stderr_future.as_mut().poll(context) {
                match result {
                    Ok(bytes) => stderr_result = Some(bytes),
                    Err(error) => return std::task::Poll::Ready(Err(error)),
                }
            }
        }
        match (stdout_result.take(), stderr_result.take()) {
            (Some(stdout), Some(stderr)) => std::task::Poll::Ready(Ok((stdout, stderr))),
            (stdout, stderr) => {
                stdout_result = stdout;
                stderr_result = stderr;
                std::task::Poll::Pending
            }
        }
    })
    .await
}

enum ProcessEvent {
    Exited(Result<bool, ProcessFailure>),
    Output(Result<(Vec<u8>, Vec<u8>), ProcessFailure>),
    TimedOut,
}

async fn settle_process_output(
    mut output_future: std::pin::Pin<
        Box<impl Future<Output = Result<(Vec<u8>, Vec<u8>), ProcessFailure>>>,
    >,
) -> Result<(Vec<u8>, Vec<u8>), ProcessFailure> {
    let mut timer = Box::pin(Timer::after(PROCESS_CLEANUP_TIMEOUT));
    poll_fn(|context| {
        if let std::task::Poll::Ready(result) = output_future.as_mut().poll(context) {
            return std::task::Poll::Ready(result);
        }
        if let std::task::Poll::Ready(_) = timer.as_mut().poll(context) {
            return std::task::Poll::Ready(Err(ProcessFailure::CleanupUncertain));
        }
        std::task::Poll::Pending
    })
    .await
}

async fn settle_process_status(
    mut status_future: std::pin::Pin<
        Box<impl Future<Output = Result<async_process::ExitStatus, io::Error>>>,
    >,
) -> Result<bool, ProcessFailure> {
    let mut timer = Box::pin(Timer::after(PROCESS_CLEANUP_TIMEOUT));
    poll_fn(|context| {
        if let std::task::Poll::Ready(result) = status_future.as_mut().poll(context) {
            return std::task::Poll::Ready(
                result
                    .map(|status| status.success())
                    .map_err(|_| ProcessFailure::Wait),
            );
        }
        if let std::task::Poll::Ready(_) = timer.as_mut().poll(context) {
            return std::task::Poll::Ready(Err(ProcessFailure::CleanupUncertain));
        }
        std::task::Poll::Pending
    })
    .await
}

fn cleanup_process(child: &mut Child) -> bool {
    if child.try_status().ok().flatten().is_some() {
        return true;
    }
    let _ = child.kill();
    block_on(future::race(
        async { child.status().await.is_ok() },
        async {
            Timer::after(PROCESS_CLEANUP_TIMEOUT).await;
            false
        },
    ))
}

fn retain_process_failure_primary(error: ProcessFailure) -> ProcessFailure {
    match error {
        ProcessFailure::TimedOut | ProcessFailure::TimedOutCleanupUncertain => {
            ProcessFailure::TimedOutCleanupUncertain
        }
        ProcessFailure::OutputTooLarge | ProcessFailure::OutputTooLargeCleanupUncertain => {
            ProcessFailure::OutputTooLargeCleanupUncertain
        }
        ProcessFailure::OutputRead | ProcessFailure::OutputReadCleanupUncertain => {
            ProcessFailure::OutputReadCleanupUncertain
        }
        ProcessFailure::Wait | ProcessFailure::WaitCleanupUncertain => {
            ProcessFailure::WaitCleanupUncertain
        }
        ProcessFailure::CleanupUncertain => ProcessFailure::CleanupUncertain,
    }
}

async fn run_process(
    mut child: Child,
    stdout: ChildStdout,
    stderr: ChildStderr,
    timeout: Duration,
) -> Result<ProcessOutput, ProcessFailure> {
    let mut status_future = Box::pin(child.status());
    let mut output_future = Box::pin(read_process_streams(stdout, stderr));
    let mut timer = Box::pin(Timer::after(timeout));
    let event = poll_fn(|context| {
        if let std::task::Poll::Ready(result) = output_future.as_mut().poll(context) {
            return std::task::Poll::Ready(ProcessEvent::Output(result));
        }
        if let std::task::Poll::Ready(result) = status_future.as_mut().poll(context) {
            return std::task::Poll::Ready(ProcessEvent::Exited(
                result
                    .map(|status| status.success())
                    .map_err(|_| ProcessFailure::Wait),
            ));
        }
        if let std::task::Poll::Ready(_) = timer.as_mut().poll(context) {
            return std::task::Poll::Ready(ProcessEvent::TimedOut);
        }
        std::task::Poll::Pending
    })
    .await;

    match event {
        ProcessEvent::Output(result) => {
            drop(output_future);
            match result {
                Err(error) => {
                    drop(status_future);
                    if cleanup_process(&mut child) {
                        Err(error)
                    } else {
                        Err(retain_process_failure_primary(error))
                    }
                }
                Ok((stdout, stderr)) => {
                    drop(timer);
                    match settle_process_status(status_future).await {
                        Ok(status_success) => Ok(ProcessOutput {
                            status_success,
                            stdout,
                            stderr,
                        }),
                        Err(_) => {
                            if cleanup_process(&mut child) {
                                Err(ProcessFailure::Wait)
                            } else {
                                Err(ProcessFailure::WaitCleanupUncertain)
                            }
                        }
                    }
                }
            }
        }
        ProcessEvent::Exited(result) => {
            drop(status_future);
            match result {
                Err(error) => {
                    drop(output_future);
                    let _ = cleanup_process(&mut child);
                    Err(error)
                }
                Ok(status_success) => match settle_process_output(output_future).await {
                    Ok((stdout, stderr)) => Ok(ProcessOutput {
                        status_success,
                        stdout,
                        stderr,
                    }),
                    Err(error) => Err(error),
                },
            }
        }
        ProcessEvent::TimedOut => {
            drop(status_future);
            drop(output_future);
            if cleanup_process(&mut child) {
                Err(ProcessFailure::TimedOut)
            } else {
                Err(ProcessFailure::TimedOutCleanupUncertain)
            }
        }
    }
}

fn read_settings(path: &Path) -> Result<ManagedSettings, io::Error> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn write_settings_atomic(path: &Path, settings: &ManagedSettings) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "settings parent is unavailable".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|_| "settings directory could not be created".to_string())?;
    let temporary = parent.join(format!(".settings-{}.tmp", Uuid::new_v4().simple()));
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|_| "settings could not be encoded".to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "temporary settings could not be created".to_string())?;
    let mut cleanup = TemporaryFileGuard::new(temporary.clone());
    set_file_mode(&temporary, 0o600)
        .map_err(|_| "temporary settings could not be secured".to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "settings could not be synchronized".to_string())?;
    fs::rename(&temporary, path).map_err(|_| "settings could not be activated".to_string())?;
    cleanup.commit();
    Ok(())
}

fn remove_settings_file(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|_| {
            actionable_json(
                "adb_remove_failed",
                "Managed Platform-Tools settings could not be removed.",
            )
        })?;
    }
    Ok(())
}

fn checked_install_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    let components = relative_path.components().collect::<Vec<_>>();
    if components.len() != 2
        || components[0] != Component::Normal("installs".as_ref())
        || !matches!(components[1], Component::Normal(_))
    {
        return Err("managed install path is invalid".to_string());
    }
    let root = root
        .canonicalize()
        .map_err(|_| "managed root is unavailable".to_string())?;
    let candidate = root.join(relative_path);
    if candidate.exists() {
        let canonical = candidate
            .canonicalize()
            .map_err(|_| "managed install is unavailable".to_string())?;
        if !canonical.starts_with(&root) {
            return Err("managed install escaped its root".to_string());
        }
        Ok(canonical)
    } else {
        Ok(candidate)
    }
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut HashWriter(&mut hasher))?;
    Ok(hex::encode(hasher.finalize()))
}

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(debug_assertions)]
fn find_system_adb() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join("adb"))
        .find(|candidate| candidate.is_file())
}

fn setup_error(code: &str, message: &str, actions: Vec<&'static str>) -> ActionableErrorDto {
    ActionableErrorDto {
        code: code.to_string(),
        message: message.to_string(),
        actions,
    }
}

fn archive_error(code: &str, message: &str) -> ActionableErrorDto {
    setup_error(code, message, vec!["import"])
}

fn actionable_json(code: &str, message: &str) -> String {
    json!({ "code": code, "message": message }).to_string()
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

fn set_directory_mode(path: &Path, mode: u32) -> io::Result<()> {
    set_file_mode(path, mode)
}

struct StagingGuard {
    path: PathBuf,
    committed: bool,
}

struct TemporaryFileGuard {
    path: PathBuf,
    committed: bool,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl StagingGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use super::*;
    use zip::write::SimpleFileOptions;

    #[derive(Default)]
    struct FakeExecutor {
        wrong_signer: bool,
        invalid_signature: bool,
        fail_version: bool,
        process_error_code: Option<&'static str>,
        version_stdout: Option<&'static str>,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl ProcessExecutor for FakeExecutor {
        fn run(
            &self,
            program: &Path,
            args: &[&str],
            _cwd: &Path,
            _timeout: Duration,
        ) -> Result<ControlledOutput, ActionableErrorDto> {
            self.calls.lock().unwrap().push((
                program.to_string_lossy().into_owned(),
                args.iter().map(|value| value.to_string()).collect(),
            ));
            if let Some(code) = self.process_error_code {
                return Err(setup_error(
                    code,
                    "sensitive fixture process details",
                    vec!["retry"],
                ));
            }
            if program == Path::new("/usr/bin/codesign") && args.first() == Some(&"-d") {
                let team = if self.wrong_signer {
                    "WRONGTEAM"
                } else {
                    GOOGLE_TEAM_IDENTIFIER
                };
                let authority = if self.wrong_signer {
                    "Developer ID Application: Not Google (WRONGTEAM)"
                } else {
                    GOOGLE_SIGNER_AUTHORITY
                };
                return Ok(ControlledOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: format!("Authority={authority}\nTeamIdentifier={team}\n"),
                });
            }
            if program == Path::new("/usr/bin/codesign") {
                return Ok(ControlledOutput {
                    success: !self.invalid_signature,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            Ok(ControlledOutput {
                success: !self.fail_version,
                stdout: self
                    .version_stdout
                    .unwrap_or("Android Debug Bridge version 1.0.41\nVersion 37.0.0-14910828\n")
                    .to_string(),
                stderr: String::new(),
            })
        }
    }

    fn fake_adb_bytes() -> Vec<u8> {
        let mut bytes = vec![0xcf, 0xfa, 0xed, 0xfe];
        bytes.extend_from_slice(&0x0100_000cu32.to_le_bytes());
        bytes.extend_from_slice(b"fake-adb-test-body");
        bytes
    }

    fn write_platform_tools_zip(path: &Path, extra: &[(&str, u32)]) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, mode) in [
            ("platform-tools/adb", 0o100755),
            ("platform-tools/NOTICE.txt", 0o100644),
            ("platform-tools/source.properties", 0o100644),
        ]
        .into_iter()
        .chain(extra.iter().copied())
        {
            if mode & 0o170000 == 0o120000 {
                zip.add_symlink(
                    name,
                    "../../escape",
                    SimpleFileOptions::default().unix_permissions(0o777),
                )
                .unwrap();
                continue;
            }
            zip.start_file(name, SimpleFileOptions::default().unix_permissions(mode))
                .unwrap();
            match name {
                "platform-tools/adb" => zip.write_all(&fake_adb_bytes()).unwrap(),
                "platform-tools/source.properties" => zip
                    .write_all(b"Pkg.UserSrc=false\nPkg.Revision=37.0.0\n")
                    .unwrap(),
                _ => zip.write_all(b"test\n").unwrap(),
            }
        }
        zip.finish().unwrap();
    }

    fn inspect_path(path: &Path) -> Result<HashMap<&'static str, usize>, ActionableErrorDto> {
        let mut file = File::open(path).unwrap();
        inspect_archive(&mut file)
    }

    fn candidate_fixture() -> (tempfile::TempDir, HashMap<String, String>) {
        let temp = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            ("adb", fake_adb_bytes()),
            ("NOTICE.txt", b"notice".to_vec()),
            (
                "source.properties",
                b"Pkg.UserSrc=false\nPkg.Revision=37.0.0\n".to_vec(),
            ),
        ] {
            fs::write(temp.path().join(name), bytes).unwrap();
        }
        let hashes = RETAINED_FILES
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    sha256_file(&temp.path().join(name)).unwrap(),
                )
            })
            .collect();
        (temp, hashes)
    }

    #[test]
    fn version_policy_accepts_minimum_and_warns_for_newer_untested() {
        assert!(enforce_version_policy(&Version::parse("35.0.0").unwrap()).is_ok());
        assert!(enforce_version_policy(&Version::parse("34.0.5").unwrap()).is_err());
        assert!(version_warning("37.0.1").is_some());
        assert!(version_warning("37.0.0").is_none());
    }

    #[test]
    fn macho_parser_accepts_native_thin_headers_and_rejects_text() {
        let mut arm = vec![0xcf, 0xfa, 0xed, 0xfe];
        arm.extend_from_slice(&0x0100_000cu32.to_le_bytes());
        assert_eq!(parse_macho_architectures(&arm).unwrap(), vec!["aarch64"]);
        assert!(parse_macho_architectures(b"not macho").is_none());
        let mut other = vec![0xcf, 0xfa, 0xed, 0xfe];
        let other_cpu = if std::env::consts::ARCH == "aarch64" {
            0x0100_0007u32
        } else {
            0x0100_000cu32
        };
        other.extend_from_slice(&other_cpu.to_le_bytes());
        assert!(!parse_macho_architectures(&other)
            .unwrap()
            .contains(&std::env::consts::ARCH.to_string()));

        let mut universal = vec![0xca, 0xfe, 0xba, 0xbe];
        universal.extend_from_slice(&2u32.to_be_bytes());
        universal.extend_from_slice(&0x0100_0007u32.to_be_bytes());
        universal.extend_from_slice(&[0; 16]);
        universal.extend_from_slice(&0x0100_000cu32.to_be_bytes());
        universal.extend_from_slice(&[0; 16]);
        assert_eq!(
            parse_macho_architectures(&universal).unwrap(),
            vec!["x86_64", "aarch64"]
        );
    }

    #[test]
    fn unsafe_member_names_are_rejected() {
        for name in [
            "../platform-tools/adb",
            "/platform-tools/adb",
            "platform-tools\\adb",
            "other/adb",
        ] {
            assert!(validate_member_name(name).is_err(), "{name}");
        }
        assert!(validate_member_name("platform-tools/adb").is_ok());
    }

    #[test]
    fn archive_requires_expected_structure_and_rejects_case_collisions() {
        let temp = tempfile::tempdir().unwrap();
        let valid = temp.path().join("valid.zip");
        write_platform_tools_zip(&valid, &[("platform-tools/fastboot", 0o100755)]);
        assert_eq!(inspect_path(&valid).unwrap().len(), 3);

        let collision = temp.path().join("collision.zip");
        write_platform_tools_zip(&collision, &[("platform-tools/ADB", 0o100755)]);
        let error = inspect_path(&collision).unwrap_err();
        assert_eq!(error.code, "archive_structure_invalid");
    }

    #[test]
    fn archive_rejects_symlink_and_encryption_flags() {
        let temp = tempfile::tempdir().unwrap();
        let symlink = temp.path().join("symlink.zip");
        write_platform_tools_zip(&symlink, &[("platform-tools/link", 0o120777)]);
        assert_eq!(
            inspect_path(&symlink).unwrap_err().code,
            "archive_structure_invalid"
        );

        let encrypted = temp.path().join("encrypted.zip");
        write_platform_tools_zip(&encrypted, &[]);
        let mut bytes = fs::read(&encrypted).unwrap();
        for index in 0..bytes.len().saturating_sub(10) {
            if bytes[index..].starts_with(&[0x50, 0x4b, 0x03, 0x04]) {
                bytes[index + 6] |= 1;
            } else if bytes[index..].starts_with(&[0x50, 0x4b, 0x01, 0x02]) {
                bytes[index + 8] |= 1;
            }
        }
        fs::write(&encrypted, bytes).unwrap();
        assert_eq!(
            inspect_path(&encrypted).unwrap_err().code,
            "archive_encrypted"
        );
    }

    #[test]
    fn archive_entry_count_is_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("many.zip");
        let names = (0..MAX_ENTRIES)
            .map(|index| (format!("platform-tools/file-{index}"), 0o100644))
            .collect::<Vec<_>>();
        let borrowed = names
            .iter()
            .map(|(name, mode)| (name.as_str(), *mode))
            .collect::<Vec<_>>();
        write_platform_tools_zip(&path, &borrowed);
        assert_eq!(
            inspect_path(&path).unwrap_err().code,
            "archive_too_many_entries"
        );
    }

    #[test]
    fn archive_file_and_expansion_limits_are_enforced() {
        let temp = tempfile::tempdir().unwrap();
        let oversized = temp.path().join("oversized.zip");
        let file = File::create(&oversized).unwrap();
        file.set_len(MAX_ZIP_BYTES + 1).unwrap();
        assert_eq!(
            secure_open_zip(&oversized).unwrap_err().code,
            "archive_too_large"
        );

        let mut total = 0;
        assert_eq!(
            add_archive_size(&mut total, MAX_ENTRY_UNCOMPRESSED + 1)
                .unwrap_err()
                .code,
            "archive_too_large"
        );
        let mut total = MAX_TOTAL_UNCOMPRESSED;
        assert_eq!(
            add_archive_size(&mut total, 1).unwrap_err().code,
            "archive_too_large"
        );
        let mut total = u64::MAX;
        assert_eq!(
            add_archive_size(&mut total, 1).unwrap_err().code,
            "archive_too_large"
        );
    }

    #[test]
    fn archive_rejects_unreadable_missing_and_overlong_structures() {
        let temp = tempfile::tempdir().unwrap();
        let unreadable = temp.path().join("not-a-zip");
        fs::write(&unreadable, b"not a zip").unwrap();
        assert_eq!(
            inspect_path(&unreadable).unwrap_err().code,
            "archive_unreadable"
        );

        let missing = temp.path().join("missing-notice.zip");
        let file = File::create(&missing).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, bytes) in [
            ("platform-tools/adb", fake_adb_bytes()),
            (
                "platform-tools/source.properties",
                b"Pkg.Revision=37.0.0\n".to_vec(),
            ),
        ] {
            zip.start_file(
                name,
                SimpleFileOptions::default().unix_permissions(0o100644),
            )
            .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        assert_eq!(
            inspect_path(&missing).unwrap_err().code,
            "archive_structure_invalid"
        );
        assert_eq!(
            validate_member_name(&format!(
                "platform-tools/{}",
                "x".repeat(MAX_MEMBER_PATH_BYTES)
            ))
            .unwrap_err()
            .code,
            "archive_structure_invalid"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn archive_picker_source_symlink_is_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("valid.zip");
        write_platform_tools_zip(&archive, &[]);
        let link = temp.path().join("link.zip");
        std::os::unix::fs::symlink(&archive, &link).unwrap();
        assert_eq!(
            secure_open_zip(&link).unwrap_err().code,
            "archive_unreadable"
        );
    }

    #[test]
    fn google_signer_identity_is_required_before_version_execution() {
        let (temp, hashes) = candidate_fixture();
        let good = FakeExecutor::default();
        let validation = validate_candidate(temp.path(), &hashes, true, &good).unwrap();
        assert_eq!(validation.version, "37.0.0");
        let calls = good.calls.lock().unwrap();
        assert_eq!(calls[0].0, "/usr/bin/codesign");
        assert_eq!(calls[1].0, "/usr/bin/codesign");
        assert!(calls[2].1.contains(&"version".to_string()));
        drop(calls);

        let wrong = FakeExecutor {
            wrong_signer: true,
            ..FakeExecutor::default()
        };
        assert_eq!(
            validate_candidate(temp.path(), &hashes, true, &wrong)
                .unwrap_err()
                .code,
            "signer_not_google"
        );
        assert_eq!(wrong.calls.lock().unwrap().len(), 2);

        let invalid = FakeExecutor {
            invalid_signature: true,
            ..FakeExecutor::default()
        };
        assert_eq!(
            validate_candidate(temp.path(), &hashes, true, &invalid)
                .unwrap_err()
                .code,
            "signature_invalid"
        );
        assert_eq!(invalid.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn candidate_rejects_hash_architecture_source_and_reported_version_mismatches() {
        let (temp, hashes) = candidate_fixture();
        let mut wrong_hash = hashes.clone();
        wrong_hash.insert("NOTICE.txt".to_string(), "00".repeat(32));
        assert_eq!(
            validate_candidate(temp.path(), &wrong_hash, true, &FakeExecutor::default())
                .unwrap_err()
                .code,
            "managed_adb_hash_mismatch"
        );

        let other_cpu = if std::env::consts::ARCH == "aarch64" {
            0x0100_0007u32
        } else {
            0x0100_000cu32
        };
        let mut incompatible = vec![0xcf, 0xfa, 0xed, 0xfe];
        incompatible.extend_from_slice(&other_cpu.to_le_bytes());
        fs::write(temp.path().join("adb"), incompatible).unwrap();
        let mut incompatible_hashes = hashes.clone();
        incompatible_hashes.insert(
            "adb".to_string(),
            sha256_file(&temp.path().join("adb")).unwrap(),
        );
        assert_eq!(
            validate_candidate(
                temp.path(),
                &incompatible_hashes,
                true,
                &FakeExecutor::default()
            )
            .unwrap_err()
            .code,
            "architecture_incompatible"
        );

        fs::write(temp.path().join("adb"), fake_adb_bytes()).unwrap();
        let mut current_hashes = hashes;
        fs::write(
            temp.path().join("source.properties"),
            b"Pkg.Revision=not-a-version\n",
        )
        .unwrap();
        current_hashes.insert(
            "source.properties".to_string(),
            sha256_file(&temp.path().join("source.properties")).unwrap(),
        );
        assert_eq!(
            validate_candidate(temp.path(), &current_hashes, true, &FakeExecutor::default())
                .unwrap_err()
                .code,
            "source_properties_invalid"
        );

        fs::write(
            temp.path().join("source.properties"),
            b"Pkg.Revision=34.0.5\n",
        )
        .unwrap();
        current_hashes.insert(
            "source.properties".to_string(),
            sha256_file(&temp.path().join("source.properties")).unwrap(),
        );
        assert_eq!(
            validate_candidate(temp.path(), &current_hashes, true, &FakeExecutor::default())
                .unwrap_err()
                .code,
            "platform_tools_too_old"
        );

        fs::write(
            temp.path().join("source.properties"),
            b"Pkg.Revision=37.0.0\n",
        )
        .unwrap();
        current_hashes.insert(
            "source.properties".to_string(),
            sha256_file(&temp.path().join("source.properties")).unwrap(),
        );
        let mismatch = FakeExecutor {
            version_stdout: Some("Android Debug Bridge version 1.0.41\nVersion 36.0.0-00000000\n"),
            ..FakeExecutor::default()
        };
        assert_eq!(
            validate_candidate(temp.path(), &current_hashes, true, &mismatch)
                .unwrap_err()
                .code,
            "adb_version_mismatch"
        );
    }

    #[test]
    fn managed_import_retains_only_three_hashed_files_and_preserves_failed_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let zip = temp.path().join("platform-tools.zip");
        write_platform_tools_zip(&zip, &[("platform-tools/fastboot", 0o100755)]);
        let root = temp.path().join("managed");
        fs::create_dir_all(root.join("installs")).unwrap();
        fs::create_dir_all(root.join("staging")).unwrap();
        let manager = AdbManager {
            root: root.clone(),
            current: None,
            last_error: None,
            revision: 0,
        };
        let installed = manager
            .import_zip_inner_with_executor(&zip, &FakeExecutor::default())
            .unwrap();
        let install = installed.path.parent().unwrap();
        let mut names = fs::read_dir(install)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["NOTICE.txt", "adb", "source.properties"]);
        let settings_before = fs::read(root.join("settings.json")).unwrap();
        let settings: ManagedSettings = serde_json::from_slice(&settings_before).unwrap();
        assert_eq!(settings.files.len(), 3);
        assert!(RETAINED_FILES
            .iter()
            .all(|name| settings.files.contains_key(*name)));

        let failed = FakeExecutor {
            fail_version: true,
            ..FakeExecutor::default()
        };
        assert_eq!(
            manager
                .import_zip_inner_with_executor(&zip, &failed)
                .unwrap_err()
                .code,
            "adb_version_failed"
        );
        assert_eq!(
            fs::read(root.join("settings.json")).unwrap(),
            settings_before
        );
        assert_eq!(fs::read_dir(root.join("staging")).unwrap().count(), 0);
    }

    #[test]
    fn execution_revalidation_repeats_managed_checks_and_classifies_identity_changes() {
        let temp = tempfile::tempdir().unwrap();
        let zip = temp.path().join("platform-tools.zip");
        write_platform_tools_zip(&zip, &[]);
        let root = temp.path().join("managed");
        fs::create_dir_all(root.join("installs")).unwrap();
        fs::create_dir_all(root.join("staging")).unwrap();
        let mut manager = AdbManager {
            root,
            current: None,
            last_error: None,
            revision: 0,
        };
        let installed = manager
            .import_zip_inner_with_executor(&zip, &FakeExecutor::default())
            .unwrap();
        let identity = installed.identity();
        manager.current = Some(installed);
        assert_eq!(
            manager
                .readiness_snapshot()
                .evaluate_with_executor(&FakeExecutor::default()),
            PlatformToolsReadiness::Ready
        );

        assert_eq!(
            manager
                .revalidate_for_execution_with_executor(&identity, &FakeExecutor::default())
                .unwrap(),
            identity.path.canonicalize().unwrap()
        );

        let changed = AdbInstallationIdentity {
            version: "36.0.0".to_string(),
            ..identity.clone()
        };
        assert_eq!(
            manager.revalidate_for_execution_with_executor(&changed, &FakeExecutor::default()),
            Err(AdbRevalidationError::Changed)
        );

        fs::remove_file(&identity.path).unwrap();
        assert_eq!(
            manager
                .readiness_snapshot()
                .evaluate_with_executor(&FakeExecutor::default()),
            PlatformToolsReadiness::NotFound
        );
        assert_eq!(
            manager.revalidate_for_execution_with_executor(&identity, &FakeExecutor::default()),
            Err(AdbRevalidationError::Unavailable)
        );
    }

    #[test]
    fn readiness_snapshot_classifies_trusted_platform_tools_without_device_commands() {
        let temp = tempfile::tempdir().unwrap();
        let zip = temp.path().join("platform-tools.zip");
        write_platform_tools_zip(&zip, &[]);
        let root = temp.path().join("managed");
        fs::create_dir_all(root.join("installs")).unwrap();
        fs::create_dir_all(root.join("staging")).unwrap();
        let mut manager = AdbManager {
            root,
            current: None,
            last_error: None,
            revision: 7,
        };

        let missing = manager.readiness_snapshot();
        let missing_executor = FakeExecutor::default();
        assert_eq!(
            missing.evaluate_with_executor(&missing_executor),
            PlatformToolsReadiness::NotFound
        );
        assert!(missing_executor.calls.lock().unwrap().is_empty());

        let installed = manager
            .import_zip_inner_with_executor(&zip, &FakeExecutor::default())
            .unwrap();
        let adb_path = installed.path.canonicalize().unwrap();
        manager.current = Some(installed);
        let snapshot = manager.readiness_snapshot();
        assert_eq!(snapshot.adb_revision(), 7);
        let executor = FakeExecutor::default();
        assert_eq!(
            snapshot.evaluate_with_executor(&executor),
            PlatformToolsReadiness::Ready
        );
        assert_eq!(
            *executor.calls.lock().unwrap(),
            vec![
                (
                    "/usr/bin/codesign".to_string(),
                    vec![
                        "--verify".to_string(),
                        "--strict".to_string(),
                        "--verbose=2".to_string(),
                        adb_path.to_string_lossy().into_owned(),
                    ],
                ),
                (
                    "/usr/bin/codesign".to_string(),
                    vec![
                        "-d".to_string(),
                        "--verbose=4".to_string(),
                        adb_path.to_string_lossy().into_owned(),
                    ],
                ),
                (
                    adb_path.to_string_lossy().into_owned(),
                    vec!["version".to_string()],
                ),
            ]
        );
    }

    #[test]
    fn readiness_snapshot_distinguishes_invalid_tooling_from_check_failures() {
        let (temp, hashes) = candidate_fixture();
        let manager = AdbManager {
            root: temp.path().to_path_buf(),
            current: Some(ResolvedAdb {
                path: temp.path().join("adb"),
                version: "37.0.0".to_string(),
                warning: None,
                managed_relative_path: None,
            }),
            last_error: None,
            revision: 3,
        };
        let snapshot = manager.readiness_snapshot();

        assert_eq!(
            snapshot.evaluate_with_executor(&FakeExecutor {
                fail_version: true,
                ..FakeExecutor::default()
            }),
            PlatformToolsReadiness::Invalid
        );
        assert_eq!(
            snapshot.evaluate_with_executor(&FakeExecutor {
                process_error_code: Some("validation_process_timeout"),
                ..FakeExecutor::default()
            }),
            PlatformToolsReadiness::CheckFailed
        );

        let mut managed = manager.clone();
        managed.current.as_mut().unwrap().managed_relative_path =
            Some("installs/fixture".to_string());
        managed.root = temp.path().to_path_buf();
        let install = temp.path().join("installs/fixture");
        fs::create_dir_all(&install).unwrap();
        for name in RETAINED_FILES {
            fs::copy(temp.path().join(name), install.join(name)).unwrap();
        }
        let settings = ManagedSettings {
            schema_version: 1,
            install_relative_path: "installs/fixture".to_string(),
            version: "37.0.0".to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            signer_team_identifier: GOOGLE_TEAM_IDENTIFIER.to_string(),
            files: hashes,
        };
        write_settings_atomic(&temp.path().join("settings.json"), &settings).unwrap();
        managed.current.as_mut().unwrap().path = install.join("adb");
        fs::write(
            &managed.current.as_ref().unwrap().path,
            b"not a Mach-O executable",
        )
        .unwrap();
        assert_eq!(
            managed
                .readiness_snapshot()
                .evaluate_with_executor(&FakeExecutor::default()),
            PlatformToolsReadiness::Invalid
        );
    }

    #[test]
    fn readiness_process_runs_after_the_live_manager_lock_is_released() {
        use std::sync::Arc;

        struct LockCheckingExecutor {
            manager: Arc<Mutex<AdbManager>>,
            delegate: FakeExecutor,
        }

        impl ProcessExecutor for LockCheckingExecutor {
            fn run(
                &self,
                program: &Path,
                args: &[&str],
                cwd: &Path,
                timeout: Duration,
            ) -> Result<ControlledOutput, ActionableErrorDto> {
                assert!(
                    self.manager.try_lock().is_ok(),
                    "live AdbManager mutex must not be held during process execution"
                );
                self.delegate.run(program, args, cwd, timeout)
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let manager = Arc::new(Mutex::new(AdbManager {
            root: temp.path().to_path_buf(),
            current: Some(ResolvedAdb {
                path: temp.path().join("adb"),
                version: "37.0.0".to_string(),
                warning: None,
                managed_relative_path: None,
            }),
            last_error: None,
            revision: 4,
        }));
        fs::write(temp.path().join("adb"), fake_adb_bytes()).unwrap();
        let snapshot = manager.lock().unwrap().readiness_snapshot();
        let executor = LockCheckingExecutor {
            manager,
            delegate: FakeExecutor::default(),
        };

        assert_eq!(
            snapshot.evaluate_with_executor(&executor),
            PlatformToolsReadiness::Ready
        );
    }

    #[test]
    fn failed_settings_activation_and_fixed_extraction_clean_temporary_state() {
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        fs::create_dir(&settings_path).unwrap();
        let settings = ManagedSettings {
            schema_version: 1,
            install_relative_path: "installs/one".to_string(),
            version: "37.0.0".to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            signer_team_identifier: GOOGLE_TEAM_IDENTIFIER.to_string(),
            files: HashMap::new(),
        };
        assert!(write_settings_atomic(&settings_path, &settings).is_err());
        assert!(!fs::read_dir(temp.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".settings-")
        }));

        let destination = temp.path().join("retained");
        fs::write(&destination, b"existing").unwrap();
        let mut source = io::Cursor::new(b"replacement");
        assert_eq!(
            extract_fixed_file(&mut source, &destination, false)
                .unwrap_err()
                .code,
            "adb_staging_failed"
        );
        assert_eq!(fs::read(&destination).unwrap(), b"existing");
    }

    #[test]
    fn controlled_process_times_out_and_bounds_output_without_a_shell() {
        let executable = std::env::current_exe().expect("test executable should be available");
        let cwd = std::env::current_dir().expect("test working directory should be available");
        let timeout = RealProcessExecutor.run(
            &executable,
            &[
                "--exact",
                "adb::tests::process_timeout_helper",
                "--ignored",
                "--nocapture",
            ],
            &cwd,
            Duration::from_millis(10),
        );
        assert_eq!(timeout.unwrap_err().code, "validation_process_timeout");
        let excessive = RealProcessExecutor.run(
            &executable,
            &[
                "--exact",
                "adb::tests::process_output_overflow_helper",
                "--ignored",
                "--nocapture",
            ],
            &cwd,
            Duration::from_secs(1),
        );
        assert_eq!(excessive.unwrap_err().code, "validation_output_too_large");
    }

    #[test]
    #[ignore]
    fn process_timeout_helper() {
        std::thread::sleep(Duration::from_secs(60));
    }

    #[test]
    #[ignore]
    fn process_output_overflow_helper() {
        let bytes = vec![b'x'; MAX_PROCESS_OUTPUT + 1];
        std::io::stdout()
            .write_all(&bytes)
            .expect("overflow helper output should be writable");
        std::io::stdout()
            .flush()
            .expect("overflow helper output should flush");
        std::thread::sleep(Duration::from_secs(60));
    }

    #[test]
    fn managed_paths_are_fixed_beneath_install_root() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("installs/one")).unwrap();
        assert!(checked_install_path(temp.path(), "installs/one").is_ok());
        assert!(checked_install_path(temp.path(), "../outside").is_err());
        assert!(checked_install_path(temp.path(), "/tmp/outside").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn staging_orphan_and_removal_cleanup_stays_inside_managed_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("managed");
        fs::create_dir_all(root.join("staging/stale")).unwrap();
        fs::create_dir_all(root.join("installs/active")).unwrap();
        fs::create_dir_all(root.join("installs/inactive")).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "outside").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("staging/outside-link")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("installs/outside-link")).unwrap();
        let settings = ManagedSettings {
            schema_version: 1,
            install_relative_path: "installs/active".to_string(),
            version: "37.0.0".to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            signer_team_identifier: GOOGLE_TEAM_IDENTIFIER.to_string(),
            files: HashMap::new(),
        };
        write_settings_atomic(&root.join("settings.json"), &settings).unwrap();
        let mut manager = AdbManager {
            root: root.clone(),
            current: Some(ResolvedAdb {
                path: root.join("installs/active/adb"),
                version: "37.0.0".to_string(),
                warning: None,
                managed_relative_path: Some("installs/active".to_string()),
            }),
            last_error: None,
            revision: 0,
        };
        manager.cleanup_staging();
        manager.cleanup_orphans();
        assert!(!root.join("staging/stale").exists());
        assert!(root.join("staging/outside-link").exists());
        assert!(root.join("installs/active").exists());
        assert!(!root.join("installs/inactive").exists());
        assert!(root.join("installs/outside-link").exists());
        manager.remove().unwrap();
        assert!(!root.join("installs/active").exists());
        assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(), "outside");
    }

    #[test]
    fn exact_serial_or_path_never_occurs_in_public_status() {
        let manager = AdbManager {
            root: PathBuf::from("/private/path"),
            current: Some(ResolvedAdb {
                path: PathBuf::from("/private/path/adb"),
                version: "37.0.0".to_string(),
                warning: None,
                managed_relative_path: Some("installs/secret".to_string()),
            }),
            last_error: None,
            revision: 0,
        };
        let status = serde_json::to_string(&manager.status()).unwrap();
        assert!(!status.contains("private"));
        assert!(!status.contains("secret"));
    }
}
