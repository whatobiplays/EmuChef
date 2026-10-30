use std::fmt;
use std::path::Path;

#[cfg(test)]
use std::collections::VecDeque;

use crate::owned_process::{
    run_owned_process, OwnedProcessError, ProcessCleanup, ProcessFailureKind, ProcessOperation,
};
#[cfg(test)]
use crate::owned_process::{run_owned_process_observed, OwnedProcessObservationHandle};
use crate::planner::{RootShellRoute, TargetDeviceBinding};

use super::copy_commit::{CopyCommitMutation, CopyEntryKind, CopyManifestEntry};
use super::identity::{IdentityCheckPhase, POST_OPERATION_IDENTITY_FAILURE_MARKER};
use super::root_authority::{self, DeviceCommandEffect};
use super::root_requirements::is_app_private_path;
use super::PermissionOperation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdbCommandResult {
    pub args: Vec<String>,
    pub returncode: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdbCommandError {
    Resolution(String),
    CommandFailed {
        args: Vec<String>,
        returncode: i32,
        stderr: String,
    },
    DeviceOffline,
    DeviceUnauthorized,
    DeviceDisconnected,
    DeviceStorageExhausted,
    AdbServerUnavailable,
    TransportReset,
    TransportFailure,
    DeviceIdentityChanged {
        phase: IdentityCheckPhase,
    },
    DeviceIdentityUnverified {
        phase: IdentityCheckPhase,
    },
    ProcessFailed {
        kind: ProcessFailureKind,
        cleanup: ProcessCleanup,
    },
    TimedOut {
        cleanup: ProcessCleanup,
    },
    RootDenied,
    RootUnavailable,
    RootCheckFailed,
    RootAuthorityRevoked,
    RootAuthorityUnverified,
}

/// Generic public result message for a completed ADB command failure.
pub(crate) const ADB_COMMAND_FAILURE_MESSAGE: &str = "The ADB command failed.";
/// Generic public result message for an interrupted ADB transport.
pub(crate) const DEVICE_TRANSPORT_INTERRUPTED_MESSAGE: &str =
    "The device connection was interrupted during execution.";
/// Stable public result message when reviewed root authority is revoked.
pub(crate) const ROOT_AUTHORITY_REVOKED_MESSAGE: &str =
    "Root authority was revoked during execution.";

impl fmt::Display for AdbCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdbCommandError::Resolution(message) => formatter.write_str(message),
            AdbCommandError::CommandFailed {
                args,
                returncode,
                stderr,
            } => write!(
                formatter,
                "The ADB command failed (exit {returncode}): {}; stderr: {stderr}",
                shell_join(args)
            ),
            AdbCommandError::DeviceOffline => {
                formatter.write_str("The reviewed device is offline.")
            }
            AdbCommandError::DeviceUnauthorized => {
                formatter.write_str("The reviewed device is unauthorized.")
            }
            AdbCommandError::DeviceDisconnected => {
                formatter.write_str("The reviewed device is disconnected or missing.")
            }
            AdbCommandError::DeviceStorageExhausted => {
                formatter.write_str("The device ran out of storage during execution.")
            }
            AdbCommandError::AdbServerUnavailable => {
                formatter.write_str("The local ADB service is unavailable.")
            }
            AdbCommandError::TransportReset => {
                formatter.write_str(DEVICE_TRANSPORT_INTERRUPTED_MESSAGE)
            }
            AdbCommandError::TransportFailure => {
                formatter.write_str("The device connection lost during execution.")
            }
            AdbCommandError::DeviceIdentityChanged { phase }
            | AdbCommandError::DeviceIdentityUnverified { phase } => {
                if *phase == IdentityCheckPhase::PostOperation {
                    formatter.write_str(POST_OPERATION_IDENTITY_FAILURE_MARKER)
                } else {
                    formatter.write_str("The reviewed device identity could not be confirmed.")
                }
            }
            AdbCommandError::ProcessFailed { .. } => {
                formatter.write_str("The ADB process failed before producing trustworthy result.")
            }
            AdbCommandError::TimedOut { .. } => formatter.write_str("The ADB operation timed out."),
            AdbCommandError::RootDenied => formatter.write_str("Root access denied by device."),
            AdbCommandError::RootUnavailable => {
                formatter.write_str("Root access unavailable on this device.")
            }
            AdbCommandError::RootCheckFailed => {
                formatter.write_str("Root access could not be checked safely.")
            }
            AdbCommandError::RootAuthorityRevoked => {
                formatter.write_str(ROOT_AUTHORITY_REVOKED_MESSAGE)
            }
            AdbCommandError::RootAuthorityUnverified => {
                formatter.write_str("Continued root authority could not be confirmed safely.")
            }
        }
    }
}

/// Keep command failures actionable without retaining command output that may contain copied data.
const MAX_ADB_DIAGNOSTIC_INPUT_CHARS: usize = 4096;
const MAX_ADB_DIAGNOSTIC_CHARS: usize = 1024;
const MAX_ADB_DIAGNOSTIC_TOKEN_CHARS: usize = 256;
const REDACTED_DIAGNOSTIC_VALUE: &str = "[REDACTED]";

fn command_failed(result: &AdbCommandResult) -> AdbCommandError {
    let mut redact_next = 0;
    let args = result
        .args
        .iter()
        .map(|argument| sanitize_adb_diagnostic_text(argument, &mut redact_next))
        .collect();
    let mut redact_stderr_next = 0;
    let stderr = sanitize_adb_diagnostic_text(&result.stderr, &mut redact_stderr_next);

    AdbCommandError::CommandFailed {
        args,
        returncode: result.returncode,
        stderr,
    }
}

fn sanitize_adb_diagnostic_text(value: &str, redact_next: &mut usize) -> String {
    let mut input_chars = value.chars();
    let bounded_input = input_chars
        .by_ref()
        .take(MAX_ADB_DIAGNOSTIC_INPUT_CHARS)
        .collect::<String>();
    let input_was_truncated = input_chars.next().is_some();
    let mut safe_tokens = Vec::new();

    for token in bounded_input.split_whitespace() {
        if *redact_next > 0 {
            safe_tokens.push(REDACTED_DIAGNOSTIC_VALUE.to_string());
            *redact_next -= 1;
            if token.eq_ignore_ascii_case("bearer") {
                *redact_next += 1;
            }
            continue;
        }

        if let Some((index, delimiter)) = token
            .char_indices()
            .find(|(_, character)| *character == '=' || *character == ':')
        {
            let key = &token[..index];
            if is_sensitive_adb_diagnostic_key(key) {
                safe_tokens.push(format!("{key}{delimiter}{REDACTED_DIAGNOSTIC_VALUE}"));
                if key.eq_ignore_ascii_case("authorization")
                    && token[index + delimiter.len_utf8()..].eq_ignore_ascii_case("bearer")
                {
                    *redact_next = 1;
                }
                continue;
            }
        }

        if is_sensitive_adb_diagnostic_key(token) || token.eq_ignore_ascii_case("bearer") {
            safe_tokens.push(token.to_string());
            *redact_next = 1;
        } else if token.chars().count() > MAX_ADB_DIAGNOSTIC_TOKEN_CHARS {
            safe_tokens.push("[REDACTED-LONG-VALUE]".to_string());
        } else {
            safe_tokens.push(token.to_string());
        }
    }

    if input_was_truncated {
        safe_tokens.push("[TRUNCATED]".to_string());
    }

    let safe_text = safe_tokens.join(" ");
    let mut safe_chars = safe_text.chars();
    let bounded_safe_text = safe_chars
        .by_ref()
        .take(MAX_ADB_DIAGNOSTIC_CHARS)
        .collect::<String>();
    if safe_chars.next().is_some() {
        format!("{bounded_safe_text}[TRUNCATED]")
    } else {
        bounded_safe_text
    }
}

