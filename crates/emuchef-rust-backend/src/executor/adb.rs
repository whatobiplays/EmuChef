use std::fmt;
use std::path::Path;

#[cfg(test)]
use std::collections::VecDeque;

use crate::owned_process::{
    run_owned_process, OwnedProcessError, ProcessCleanup, ProcessFailureKind, ProcessOperation,
};
use crate::planner::TargetDeviceBinding;

use super::identity::{IdentityCheckPhase, POST_OPERATION_IDENTITY_FAILURE_MARKER};
use super::root_authority::{self, DeviceCommandEffect};
use super::root_requirements::is_app_private_path;

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
    CommandFailed,
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
    InvalidPlanCommand(String),
}

impl fmt::Display for AdbCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdbCommandError::Resolution(message) | AdbCommandError::InvalidPlanCommand(message) => {
                formatter.write_str(message)
            }
            AdbCommandError::CommandFailed => formatter.write_str("The ADB command failed."),
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
                formatter.write_str("The device connection was reset during execution.")
            }
            AdbCommandError::TransportFailure => {
                formatter.write_str("The device connection was lost during execution.")
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
                formatter.write_str("The ADB process failed before producing a trustworthy result.")
            }
            AdbCommandError::TimedOut { .. } => formatter.write_str("The ADB operation timed out."),
            AdbCommandError::RootDenied => {
                formatter.write_str("Root access was denied by the device.")
            }
            AdbCommandError::RootUnavailable => {
                formatter.write_str("Root access is unavailable on this device.")
            }
            AdbCommandError::RootCheckFailed => {
                formatter.write_str("Root access could not be checked safely.")
            }
            AdbCommandError::RootAuthorityRevoked => {
                formatter.write_str("Root authority was revoked during execution.")
            }
            AdbCommandError::RootAuthorityUnverified => {
                formatter.write_str("Continued root authority could not be confirmed safely.")
            }
        }
    }
}

