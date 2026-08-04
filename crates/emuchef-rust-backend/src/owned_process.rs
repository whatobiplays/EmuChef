//! Private ownership boundary for bounded, one-shot host processes.
//!
//! The caller owns the child and both output pipes for the complete operation.
//! A single locally driven future tree observes process exit, output, and the
//! deadline. No executor task, reader thread, or channel producer is created,
//! so dropping a pending local future cannot leave work running after return.

#[cfg(test)]
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::task::Poll;
use std::time::Duration;

use async_io::{block_on, Timer};
use async_process::{Child, ChildStderr, ChildStdout, Command};
use futures_lite::future::{self, poll_fn};
use futures_lite::io::{AsyncRead, AsyncReadExt};

pub(crate) const OUTPUT_CAPTURE_LIMIT: usize = 4 * 1024 * 1024;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const STREAM_BUFFER_BYTES: usize = 16 * 1024;

#[cfg(test)]
thread_local! {
    /// A one-shot delay arm belongs only to the current test thread.  Keeping
    /// the arm in thread-local storage prevents one qualification invocation
    /// from changing an unrelated probe or parallel test.
    static TEST_PROCESS_DELAY: RefCell<Option<(ProcessOperation, Duration)>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct TestProcessDelayGuard;

#[cfg(test)]
impl Drop for TestProcessDelayGuard {
    fn drop(&mut self) {
        TEST_PROCESS_DELAY.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
pub(crate) fn arm_test_process_delay(
    operation: ProcessOperation,
    delay: Duration,
) -> TestProcessDelayGuard {
    TEST_PROCESS_DELAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(
            slot.replace((operation, delay)).is_none(),
            "a process-delay arm must be scoped and cannot be replaced"
        );
    });
    TestProcessDelayGuard
}

#[cfg(test)]
fn take_test_process_delay(operation: ProcessOperation) -> Option<Duration> {
    TEST_PROCESS_DELAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        match slot.take() {
            Some((armed_operation, delay)) if armed_operation == operation => Some(delay),
            other => {
                *slot = other;
                None
            }
        }
    })
}

#[cfg(not(test))]
fn take_test_process_delay(_operation: ProcessOperation) -> Option<Duration> {
    None
}

/// Fixed internal classes for every owned backend ADB process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessOperation {
    Probe,
    Predicate,
    RootPreflight,
    Launch,
    ForceStop,
    ShellMutation,
    Install,
    Push,
    DeviceCopy,
    /// Defensive fallback reserved for generic reviewed-plan ADB commands.
    GenericFallback,
}

impl ProcessOperation {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 10] = [
        Self::Probe,
        Self::Predicate,
        Self::RootPreflight,
        Self::Launch,
        Self::ForceStop,
        Self::ShellMutation,
        Self::Install,
        Self::Push,
        Self::DeviceCopy,
        Self::GenericFallback,
    ];

    pub(crate) const fn deadline(self) -> Duration {
        match self {
            Self::Probe
            | Self::Predicate
            | Self::RootPreflight
            | Self::Launch
            | Self::ForceStop => Duration::from_secs(30),
            Self::ShellMutation => Duration::from_secs(120),
            Self::Install | Self::Push | Self::DeviceCopy | Self::GenericFallback => {
                Duration::from_secs(300)
            }
        }
    }
}

/// Stable reason why an owned process did not produce a usable result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessFailureKind {
    Spawn,
    Wait,
    StdoutRead,
    StderrRead,
    StdoutOverflow,
    StderrOverflow,
    TimedOut,
    ReaderJoin,
}

/// Whether child reap and local I/O completion were confirmed before return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessCleanup {
    NotRequired,
    Confirmed,
    Uncertain,
}

/// Process failure together with independent lifecycle-cleanup evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnedProcessError {
    pub(crate) kind: ProcessFailureKind,
    pub(crate) cleanup: ProcessCleanup,
}

impl OwnedProcessError {
    fn new(kind: ProcessFailureKind, cleanup: ProcessCleanup) -> Self {
        Self { kind, cleanup }
    }
}