fn is_sensitive_adb_diagnostic_key(key: &str) -> bool {
    let normalized = key
        .trim_start_matches('-')
        .trim_matches(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .replace('-', "_")
        .to_ascii_lowercase();

    matches!(
        normalized.as_str(),
        "password"
            | "passwd"
            | "token"
            | "secret"
            | "api_key"
            | "apikey"
            | "api_secret"
            | "authorization"
            | "cookie"
            | "private_key"
            | "client_secret"
            | "access_key"
            | "refresh_token"
            | "credential"
            | "credentials"
    )
}

pub trait AdbCommandExecutor: fmt::Debug {
    fn run_for(
        &mut self,
        args: &[String],
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError>;
}

#[derive(Debug, Default)]
pub struct ProcessAdbCommandExecutor {
    #[cfg(test)]
    process_observer: Option<OwnedProcessObservationHandle>,
}

impl ProcessAdbCommandExecutor {
    #[cfg(test)]
    pub(crate) fn with_process_observer(observer: OwnedProcessObservationHandle) -> Self {
        Self {
            process_observer: Some(observer),
        }
    }
}

impl AdbCommandExecutor for ProcessAdbCommandExecutor {
    fn run_for(
        &mut self,
        args: &[String],
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        let (executable, command_args) = args.split_first().ok_or_else(|| {
            AdbCommandError::Resolution("ADB command must not be empty.".to_string())
        })?;
        #[cfg(test)]
        let output = match self.process_observer.as_ref() {
            Some(observer) => {
                run_owned_process_observed(executable, command_args, operation, observer.clone())
            }
            None => run_owned_process(executable, command_args, operation),
        }
        .map_err(|error| map_process_error(args, error))?;
        #[cfg(not(test))]
        let output = run_owned_process(executable, command_args, operation)
            .map_err(|error| map_process_error(args, error))?;
        classify_completed_transport(AdbCommandResult {
            args: args.to_vec(),
            returncode: output.status_code.unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootProbeOutcome {
    Granted,
    Denied,
    Unavailable,
    CheckFailed {
        reason: RootProbeFailureReason,
        message: &'static str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootProbeFailureReason {
    TimedOut,
    Transport,
    UnexpectedResponse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RootProbeEvidence {
    pub outcome: RootProbeOutcome,
    pub route: Option<RootShellRoute>,
}

impl RootProbeEvidence {
    pub(crate) fn status_json(&self) -> serde_json::Value {
        let mut value = self.outcome.status_json();
        if let Some(route) = self.route {
            value["rootShellRoute"] =
                serde_json::to_value(route).expect("a root-shell route is always serializable");
        }
        value
    }
}

impl RootProbeOutcome {
    pub fn status_json(&self) -> serde_json::Value {
        match self {
            Self::Granted => serde_json::json!({ "status": "granted" }),
            Self::Denied => serde_json::json!({ "status": "denied" }),
            Self::Unavailable => serde_json::json!({ "status": "unavailable" }),
            Self::CheckFailed { reason, message } => serde_json::json!({
                "status": "checkFailed",
                "reason": match reason {
                    RootProbeFailureReason::TimedOut => "timedOut",
                    RootProbeFailureReason::Transport => "transport",
                    RootProbeFailureReason::UnexpectedResponse => "unexpectedResponse",
                },
                "message": message,
            }),
        }
    }
}

#[cfg(test)]
pub fn probe_root<E: AdbCommandExecutor>(
    executor: &mut E,
    adb_path: &str,
    serial: &str,
) -> RootProbeOutcome {
    match probe_root_typed(executor, adb_path, serial) {
        Ok(outcome) => outcome,
        Err(AdbCommandError::TimedOut { .. }) => RootProbeOutcome::CheckFailed {
            reason: RootProbeFailureReason::TimedOut,
            message: "Root authorization timed out. Try again.",
        },
        Err(error) => {
            if error.is_transport_failure() {
                RootProbeOutcome::CheckFailed {
                    reason: RootProbeFailureReason::Transport,
                    message: "The device connection failed during the root access check.",
                }
            } else {
                RootProbeOutcome::CheckFailed {
                    reason: RootProbeFailureReason::UnexpectedResponse,
                    message: "The root access check could not be completed.",
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn probe_root_typed<E: AdbCommandExecutor>(
    executor: &mut E,
    adb_path: &str,
    serial: &str,
) -> Result<RootProbeOutcome, AdbCommandError> {
    probe_root_route_typed(executor, adb_path, serial).map(|evidence| evidence.outcome)
}

pub(crate) fn probe_root_with_route<E: AdbCommandExecutor>(
    executor: &mut E,
    adb_path: &str,
    serial: &str,
) -> RootProbeEvidence {
    match probe_root_route_typed(executor, adb_path, serial) {
        Ok(evidence) => evidence,
        Err(AdbCommandError::TimedOut { .. }) => RootProbeEvidence {
            outcome: RootProbeOutcome::CheckFailed {
                reason: RootProbeFailureReason::TimedOut,
                message: "Root authorization timed out. Try again.",
            },
            route: None,
        },
        Err(error) => {
            let (reason, message) = if error.is_transport_failure() {
                (
                    RootProbeFailureReason::Transport,
                    "The device connection failed during root access check.",
                )
            } else {
                (
                    RootProbeFailureReason::UnexpectedResponse,
                    "The root access check could not be completed.",
                )
            };
            RootProbeEvidence {
                outcome: RootProbeOutcome::CheckFailed { reason, message },
                route: None,
            }
        }
    }
}

fn probe_root_route_typed<E: AdbCommandExecutor>(
    executor: &mut E,
    adb_path: &str,
    serial: &str,
) -> Result<RootProbeEvidence, AdbCommandError> {
    let su_command_result =
        run_root_probe_with_mode(executor, adb_path, serial, RootShellMode::SuCommand)?;
    let su_command_outcome = classify_root_result(&su_command_result);
    if su_command_outcome == RootProbeOutcome::Granted {
        return Ok(RootProbeEvidence {
            outcome: su_command_outcome,
            route: Some(RootShellRoute::SuCommand),
        });
    }
    if !is_unsupported_su_command(&su_command_result) {
        return Ok(RootProbeEvidence {
            outcome: su_command_outcome,
            route: None,
        });
    }

    let su_zero_result =
        run_root_probe_with_mode(executor, adb_path, serial, RootShellMode::SuUserId)?;
    let su_zero_outcome = classify_root_result(&su_zero_result);
    Ok(RootProbeEvidence {
        route: (su_zero_outcome == RootProbeOutcome::Granted).then_some(RootShellRoute::SuZero),
        outcome: su_zero_outcome,
    })
}

fn run_root_probe_with_mode<E: AdbCommandExecutor>(
    executor: &mut E,
    adb_path: &str,
    serial: &str,
    root_shell_mode: RootShellMode,
) -> Result<AdbCommandResult, AdbCommandError> {
    let mut args = vec![
        adb_path.to_string(),
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
    ];
    match root_shell_mode {
        RootShellMode::SuCommand => {
            args.extend(["su".to_string(), "-c".to_string(), "id".to_string()]);
        }
        RootShellMode::SuUserId => {
            args.extend(["su".to_string(), "0".to_string(), "id".to_string()]);
        }
    }
    let result = executor.run_for(&args, ProcessOperation::RootPreflight)?;
    classify_completed_transport(result)
}

fn is_unsupported_su_command(result: &AdbCommandResult) -> bool {
    if result.returncode == 0 {
        return false;
    }
    let output = format!("{}\n{}", result.stdout, result.stderr).to_ascii_lowercase();
    output.contains("invalid uid/gid '-c'")
}

fn root_identity_has_uid_zero(stdout: &str) -> bool {
    let normalized = stdout.trim().to_ascii_lowercase();
    normalized == "0"
        || normalized
            .split_whitespace()
            .any(|part| part.starts_with("uid=0(") || part == "uid=0")
}

fn root_identity_has_verified_context(stdout: &str) -> bool {
    stdout.split_whitespace().any(|part| {
        let Some(context) = part.strip_prefix("context=") else {
            return false;
        };
        let mut components = context.split(':');
        let user = components.next();
        let role = components.next();
        let domain = components.next();
        let level = components.next();
        matches!(user, Some("u"))
            && matches!(role, Some("r"))
            && matches!(domain, Some("su" | "magisk" | "superuser"))
            && level.is_some_and(|level| level.starts_with('s'))
    })
}

fn probe_root_typed_with_mode<E: AdbCommandExecutor>(
    executor: &mut E,
    adb_path: &str,
    serial: &str,
    root_shell_mode: RootShellMode,
) -> Result<RootProbeOutcome, AdbCommandError> {
    let result = run_root_probe_with_mode(executor, adb_path, serial, root_shell_mode)?;
    Ok(classify_root_result(&result))
}

fn classify_root_result(result: &AdbCommandResult) -> RootProbeOutcome {
    if root_identity_has_uid_zero(&result.stdout)
        && root_identity_has_verified_context(&result.stdout)
    {
        return RootProbeOutcome::Granted;
    }
    let combined = format!("{}\n{}", result.stdout, result.stderr);
    let lower = combined.to_ascii_lowercase();
    if result.returncode == 0 && root_identity_has_uid_zero(&result.stdout) {
        RootProbeOutcome::CheckFailed {
            reason: RootProbeFailureReason::UnexpectedResponse,
            message: "The root access check returned an unexpected response.",
        }
    } else if is_unavailable_root(&lower) {
        RootProbeOutcome::Unavailable
    } else if is_denied_root(&lower) {
        RootProbeOutcome::Denied
    } else {
        RootProbeOutcome::CheckFailed {
            reason: RootProbeFailureReason::UnexpectedResponse,
            message: "The root access check returned an unexpected response.",
        }
    }
}

/// Interprets the fixed `test` status protocol used by CopyCommit path checks.
/// Android `test` returns 1 for a false predicate; diagnostics and every other
/// status indicate that the check itself did not complete safely.
fn copy_commit_predicate_result(result: &AdbCommandResult) -> Result<bool, AdbCommandError> {
    if result.stdout.is_empty() && result.stderr.is_empty() {
        match result.returncode {
            0 => return Ok(true),
            1 => return Ok(false),
            _ => {}
        }
    }

    Err(AdbCommandError::CommandFailed {
        args: result.args.clone(),
        returncode: result.returncode,
        stderr: "CopyCommit path predicate could not be evaluated safely".to_string(),
    })
}

impl AdbCommandError {
    fn allows_post_identity_probe(&self) -> bool {
        matches!(
            self,
            Self::CommandFailed { .. }
                | Self::RootDenied
                | Self::RootUnavailable
                | Self::RootCheckFailed
                | Self::DeviceStorageExhausted
        )
    }

    fn is_transport_failure(&self) -> bool {
        matches!(
            self,
            Self::DeviceOffline
                | Self::DeviceUnauthorized
                | Self::DeviceDisconnected
                | Self::AdbServerUnavailable
                | Self::TransportReset
                | Self::TransportFailure
        )
    }
}

fn is_unavailable_root(text: &str) -> bool {
    [
        "su: not found",
        "su: inaccessible",
        "command not found",
        "no such file or directory",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn is_denied_root(text: &str) -> bool {
    [
        "permission denied",
        "not permitted",
        "operation not permitted",
        "access denied",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[derive(Debug)]
pub struct AdbCommandRunner<E: AdbCommandExecutor = ProcessAdbCommandExecutor> {
    executable: String,
    serial: Option<String>,
    executor: E,
}

impl AdbCommandRunner<ProcessAdbCommandExecutor> {
    pub fn new(executable: impl Into<String>, serial: Option<String>) -> Self {
        Self::with_executor(executable, serial, ProcessAdbCommandExecutor::default())
    }
}

impl<E: AdbCommandExecutor> AdbCommandRunner<E> {
    pub fn with_executor(
        executable: impl Into<String>,
        serial: Option<String>,
        executor: E,
    ) -> Self {
        Self {
            executable: executable.into(),
            serial,
            executor,
        }
    }

    #[cfg(test)]
    pub fn executor(&self) -> &E {
        &self.executor
    }

    pub(super) fn serial(&self) -> Option<&str> {
        self.serial.as_deref()
    }

    pub(super) fn run_identity_command(
        &mut self,
        args: Vec<String>,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        self.run(args, true, ProcessOperation::Probe)
    }

    fn run(
        &mut self,
        args: Vec<String>,
        check: bool,
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        let mut full_args = vec![self.executable.clone()];
        if let Some(serial) = self.serial.as_deref() {
            full_args.push("-s".to_string());
            full_args.push(serial.to_string());
        }
        full_args.extend(args);
        self.run_raw(full_args, check, operation)
    }

    fn run_raw(
        &mut self,
        full_args: Vec<String>,
        check: bool,
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        let result = self.executor.run_for(&full_args, operation)?;
        let result = classify_completed_transport(result)?;
        if classify_completed_storage(operation, &full_args, &result) {
            return Err(AdbCommandError::DeviceStorageExhausted);
        }
        if check && result.returncode != 0 {
            if operation == ProcessOperation::Install {
                Err(classify_install_rejection(&result))
            } else {
                Err(command_failed(&result))
            }
        } else {
            Ok(result)
        }
    }
}

/// Convert Android package-manager rejection markers to stable, sanitized
/// categories without retaining command output, package names, or paths.
fn classify_install_rejection(result: &AdbCommandResult) -> AdbCommandError {
    let diagnostic = format!("{}\n{}", result.stdout, result.stderr).to_ascii_uppercase();
    if diagnostic.contains("INSTALL_FAILED_VERSION_DOWNGRADE") {
        return AdbCommandError::Resolution(
            "android_version_failed: Android rejected a version downgrade.".to_owned(),
        );
    }

    const SIGNING_FAILURES: [&str; 4] = [
        "INSTALL_FAILED_UPDATE_INCOMPATIBLE",
        "INSTALL_FAILED_SHARED_USER_INCOMPATIBLE",
        "INSTALL_PARSE_FAILED_NO_CERTIFICATES",
        "INSTALL_PARSE_FAILED_INCONSISTENT_CERTIFICATES",
    ];
    if SIGNING_FAILURES
        .iter()
        .any(|marker| diagnostic.contains(marker))
    {
        return AdbCommandError::Resolution(
            "android_signing_failed: Android rejected the APK signing identity.".to_owned(),
        );
    }

    if diagnostic.contains("INSTALL_FAILED_") || diagnostic.contains("INSTALL_PARSE_FAILED_") {
        return AdbCommandError::Resolution(
            "android_install_failed: Android rejected the APK installation.".to_owned(),
        );
    }

    command_failed(result)
}

/// Bind generic reviewed-plan commands to a known mutating operation shape
/// before storage classification.  Unknown commands remain a defensive
/// fallback and their echoed text cannot manufacture an ENOSPC result.
fn reviewed_plan_operation(args: &[String]) -> ProcessOperation {
    let args = if args.first().map(String::as_str) == Some("-s") {
        args.get(2..).unwrap_or_default()
    } else {
        args
    };
    match args.first().map(String::as_str) {
        Some("install") => ProcessOperation::Install,
        Some("push") => ProcessOperation::Push,
        Some("shell") => match args.get(1).map(String::as_str) {
            Some("mkdir" | "mkdirs" | "rm" | "cp" | "mv" | "touch" | "dd" | "unzip") => {
                ProcessOperation::ShellMutation
            }
            Some("pm")
                if matches!(
                    args.get(2).map(String::as_str),
                    Some("grant" | "revoke" | "clear")
                ) =>
            {
                ProcessOperation::ShellMutation
            }
            Some("appops") if args.get(2).map(String::as_str) == Some("set") => {
                ProcessOperation::ShellMutation
            }
            _ => ProcessOperation::GenericFallback,
        },
        _ => ProcessOperation::GenericFallback,
    }
}

/// Stage the helper embedded in the backend executable for `adb push`.
///
/// Tauri packages the backend sidecar, not Cargo's build directory. Embedding
/// the Android helper keeps the packaged sidecar self-contained; a private
/// temporary file supplies the local pathname required by `adb push`.
#[cfg(feature = "android-copy-commit-helper")]
fn stage_android_copy_commit_helper(abi: &str) -> Result<tempfile::NamedTempFile, AdbCommandError> {
    let payload: &[u8] = match abi {
        "arm64-v8a" => include_bytes!(env!("EMU_CHEF_COPY_COMMIT_HELPER_AARCH64")),
        "x86_64" => include_bytes!(env!("EMU_CHEF_COPY_COMMIT_HELPER_X86_64")),
        other => {
            return Err(AdbCommandError::Resolution(format!(
                "selected-root helper does not include Android binary device ABI {other:?} (supported: arm64-v8a, x86_64)"
            )));
        }
    };
    let mut staged = tempfile::NamedTempFile::new().map_err(|_| {
        AdbCommandError::Resolution("selected-root helper payload could not be staged".into())
    })?;
    std::io::Write::write_all(&mut staged, payload).map_err(|_| {
        AdbCommandError::Resolution("selected-root helper payload could not be written".into())
    })?;
    Ok(staged)
}

#[derive(Debug)]
pub struct RealAdbDevice<E: AdbCommandExecutor = ProcessAdbCommandExecutor> {
    runner: AdbCommandRunner<E>,
    last_error: Option<AdbCommandError>,
    identity: super::identity::IdentityGuard,
    root_authority: root_authority::RootAuthorityGuard,
    root_shell_mode: RootShellMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootShellMode {
    SuCommand,
    SuUserId,
}

impl From<RootShellRoute> for RootShellMode {
    fn from(route: RootShellRoute) -> Self {
        match route {
            RootShellRoute::SuCommand => Self::SuCommand,
            RootShellRoute::SuZero => Self::SuUserId,
        }
    }
}

impl RealAdbDevice<ProcessAdbCommandExecutor> {
    pub fn new(executable: impl Into<String>, serial: Option<String>) -> Self {
        Self {
            runner: AdbCommandRunner::new(executable, serial),
            last_error: None,
            identity: super::identity::IdentityGuard::default(),
            root_authority: root_authority::RootAuthorityGuard::default(),
            root_shell_mode: RootShellMode::SuCommand,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_process_observer(
        executable: impl Into<String>,
        serial: Option<String>,
        observer: OwnedProcessObservationHandle,
    ) -> Self {
        Self {
            runner: AdbCommandRunner::with_executor(
                executable,
                serial,
                ProcessAdbCommandExecutor::with_process_observer(observer),
            ),
            last_error: None,
            identity: super::identity::IdentityGuard::default(),
            root_authority: root_authority::RootAuthorityGuard::default(),
            root_shell_mode: RootShellMode::SuCommand,
        }
    }
}

impl<E: AdbCommandExecutor> RealAdbDevice<E> {
    #[cfg(test)]
    pub fn with_executor(executable: impl Into<String>, serial: Option<&str>, executor: E) -> Self {
        Self {
            runner: AdbCommandRunner::with_executor(
                executable,
                serial.map(ToString::to_string),
                executor,
            ),
            last_error: None,
            identity: super::identity::IdentityGuard::default(),
            root_authority: root_authority::RootAuthorityGuard::default(),
            root_shell_mode: RootShellMode::SuCommand,
        }
    }

    #[cfg(test)]
    pub fn command_executor(&self) -> &E {
        self.runner.executor()
    }

    pub(crate) fn take_last_error(&mut self) -> Option<AdbCommandError> {
        self.last_error.take()
    }

    pub(crate) fn configure_identity_guard(&mut self, target: Option<&TargetDeviceBinding>) {
        if target.is_some() {
            self.identity.configure(target);
        } else {
            self.identity.disable();
        }
    }

    pub(crate) fn configure_root_authority(&mut self, reviewed_root_authorized: bool) {
        self.root_authority.configure(reviewed_root_authorized);
    }

    pub(crate) fn configure_root_shell_route(&mut self, route: Option<RootShellRoute>) {
        self.root_shell_mode = route
            .map(RootShellMode::from)
            .unwrap_or(RootShellMode::SuCommand);
    }

    pub(crate) fn root_authority_failure_after_mutation(&self) -> bool {
        self.root_authority.has_trustworthy_mutation()
    }

    fn map_public<T>(&mut self, result: Result<T, AdbCommandError>) -> Result<T, String> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                self.last_error = Some(error.clone());
                Err(error_string(error))
            }
        }
    }

    fn guarded<T>(
        &mut self,
        effect: DeviceCommandEffect,
        operation: impl FnOnce(&mut Self) -> Result<T, AdbCommandError>,
    ) -> Result<T, String> {
        let result = self.guarded_typed(effect, operation);
        self.map_public(result)
    }

    fn guarded_typed<T>(
        &mut self,
        effect: DeviceCommandEffect,
        operation: impl FnOnce(&mut Self) -> Result<T, AdbCommandError>,
    ) -> Result<T, AdbCommandError> {
        if let Err(error) = self
            .identity
            .check(&mut self.runner, IdentityCheckPhase::PreOperation)
        {
            self.last_error = Some(error.clone());
            return Err(error);
        }
        let result = operation(self);
        self.root_authority.record_result(effect, &result);
        let should_post_check = result
            .as_ref()
            .err()
            .is_none_or(AdbCommandError::allows_post_identity_probe);
        if should_post_check {
            if let Err(error) = self
                .identity
                .check(&mut self.runner, IdentityCheckPhase::PostOperation)
            {
                self.last_error = Some(error.clone());
                return Err(error);
            }
        }
        if let Err(error) = &result {
            self.last_error = Some(error.clone());
        }
        result
    }

    pub fn check_root(&mut self) -> Result<(), String> {
        self.guarded(DeviceCommandEffect::ReadOnly, |device| {
            device.check_root_unchecked()
        })
    }

    fn check_root_unchecked(&mut self) -> Result<(), AdbCommandError> {
        let Some(serial) = self.runner.serial.clone() else {
            let error = AdbCommandError::RootCheckFailed;
            return Err(error);
        };
        let result = probe_root_typed_with_mode(
            &mut self.runner.executor,
            &self.runner.executable,
            &serial,
            self.root_shell_mode,
        );
        let result = result?;
        match result {
            RootProbeOutcome::Granted => Ok(()),
            RootProbeOutcome::Denied => Err(AdbCommandError::RootDenied),
            RootProbeOutcome::Unavailable => Err(AdbCommandError::RootUnavailable),
            RootProbeOutcome::CheckFailed { .. } => Err(AdbCommandError::RootCheckFailed),
        }
    }

    pub fn install_apk(&mut self, apk_path: &Path, replace_existing: bool) -> Result<(), String> {
        let mut args = vec!["install".to_string()];
        if replace_existing {
            args.push("-r".to_string());
        }
        args.push(apk_path.to_string_lossy().to_string());
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device
                .runner
                .run(args, true, ProcessOperation::Install)
                .map(|_| ())
        })
    }

    pub fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), String> {
        let durable_file = !source.is_dir();
        let mut args = vec!["push".to_string()];
        if sync {
            args.push("--sync".to_string());
        }
        args.push(source.to_string_lossy().to_string());
        args.push(dest.to_string());
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device.runner.run(args, true, ProcessOperation::Push)?;
            if durable_file {
                // ADB sync completion is not a durability boundary. Reopen the
                // exact copied file writable through dd and request conv=fsync
                // without truncating or changing its contents. The standalone
                // fsync applet opens shared-storage paths in a mode that can be
                // rejected even when a writable descriptor can be synchronized.
                device.run_shell_unchecked(
                    vec![
                        "dd".to_string(),
                        "if=/dev/null".to_string(),
                        format!("of={dest}"),
                        "count=0".to_string(),
                        "conv=notrunc,fsync".to_string(),
                    ],
                    true,
                    ProcessOperation::DeviceCopy,
                )?;
            }
            Ok(())
        })
    }

    pub fn mkdir_p(&mut self, path: &str) -> Result<(), String> {
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device
                .run_shell_unchecked(
                    vec!["mkdir".to_string(), "-p".to_string(), path.to_string()],
                    true,
                    ProcessOperation::ShellMutation,
                )
                .map(|_| ())
        })
    }

    pub fn remove_file(&mut self, path: &str) -> Result<(), String> {
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device
                .run_shell_unchecked(
                    vec!["rm".to_string(), "-f".to_string(), path.to_string()],
                    true,
                    ProcessOperation::ShellMutation,
                )
                .map(|_| ())
        })
    }

    pub fn remove_tree(&mut self, path: &str) -> Result<(), String> {
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device
                .run_shell_unchecked(
                    vec!["rm".to_string(), "-rf".to_string(), path.to_string()],
                    true,
                    ProcessOperation::ShellMutation,
                )
                .map(|_| ())
        })
    }

    pub fn copy_on_device(
        &mut self,
        source: &str,
        dest: &str,
        recursive: bool,
        privileged: bool,
    ) -> Result<(), String> {
        let mut args = vec!["cp".to_string()];
        if recursive {
            args.push("-R".to_string());
        }
        args.push(source.to_string());
        args.push(dest.to_string());
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device
                .run_shell_with_privilege_unchecked(
                    args,
                    true,
                    privileged || is_app_private_path(source) || is_app_private_path(dest),
                    ProcessOperation::DeviceCopy,
                )
                .map(|_| ())
        })
    }

    pub fn package_installed(&mut self, package_name: &str) -> Result<bool, String> {
        self.guarded(DeviceCommandEffect::ReadOnly, |device| {
            let result = device.runner.run(
                vec![
                    "shell".to_string(),
                    "pm".to_string(),
                    "path".to_string(),
                    package_name.to_string(),
                ],
                false,
                ProcessOperation::Predicate,
            )?;
            Ok(result.returncode == 0 && result.stdout.contains("package:"))
        })
    }

    pub fn path_exists(&mut self, path: &str) -> Result<bool, String> {
        self.guarded(DeviceCommandEffect::ReadOnly, |device| {
            let result = device.run_shell_unchecked(
                vec!["test".to_string(), "-e".to_string(), path.to_string()],
                false,
                ProcessOperation::Predicate,
            )?;
            Ok(result.returncode == 0)
        })
    }

    pub fn path_is_dir(&mut self, path: &str) -> Result<bool, String> {
        self.guarded(DeviceCommandEffect::ReadOnly, |device| {
            let result = device.run_shell_unchecked(
                vec!["test".to_string(), "-d".to_string(), path.to_string()],
                false,
                ProcessOperation::Predicate,
            )?;
            Ok(result.returncode == 0)
        })
    }

    /// Inspects a selected directory through bounded `find` and `sha256sum`
    /// operations. Symbolic links and unsupported filesystem objects are
    /// rejected rather than followed into another destination boundary.
    pub(crate) fn copy_commit_tree_manifest(
        &mut self,
        root: &str,
    ) -> Result<Vec<CopyManifestEntry>, AdbCommandError> {
        let root_path = Path::new(root);
        if !root_path.is_absolute()
            || root_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir
                        | std::path::Component::ParentDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(AdbCommandError::Resolution(
                "copy commit tree root must be absolute and traversal-free".to_string(),
            ));
        }
        let privileged = super::root_requirements::is_app_private_path(root);
        self.guarded_typed(DeviceCommandEffect::ReadOnly, |device| {
            if !privileged {
                copy_commit_require_shell_uid(device)?;
                copy_commit_verify_sdcard_alias_chain(device, root)?;
            }
            let directory = device.run_shell_with_privilege_unchecked(
                vec!["test".to_string(), "-d".to_string(), root.to_string()],
                false,
                privileged,
                ProcessOperation::Predicate,
            )?;
            if !copy_commit_predicate_result(&directory)? {
                return Err(AdbCommandError::CommandFailed {
                    args: directory.args,
                    returncode: directory.returncode,
                    stderr: "selected copy root is not a directory".to_string(),
                });
            }
            let symlink = device.run_shell_with_privilege_unchecked(
                vec!["test".to_string(), "-L".to_string(), root.to_string()],
                false,
                privileged,
                ProcessOperation::Predicate,
            )?;
            if copy_commit_predicate_result(&symlink)? {
                return Err(AdbCommandError::CommandFailed {
                    args: symlink.args,
                    returncode: symlink.returncode,
                    stderr: "selected copy root is a symbolic link".to_string(),
                });
            }

            let all = copy_commit_find_paths(
                device,
                root,
                &["find", root, "-mindepth", "1", "-print0"],
                privileged,
            )?;
            let directories = copy_commit_find_paths(
                device,
                root,
                &["find", root, "-mindepth", "1", "-type", "d", "-print0"],
                privileged,
            )?;
            let files = copy_commit_find_paths(
                device,
                root,
                &["find", root, "-mindepth", "1", "-type", "f", "-print0"],
                privileged,
            )?;
            let links = copy_commit_find_paths(
                device,
                root,
                &["find", root, "-mindepth", "1", "-type", "l", "-print0"],
                privileged,
            )?;
            if !links.is_empty() {
                return Err(AdbCommandError::CommandFailed {
                    args: vec!["find".to_string(), root.to_string()],
                    returncode: 1,
                    stderr: format!("selected copy tree contains symbolic links: {links:?}"),
                });
            }
            if all.len() != directories.len() + files.len() {
                return Err(AdbCommandError::CommandFailed {
                    args: vec!["find".to_string(), root.to_string()],
                    returncode: 1,
                    stderr: "selected copy tree contains an unsupported filesystem object"
                        .to_string(),
                });
            }

            let directory_paths = directories
                .iter()
                .map(|(full_path, _)| full_path.clone())
                .collect::<std::collections::HashSet<_>>();
            let file_paths = files
                .iter()
                .map(|(full_path, _)| full_path.clone())
                .collect::<std::collections::HashSet<_>>();
            let mut manifest = Vec::with_capacity(all.len());
            for (full_path, relative_path) in all {
                let entry_identity =
                    Some(copy_commit_stat_identity(device, &full_path, privileged)?);
                if directory_paths.contains(&full_path) {
                    manifest.push(CopyManifestEntry {
                        relative_path,
                        kind: CopyEntryKind::Directory,
                        sha256: None,
                        entry_identity,
                    });
                } else if file_paths.contains(&full_path) {
                    let digest = device.run_shell_with_privilege_unchecked(
                        vec!["sha256sum".to_string(), full_path.clone()],
                        true,
                        privileged,
                        ProcessOperation::Predicate,
                    )?;
                    let sha256 = digest
                        .stdout
                        .split_whitespace()
                        .next()
                        .filter(|value| {
                            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                        .ok_or_else(|| AdbCommandError::CommandFailed {
                            args: digest.args.clone(),
                            returncode: digest.returncode,
                            stderr: "sha256sum returned an invalid digest".to_string(),
                        })?
                        .to_ascii_lowercase();
                    manifest.push(CopyManifestEntry {
                        relative_path,
                        kind: CopyEntryKind::File,
                        sha256: Some(sha256),
                        entry_identity,
                    });
                } else {
                    return Err(AdbCommandError::CommandFailed {
                        args: vec!["find".to_string(), root.to_string()],
                        returncode: 1,
                        stderr: "copy tree entry type changed during inspection".to_string(),
                    });
                }
            }
            manifest.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
            Ok(manifest)
        })
    }

    /// Captures the canonical pathname and device/inode identities of a
    /// selected directory and its parent for mutation-time revalidation.
    pub(crate) fn copy_commit_path_binding(
        &mut self,
        path: &str,
    ) -> Result<String, AdbCommandError> {
        let selected = Path::new(path);
        if !selected.is_absolute()
            || selected.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir
                        | std::path::Component::ParentDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(AdbCommandError::Resolution(
                "copy commit destination must be absolute and traversal-free".to_string(),
            ));
        }
        let parent = selected
            .parent()
            .filter(|parent| *parent != Path::new("/"))
            .ok_or_else(|| {
                AdbCommandError::Resolution(
                    "copy commit destination has no bindable parent".to_string(),
                )
            })?;
        let parent = parent.to_string_lossy().into_owned();
        let privileged = super::root_requirements::is_app_private_path(path);
        self.guarded_typed(DeviceCommandEffect::ReadOnly, |device| {
            if !privileged {
                copy_commit_require_shell_uid(device)?;
                copy_commit_verify_sdcard_alias_chain(device, path)?;
            }
            let canonical = device.run_shell_with_privilege_unchecked(
                vec!["readlink".to_string(), "-f".to_string(), path.to_string()],
                true,
                privileged,
                ProcessOperation::Predicate,
            )?;
            let canonical_path = canonical.stdout.trim();
            let expected_canonical = if privileged {
                path.to_string()
            } else {
                copy_commit_expected_canonical_path(path)
            };
            if canonical_path != expected_canonical {
                return Err(AdbCommandError::CommandFailed {
                    args: canonical.args,
                    returncode: 1,
                    stderr: format!(
                        "selected path resolves to {canonical_path:?}, not expected canonical path {expected_canonical:?}"
                    ),
                });
            }
            let parent_identity = copy_commit_stat_identity(device, &parent, privileged)?;
            let selected_identity = copy_commit_stat_identity(device, path, privileged)?;
            Ok(format!(
                "{canonical_path}|parent:{parent_identity}|selected:{selected_identity}"
            ))
        })
    }

    /// Creates a single directory without accepting an existing path.
    pub(crate) fn copy_commit_create_directory(
        &mut self,
        path: &str,
    ) -> Result<(), AdbCommandError> {
        self.guarded_typed(DeviceCommandEffect::Mutating, |device| {
            device
                .run_shell_unchecked(
                    vec!["mkdir".to_string(), path.to_string()],
                    true,
                    ProcessOperation::ShellMutation,
                )
                .map(|_| ())
        })
    }

    /// Removes exactly one empty directory; it never recurses.
    pub(crate) fn copy_commit_remove_empty_directory(
        &mut self,
        path: &str,
    ) -> Result<(), AdbCommandError> {
        self.guarded_typed(DeviceCommandEffect::Mutating, |device| {
            device
                .run_shell_unchecked(
                    vec!["rmdir".to_string(), path.to_string()],
                    true,
                    ProcessOperation::ShellMutation,
                )
                .map(|_| ())
        })
    }

    /// Installs the ABI-matched one-shot helper beside the reviewed staging
    /// tree. The helper is used only for manifest-derived selected-root edits.
    pub(crate) fn copy_commit_prepare_mutation_helper(
        &mut self,
        stage_root: &str,
        helper_path: &str,
    ) -> Result<(), AdbCommandError> {
        #[cfg(not(feature = "android-copy-commit-helper"))]
        {
            let _ = (stage_root, helper_path);
            Err(AdbCommandError::Resolution(
                "selected-root no-follow mutation is unavailable because this build lacks the `android-copy-commit-helper` feature"
                    .to_string(),
            ))
        }

        #[cfg(feature = "android-copy-commit-helper")]
        {
            if helper_path != format!("{stage_root}-helper") {
                return Err(AdbCommandError::Resolution(
                    "selected-root helper path is outside its reviewed staging identity"
                        .to_string(),
                ));
            }
            self.guarded_typed(DeviceCommandEffect::Mutating, |device| {
                let abi = device.run_shell_unchecked(
                    vec!["getprop".to_string(), "ro.product.cpu.abi".to_string()],
                    true,
                    ProcessOperation::Predicate,
                )?;
            match abi.stdout.trim() {
                "arm64-v8a" | "x86_64" => {}
                other => {
                    return Err(AdbCommandError::Resolution(format!(
                        "selected-root helper does not include an Android binary for device ABI {other:?} (supported: arm64-v8a, x86_64)"
                    )));
                }
            }
            let staged_helper = stage_android_copy_commit_helper(abi.stdout.trim())?;
            let local_helper = staged_helper.path().to_string_lossy().into_owned();
            device.runner.run(
                    vec![
                        "push".to_string(),
                    local_helper,
                        helper_path.to_string(),
                    ],
                    true,
                    ProcessOperation::Push,
                )?;
                device.run_shell_unchecked(
                    vec![
                        "chmod".to_string(),
                        "700".to_string(),
                        helper_path.to_string(),
                    ],
                    true,
                    ProcessOperation::ShellMutation,
                )?;
                Ok(())
            })
        }
    }

    /// Applies one reviewed manifest entry from a descriptor-anchored
    /// selected-root directory using the one-shot helper.
    pub(crate) fn copy_commit_apply_selected_root_mutation(
        &mut self,
        selected_root: &str,
        destination_binding: &str,
        stage_root: &str,
        helper_path: &str,
        expected_entry: &str,
        mutation: &CopyCommitMutation,
    ) -> Result<(), AdbCommandError> {
        #[cfg(not(feature = "android-copy-commit-helper"))]
        {
            let _ = (
                selected_root,
                destination_binding,
                stage_root,
                helper_path,
                expected_entry,
                mutation,
            );
            Err(AdbCommandError::Resolution(
                "selected-root no-follow mutation is unavailable because this build lacks the `android-copy-commit-helper` feature"
                    .to_string(),
            ))
        }

        #[cfg(feature = "android-copy-commit-helper")]
        {
            let privileged = super::root_requirements::is_app_private_path(selected_root);
            if !privileged && self.copy_commit_path_binding(selected_root)? != destination_binding {
                return Err(AdbCommandError::Resolution(
                    "shared-storage destination identity changed immediately before mutation"
                        .to_string(),
                ));
            }
            let (action, relative_path, expected_source_sha256) = match mutation {
                CopyCommitMutation::EnsureDirectory { relative_path } => {
                    ("ensure-dir", relative_path, None)
                }
                CopyCommitMutation::RemoveFile { relative_path } => {
                    ("remove-file", relative_path, None)
                }
                CopyCommitMutation::RemoveDirectory { relative_path } => {
                    ("remove-dir", relative_path, None)
                }
                CopyCommitMutation::PublishFile {
                    relative_path,
                    expected_source_sha256,
                } => (
                    "publish",
                    relative_path,
                    Some(expected_source_sha256.as_str()),
                ),
            };
            let mut helper_arguments = vec![
                helper_path.to_string(),
                selected_root.to_string(),
                destination_binding.to_string(),
                stage_root.to_string(),
                action.to_string(),
                relative_path.to_string(),
                expected_entry.to_string(),
            ];
            if let Some(expected_source_sha256) = expected_source_sha256 {
                helper_arguments.push(expected_source_sha256.to_string());
            }
            self.guarded_typed(DeviceCommandEffect::Mutating, |device| {
                device
                    .run_shell_with_privilege_unchecked(
                        helper_arguments,
                        true,
                        privileged,
                        ProcessOperation::ShellMutation,
                    )
                    .map(|_| ())
            })
        }
    }

    /// Removes only the temporary helper executable after the selected-root
    /// operation completes.
    pub(crate) fn copy_commit_cleanup_mutation_helper(
        &mut self,
        helper_path: &str,
    ) -> Result<(), AdbCommandError> {
        self.guarded_typed(DeviceCommandEffect::Mutating, |device| {
            device
                .run_shell_unchecked(
                    vec!["rm".to_string(), "-f".to_string(), helper_path.to_string()],
                    true,
                    ProcessOperation::ShellMutation,
                )
                .map(|_| ())
        })
    }

    pub(crate) fn apply_permission_operation(
        &mut self,
        operation: PermissionOperation,
    ) -> Result<(), String> {
        let privileged = operation.is_elevated();
        let args = operation.adb_shell_arguments();
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device
                .run_shell_with_privilege_unchecked(
                    args,
                    true,
                    privileged,
                    ProcessOperation::ShellMutation,
                )
                .map(|_| ())
        })
    }

    pub fn launch_app(&mut self, package_name: &str, activity: Option<&str>) -> Result<(), String> {
        let package_name = package_name.to_string();
        let activity = activity.map(ToString::to_string);
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device.launch_app_unchecked(&package_name, activity.as_deref())
        })
    }

    fn launch_app_unchecked(
        &mut self,
        package_name: &str,
        activity: Option<&str>,
    ) -> Result<(), AdbCommandError> {
        if let Some(activity) = activity.filter(|value| !value.is_empty()) {
            let result = self.runner.run(
                vec![
                    "shell".to_string(),
                    "am".to_string(),
                    "start".to_string(),
                    "-n".to_string(),
                    format!("{package_name}/{activity}"),
                ],
                true,
                ProcessOperation::Launch,
            );
            return result.map(|_| ());
        }

        if let Some(resolved_activity) = self.resolve_launcher_activity_unchecked(package_name)? {
            let result = self.runner.run(
                vec![
                    "shell".to_string(),
                    "am".to_string(),
                    "start".to_string(),
                    "-n".to_string(),
                    resolved_activity,
                ],
                true,
                ProcessOperation::Launch,
            );
            return result.map(|_| ());
        }

        let result = self.runner.run(
            vec![
                "shell".to_string(),
                "monkey".to_string(),
                "-p".to_string(),
                package_name.to_string(),
                "-c".to_string(),
                "android.intent.category.LAUNCHER".to_string(),
                "1".to_string(),
            ],
            true,
            ProcessOperation::Launch,
        );
        result.map(|_| ())
    }

    pub fn force_stop_app(&mut self, package_name: &str) -> Result<(), String> {
        let package_name = package_name.to_string();
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device
                .runner
                .run(
                    vec![
                        "shell".to_string(),
                        "am".to_string(),
                        "force-stop".to_string(),
                        package_name,
                    ],
                    true,
                    ProcessOperation::ForceStop,
                )
                .map(|_| ())
        })
    }

    fn resolve_launcher_activity_unchecked(
        &mut self,
        package_name: &str,
    ) -> Result<Option<String>, AdbCommandError> {
        for command in [
            vec![
                "shell".to_string(),
                "cmd".to_string(),
                "package".to_string(),
                "resolve-activity".to_string(),
                "--brief".to_string(),
                package_name.to_string(),
            ],
            vec![
                "shell".to_string(),
                "pm".to_string(),
                "resolve-activity".to_string(),
                "--brief".to_string(),
                package_name.to_string(),
            ],
        ] {
            let result = self.runner.run(command, false, ProcessOperation::Launch);
            let result = result?;
            if result.returncode != 0 {
                continue;
            }
            if let Some(component) = parse_resolved_launcher_component(&result.stdout) {
                return Ok(Some(component));
            }
        }
        Ok(None)
    }

    fn run_shell_unchecked(
        &mut self,
        args: Vec<String>,
        check: bool,
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        let privileged = args
            .last()
            .map(|path| is_app_private_path(path))
            .unwrap_or(false);
        self.run_shell_with_privilege_unchecked(args, check, privileged, operation)
    }

    fn run_shell_with_privilege_unchecked(
        &mut self,
        args: Vec<String>,
        check: bool,
        privileged: bool,
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        if privileged {
            self.revalidate_root_authority_unchecked()?;
        }
        self.runner.run(
            vec![
                "shell".to_string(),
                build_shell_command_with_mode(&args, privileged, self.root_shell_mode),
            ],
            check,
            operation,
        )
    }

    fn revalidate_root_authority_unchecked(&mut self) -> Result<(), AdbCommandError> {
        if !self.root_authority.is_authorized() {
            return Err(AdbCommandError::RootAuthorityUnverified);
        }
        let Some(serial) = self.runner.serial.clone() else {
            return Err(AdbCommandError::RootAuthorityUnverified);
        };
        let outcome = probe_root_typed_with_mode(
            &mut self.runner.executor,
            &self.runner.executable,
            &serial,
            self.root_shell_mode,
        )?;
        match root_authority::classify_probe(outcome) {
            Ok(()) => Ok(()),
            Err(error @ AdbCommandError::RootAuthorityRevoked)
            | Err(error @ AdbCommandError::RootAuthorityUnverified) => {
                self.identity
                    .check(&mut self.runner, IdentityCheckPhase::PreOperation)?;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct FakeAdbCommandExecutor {
    calls: Vec<Vec<String>>,
    operations: Vec<ProcessOperation>,
    responses: VecDeque<FakeAdbResponse>,
}

#[cfg(test)]
#[derive(Debug)]
enum FakeAdbResponse {
    Completed {
        returncode: i32,
        stdout: String,
        stderr: String,
    },
    MissingBinary,
    TimedOut,
    ProcessFailed(ProcessFailureKind),
}

fn copy_commit_require_shell_uid<E: AdbCommandExecutor>(
    device: &mut RealAdbDevice<E>,
) -> Result<(), AdbCommandError> {
    let result = device.run_shell_with_privilege_unchecked(
        vec!["id".to_string(), "-u".to_string()],
        true,
        false,
        ProcessOperation::Predicate,
    )?;
    let uid = result.stdout.trim();
    if uid != "2000" {
        return Err(AdbCommandError::CommandFailed {
            args: result.args,
            returncode: result.returncode,
            stderr: format!("shared-storage CopyCommit requires shell UID 2000, got {uid:?}"),
        });
    }
    Ok(())
}

fn copy_commit_verify_sdcard_alias_chain<E: AdbCommandExecutor>(
    device: &mut RealAdbDevice<E>,
    path: &str,
) -> Result<(), AdbCommandError> {
    let known_prefixes = ["/sdcard", "/storage/self/primary", "/storage/emulated/0"];
    let uses_known_alias = known_prefixes.iter().any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('/'))
    });
    if !uses_known_alias {
        return Ok(());
    }

    for (alias, expected) in [
        ("/sdcard", "/storage/self/primary"),
        ("/storage/self/primary", "/storage/emulated/0"),
    ] {
        let result = device.run_shell_with_privilege_unchecked(
            vec!["readlink".to_string(), alias.to_string()],
            true,
            false,
            ProcessOperation::Predicate,
        )?;
        if result.stdout.trim() != expected {
            return Err(AdbCommandError::CommandFailed {
                args: result.args,
                returncode: 1,
                stderr: format!(
                    "unsupported shared-storage alias: {alias:?} must resolve to {expected:?}"
                ),
            });
        }
    }
    Ok(())
}