pub trait AdbCommandExecutor: fmt::Debug {
    fn run_for(
        &mut self,
        args: &[String],
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError>;
}

#[derive(Debug, Default)]
pub struct ProcessAdbCommandExecutor;

impl AdbCommandExecutor for ProcessAdbCommandExecutor {
    fn run_for(
        &mut self,
        args: &[String],
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        let (executable, command_args) = args.split_first().ok_or_else(|| {
            AdbCommandError::InvalidPlanCommand("ADB command must not be empty.".to_string())
        })?;
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

pub(crate) fn probe_root_typed<E: AdbCommandExecutor>(
    executor: &mut E,
    adb_path: &str,
    serial: &str,
) -> Result<RootProbeOutcome, AdbCommandError> {
    let args = vec![
        adb_path.to_string(),
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
        "su".to_string(),
        "-c".to_string(),
        "id".to_string(),
    ];
    let result = executor.run_for(&args, ProcessOperation::RootPreflight)?;
    let result = classify_completed_transport(result)?;
    Ok(classify_root_result(&result))
}

fn classify_root_result(result: &AdbCommandResult) -> RootProbeOutcome {
    let normalized_stdout = result.stdout.trim().to_ascii_lowercase();
    if normalized_stdout.starts_with("uid=0(") || normalized_stdout.starts_with("uid=0 ") {
        return RootProbeOutcome::Granted;
    }
    let combined = format!("{}\n{}", result.stdout, result.stderr);
    let lower = combined.to_ascii_lowercase();
    if is_unavailable_root(&lower) {
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

impl AdbCommandError {
    fn allows_post_identity_probe(&self) -> bool {
        matches!(
            self,
            Self::CommandFailed
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
        Self::with_executor(executable, serial, ProcessAdbCommandExecutor)
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
            Err(AdbCommandError::CommandFailed)
        } else {
            Ok(result)
        }
    }

    fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), AdbCommandError> {
        if command.is_empty() {
            return Err(AdbCommandError::InvalidPlanCommand(
                "Plan command must not be empty.".to_string(),
            ));
        }
        if command[0] != "adb" {
            return Err(AdbCommandError::InvalidPlanCommand(format!(
                "Plan command must start with 'adb': {}",
                list_repr(&command)
            )));
        }

        let mut tail = command.into_iter().skip(1).collect::<Vec<_>>();
        if self.serial.is_some() && !command_has_serial_flag(&tail) {
            let serial = self
                .serial
                .as_deref()
                .expect("serial should be present after is_some guard")
                .to_string();
            tail.splice(0..0, ["-s".to_string(), serial]);
        }
        let mut full_args = vec![self.executable.clone()];
        full_args.extend(tail);
        self.run_raw(full_args, true, ProcessOperation::GenericFallback)
            .map(|_| ())
    }
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

#[derive(Debug)]
pub struct RealAdbDevice<E: AdbCommandExecutor = ProcessAdbCommandExecutor> {
    runner: AdbCommandRunner<E>,
    last_error: Option<AdbCommandError>,
    identity: super::identity::IdentityGuard,
    root_authority: root_authority::RootAuthorityGuard,
}

impl RealAdbDevice<ProcessAdbCommandExecutor> {
    pub fn new(executable: impl Into<String>, serial: Option<String>) -> Self {
        Self {
            runner: AdbCommandRunner::new(executable, serial),
            last_error: None,
            identity: super::identity::IdentityGuard::default(),
            root_authority: root_authority::RootAuthorityGuard::default(),
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
        if let Err(error) = self
            .identity
            .check(&mut self.runner, IdentityCheckPhase::PreOperation)
        {
            return self.map_public(Err(error));
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
                return self.map_public(Err(error));
            }
        }
        self.map_public(result)
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
        let result = probe_root_typed(&mut self.runner.executor, &self.runner.executable, &serial);
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
        let mut args = vec!["push".to_string()];
        if sync {
            args.push("--sync".to_string());
        }
        args.push(source.to_string_lossy().to_string());
        args.push(dest.to_string());
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device
                .runner
                .run(args, true, ProcessOperation::Push)
                .map(|_| ())
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

    pub fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), String> {
        self.guarded(DeviceCommandEffect::Mutating, |device| {
            device.runner.run_plan_command(command)
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
            vec!["shell".to_string(), build_shell_command(&args, privileged)],
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
        let outcome =
            probe_root_typed(&mut self.runner.executor, &self.runner.executable, &serial)?;
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
        if body == "closed" || body.starts_with("failed to read response from device") {
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
    error.to_string()
}

fn adb_not_found_message() -> String {
    "The configured ADB executable could not be started. Ensure adb is available on PATH or pass an explicit executable when constructing RealAdbDevice.".to_string()
}

fn command_has_serial_flag(args: &[String]) -> bool {
    args.len() >= 2 && args[0] == "-s"
}

fn build_shell_command(args: &[String], privileged: bool) -> String {
    let command = shell_join(args);
    if privileged {
        shell_join(&["su".to_string(), "-c".to_string(), command])
    } else {
        command
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

fn list_repr(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn root_probe_uses_exact_serialized_su_id_command_and_grants_uid_zero() {
        let (outcome, calls) = probe_with(|executor| {
            executor.push_completed(0, "uid=0(root) gid=0(root) groups=0(root)\n", "");
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
                "uid=0(root) gid=0(root)\n",
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
                AdbCommandError::CommandFailed
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
            let result = runner.run_plan_command(vec![
                "adb".to_string(),
                "shell".to_string(),
                "printf".to_string(),
                "%s".to_string(),
                "mkdir: /sdcard/phase6d6-marker: No space left on device".to_string(),
            ]);
            assert!(
                matches!(result, Err(AdbCommandError::CommandFailed)),
                "generic command echo must remain ordinary failure: stdout={stdout:?} stderr={stderr:?}"
            );
        }
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
            .run_plan_command(vec!["adb".to_string(), "version".to_string()])
            .unwrap();
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
                ProcessOperation::ShellMutation,
                ProcessOperation::ShellMutation,
                ProcessOperation::ShellMutation,
                ProcessOperation::DeviceCopy,
                ProcessOperation::Predicate,
                ProcessOperation::Predicate,
                ProcessOperation::Predicate,
                ProcessOperation::GenericFallback,
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