/// Bounded output and exit status from a fully settled one-shot child process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CapturedProcessOutput {
    pub(crate) status_code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    const fn read_failure(self) -> ProcessFailureKind {
        match self {
            Self::Stdout => ProcessFailureKind::StdoutRead,
            Self::Stderr => ProcessFailureKind::StderrRead,
        }
    }

    const fn overflow_failure(self) -> ProcessFailureKind {
        match self {
            Self::Stdout => ProcessFailureKind::StdoutOverflow,
            Self::Stderr => ProcessFailureKind::StderrOverflow,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StreamCapture {
    bytes: Vec<u8>,
}

/// Read one pipe until EOF or a fatal bounded-output condition.
///
/// The retained prefix never exceeds `OUTPUT_CAPTURE_LIMIT`. Once the limit is
/// crossed the owner stops the process; the remainder is deliberately not
/// retained or allocated.
async fn capture_stream<R: AsyncRead + Unpin>(
    mut reader: R,
    stream: OutputStream,
) -> Result<StreamCapture, ProcessFailureKind> {
    let mut retained = Vec::with_capacity(OUTPUT_CAPTURE_LIMIT.min(64 * 1024));
    let mut buffer = [0_u8; STREAM_BUFFER_BYTES];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => return Ok(StreamCapture { bytes: retained }),
            Ok(count) => {
                let remaining = OUTPUT_CAPTURE_LIMIT.saturating_sub(retained.len());
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

async fn read_streams(
    stdout: ChildStdout,
    stderr: ChildStderr,
) -> Result<(StreamCapture, StreamCapture), ProcessFailureKind> {
    let mut stdout_future = Box::pin(capture_stream(stdout, OutputStream::Stdout));
    let mut stderr_future = Box::pin(capture_stream(stderr, OutputStream::Stderr));
    let mut stdout_result = None;
    let mut stderr_result = None;

    poll_fn(|context| {
        if stdout_result.is_none() {
            if let Poll::Ready(result) = stdout_future.as_mut().poll(context) {
                match result {
                    Ok(capture) => stdout_result = Some(capture),
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
        }
        if stderr_result.is_none() {
            if let Poll::Ready(result) = stderr_future.as_mut().poll(context) {
                match result {
                    Ok(capture) => stderr_result = Some(capture),
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
        }
        match (stdout_result.take(), stderr_result.take()) {
            (Some(stdout), Some(stderr)) => Poll::Ready(Ok((stdout, stderr))),
            (stdout, stderr) => {
                stdout_result = stdout;
                stderr_result = stderr;
                Poll::Pending
            }
        }
    })
    .await
}

enum ProcessEvent {
    Exited(Result<i32, ()>),
    Output(Result<(StreamCapture, StreamCapture), ProcessFailureKind>),
    TimedOut,
}

enum SettledOutput {
    Complete(Result<(StreamCapture, StreamCapture), ProcessFailureKind>),
    TimedOut,
}

enum SettledStatus {
    Complete(Result<i32, ()>),
    TimedOut,
}

async fn settle_output(
    mut output_future: Pin<
        Box<impl Future<Output = Result<(StreamCapture, StreamCapture), ProcessFailureKind>>>,
    >,
) -> SettledOutput {
    let mut timer = Box::pin(Timer::after(CLEANUP_TIMEOUT));
    poll_fn(|context| {
        if let Poll::Ready(result) = output_future.as_mut().poll(context) {
            return Poll::Ready(SettledOutput::Complete(result));
        }
        if timer.as_mut().poll(context).is_ready() {
            return Poll::Ready(SettledOutput::TimedOut);
        }
        Poll::Pending
    })
    .await
}

async fn settle_status(
    mut status_future: Pin<
        Box<impl Future<Output = Result<async_process::ExitStatus, std::io::Error>>>,
    >,
) -> SettledStatus {
    let mut timer = Box::pin(Timer::after(CLEANUP_TIMEOUT));
    poll_fn(|context| {
        if let Poll::Ready(result) = status_future.as_mut().poll(context) {
            return Poll::Ready(SettledStatus::Complete(
                result
                    .map(|status| status.code().unwrap_or(-1))
                    .map_err(|_| ()),
            ));
        }
        if timer.as_mut().poll(context).is_ready() {
            return Poll::Ready(SettledStatus::TimedOut);
        }
        Poll::Pending
    })
    .await
}

fn cleanup_child(child: &mut Child, primary: ProcessFailureKind) -> OwnedProcessError {
    let already_reaped = child.try_status().ok().flatten().is_some();
    let reaped = if already_reaped {
        true
    } else {
        let _ = child.kill();
        if child.try_status().ok().flatten().is_some() {
            true
        } else {
            block_on(async {
                future::race(async { child.status().await.is_ok() }, async {
                    Timer::after(CLEANUP_TIMEOUT).await;
                    false
                })
                .await
            })
        }
    };
    OwnedProcessError::new(
        primary,
        if reaped {
            ProcessCleanup::Confirmed
        } else {
            ProcessCleanup::Uncertain
        },
    )
}

async fn run_child(
    child: Child,
    stdout: ChildStdout,
    stderr: ChildStderr,
    deadline: Duration,
    process_delay: Option<Duration>,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    run_child_with_deadline_signal(child, stdout, stderr, Timer::after(deadline), process_delay)
        .await
}

async fn run_child_with_deadline_signal<D: Future>(
    mut child: Child,
    stdout: ChildStdout,
    stderr: ChildStderr,
    deadline_signal: D,
    process_delay: Option<Duration>,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    let mut status_future = Box::pin(child.status());
    let mut output_future = Box::pin(read_streams(stdout, stderr));
    let mut timer = Box::pin(deadline_signal);
    let mut process_delay = process_delay.map(|delay| Box::pin(Timer::after(delay)));
    let mut process_delay_ready = process_delay.is_none();

    let event = poll_fn(|context| {
        if !process_delay_ready {
            if let Some(delay) = process_delay.as_mut() {
                if delay.as_mut().poll(context).is_ready() {
                    process_delay_ready = true;
                }
            }
        }
        // Output is checked first so an already-observed overflow/read failure
        // retains precedence over a status or deadline observed in the same poll.
        if let Poll::Ready(result) = output_future.as_mut().poll(context) {
            return Poll::Ready(ProcessEvent::Output(result));
        }
        if let Poll::Ready(result) = status_future.as_mut().poll(context) {
            return Poll::Ready(ProcessEvent::Exited(
                result
                    .map(|status| status.code().unwrap_or(-1))
                    .map_err(|_| ()),
            ));
        }
        if timer.as_mut().poll(context).is_ready() {
            return Poll::Ready(ProcessEvent::TimedOut);
        }
        Poll::Pending
    })
    .await;

    match event {
        ProcessEvent::Output(result) => {
            drop(output_future);
            match result {
                Err(kind) => {
                    drop(status_future);
                    Err(cleanup_child(&mut child, kind))
                }
                Ok((stdout, stderr)) => {
                    drop(timer);
                    match settle_status(status_future).await {
                        SettledStatus::Complete(Ok(status_code)) => Ok(CapturedProcessOutput {
                            status_code: Some(status_code),
                            stdout: stdout.bytes,
                            stderr: stderr.bytes,
                        }),
                        SettledStatus::Complete(Err(())) => {
                            Err(cleanup_child(&mut child, ProcessFailureKind::Wait))
                        }
                        SettledStatus::TimedOut => {
                            Err(cleanup_child(&mut child, ProcessFailureKind::TimedOut))
                        }
                    }
                }
            }
        }
        ProcessEvent::Exited(result) => {
            drop(status_future);
            match result {
                Err(()) => {
                    drop(output_future);
                    Err(cleanup_child(&mut child, ProcessFailureKind::Wait))
                }
                Ok(status_code) => match settle_output(output_future).await {
                    SettledOutput::Complete(Ok((stdout, stderr))) => Ok(CapturedProcessOutput {
                        status_code: Some(status_code),
                        stdout: stdout.bytes,
                        stderr: stderr.bytes,
                    }),
                    SettledOutput::Complete(Err(kind)) => {
                        Err(OwnedProcessError::new(kind, ProcessCleanup::Confirmed))
                    }
                    SettledOutput::TimedOut => Err(OwnedProcessError::new(
                        ProcessFailureKind::ReaderJoin,
                        ProcessCleanup::Confirmed,
                    )),
                },
            }
        }
        ProcessEvent::TimedOut => {
            drop(status_future);
            drop(output_future);
            Err(cleanup_child(&mut child, ProcessFailureKind::TimedOut))
        }
    }
}

fn run_owned_process_with_deadline(
    program: &str,
    args: &[String],
    deadline: Duration,
    operation: ProcessOperation,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            OwnedProcessError::new(ProcessFailureKind::Spawn, ProcessCleanup::NotRequired)
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| cleanup_child(&mut child, ProcessFailureKind::Spawn))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| cleanup_child(&mut child, ProcessFailureKind::Spawn))?;
    block_on(run_child(
        child,
        stdout,
        stderr,
        deadline,
        take_test_process_delay(operation),
    ))
}

#[cfg(test)]
/// Run an owned child with an injected deadline future for timer tests.
fn run_owned_process_with_deadline_signal<D: Future + 'static>(
    program: &str,
    args: &[String],
    deadline_signal: D,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            OwnedProcessError::new(ProcessFailureKind::Spawn, ProcessCleanup::NotRequired)
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| cleanup_child(&mut child, ProcessFailureKind::Spawn))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| cleanup_child(&mut child, ProcessFailureKind::Spawn))?;
    block_on(run_child_with_deadline_signal(
        child,
        stdout,
        stderr,
        deadline_signal,
        None,
    ))
}

/// Execute one command without a shell and retain explicit ownership until its
/// process and local I/O have reached a bounded terminal lifecycle result.
pub(crate) fn run_owned_process(
    program: &str,
    args: &[String],
    operation: ProcessOperation,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    run_owned_process_with_deadline(program, args, operation.deadline(), operation)
}

#[cfg(test)]
fn run_owned_process_for_test(
    program: &str,
    args: &[String],
    deadline: Duration,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    run_owned_process_with_deadline(program, args, deadline, ProcessOperation::Probe)
}

#[cfg(test)]
fn run_owned_process_for_test_with_operation(
    program: &str,
    args: &[String],
    deadline: Duration,
    operation: ProcessOperation,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    run_owned_process_with_deadline(program, args, deadline, operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_operation_deadlines_cover_the_exhaustive_inventory() {
        let actual = ProcessOperation::ALL
            .into_iter()
            .map(|operation| (operation, operation.deadline()))
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                (ProcessOperation::Probe, Duration::from_secs(30)),
                (ProcessOperation::Predicate, Duration::from_secs(30)),
                (ProcessOperation::RootPreflight, Duration::from_secs(30)),
                (ProcessOperation::Launch, Duration::from_secs(30)),
                (ProcessOperation::ForceStop, Duration::from_secs(30)),
                (ProcessOperation::ShellMutation, Duration::from_secs(120)),
                (ProcessOperation::Install, Duration::from_secs(300)),
                (ProcessOperation::Push, Duration::from_secs(300)),
                (ProcessOperation::DeviceCopy, Duration::from_secs(300)),
                (ProcessOperation::GenericFallback, Duration::from_secs(300)),
            ]
        );
    }

    #[test]
    fn timed_out_process_reports_timeout_with_cleanup_evidence() {
        let executable = std::env::current_exe().expect("test executable should be available");
        let error = run_owned_process_for_test(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::timeout_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            Duration::from_millis(100),
        )
        .expect_err("the helper must exceed the test deadline");

        assert_eq!(error.kind, ProcessFailureKind::TimedOut);
        assert!(matches!(
            error.cleanup,
            ProcessCleanup::Confirmed | ProcessCleanup::Uncertain
        ));
    }

    #[test]
    fn completed_process_returns_status_and_bounded_output() {
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = run_owned_process_for_test(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::normal_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            Duration::from_secs(2),
        )
        .expect("the helper should complete");

        assert_eq!(output.status_code, Some(0));
        assert!(output
            .stdout
            .windows(b"normal-helper\n".len())
            .any(|window| { window == b"normal-helper\n" }));
        assert!(output.stdout.len() <= OUTPUT_CAPTURE_LIMIT);
        assert!(output.stderr.len() <= OUTPUT_CAPTURE_LIMIT);
    }

    #[test]
    fn output_overflow_is_distinct_from_timeout() {
        let executable = std::env::current_exe().expect("test executable should be available");
        let error = run_owned_process_for_test(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::overflow_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            Duration::from_secs(2),
        )
        .expect_err("the helper must exceed the retained output bound");

        assert_eq!(error.kind, ProcessFailureKind::StdoutOverflow);
        assert!(matches!(
            error.cleanup,
            ProcessCleanup::Confirmed | ProcessCleanup::Uncertain
        ));
    }

    #[test]
    fn controlled_elapsed_jump_triggers_timeout_and_reaps_owned_child() {
        let executable = std::env::current_exe().expect("test executable should be available");
        let error = run_owned_process_with_deadline_signal(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::timeout_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            ElapsedJumpTimer::default(),
        )
        .expect_err("the simulated elapsed-time jump must trigger the deadline");

        assert_eq!(error.kind, ProcessFailureKind::TimedOut);
        assert!(matches!(
            error.cleanup,
            ProcessCleanup::Confirmed | ProcessCleanup::Uncertain
        ));
    }

    #[test]
    fn child_that_exits_during_observer_delay_cannot_be_relabelled_as_timed_out() {
        let _delay =
            arm_test_process_delay(ProcessOperation::DeviceCopy, Duration::from_millis(100));
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = run_owned_process_for_test_with_operation(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::normal_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            Duration::from_millis(10),
            ProcessOperation::DeviceCopy,
        )
        .expect("a completed exact child must remain completed while observation is delayed");

        assert_eq!(output.status_code, Some(0));
    }

    #[test]
    fn test_process_delay_is_scoped_to_one_operation_and_consumed_once() {
        let _delay =
            arm_test_process_delay(ProcessOperation::DeviceCopy, Duration::from_millis(100));
        let executable = std::env::current_exe().expect("test executable should be available");
        let executable = executable
            .to_str()
            .expect("test executable should be utf-8");
        let args = || {
            vec![
                "--exact".to_string(),
                "owned_process::tests::normal_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ]
        };

        let probe = run_owned_process_for_test_with_operation(
            executable,
            &args(),
            Duration::from_millis(50),
            ProcessOperation::Probe,
        )
        .expect("an identity probe must not consume the copy delay");
        assert_eq!(probe.status_code, Some(0));

        let delayed_observation = run_owned_process_for_test_with_operation(
            executable,
            &args(),
            Duration::from_millis(10),
            ProcessOperation::DeviceCopy,
        )
        .expect("observer delay cannot turn a completed exact child into a timeout");
        assert_eq!(delayed_observation.status_code, Some(0));

        let immediate = run_owned_process_for_test_with_operation(
            executable,
            &args(),
            Duration::from_millis(50),
            ProcessOperation::DeviceCopy,
        )
        .expect("the one-shot delay must not affect a second copy");
        assert_eq!(immediate.status_code, Some(0));
    }

    #[test]
    fn test_process_delay_arm_is_thread_local() {
        let executable = std::env::current_exe().expect("test executable should be available");
        let executable = executable
            .to_str()
            .expect("test executable should be utf-8")
            .to_string();
        let args = vec![
            "--exact".to_string(),
            "owned_process::tests::normal_helper".to_string(),
            "--ignored".to_string(),
            "--nocapture".to_string(),
        ];
        let worker_executable = executable.clone();
        let worker_args = args.clone();
        let worker = std::thread::spawn(move || {
            let _delay =
                arm_test_process_delay(ProcessOperation::DeviceCopy, Duration::from_millis(100));
            run_owned_process_for_test_with_operation(
                &worker_executable,
                &worker_args,
                Duration::from_millis(10),
                ProcessOperation::DeviceCopy,
            )
            .expect("the worker's observer delay must preserve child completion")
            .status_code
        });
        let main_result = run_owned_process_for_test_with_operation(
            &executable,
            &args,
            Duration::from_millis(50),
            ProcessOperation::DeviceCopy,
        )
        .expect("the worker's delay must not cross thread boundaries");
        assert_eq!(main_result.status_code, Some(0));
        assert_eq!(worker.join().expect("worker should finish"), Some(0));
    }

    #[test]
    fn test_process_delay_guard_clears_after_unwind() {
        let _ = std::panic::catch_unwind(|| {
            let _delay =
                arm_test_process_delay(ProcessOperation::DeviceCopy, Duration::from_millis(100));
            panic!("exercise guard unwinding");
        });
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = run_owned_process_for_test_with_operation(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::normal_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            Duration::from_millis(50),
            ProcessOperation::DeviceCopy,
        )
        .expect("an unwound arm must not delay a later invocation");
        assert_eq!(output.status_code, Some(0));
    }

    #[derive(Default)]
    struct ElapsedJumpTimer {
        polled: bool,
    }

    impl Future for ElapsedJumpTimer {
        type Output = ();

        fn poll(
            mut self: Pin<&mut Self>,
            context: &mut std::task::Context<'_>,
        ) -> Poll<Self::Output> {
            if self.polled {
                Poll::Ready(())
            } else {
                self.polled = true;
                context.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    #[test]
    #[ignore]
    fn timeout_helper() {
        std::thread::sleep(Duration::from_secs(60));
    }

    #[test]
    #[ignore]
    fn normal_helper() {
        println!("normal-helper");
    }

    #[test]
    #[ignore]
    fn overflow_helper() {
        use std::io::Write;

        let bytes = vec![b'x'; OUTPUT_CAPTURE_LIMIT + 1];
        std::io::stdout()
            .write_all(&bytes)
            .expect("overflow helper output should be writable");
        std::io::stdout()
            .flush()
            .expect("overflow helper output should flush");
        std::thread::sleep(Duration::from_secs(60));
    }
}