fn copy_commit_expected_canonical_path(path: &str) -> String {
    for (alias, canonical) in [
        ("/sdcard", "/storage/emulated/0"),
        ("/storage/self/primary", "/storage/emulated/0"),
    ] {
        if path == alias {
            return canonical.to_string();
        }
        if let Some(suffix) = path
            .strip_prefix(alias)
            .filter(|suffix| suffix.starts_with('/'))
        {
            return format!("{canonical}{suffix}");
        }
    }
    path.to_string()
}

fn copy_commit_find_paths<E: AdbCommandExecutor>(
    device: &mut RealAdbDevice<E>,
    root: &str,
    args: &[&str],
    privileged: bool,
) -> Result<Vec<(String, String)>, AdbCommandError> {
    let result = device.run_shell_with_privilege_unchecked(
        args.iter()
            .map(|argument| (*argument).to_string())
            .collect(),
        true,
        privileged,
        ProcessOperation::Predicate,
    )?;
    let bytes = result.stdout.as_bytes();
    if !bytes.is_empty() && !bytes.ends_with(&[0]) {
        return Err(AdbCommandError::CommandFailed {
            args: result.args,
            returncode: result.returncode,
            stderr: "find returned a non-NUL-terminated manifest".to_string(),
        });
    }
    let prefix = format!("{}/", root.trim_end_matches('/'));
    let mut paths = Vec::new();
    for raw_path in bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let full_path = std::str::from_utf8(raw_path)
            .map_err(|_| AdbCommandError::CommandFailed {
                args: result.args.clone(),
                returncode: result.returncode,
                stderr: "find returned a non-UTF-8 path".to_string(),
            })?
            .to_string();
        let relative_path =
            full_path
                .strip_prefix(&prefix)
                .ok_or_else(|| AdbCommandError::CommandFailed {
                    args: result.args.clone(),
                    returncode: result.returncode,
                    stderr: format!(
                        "find returned a path outside the selected root: {full_path:?}"
                    ),
                })?;
        let relative = Path::new(relative_path);
        if relative_path.is_empty()
            || relative_path.contains(['\n', '\r'])
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(AdbCommandError::CommandFailed {
                args: result.args.clone(),
                returncode: result.returncode,
                stderr: format!("find returned an unsafe selected-tree path: {relative_path:?}"),
            });
        }
        let relative_path = relative_path.to_string();
        paths.push((full_path, relative_path));
    }
    Ok(paths)
}

