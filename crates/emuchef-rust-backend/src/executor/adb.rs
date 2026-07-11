use std::fmt;
use std::io;
use std::path::Path;
use std::process::Command;

#[cfg(test)]
use std::collections::VecDeque;

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
        }
    }
}

pub trait AdbCommandExecutor: fmt::Debug {
    fn run(&mut self, args: &[String]) -> Result<AdbCommandResult, AdbCommandError>;
}

#[derive(Debug, Default)]
pub struct ProcessAdbCommandExecutor;

impl AdbCommandExecutor for ProcessAdbCommandExecutor {
    fn run(&mut self, args: &[String]) -> Result<AdbCommandResult, AdbCommandError> {
        let executable = args.first().ok_or_else(|| {
            AdbCommandError::InvalidPlanCommand("ADB command must not be empty.".to_string())
        })?;
        let output = Command::new(executable)
            .args(args.iter().skip(1))
            .output()
            .map_err(map_command_spawn_error)?;
        Ok(AdbCommandResult {
            args: args.to_vec(),
            returncode: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
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

    fn run(&mut self, args: Vec<String>, check: bool) -> Result<AdbCommandResult, AdbCommandError> {
        let mut full_args = vec![self.executable.clone()];
        if let Some(serial) = self.serial.as_deref() {
            full_args.push("-s".to_string());
            full_args.push(serial.to_string());
        }
        full_args.extend(args);
        self.run_raw(full_args, check)
    }

    fn run_raw(
        &mut self,
        full_args: Vec<String>,
        check: bool,
    ) -> Result<AdbCommandResult, AdbCommandError> {
        let result = self.executor.run(&full_args)?;
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
                python_list_repr(&command)
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
        self.run_raw(full_args, true).map(|_| ())
    }
}

#[derive(Debug)]
pub struct RealAdbDevice<E: AdbCommandExecutor = ProcessAdbCommandExecutor> {
    runner: AdbCommandRunner<E>,
}

impl RealAdbDevice<ProcessAdbCommandExecutor> {
    pub fn new(executable: impl Into<String>, serial: Option<String>) -> Self {
        Self {
            runner: AdbCommandRunner::new(executable, serial),
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
        }
    }

    #[cfg(test)]
    pub fn command_executor(&self) -> &E {
        self.runner.executor()
    }

    pub fn install_apk(&mut self, apk_path: &Path, replace_existing: bool) -> Result<(), String> {
        let mut args = vec!["install".to_string()];
        if replace_existing {
            args.push("-r".to_string());
        }
        args.push(apk_path.to_string_lossy().to_string());
        self.runner
            .run(args, true)
            .map(|_| ())
            .map_err(error_string)
    }

    pub fn push(&mut self, source: &Path, dest: &str, sync: bool) -> Result<(), String> {
        let mut args = vec!["push".to_string()];
        if sync {
            args.push("--sync".to_string());
        }
        args.push(source.to_string_lossy().to_string());
        args.push(dest.to_string());
        self.runner
            .run(args, true)
            .map(|_| ())
            .map_err(error_string)
    }

    pub fn mkdir_p(&mut self, path: &str) -> Result<(), String> {
        self.run_shell(
            vec!["mkdir".to_string(), "-p".to_string(), path.to_string()],
            true,
        )
        .map(|_| ())
    }

    pub fn remove_file(&mut self, path: &str) -> Result<(), String> {
        self.run_shell(
            vec!["rm".to_string(), "-f".to_string(), path.to_string()],
            true,
        )
        .map(|_| ())
    }

    pub fn remove_tree(&mut self, path: &str) -> Result<(), String> {
        self.run_shell(
            vec!["rm".to_string(), "-rf".to_string(), path.to_string()],
            true,
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
        )
        .map(|_| ())
    }

    pub fn package_installed(&mut self, package_name: &str) -> Result<bool, String> {
        let result = self
            .runner
            .run(
                vec![
                    "shell".to_string(),
                    "pm".to_string(),
                    "path".to_string(),
                    package_name.to_string(),
                ],
                false,
            )
            .map_err(error_string)?;
        Ok(result.returncode == 0 && result.stdout.contains("package:"))
    }

    pub fn path_exists(&mut self, path: &str) -> Result<bool, String> {
        let result = self.run_shell(
            vec!["test".to_string(), "-e".to_string(), path.to_string()],
            false,
        )?;
        Ok(result.returncode == 0)
    }

    pub fn path_is_dir(&mut self, path: &str) -> Result<bool, String> {
        let result = self.run_shell(
            vec!["test".to_string(), "-d".to_string(), path.to_string()],
            false,
        )?;
        Ok(result.returncode == 0)
    }

    pub fn run_plan_command(&mut self, command: Vec<String>) -> Result<(), String> {
        self.runner.run_plan_command(command).map_err(error_string)
    }

    pub fn launch_app(&mut self, package_name: &str, activity: Option<&str>) -> Result<(), String> {
        if let Some(activity) = activity.filter(|value| !value.is_empty()) {
            return self
                .runner
                .run(
                    vec![
                        "shell".to_string(),
                        "am".to_string(),
                        "start".to_string(),
                        "-n".to_string(),
                        format!("{package_name}/{activity}"),
                    ],
                    true,
                )
                .map(|_| ())
                .map_err(error_string);
        }

        if let Some(resolved_activity) = self.resolve_launcher_activity(package_name)? {
            return self
                .runner
                .run(
                    vec![
                        "shell".to_string(),
                        "am".to_string(),
                        "start".to_string(),
                        "-n".to_string(),
                        resolved_activity,
                    ],
                    true,
                )
                .map(|_| ())
                .map_err(error_string);
        }

        self.runner
            .run(
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
            )
            .map(|_| ())
            .map_err(error_string)
    }

    pub fn force_stop_app(&mut self, package_name: &str) -> Result<(), String> {
        self.runner
            .run(
                vec![
                    "shell".to_string(),
                    "am".to_string(),
                    "force-stop".to_string(),
                    package_name.to_string(),
                ],
                true,
            )
            .map(|_| ())
            .map_err(error_string)
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
            let result = self.runner.run(command, false).map_err(error_string)?;
            if result.returncode != 0 {
                continue;
            }
            if let Some(component) = parse_resolved_launcher_component(&result.stdout) {
                return Ok(Some(component));
            }
        }
        Ok(None)
    }

    fn run_shell(&mut self, args: Vec<String>, check: bool) -> Result<AdbCommandResult, String> {
        let privileged = args
            .last()
            .map(|path| is_app_private_path(path))
            .unwrap_or(false);
        self.run_shell_with_privilege(args, check, privileged)
    }

    fn run_shell_with_privilege(
        &mut self,
        args: Vec<String>,
        check: bool,
        privileged: bool,
    ) -> Result<AdbCommandResult, String> {
        self.runner
            .run(
                vec!["shell".to_string(), build_shell_command(&args, privileged)],
                check,
            )
            .map_err(error_string)
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct FakeAdbCommandExecutor {
    calls: Vec<Vec<String>>,
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
}

#[cfg(test)]
impl FakeAdbCommandExecutor {
    pub fn calls(&self) -> &[Vec<String>] {
        &self.calls
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
}

#[cfg(test)]
impl AdbCommandExecutor for FakeAdbCommandExecutor {
    fn run(&mut self, args: &[String]) -> Result<AdbCommandResult, AdbCommandError> {
        self.calls.push(args.to_vec());
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
            None => Ok(AdbCommandResult {
                args: args.to_vec(),
                returncode: 0,
                stdout: String::new(),
                stderr: String::new(),
            }),
        }
    }
}

fn map_command_spawn_error(error: io::Error) -> AdbCommandError {
    if error.kind() == io::ErrorKind::NotFound {
        AdbCommandError::Resolution(adb_not_found_message())
    } else {
        AdbCommandError::Resolution(format!(
            "The configured ADB executable could not be started: {error}"
        ))
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

fn python_list_repr(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}
