use std::fmt;
use std::path::Path;

#[cfg(test)]
use std::collections::VecDeque;

use crate::owned_process::{
    run_owned_process, OwnedProcessError, ProcessCleanup, ProcessFailureKind, ProcessOperation,
};

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
    CommandFailed(AdbCommandResult),
    ProcessFailed {
        kind: ProcessFailureKind,
        cleanup: ProcessCleanup,
    },
    TimedOut {
        args: Vec<String>,
        cleanup: ProcessCleanup,
    },
    RootDenied,
    RootUnavailable,
    RootCheckFailed,
    InvalidPlanCommand(String),
}

impl fmt::Display for AdbCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdbCommandError::Resolution(message) | AdbCommandError::InvalidPlanCommand(message) => {
                formatter.write_str(message)
            }
            AdbCommandError::CommandFailed(result) => {
                write!(
                    formatter,
                    "ADB command failed ({}): {}",
                    result.returncode,
                    result.args.join(" ")
                )?;
                if !result.stderr.is_empty() {
                    write!(formatter, "\n{}", result.stderr.trim())?;
                }
                Ok(())
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
        Ok(AdbCommandResult {
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
            let message = error.to_string();
            if is_transport_failure(&message) {
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
    Ok(classify_root_result(&result))
}

fn classify_root_result(result: &AdbCommandResult) -> RootProbeOutcome {
    if result.returncode == 0 {
        let normalized = result.stdout.trim().to_ascii_lowercase();
        if normalized.starts_with("uid=0(") || normalized.starts_with("uid=0 ") {
            return RootProbeOutcome::Granted;
        }
        return RootProbeOutcome::CheckFailed {
            reason: RootProbeFailureReason::UnexpectedResponse,
            message: "The root access check returned an unexpected response.",
        };
    }
    let combined = format!("{}\n{}", result.stdout, result.stderr);
    let lower = combined.to_ascii_lowercase();
    if is_transport_failure(&lower) {
        RootProbeOutcome::CheckFailed {
            reason: RootProbeFailureReason::Transport,
            message: "The device connection failed during the root access check.",
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

fn is_transport_failure(text: &str) -> bool {
    [
        "device not found",
        "no devices/emulators found",
        "no devices found",
        "device offline",
        "device unauthorized",
        "cannot connect",
        "failed to connect",
        "transport error",
        "transport id",
    ]
    .iter()
    .any(|marker| text.to_ascii_lowercase().contains(marker))
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
        if check && result.returncode != 0 {
            Err(AdbCommandError::CommandFailed(result))
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

#[derive(Debug)]
pub struct RealAdbDevice<E: AdbCommandExecutor = ProcessAdbCommandExecutor> {
    runner: AdbCommandRunner<E>,
    last_error: Option<AdbCommandError>,
}

impl RealAdbDevice<ProcessAdbCommandExecutor> {
    pub fn new(executable: impl Into<String>, serial: Option<String>) -> Self {
        Self {
            runner: AdbCommandRunner::new(executable, serial),
            last_error: None,
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
        }
    }

    #[cfg(test)]
    pub fn command_executor(&self) -> &E {
        self.runner.executor()
    }

    pub(crate) fn take_last_error(&mut self) -> Option<AdbCommandError> {
        self.last_error.take()
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

    pub fn check_root(&mut self) -> Result<(), String> {
        let Some(serial) = self.runner.serial.clone() else {
            let error = AdbCommandError::RootCheckFailed;
            self.last_error = Some(error.clone());
            return Err(error_string(error));
        };
        let result = probe_root_typed(&mut self.runner.executor, &self.runner.executable, &serial);
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.last_error = Some(error.clone());
                return Err(error_string(error));
            }
        };
        match result {
            RootProbeOutcome::Granted => Ok(()),
            RootProbeOutcome::Denied => {
                let error = AdbCommandError::RootDenied;
                self.last_error = Some(error.clone());
                Err(error_string(error))
            }
            RootProbeOutcome::Unavailable => {
                let error = AdbCommandError::RootUnavailable;
                self.last_error = Some(error.clone());
                Err(error_string(error))
            }
            RootProbeOutcome::CheckFailed { .. } => {
                let error = AdbCommandError::RootCheckFailed;
                self.last_error = Some(error.clone());
                Err(error_string(error))
            }
        }
    }

    pub fn install_apk(&mut self, apk_path: &Path, replace_existing: bool) -> Result<(), String> {
        let mut args = vec!["install".to_string()];
        if replace_existing {
            args.push("-r".to_string());
        }
        args.push(apk_path.to_string_lossy().to_string());
        let result = self.runner.run(args, true, ProcessOperation::Install);
        self.map_public(result).map(|_| ())
    }

    pub fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), String> {
        let mut args = vec!["push".to_string()];
        if sync {
            args.push("--sync".to_string());
        }
        args.push(source.to_string_lossy().to_string());
        args.push(dest.to_string());
        let result = self.runner.run(args, true, ProcessOperation::Push);
        self.map_public(result).map(|_| ())
    }

    pub fn mkdir_p(&mut self, path: &str) -> Result<(), String> {
        self.run_shell(
            vec!["mkdir".to_string(), "-p".to_string(), path.to_string()],
            true,
            ProcessOperation::ShellMutation,
        )
        .map(|_| ())
    }

    pub fn remove_file(&mut self, path: &str) -> Result<(), String> {
        self.run_shell(
            vec!["rm".to_string(), "-f".to_string(), path.to_string()],
            true,
            ProcessOperation::ShellMutation,
        )
        .map(|_| ())
    }

    pub fn remove_tree(&mut self, path: &str) -> Result<(), String> {
        self.run_shell(
            vec!["rm".to_string(), "-rf".to_string(), path.to_string()],
            true,
            ProcessOperation::ShellMutation,
        )
        .map(|_| ())
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
        self.run_shell_with_privilege(
            args,
            true,
            privileged || is_app_private_path(source) || is_app_private_path(dest),
            ProcessOperation::DeviceCopy,
        )
        .map(|_| ())
    }

    pub fn package_installed(&mut self, package_name: &str) -> Result<bool, String> {
        let result = self.runner.run(
            vec![
                "shell".to_string(),
                "pm".to_string(),
                "path".to_string(),
                package_name.to_string(),
            ],
            false,
            ProcessOperation::Predicate,
        );
        let result = self.map_public(result)?;
        Ok(result.returncode == 0 && result.stdout.contains("package:"))
    }

    pub fn path_exists(&mut self, path: &str) -> Result<bool, String> {
        let result = self.run_shell(
            vec!["test".to_string(), "-e".to_string(), path.to_string()],
            false,
            ProcessOperation::Predicate,
        )?;
        Ok(result.returncode == 0)
    }

    pub fn path_is_dir(&mut self, path: &str) -> Result<bool, String> {
        let result = self.run_shell(
            vec!["test".to_string(), "-d".to_string(), path.to_string()],
            false,
            ProcessOperation::Predicate,
        )?;
        Ok(result.returncode == 0)
    }

    pub fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), String> {
        let result = self.runner.run_plan_command(command);
        self.map_public(result).map(|_| ())
    }

    pub fn launch_app(&mut self, package_name: &str, activity: Option<&str>) -> Result<(), String> {
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
            return self.map_public(result).map(|_| ());
        }

        if let Some(resolved_activity) = self.resolve_launcher_activity(package_name)? {
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
            return self.map_public(result).map(|_| ());
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
        self.map_public(result).map(|_| ())
    }

    pub fn force_stop_app(&mut self, package_name: &str) -> Result<(), String> {
        let result = self.runner.run(
            vec![
                "shell".to_string(),
                "am".to_string(),
                "force-stop".to_string(),
                package_name.to_string(),
            ],
            true,
            ProcessOperation::ForceStop,
        );
        self.map_public(result).map(|_| ())
    }

    fn resolve_launcher_activity(&mut self, package_name: &str) -> Result<Option<String>, String> {
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
            let result = self.map_public(result)?;
            if result.returncode != 0 {
                continue;
            }
            if let Some(component) = parse_resolved_launcher_component(&result.stdout) {
                return Ok(Some(component));
            }
        }
        Ok(None)
    }

    fn run_shell(
        &mut self,
        args: Vec<String>,
        check: bool,
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, String> {
        let privileged = args
            .last()
            .map(|path| is_app_private_path(path))
            .unwrap_or(false);
        self.run_shell_with_privilege(args, check, privileged, operation)
    }

    fn run_shell_with_privilege(
        &mut self,
        args: Vec<String>,
        check: bool,
        privileged: bool,
        operation: ProcessOperation,
    ) -> Result<AdbCommandResult, String> {
        let result = self.runner.run(
            vec!["shell".to_string(), build_shell_command(&args, privileged)],
            check,
            operation,
        );
        self.map_public(result)
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
                args: args.to_vec(),
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

fn map_process_error(args: &[String], error: OwnedProcessError) -> AdbCommandError {
    match error.kind {
        ProcessFailureKind::Spawn => AdbCommandError::Resolution(adb_not_found_message()),
        ProcessFailureKind::TimedOut => AdbCommandError::TimedOut {
            args: args.to_vec(),
            cleanup: error.cleanup,
        },
        kind => AdbCommandError::ProcessFailed {
            kind,
            cleanup: error.cleanup,
        },
    }
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

fn is_app_private_path(path: &str) -> bool {
    path.starts_with("/data/user/") || path.starts_with("/data/data/")
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
    fn root_probe_classifies_only_recognized_transport_failures_as_transport() {
        let (transport, _) =
            probe_with(|executor| executor.push_completed(1, "", "device offline"));
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