fn copy_commit_stat_identity<E: AdbCommandExecutor>(
    device: &mut RealAdbDevice<E>,
    path: &str,
    privileged: bool,
) -> Result<String, AdbCommandError> {
    let result = device.run_shell_with_privilege_unchecked(
        vec![
            "stat".to_string(),
            "-c".to_string(),
            "%d:%i".to_string(),
            path.to_string(),
        ],
        true,
        privileged,
        ProcessOperation::Predicate,
    )?;
    let identity = result.stdout.trim();
    if identity.is_empty() || identity.contains(['\n', '\r']) {
        return Err(AdbCommandError::CommandFailed {
            args: result.args,
            returncode: result.returncode,
            stderr: "stat returned an invalid device/inode identity".to_string(),
        });
    }
    Ok(identity.to_string())
}

#[cfg(test)]
impl FakeAdbCommandExecutor {
    pub fn calls(&self) -> &[Vec<String>] {
        &self.calls
    }

    pub fn operations(&self) -> &[ProcessOperation] {
        &self.operations
    }

    pub fn push_completed(&mut self, returncode: i32, stdout: &str, stderr: &str) {
        self.responses.push_back(FakeAdbResponse::Completed {
            returncode,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        });
    }

    pub fn push_missing_binary(&mut self) {
        self.responses.push_back(FakeAdbResponse::MissingBinary);
    }

