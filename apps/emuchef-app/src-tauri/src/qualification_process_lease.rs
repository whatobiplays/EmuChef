//! Bind a configuration-create development launch to the native application process lifetime.
//!
//! The launcher creates a verified record before Tauri starts. This module
//! claims that record under the shared Git-admin lock, stores the application
//! process identity, and records a clean exit only after Tauri's event loop
//! returns.

use serde_json::{json, Value};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const TARGET_ID: &str = "konkr-pocket-advance-retroarch-config-create";
const SESSION_FILENAME: &str = "retroarch-config-create-qualification-session.json";
const LOCK_FILENAME: &str = "retroarch-config-create-sidecar-build.lock";

#[derive(Clone, Debug)]
pub struct LaunchConfig {
    pub session_path: PathBuf,
    pub lock_path: PathBuf,
    pub token: String,
    pub target_id: String,
    pub host: String,
    pub launch_pid: u32,
}

pub struct QualificationProcessLease {
    config: LaunchConfig,
    app_pid: u32,
}

struct SharedLock {
    path: PathBuf,
    token: String,
}

impl SharedLock {
    fn acquire(path: &Path, host: &str, target_id: &str) -> Result<Self, String> {
        let token = Uuid::new_v4().to_string();
        let parent = path.parent().ok_or_else(|| {
            "configuration-create lifecycle lock has no parent directory".to_owned()
        })?;
        let filename = path
            .file_name()
            .ok_or_else(|| "configuration-create lifecycle lock has no file name".to_owned())?
            .to_string_lossy();
        let temporary_path = parent.join(format!(".{filename}.tmp-{}-{token}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|error| {
                format!("could not create temporary configuration-create lifecycle lock: {error}")
            })?;
        let owner = json!({
            "schemaVersion": 1,
            "pid": std::process::id(),
            "host": host,
            "token": token,
            "profile": "qualification-lifecycle",
            "qualificationTargetId": target_id,
            "artifactPaths": [],
        });
        let record = format!(
            "{}\n",
            serde_json::to_string_pretty(&owner).map_err(|error| error.to_string())?
        );
        if let Err(error) = file
            .write_all(record.as_bytes())
            .and_then(|()| file.sync_all())
        {
            drop(file);
            let _ = fs::remove_file(&temporary_path);
            return Err(format!(
                "could not write configuration-create lifecycle lock: {error}"
            ));
        }
        drop(file);
        if let Err(error) = fs::hard_link(&temporary_path, path) {
            let _ = fs::remove_file(&temporary_path);
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                return Err(
                    "configuration-create lifecycle is locked by another host operation".to_owned(),
                );
            }
            return Err(format!(
                "could not publish configuration-create lifecycle lock: {error}"
            ));
        }
        let _ = fs::remove_file(&temporary_path);
        Ok(Self {
            path: path.to_owned(),
            token,
        })
    }
}
impl Drop for SharedLock {
    fn drop(&mut self) {
        let Ok(bytes) = fs::read(&self.path) else {
            return;
        };
        let Ok(owner) = serde_json::from_slice::<Value>(&bytes) else {
            return;
        };
        if owner.get("token").and_then(Value::as_str) == Some(self.token.as_str()) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl QualificationProcessLease {
    /// Claim the launcher-created record for the running native app process.
    pub fn start(config: LaunchConfig) -> Result<Self, String> {
        validate_config(&config)?;
        let _lock = SharedLock::acquire(&config.lock_path, &config.host, &config.target_id)?;
        ensure_process_alive(config.launch_pid)?;
        let mut record = read_session(&config.session_path)?;
        validate_verified_record(&record, &config)?;
        record["status"] = json!("active");
        record["appProcess"] = json!({
            "pid": std::process::id(),
            "host": config.host,
        });
        record["startedAtUnixMillis"] = json!(now_millis());
        write_session_atomically(&config.session_path, &record)?;
        Ok(Self {
            config,
            app_pid: std::process::id(),
        })
    }

    /// Record a clean app exit after the native Tauri event loop returns.
    pub fn finish(self) -> Result<(), String> {
        let _lock = SharedLock::acquire(
            &self.config.lock_path,
            &self.config.host,
            &self.config.target_id,
        )?;
        let mut record = read_session(&self.config.session_path)?;
        if record.get("token").and_then(Value::as_str) != Some(self.config.token.as_str())
            || record.get("qualificationTargetId").and_then(Value::as_str)
                != Some(self.config.target_id.as_str())
            || record.get("status").and_then(Value::as_str) != Some("active")
            || record.pointer("/appProcess/pid").and_then(Value::as_u64)
                != Some(u64::from(self.app_pid))
            || record.pointer("/appProcess/host").and_then(Value::as_str)
                != Some(self.config.host.as_str())
        {
            return Err(
                "configuration-create session ownership changed before clean exit".to_owned(),
            );
        }
        record["status"] = json!("ended-cleanly");
        record["endedAtUnixMillis"] = json!(now_millis());
        write_session_atomically(&self.config.session_path, &record)
    }

    /// Read the launch lease passed to the native process by the Tauri launcher.
    pub fn start_from_environment() -> Result<Option<Self>, String> {
        if env::var("EMUCHEF_CONFIG_CREATE_QUALIFICATION").as_deref() != Ok("1") {
            return Ok(None);
        }
        if !cfg!(feature = "real-execution") {
            return Err(
                "configuration-create qualification requires the real-execution feature".to_owned(),
            );
        }
        let required = |name: &str| {
            env::var(name).map_err(|_| format!("configuration-create launch is missing {name}"))
        };
        let launch_pid = required("EMUCHEF_CONFIG_CREATE_QUALIFICATION_LAUNCH_PID")?
            .parse::<u32>()
            .map_err(|_| "configuration-create launch PID is invalid".to_owned())?;
        let config = LaunchConfig {
            session_path: PathBuf::from(required(
                "EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_FILE",
            )?),
            lock_path: PathBuf::from(required(
                "EMUCHEF_CONFIG_CREATE_QUALIFICATION_BUILD_LOCK_FILE",
            )?),
            token: required("EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_TOKEN")?,
            target_id: required("EMUCHEF_CONFIG_CREATE_QUALIFICATION_TARGET_ID")?,
            host: required("EMUCHEF_CONFIG_CREATE_QUALIFICATION_SESSION_HOST")?,
            launch_pid,
        };
        Self::start(config).map(Some)
    }
}

fn validate_config(config: &LaunchConfig) -> Result<(), String> {
    if config.target_id != TARGET_ID {
        return Err(
            "configuration-create launch target does not match the native qualification target"
                .to_owned(),
        );
    }
    if config.token.is_empty()
        || config.host.is_empty()
        || config.launch_pid == 0
        || !config.session_path.is_absolute()
        || !config.lock_path.is_absolute()
    {
        return Err("configuration-create launch authority is incomplete".to_owned());
    }
    if config
        .session_path
        .file_name()
        .and_then(|name| name.to_str())
        != Some(SESSION_FILENAME)
        || config.lock_path.file_name().and_then(|name| name.to_str()) != Some(LOCK_FILENAME)
        || config.session_path.parent() != config.lock_path.parent()
    {
        return Err("configuration-create launch control paths are unexpected".to_owned());
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_process_alive(pid: u32) -> Result<(), String> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| {
        "configuration-create launch owner PID is outside the host process range".to_owned()
    })?;
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return Ok(());
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EPERM) => Ok(()),
        Some(libc::ESRCH) => Err("configuration-create launch owner process has ended".to_owned()),
        _ => Err("configuration-create launch owner process state is uncertain".to_owned()),
    }
}