    pub fn push_timed_out(&mut self) {
        self.responses.push_back(FakeAdbResponse::TimedOut);
    }

    pub fn push_process_failed(&mut self, kind: ProcessFailureKind) {
        self.responses
            .push_back(FakeAdbResponse::ProcessFailed(kind));
    }
}

#[cfg(test)]
impl AdbCommandExecutor for FakeAdbCommandExecutor {
    fn run_for(
        &mut self,
        args: &[String],
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        self.calls.push(args.to_vec());
        self.operations.push(operation);
        match self.responses.pop_front() {
            Some(FakeAdbResponse::Completed {
                returncode,
                stdout,
                stderr,
            }) => Ok(AdbCommandResult {
                args: args.to_vec(),
                returncode,
                stdout,
                stderr,
            }),
            Some(FakeAdbResponse::MissingBinary) => {
                Err(AdbCommandError::Resolution(adb_not_found_message()))
            }
            Some(FakeAdbResponse::TimedOut) => Err(AdbCommandError::TimedOut {
                cleanup: ProcessCleanup::Confirmed,
            }),
            Some(FakeAdbResponse::ProcessFailed(kind)) => Err(AdbCommandError::ProcessFailed {
                kind,
                cleanup: ProcessCleanup::Confirmed,
            }),
            None => Ok(AdbCommandResult {
                args: args.to_vec(),
                returncode: 0,
                stdout: String::new(),
                stderr: String::new(),
            }),
        }
    }
}

fn map_process_error(_args: &[String], error: OwnedProcessError) -> AdbCommandError {
    match error.kind {
        ProcessFailureKind::Spawn => AdbCommandError::Resolution(adb_not_found_message()),
        ProcessFailureKind::TimedOut => AdbCommandError::TimedOut {
            cleanup: error.cleanup,
        },
        kind => AdbCommandError::ProcessFailed {
            kind,
            cleanup: error.cleanup,
        },
    }
}

/// Classifies only complete, bounded ADB-owned response lines. Remote shell
/// output is deliberately ignored unless it has an ADB-owned error prefix.
fn classify_completed_transport(
    result: AdbCommandResult,
) -> Result<AdbCommandResult, AdbCommandError> {
    if result.returncode == 0 {
        return Ok(result);
    }

    for line in result
        .stdout
        .lines()
        .chain(result.stderr.lines())
        .map(str::trim)
        .map(|line| line.to_ascii_lowercase())
    {
        if let Some(error) = classify_transport_line(&line) {
            return Err(error);
        }
    }
    Ok(result)
}

/// Recognizes only bounded, operation-relevant Android/ADB no-space responses.
/// Bare phrases and arbitrary remote output are deliberately ignored so a
/// filename, echoed argument, or unrelated diagnostic cannot create a storage
/// failure classification.
fn classify_completed_storage(
    operation: ProcessOperation,
    full_args: &[String],
    result: &AdbCommandResult,
) -> bool {
    let known_mutating_operation = matches!(
        operation,
        ProcessOperation::Install
            | ProcessOperation::Push
            | ProcessOperation::DeviceCopy
            | ProcessOperation::ShellMutation
    );
    let generic_reviewed_mutation = operation == ProcessOperation::GenericFallback
        && reviewed_plan_operation(full_args.get(1..).unwrap_or_default())
            == ProcessOperation::ShellMutation;
    if !known_mutating_operation && !generic_reviewed_mutation {
        return false;
    }

    result
        .stdout
        .lines()
        .chain(result.stderr.lines())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .any(|line| {
            if line == "failure [install_failed_insufficient_storage]"
                || (line.starts_with("failure [install_failed_insufficient_storage:")
                    && line.ends_with(']'))
            {
                return true;
            }
            if operation == ProcessOperation::DeviceCopy
                && line.ends_with(": fsync: no space left on device")
            {
                return true;
            }
            [
                "adb: error: failed to copy ",
                "error: failed to copy ",
                "failed to copy ",
                "mkdir: ",
                "cp: ",
                "mv: ",
                "rm: ",
                "touch: ",
                "unzip: ",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix) && line.ends_with(": no space left on device"))
        })
}

fn classify_transport_line(line: &str) -> Option<AdbCommandError> {
    if let Some(serial) = line
        .strip_prefix("adb: device '")
        .and_then(|value| value.strip_suffix("' not found"))
    {
        if !serial.is_empty() && !serial.contains('\'') {
            return Some(AdbCommandError::DeviceDisconnected);
        }
    }

    let adb_error = line
        .strip_prefix("adb: error:")
        .or_else(|| line.strip_prefix("error:"))
        .map(str::trim);

    if let Some(body) = adb_error {
        if body == "device offline" {
            return Some(AdbCommandError::DeviceOffline);
        }
        if body.starts_with("device unauthorized") {
            return Some(AdbCommandError::DeviceUnauthorized);
        }
        if let Some(serial) = body.strip_prefix("device '") {
            if let Some(serial) = serial.strip_suffix("' not found") {
                if !serial.is_empty() && !serial.contains('\'') {
                    return Some(AdbCommandError::DeviceDisconnected);
                }
            }
        }
        if body == "no devices/emulators found" {
            return Some(AdbCommandError::DeviceDisconnected);
        }
        if body.starts_with("cannot connect to daemon")
            || body.starts_with("failed to connect to daemon")
        {
            return Some(AdbCommandError::AdbServerUnavailable);
        }
        if body == "connection reset by peer" {
            return Some(AdbCommandError::TransportReset);
        }
        if body == "transport error" {
            return Some(AdbCommandError::TransportFailure);
        }
        if body == "closed"
            || body.starts_with("failed to read response from device")
            || body == "failed to read copy response"
            || body.starts_with("failed to read copy response:")
        {
            return Some(AdbCommandError::TransportFailure);
        }
    }

    if line.starts_with("* failed to start daemon") || line.starts_with("* cannot start daemon") {
        return Some(AdbCommandError::AdbServerUnavailable);
    }
    if line.starts_with("adb server didn't ack") {
        return Some(AdbCommandError::AdbServerUnavailable);
    }

    None
}

fn error_string(error: AdbCommandError) -> String {
    match error {
        AdbCommandError::CommandFailed { .. } => ADB_COMMAND_FAILURE_MESSAGE.to_string(),
        error => error.to_string(),
    }
}

fn adb_not_found_message() -> String {
    "The configured ADB executable could not be started. Ensure adb is available on PATH or pass an explicit executable when constructing RealAdbDevice.".to_string()
}

fn build_shell_command_with_mode(
    args: &[String],
    privileged: bool,
    root_shell_mode: RootShellMode,
) -> String {
    let command = shell_join(args);
    if !privileged {
        return command;
    }
    match root_shell_mode {
        RootShellMode::SuCommand => shell_join(&["su".to_string(), "-c".to_string(), command]),
        RootShellMode::SuUserId => shell_join(&[
            "su".to_string(),
            "0".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            command,
        ]),
    }
}

fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character))
    {
        return arg.to_string();
    }

    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