#[cfg(not(unix))]
fn ensure_process_alive(_pid: u32) -> Result<(), String> {
    Err("configuration-create launch process verification is unsupported on this host".to_owned())
}

fn read_session(path: &Path) -> Result<Value, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!("could not inspect the configuration-create session lease: {error}")
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("configuration-create session lease is not a regular file".to_owned());
    }
    let bytes = fs::read(path).map_err(|error| {
        format!("could not read the configuration-create session lease: {error}")
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("configuration-create session lease is malformed: {error}"))
}

fn validate_verified_record(record: &Value, config: &LaunchConfig) -> Result<(), String> {
    let launch_owner_matches = record.pointer("/launchOwner/pid").and_then(Value::as_u64)
        == Some(u64::from(config.launch_pid))
        && record.pointer("/launchOwner/host").and_then(Value::as_str)
            == Some(config.host.as_str());
    if record.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || record.get("qualificationTargetId").and_then(Value::as_str)
            != Some(config.target_id.as_str())
        || record.get("token").and_then(Value::as_str) != Some(config.token.as_str())
        || record.get("status").and_then(Value::as_str) != Some("verified")
        || !launch_owner_matches
        || !record.get("appProcess").is_some_and(Value::is_null)
    {
        return Err(
            "configuration-create verified launch lease does not match this app process".to_owned(),
        );
    }
    let artifact = record.get("verifiedArtifact").ok_or_else(|| {
        "configuration-create launch lease has no verified artifact identity".to_owned()
    })?;
    let required_digest = |key: &str| {
        artifact
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(is_sha256)
    };
    let features = artifact
        .get("backendFeatures")
        .and_then(Value::as_array)
        .map(|features| {
            features
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
        });
    let sidecar = artifact.get("sidecarSha256").and_then(Value::as_str);
    let runtime = artifact.get("runtimeSidecarSha256").and_then(Value::as_str);
    if !required_digest("manifestSha256")
        || !required_digest("materialBuildDigest")
        || !required_digest("materialInputsDigest")
        || !required_digest("arm64HelperSha256")
        || !required_digest("sidecarSha256")
        || sidecar != runtime
        || artifact
            .get("gitCommit")
            .and_then(Value::as_str)
            .is_none_or(|commit| {
                commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        || features != Some(vec!["real-execution", "android-copy-commit-helper"])
    {
        return Err(
            "configuration-create launch artifact identity is incomplete or inconsistent"
                .to_owned(),
        );
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn write_session_atomically(path: &Path, record: &Value) -> Result<(), String> {
    let temporary_path = path.with_file_name(format!(
        ".{SESSION_FILENAME}.tmp-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|error| format!("could not encode configuration-create session lease: {error}"))?;
    let result = (|| {
        let mut file: File = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|error| {
                format!("could not create configuration-create session update: {error}")
            })?;
        file.write_all(&bytes).map_err(|error| {
            format!("could not write configuration-create session update: {error}")
        })?;
        file.sync_all().map_err(|error| {
            format!("could not sync configuration-create session update: {error}")
        })?;
        fs::rename(&temporary_path, path).map_err(|error| {
            format!("could not publish configuration-create session update: {error}")
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    result
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