fn parse_resolved_launcher_component(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.contains('/') && !line.contains(' '))
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "android-copy-commit-helper")]
    #[test]
    fn packaged_android_helper_materializes_outside_the_cargo_build_tree() {
        let helper = stage_android_copy_commit_helper("arm64-v8a")
            .expect("packaged Android helper should materialize for adb push");
        let bytes = std::fs::read(helper.path()).expect("materialized helper should be readable");
        let build_output = std::path::Path::new(env!("EMU_CHEF_COPY_COMMIT_HELPER_AARCH64"));

        assert_eq!(
            &bytes[..4],
            b"\x7fELF",
            "helper payload should be an Android ELF"
        );
        assert_ne!(
            helper.path(),
            build_output,
            "ADB must push the packaged payload staged at runtime, not Cargo OUT_DIR"
        );
        assert!(
            !helper.path().starts_with(build_output.parent().unwrap()),
            "runtime helper payload must not depend on the Cargo build tree"
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_executor_times_out_and_reaps_a_sleeping_fake_adb_binary() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;

        let directory = tempfile::tempdir().expect("fake ADB directory should be created");
        let executable = directory.path().join("sleeping-adb");
        std::fs::write(&executable, "#!/bin/sh\nexec /bin/sleep 5\n")
            .expect("fake ADB script should be written");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fake ADB script metadata should be readable")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions)
            .expect("fake ADB script should be executable");

        let mut executor = ProcessAdbCommandExecutor::default();
        let _deadline = crate::owned_process::arm_test_process_deadline(
            ProcessOperation::Probe,
            Duration::from_millis(100),
        );
        let error = executor
            .run_for(
                &[executable.to_string_lossy().into_owned()],
                ProcessOperation::Probe,
            )
            .expect_err("sleeping fake ADB should be terminated at its deadline");

        assert!(matches!(
            error,
            AdbCommandError::TimedOut {
                cleanup: ProcessCleanup::Confirmed
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn mutating_adb_timeout_is_reported_after_child_reaping() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;

        const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
        const COMMAND_TIMEOUT: Duration = Duration::from_millis(300);
        const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(5);

        let directory = tempfile::tempdir().expect("fake ADB directory should be created");
        let executable = directory.path().join("sleeping-mutating-adb");
        let started_pid_file = directory.path().join("mutation-started-pid");
        let quoted_started_pid_file = shell_quote(started_pid_file.to_string_lossy().as_ref());
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$$\" > {quoted_started_pid_file}\nexec /bin/sleep 5\n"
            ),
        )
        .expect("fake ADB script should be written");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fake ADB script metadata should be readable")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions)
            .expect("fake ADB script should be executable");

        let args = vec![
            executable.to_string_lossy().into_owned(),
            "-s".to_string(),
            "reviewed-emulator".to_string(),
            "install".to_string(),
            "-r".to_string(),
            "fixture.apk".to_string(),
        ];
        let startup_wait_started = std::time::Instant::now();
        let runner = std::thread::spawn(move || {
            // The owned-process runner starts its relative deadline immediately after spawn.
            // Reserve a bounded startup window so the child gets the full command timeout
            // after the parent confirms that the fake mutating command has started.
            let _deadline = crate::owned_process::arm_test_process_deadline(
                ProcessOperation::Install,
                STARTUP_TIMEOUT + COMMAND_TIMEOUT,
            );
            let mut executor = ProcessAdbCommandExecutor::default();
            executor.run_for(&args, ProcessOperation::Install)
        });

        let startup_confirmed = loop {
            if started_pid_file.exists() {
                break startup_wait_started.elapsed() <= STARTUP_TIMEOUT;
            }
            if startup_wait_started.elapsed() >= STARTUP_TIMEOUT {
                break false;
            }
            std::thread::sleep(STARTUP_POLL_INTERVAL);
        };
        let command_started = std::time::Instant::now();
        let result = runner
            .join()
            .expect("fake ADB runner thread should finish after process cleanup");
        let command_elapsed = command_started.elapsed();
        let error = result.expect_err("mutating fake ADB should stop at its deadline");

        assert!(
            startup_confirmed,
            "fake install must report child startup within the bounded handshake"
        );
        assert!(
            command_elapsed >= COMMAND_TIMEOUT - Duration::from_millis(50),
            "the fake child must receive the command timeout after confirmed startup"
        );
        assert!(
            command_elapsed <= STARTUP_TIMEOUT + COMMAND_TIMEOUT,
            "fake install timeout must remain within the bounded startup and command budgets"
        );

        assert!(
            matches!(
                error,
                AdbCommandError::TimedOut {
                    cleanup: ProcessCleanup::Confirmed
                }
            ),
            "fake install must time out only after confirmed cleanup"
        );

        let child_pid = std::fs::read_to_string(&started_pid_file)
            .expect("fake install must record its PID before blocking")
            .trim()
            .to_string();
        assert!(!child_pid.is_empty(), "fake install PID must be recorded");
        let child_is_alive = std::process::Command::new("/bin/kill")
            .args(["-0", child_pid.as_str()])
            .status()
            .expect("the Unix process liveness check should run")
            .success();
        assert!(!child_is_alive, "timed out fake install must be reaped");
    }

    #[test]
    fn process_executor_times_out_and_reaps_a_fake_preflight_child() {
        use std::time::Duration;

        let observer = OwnedProcessObservationHandle::default();
        let mut executor = ProcessAdbCommandExecutor::with_process_observer(observer.clone());
        let executable = std::env::current_exe()
            .expect("test executable should be available")
            .to_string_lossy()
            .into_owned();
        let _deadline = crate::owned_process::arm_test_process_deadline(
            ProcessOperation::Probe,
            Duration::from_millis(100),
        );
        let error = executor
            .run_for(
                &[
                    executable,
                    "--exact".to_string(),
                    "owned_process::tests::timeout_helper".to_string(),
                    "--ignored".to_string(),
                    "--nocapture".to_string(),
                ],
                ProcessOperation::Probe,
            )
            .expect_err("the fake preflight child should exceed its deadline");

        assert!(matches!(
            error,
            AdbCommandError::TimedOut {
                cleanup: ProcessCleanup::Confirmed
            }
        ));
        assert!(observer.events().iter().any(|event| {
            event.operation() == ProcessOperation::Probe
                && matches!(
                    event,
                    crate::owned_process::OwnedProcessLifecycleEvent::Terminal { .. }
                )
        }));
    }

    #[test]
    fn process_executor_forwards_device_copy_observation_without_changing_result() {
        let observer = crate::owned_process::OwnedProcessObservationHandle::default();
        let mut executor = ProcessAdbCommandExecutor::with_process_observer(observer.clone());
        let executable = std::env::current_exe()
            .expect("test executable should be available")
            .to_string_lossy()
            .into_owned();
        let result = executor
            .run_for(
                &[
                    executable,
                    "--exact".to_string(),
                    "owned_process::tests::normal_helper".to_string(),
                    "--ignored".to_string(),
                    "--nocapture".to_string(),
                ],
                ProcessOperation::DeviceCopy,
            )
            .expect("the observed process should complete");

        assert_eq!(result.returncode, 0);
        assert!(observer.events().iter().any(|event| {
            event.operation() == ProcessOperation::DeviceCopy
                && matches!(
                    event,
                    crate::owned_process::OwnedProcessLifecycleEvent::Terminal { .. }
                )
        }));
    }

    #[test]
    fn install_signing_rejection_is_distinct_and_does_not_echo_adb_diagnostics() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(
            1,
            "",
            "Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: package=com.private.app cert=private-fingerprint]",
        );
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("emulator-5554".into()), executor);

        let error = runner
            .run(
                vec!["install".into(), "-r".into(), "/tmp/private.apk".into()],
                true,
                ProcessOperation::Install,
            )
            .expect_err("Android signer mismatch should reject the replacement");

        assert_eq!(
            error,
            AdbCommandError::Resolution(
                "android_signing_failed: Android rejected the APK signing identity.".into()
            )
        );
        let message = error.to_string();
        for forbidden in [
            "emulator-5554",
            "private.app",
            "private-fingerprint",
            "/tmp/private.apk",
        ] {
            assert!(!message.contains(forbidden));
        }
    }

    #[test]
    fn other_install_rejections_use_a_distinct_sanitized_install_error() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(
            1,
            "",
            "Failure [INSTALL_FAILED_INVALID_APK: /tmp/private.apk]",
        );
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);

        let error = runner
            .run(
                vec!["install".into(), "fixture.apk".into()],
                true,
                ProcessOperation::Install,
            )
            .expect_err("invalid APK should fail installation");

        assert_eq!(
            error,
            AdbCommandError::Resolution(
                "android_install_failed: Android rejected the APK installation.".into()
            )
        );
        assert!(!error.to_string().contains("/tmp/private.apk"));
    }

    #[test]
    fn install_version_downgrade_is_classified_without_echoing_diagnostics() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(
            1,
            "",
            "Failure [INSTALL_FAILED_VERSION_DOWNGRADE: package=com.private.app]",
        );
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);

        let error = runner
            .run(
                vec!["install".into(), "fixture.apk".into()],
                true,
                ProcessOperation::Install,
            )
            .expect_err("version downgrade should be rejected");

        assert_eq!(
            error,
            AdbCommandError::Resolution(
                "android_version_failed: Android rejected a version downgrade.".into()
            )
        );
        assert!(!error.to_string().contains("private.app"));
    }

    fn probe_with(
        response: impl FnOnce(&mut FakeAdbCommandExecutor),
    ) -> (RootProbeOutcome, Vec<Vec<String>>) {
        let mut executor = FakeAdbCommandExecutor::default();
        response(&mut executor);
        let outcome = probe_root(
            &mut executor,
            "/managed/platform-tools/adb",
            "opaque-serial",
        );
        (outcome, executor.calls().to_vec())
    }

    #[test]
    fn root_shell_modes_build_exact_helper_argv_for_both_supported_su_forms() {
        let helper_argv = vec![
            "/data/local/tmp/emuchef/t03-copy-helper".to_string(),
            "--manifest".to_string(),
            "/data/local/tmp/emuchef/plan.json".to_string(),
        ];

        let su_command =
            build_shell_command_with_mode(&helper_argv, true, RootShellMode::SuCommand);
        assert_eq!(
            su_command,
            "su -c '/data/local/tmp/emuchef/t03-copy-helper --manifest /data/local/tmp/emuchef/plan.json'"
        );

        let su_zero = build_shell_command_with_mode(&helper_argv, true, RootShellMode::SuUserId);
        assert_eq!(
            su_zero,
            "su 0 sh -c '/data/local/tmp/emuchef/t03-copy-helper --manifest /data/local/tmp/emuchef/plan.json'"
        );
    }

    #[test]
    fn root_probe_preserves_su_c_when_uid_and_selinux_context_are_verified() {
        let (outcome, calls) = probe_with(|executor| {
            executor.push_completed(0, "uid=0(root) gid=0(root) context=u:r:su:s0\n", "");
        });

        assert_eq!(outcome, RootProbeOutcome::Granted);
        assert_eq!(
            calls,
            vec![vec![
                "/managed/platform-tools/adb",
                "-s",
                "opaque-serial",
                "shell",
                "su",
                "-c",
                "id",
            ]]
        );
    }

    #[test]
    fn root_probe_uses_su_zero_after_su_c_reports_unsupported_syntax() {
        let (outcome, calls) = probe_with(|executor| {
            executor.push_completed(1, "", "su: invalid uid/gid '-c'\n");
            executor.push_completed(0, "uid=0(root) gid=0(root) context=u:r:su:s0\n", "");
        });

        assert_eq!(outcome, RootProbeOutcome::Granted);
        assert_eq!(
            calls,
            vec![
                vec![
                    "/managed/platform-tools/adb",
                    "-s",
                    "opaque-serial",
                    "shell",
                    "su",
                    "-c",
                    "id",
                ],
                vec![
                    "/managed/platform-tools/adb",
                    "-s",
                    "opaque-serial",
                    "shell",
                    "su",
                    "0",
                    "id",
                ],
            ]
        );
    }

    #[test]
    fn root_probe_does_not_fallback_for_unrecognized_su_c_error() {
        let (outcome, calls) = probe_with(|executor| {
            executor.push_completed(1, "", "su: invalid uid '-c'\n");
        });

        assert!(matches!(outcome, RootProbeOutcome::CheckFailed { .. }));
        assert_eq!(
            calls.len(),
            1,
            "unrecognized su -c errors must not trigger su 0"
        );
    }

    #[test]
    fn root_probe_fails_closed_when_uid_zero_has_no_verified_selinux_context() {
        let (outcome, calls) = probe_with(|executor| {
            executor.push_completed(0, "uid=0(root) gid=0(root)\n", "");
        });

        assert!(!matches!(outcome, RootProbeOutcome::Granted));
        assert_eq!(
            calls.len(),
            1,
            "an incomplete identity must not trigger fallback"
        );
    }

    #[test]
    fn copy_commit_predicate_maps_status_one_to_false_and_rejects_probe_errors() {
        let result = |returncode, stdout: &str, stderr: &str| AdbCommandResult {
            args: vec![
                "adb".to_string(),
                "shell".to_string(),
                "test -L path".to_string(),
            ],
            returncode,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        };

        assert_eq!(
            copy_commit_predicate_result(&result(0, "", "")),
            Ok(true),
            "a successful predicate status is true"
        );
        assert_eq!(
            copy_commit_predicate_result(&result(1, "", "")),
            Ok(false),
            "test -L returns 1 for a normal non-symlink result"
        );
        assert!(matches!(
            copy_commit_predicate_result(&result(1, "", "Permission denied")),
            Err(AdbCommandError::CommandFailed { returncode: 1, .. })
        ));
        assert!(matches!(
            copy_commit_predicate_result(&result(2, "", "")),
            Err(AdbCommandError::CommandFailed { returncode: 2, .. })
        ));
    }

    #[test]
    fn shared_copy_commit_binding_checks_shell_uid_and_reviewed_sdcard_alias_chain() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "2000\n", "");
        executor.push_completed(0, "/storage/self/primary\n", "");
        executor.push_completed(0, "/storage/emulated/0\n", "");
        executor.push_completed(0, "/storage/emulated/0/ROMs\n", "");
        executor.push_completed(0, "253:102\n", "");
        executor.push_completed(0, "253:103\n", "");
        let mut device = RealAdbDevice::with_executor("adb", Some("emulator-5554"), executor);

        let binding = device
            .copy_commit_path_binding("/sdcard/ROMs")
            .expect("reviewed shared ROM path binding should be captured");

        assert_eq!(
            binding,
            "/storage/emulated/0/ROMs|parent:253:102|selected:253:103"
        );
        let calls = device.command_executor().calls();
        assert_eq!(
            calls[0],
            vec!["adb", "-s", "emulator-5554", "shell", "id -u"]
        );
        assert!(calls.iter().any(|call| {
            call.last()
                .is_some_and(|command| command.contains("readlink /sdcard"))
        }));
        assert!(calls.iter().any(|call| {
            call.last()
                .is_some_and(|command| command.contains("readlink /storage/self/primary"))
        }));
        assert!(calls
            .iter()
            .all(|call| !call.iter().any(|argument| argument == "su")));
    }

    #[test]
    fn shared_copy_commit_binding_rejects_non_shell_uid_without_root_fallback() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "0\n", "");
        let mut device = RealAdbDevice::with_executor("adb", Some("emulator-5554"), executor);

        assert!(device.copy_commit_path_binding("/sdcard/ROMs").is_err());

        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].last().map(String::as_str), Some("id -u"));
        assert!(calls
            .iter()
            .all(|call| !call.iter().any(|argument| argument == "su")));
    }

    #[test]
    fn shared_copy_commit_binding_rejects_unreviewed_sdcard_alias_chain() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "2000\n", "");
        executor.push_completed(0, "/storage/emulated/0\n", "");
        let mut device = RealAdbDevice::with_executor("adb", Some("emulator-5554"), executor);

        assert!(device.copy_commit_path_binding("/sdcard/ROMs").is_err());

        let calls = device.command_executor().calls();
        assert_eq!(calls.len(), 2);
        assert!(calls
            .iter()
            .all(|call| !call.iter().any(|argument| argument == "su")));
    }

    #[test]
    fn root_probe_preserves_completed_shell_classifications() {
        let (denied, _) =
            probe_with(|executor| executor.push_completed(1, "", "Permission denied"));
        assert_eq!(denied, RootProbeOutcome::Denied);
        let (unavailable, _) =
            probe_with(|executor| executor.push_completed(1, "", "su: not found"));
        assert_eq!(unavailable, RootProbeOutcome::Unavailable);
        let (unexpected, _) =
            probe_with(|executor| executor.push_completed(1, "shell failed", "bad response"));
        assert!(matches!(
            unexpected,
            RootProbeOutcome::CheckFailed {
                reason: RootProbeFailureReason::UnexpectedResponse,
                ..
            }
        ));
    }

    #[test]
    fn root_probe_classifies_completed_denial_and_unavailable_evidence_before_status() {
        let (denied_stdout, _) =
            probe_with(|executor| executor.push_completed(0, "Permission denied", ""));
        assert_eq!(denied_stdout, RootProbeOutcome::Denied);

        let (denied_stderr, _) =
            probe_with(|executor| executor.push_completed(0, "", "Operation not permitted"));
        assert_eq!(denied_stderr, RootProbeOutcome::Denied);

        let (unavailable, _) =
            probe_with(|executor| executor.push_completed(0, "", "su: not found"));
        assert_eq!(unavailable, RootProbeOutcome::Unavailable);

        let (granted, _) = probe_with(|executor| {
            executor.push_completed(
                1,
                "uid=0(root) gid=0(root) context=u:r:su:s0\n",
                "permission denied for a later diagnostic",
            )
        });
        assert_eq!(granted, RootProbeOutcome::Granted);

        let (unexpected, _) =
            probe_with(|executor| executor.push_completed(0, "unexpected output", ""));
        assert!(matches!(
            unexpected,
            RootProbeOutcome::CheckFailed {
                reason: RootProbeFailureReason::UnexpectedResponse,
                ..
            }
        ));
    }

    #[test]
    fn root_probe_classifies_only_recognized_transport_failures_as_transport() {
        let (transport, _) =
            probe_with(|executor| executor.push_completed(1, "", "error: device offline"));
        assert!(matches!(
            transport,
            RootProbeOutcome::CheckFailed {
                reason: RootProbeFailureReason::Transport,
                ..
            }
        ));
        let (missing_adb, _) = probe_with(|executor| executor.push_missing_binary());
        assert!(matches!(
            missing_adb,
            RootProbeOutcome::CheckFailed {
                reason: RootProbeFailureReason::UnexpectedResponse,
                ..
            }
        ));
        let (remote_phrase, _) =
            probe_with(|executor| executor.push_completed(1, "", "connection reset by peer"));
        assert!(matches!(
            remote_phrase,
            RootProbeOutcome::CheckFailed {
                reason: RootProbeFailureReason::UnexpectedResponse,
                ..
            }
        ));
    }

    #[test]
    fn root_probe_classifies_timeout_without_running_followup_commands() {
        let (outcome, calls) = probe_with(|executor| executor.push_timed_out());
        assert!(matches!(
            outcome,
            RootProbeOutcome::CheckFailed {
                reason: RootProbeFailureReason::TimedOut,
                ..
            }
        ));
        assert_eq!(calls.len(), 1);
    }

    fn checked_error(stdout: &str, stderr: &str) -> AdbCommandError {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(1, stdout, stderr);
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("reviewed-serial".to_string()), executor);
        runner
            .run(
                vec!["shell".to_string(), "true".to_string()],
                true,
                ProcessOperation::ShellMutation,
            )
            .unwrap_err()
    }

    #[test]
    fn completed_adb_transport_forms_are_classified_without_retaining_details() {
        let cases = [
            ("error: device offline", "", AdbCommandError::DeviceOffline),
            (
                "ADB: ERROR: DEVICE UNAUTHORIZED. Please check the confirmation dialog.",
                "",
                AdbCommandError::DeviceUnauthorized,
            ),
            (
                "",
                "error: device 'reviewed-serial' not found",
                AdbCommandError::DeviceDisconnected,
            ),
            (
                "",
                "adb: device 'reviewed-serial' not found",
                AdbCommandError::DeviceDisconnected,
            ),
            (
                "error: no devices/emulators found",
                "",
                AdbCommandError::DeviceDisconnected,
            ),
            (
                "",
                "adb: error: cannot connect to daemon at tcp:5037",
                AdbCommandError::AdbServerUnavailable,
            ),
            (
                "* failed to start daemon",
                "",
                AdbCommandError::AdbServerUnavailable,
            ),
            (
                "adb: error: connection reset by peer",
                "",
                AdbCommandError::TransportReset,
            ),
            (
                "error: failed to read response from device",
                "",
                AdbCommandError::TransportFailure,
            ),
            (
                "",
                "adb: error: failed to read copy response: EOF",
                AdbCommandError::TransportFailure,
            ),
            ("error: closed", "", AdbCommandError::TransportFailure),
        ];

        for (stdout, stderr, expected) in cases {
            let actual = checked_error(stdout, stderr);
            assert_eq!(actual, expected);
            assert!(!actual.to_string().contains("reviewed-serial"));
            assert!(!actual.to_string().contains("connection reset"));
        }
    }

    #[test]
    fn non_adb_transport_phrases_remain_ordinary_command_failures() {
        for text in [
            "remote command reported connection reset by peer",
            "application transport error while handling the request",
            "remote stream closed unexpectedly",
            "remote shell cannot connect to its service",
            "remote command failed to connect to a socket",
        ] {
            assert!(matches!(
                checked_error(text, ""),
                AdbCommandError::CommandFailed { .. }
            ));
        }
    }

    #[test]
    fn successful_output_remains_available_and_timeout_or_overflow_keeps_precedence() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "package:com.example.app/base.apk\n", "");
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        let result = runner
            .run(
                vec!["shell".to_string(), "pm".to_string(), "path".to_string()],
                false,
                ProcessOperation::Predicate,
            )
            .unwrap();
        assert_eq!(result.stdout, "package:com.example.app/base.apk\n");

        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_timed_out();
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        assert!(matches!(
            runner.run(
                vec!["shell".to_string(), "true".to_string()],
                true,
                ProcessOperation::ShellMutation,
            ),
            Err(AdbCommandError::TimedOut { .. })
        ));

        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_process_failed(ProcessFailureKind::StdoutOverflow);
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        assert!(matches!(
            runner.run(
                vec!["shell".to_string(), "true".to_string()],
                true,
                ProcessOperation::ShellMutation,
            ),
            Err(AdbCommandError::ProcessFailed {
                kind: ProcessFailureKind::StdoutOverflow,
                ..
            })
        ));
    }

    #[test]
    fn completed_storage_evidence_is_classified_from_stdout_and_stderr() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]\n", "");
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        assert!(matches!(
            runner.run(
                vec!["install".to_string(), "fixture.apk".to_string()],
                false,
                ProcessOperation::Install,
            ),
            Err(AdbCommandError::DeviceStorageExhausted)
        ));

        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(
            1,
            "",
            "adb: error: failed to copy 'fixture' to '/sdcard/fixture': remote couldn't create file: No space left on device\n",
        );
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        assert!(matches!(
            runner.run(
                vec![
                    "push".to_string(),
                    "fixture".to_string(),
                    "/sdcard/fixture".to_string()
                ],
                false,
                ProcessOperation::Push,
            ),
            Err(AdbCommandError::DeviceStorageExhausted)
        ));
    }

    #[test]
    fn file_push_uses_writable_dd_fsync_and_classifies_delayed_storage_exhaustion() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "", "");
        executor.push_completed(
            1,
            "",
            "/sdcard/fixture.bin: fsync: No space left on device\n",
        );
        let mut device = RealAdbDevice::with_executor("adb", Some("serial"), executor);

        assert!(device
            .push(Path::new("fixture.bin"), "/sdcard/fixture.bin", false)
            .is_err());
        assert_eq!(
            device.take_last_error(),
            Some(AdbCommandError::DeviceStorageExhausted)
        );
        assert_eq!(
            device.command_executor().operations(),
            [ProcessOperation::Push, ProcessOperation::DeviceCopy]
        );
        assert_eq!(
            device.command_executor().calls()[1],
            vec![
                "adb".to_string(),
                "-s".to_string(),
                "serial".to_string(),
                "shell".to_string(),
                "dd if=/dev/null of=/sdcard/fixture.bin count=0 conv=notrunc,fsync".to_string(),
            ]
        );
    }

    #[test]
    fn failed_file_push_does_not_run_durability_flush() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(1, "", "remote write failed\n");
        let mut device = RealAdbDevice::with_executor("adb", Some("serial"), executor);

        assert!(device
            .push(Path::new("fixture.bin"), "/sdcard/fixture.bin", false)
            .is_err());
        assert_eq!(
            device.command_executor().operations(),
            [ProcessOperation::Push]
        );
    }

    #[test]
    fn directory_push_does_not_run_file_durability_flush() {
        let source = tempfile::tempdir().expect("temporary source directory should be available");
        let executor = FakeAdbCommandExecutor::default();
        let mut device = RealAdbDevice::with_executor("adb", Some("serial"), executor);

        device
            .push(source.path(), "/sdcard/fixture-dir", false)
            .expect("directory push should retain its existing behavior");
        assert_eq!(
            device.command_executor().operations(),
            [ProcessOperation::Push]
        );
    }

    #[test]
    fn file_push_durability_flush_keeps_timeout_precedence() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "", "");
        executor.push_timed_out();
        let mut device = RealAdbDevice::with_executor("adb", Some("serial"), executor);

        assert!(device
            .push(Path::new("fixture.bin"), "/sdcard/fixture.bin", false)
            .is_err());
        assert!(matches!(
            device.take_last_error(),
            Some(AdbCommandError::TimedOut { .. })
        ));
        assert_eq!(
            device.command_executor().operations(),
            [ProcessOperation::Push, ProcessOperation::DeviceCopy]
        );
    }

    #[test]
    fn storage_classifier_rejects_unanchored_or_non_mutating_text() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "echo: No space left on device\n", "");
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        assert!(runner
            .run(
                vec!["shell".to_string(), "getprop".to_string()],
                false,
                ProcessOperation::Probe,
            )
            .is_ok());

        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(1, "", "No space left on device\n");
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        assert!(runner
            .run(
                vec!["shell".to_string(), "true".to_string()],
                false,
                ProcessOperation::ShellMutation,
            )
            .is_ok());
    }

    #[test]
    fn generic_reviewed_command_echoes_cannot_create_storage_classification() {
        for (stdout, stderr) in [
            (
                "mkdir: /sdcard/phase6d6-marker: No space left on device\n",
                "",
            ),
            (
                "argument=/sdcard/phase6d6-marker\n",
                "No space left on device\n",
            ),
            ("prose: mkdir: ignored: No space left on device\n", ""),
        ] {
            let mut executor = FakeAdbCommandExecutor::default();
            executor.push_completed(1, stdout, stderr);
            let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
            let result = runner.run(
                vec![
                    "shell".to_string(),
                    "printf".to_string(),
                    "%s".to_string(),
                    "mkdir: /sdcard/phase6d6-marker: No space left on device".to_string(),
                ],
                true,
                ProcessOperation::GenericFallback,
            );
            assert!(
                matches!(result, Err(AdbCommandError::CommandFailed { .. })),
                "generic command echo must remain ordinary failure: stdout={stdout:?} stderr={stderr:?}"
            );
        }
    }

    #[test]
    fn command_failure_diagnostic_is_preserved_internally_without_stdout_payload() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(17, "private stdout payload", "mkdir: Permission denied\n");
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("emulator-5554".into()), executor);

        let error = runner
            .run(
                vec![
                    "shell".to_string(),
                    "mkdir".to_string(),
                    "/data/user/0/com.emuchef.fixture/files/assets".to_string(),
                ],
                true,
                ProcessOperation::ShellMutation,
            )
            .expect_err("nonzero ADB result should be reported as a command failure");
        let expected_args = vec![
            "adb",
            "-s",
            "emulator-5554",
            "shell",
            "mkdir",
            "/data/user/0/com.emuchef.fixture/files/assets",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let expected = "The ADB command failed (exit 17): adb -s emulator-5554 shell mkdir /data/user/0/com.emuchef.fixture/files/assets; stderr: mkdir: Permission denied";

        match &error {
            AdbCommandError::CommandFailed {
                args,
                returncode,
                stderr,
            } => {
                assert_eq!(args, &expected_args);
                assert_eq!(*returncode, 17);
                assert_eq!(stderr, "mkdir: Permission denied");
            }
            other => panic!("expected a structured command failure, got {other:?}"),
        }

        assert_eq!(error.to_string(), expected);
        let operation_error = crate::executor::map_adb_error(error);
        assert_eq!(
            operation_error.kind,
            crate::executor::DeviceOperationKind::CommandFailed
        );
        assert_eq!(operation_error.message, "The ADB command failed.");
        assert!(!operation_error.message.contains("emulator-5554"));
        assert!(!operation_error.message.contains("Permission denied"));
        assert!(!operation_error.message.contains("private stdout payload"));
    }

    #[test]
    fn command_failure_diagnostic_redacts_sensitive_values() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(19, "", "token=stderr-secret\n");
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);

        let error = runner
            .run(
                vec![
                    "shell".to_string(),
                    "echo".to_string(),
                    "--token=argument-secret".to_string(),
                ],
                true,
                ProcessOperation::ShellMutation,
            )
            .expect_err("nonzero ADB result should be reported as a command failure");
        let diagnostic = error.to_string();

        assert!(diagnostic.contains("--token=[REDACTED]"));
        assert!(diagnostic.contains("token=[REDACTED]"));
        assert!(!diagnostic.contains("argument-secret"));
        assert!(!diagnostic.contains("stderr-secret"));
    }

    #[test]
    fn timeout_and_transport_errors_keep_precedence_over_storage_text() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_timed_out();
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        assert!(matches!(
            runner.run(
                vec!["shell".to_string(), "true".to_string()],
                false,
                ProcessOperation::ShellMutation,
            ),
            Err(AdbCommandError::TimedOut { .. })
        ));

        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(1, "", "error: device offline\nNo space left on device\n");
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        assert!(matches!(
            runner.run(
                vec!["shell".to_string(), "true".to_string()],
                false,
                ProcessOperation::ShellMutation,
            ),
            Err(AdbCommandError::DeviceOffline)
        ));
    }

    #[test]
    fn unchecked_completed_transport_results_are_not_interpreted_as_false() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(1, "", "error: device offline");
        let mut runner = AdbCommandRunner::with_executor("adb", None, executor);
        let error = runner
            .run(
                vec!["shell".to_string(), "test".to_string()],
                false,
                ProcessOperation::Predicate,
            )
            .unwrap_err();
        assert_eq!(error, AdbCommandError::DeviceOffline);
    }

    #[test]
    fn root_probe_uses_the_same_completed_transport_classifier() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(1, "", "error: device unauthorized");
        assert_eq!(
            probe_root_typed(&mut executor, "adb", "reviewed-serial"),
            Err(AdbCommandError::DeviceUnauthorized)
        );
    }

    #[test]
    fn controlled_review_device_uses_the_fixed_su_zero_route() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, "uid=0(root) gid=0(root) context=u:r:su:s0\n", "");
        let mut device = RealAdbDevice::with_executor("adb", Some("serial"), executor);
        device.configure_root_shell_route(Some(RootShellRoute::SuZero));

        device
            .check_root()
            .expect("the fixed su 0 probe should verify UID 0");

        assert_eq!(
            device.command_executor().calls(),
            &[vec![
                "adb".to_string(),
                "-s".to_string(),
                "serial".to_string(),
                "shell".to_string(),
                "su".to_string(),
                "0".to_string(),
                "id".to_string(),
            ]]
        );
    }

    #[test]
    fn real_device_methods_select_specific_operation_classes_exhaustively() {
        use crate::owned_process::ProcessOperation;

        let executor = FakeAdbCommandExecutor::default();
        let mut device = RealAdbDevice::with_executor("adb", Some("serial"), executor);

        let _ = device.check_root();
        device.install_apk(Path::new("fixture.apk"), true).unwrap();
        device
            .push(Path::new("fixture.bin"), "/sdcard/fixture.bin", false)
            .unwrap();
        device.mkdir_p("/sdcard/fixture").unwrap();
        device.remove_file("/sdcard/fixture.bin").unwrap();
        device.remove_tree("/sdcard/fixture").unwrap();
        device
            .copy_on_device("/sdcard/source", "/sdcard/dest", true, false)
            .unwrap();
        let _ = device.package_installed("com.example.fixture").unwrap();
        let _ = device.path_exists("/sdcard/fixture").unwrap();
        let _ = device.path_is_dir("/sdcard/fixture").unwrap();
        device
            .launch_app("com.example.fixture", Some(".MainActivity"))
            .unwrap();
        device.force_stop_app("com.example.fixture").unwrap();

        assert_eq!(
            device.command_executor().operations(),
            [
                ProcessOperation::RootPreflight,
                ProcessOperation::Install,
                ProcessOperation::Push,
                ProcessOperation::DeviceCopy,
                ProcessOperation::ShellMutation,
                ProcessOperation::ShellMutation,
                ProcessOperation::ShellMutation,
                ProcessOperation::DeviceCopy,
                ProcessOperation::Predicate,
                ProcessOperation::Predicate,
                ProcessOperation::Predicate,
                ProcessOperation::Launch,
                ProcessOperation::ForceStop,
            ]
        );
    }

    #[test]
    fn launcher_resolution_and_fallback_stay_in_the_launch_class() {
        use crate::owned_process::ProcessOperation;

        let executor = FakeAdbCommandExecutor::default();
        let mut device = RealAdbDevice::with_executor("adb", Some("serial"), executor);

        device.launch_app("com.example.fixture", None).unwrap();

        assert_eq!(
            device.command_executor().operations(),
            [
                ProcessOperation::Launch,
                ProcessOperation::Launch,
                ProcessOperation::Launch,
            ]
        );
    }
}
