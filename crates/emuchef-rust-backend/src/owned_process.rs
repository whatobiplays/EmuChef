//! Private ownership boundary for bounded, one-shot host processes.
//!
//! The caller owns the child and both output pipes for the complete operation.
//! A single locally driven future tree observes process exit, output, and the
//! deadline. No executor task, reader thread, or channel producer is created,
//! so dropping a pending local future cannot leave work running after return.

#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Instant;
use std::time::{Duration, SystemTime};

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
    /// A one-shot deadline arm belongs only to the current test thread.  It is
    /// consumed by exactly one matching owned-process invocation and cannot
    /// change production deadlines in non-test builds.
    static TEST_PROCESS_DEADLINE: RefCell<Option<(ProcessOperation, Duration)>> = const { RefCell::new(None) };
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

#[cfg(test)]
/// RAII scope for a test-only deadline override.
///
/// Dropping the guard clears an unconsumed arm, so a qualification or unit test
/// cannot leak a shorter deadline into a later owned-process invocation.
pub(crate) struct TestProcessDeadlineGuard;

#[cfg(test)]
impl Drop for TestProcessDeadlineGuard {
    fn drop(&mut self) {
        TEST_PROCESS_DEADLINE.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
/// Arm one matching, thread-local process deadline for a test invocation.
///
/// The production `ProcessOperation::deadline` table remains authoritative in
/// non-test builds. A nonmatching operation leaves this arm available for the
/// intended operation; the matching invocation consumes it exactly once.
pub(crate) fn arm_test_process_deadline(
    operation: ProcessOperation,
    deadline: Duration,
) -> TestProcessDeadlineGuard {
    TEST_PROCESS_DEADLINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(
            slot.replace((operation, deadline)).is_none(),
            "a process-deadline arm must be scoped and cannot be replaced"
        );
    });
    TestProcessDeadlineGuard
}

#[cfg(test)]
fn take_test_process_deadline(operation: ProcessOperation) -> Option<Duration> {
    TEST_PROCESS_DEADLINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        match slot.take() {
            Some((armed_operation, deadline)) if armed_operation == operation => Some(deadline),
            other => {
                *slot = other;
                None
            }
        }
    })
}

#[cfg(not(test))]
fn take_test_process_deadline(_operation: ProcessOperation) -> Option<Duration> {
    None
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

    pub(crate) const fn is_mutating(self) -> bool {
        !matches!(self, Self::Probe | Self::Predicate | Self::RootPreflight)
    }
}

static NEXT_OBSERVED_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct OwnedProcessOperationId(u64);

impl OwnedProcessOperationId {
    #[cfg(test)]
    pub(crate) const fn as_u64(self) -> u64 {
        self.0
    }

    #[cfg(test)]
    pub(crate) const fn from_raw_for_test(value: u64) -> Self {
        Self(value)
    }
}

/// The exact monotonic start/deadline pair that drives the owned-process
/// timer. The timer and every qualification clock sample derive from this one
/// captured pair; no second clock authority is created.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeadlineClockBasis {
    start: Instant,
    /// Wall observation captured adjacent to construction of the authoritative
    /// monotonic start. It is correlation/evidence only: `Instant` remains the
    /// sole deadline authority, and `SystemTime`/`Instant` are not claimed to
    /// have been read literally simultaneously.
    start_wall: SystemTime,
    deadline_at: Instant,
    deadline: Duration,
}

/// A sanitized, run-local monotonic clock sample derived from the exact
/// owned-process deadline basis. Nanoseconds are relative to the basis start
/// and never expose a raw `Instant`, PID, or command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeadlineClockSample {
    deadline_clock_ns: u64,
    remaining_ns: u64,
    deadline_reached: bool,
}

/// Build the explicit deadline basis for one owned-process invocation.
///
/// This is semantically equivalent to `async_io::Timer::after(deadline)`,
/// which itself computes `Instant::now().checked_add(deadline)` and falls
/// back to a never-firing timer on overflow.
fn deadline_basis(
    start: Instant,
    start_wall: SystemTime,
    deadline: Duration,
) -> Option<DeadlineClockBasis> {
    start
        .checked_add(deadline)
        .map(|deadline_at| DeadlineClockBasis {
            start,
            start_wall,
            deadline_at,
            deadline,
        })
}

impl DeadlineClockBasis {
    fn sample(&self, now: Instant) -> DeadlineClockSample {
        let elapsed = now.duration_since(self.start);
        let remaining = self.deadline_at.saturating_duration_since(now);
        DeadlineClockSample {
            deadline_clock_ns: u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
            remaining_ns: u64::try_from(remaining.as_nanos()).unwrap_or(u64::MAX),
            deadline_reached: now >= self.deadline_at,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OwnedProcessLivenessSample {
    pub(crate) operation_id: OwnedProcessOperationId,
    pub(crate) at: SystemTime,
    pub(crate) alive: Option<bool>,
    pub(crate) terminal_reported: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OwnedProcessLifecycleEvent {
    Spawned {
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        at: SystemTime,
    },
    MutationStarted {
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        at: SystemTime,
    },
    LivenessSampled {
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        at: SystemTime,
        alive: Option<bool>,
        terminal_reported: bool,
    },
    DeadlineReached {
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        deadline: Duration,
        at: SystemTime,
    },
    DeadlineClockStarted {
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        deadline_clock_start_ns: u64,
        deadline_ns: u64,
        at: SystemTime,
    },
    DeadlineClockSampled {
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        at: SystemTime,
        deadline_clock_ns: u64,
        remaining_ns: u64,
        deadline_reached: bool,
        owner_reported: bool,
    },
    Terminal {
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        at: SystemTime,
    },
}

#[cfg(test)]
impl OwnedProcessLifecycleEvent {
    pub(crate) const fn operation_id(&self) -> OwnedProcessOperationId {
        match self {
            Self::Spawned { operation_id, .. }
            | Self::MutationStarted { operation_id, .. }
            | Self::LivenessSampled { operation_id, .. }
            | Self::DeadlineReached { operation_id, .. }
            | Self::DeadlineClockStarted { operation_id, .. }
            | Self::DeadlineClockSampled { operation_id, .. }
            | Self::Terminal { operation_id, .. } => *operation_id,
        }
    }

    pub(crate) const fn operation(&self) -> ProcessOperation {
        match self {
            Self::Spawned { operation, .. }
            | Self::MutationStarted { operation, .. }
            | Self::LivenessSampled { operation, .. }
            | Self::DeadlineReached { operation, .. }
            | Self::DeadlineClockStarted { operation, .. }
            | Self::DeadlineClockSampled { operation, .. }
            | Self::Terminal { operation, .. } => *operation,
        }
    }

    pub(crate) const fn at(&self) -> SystemTime {
        match self {
            Self::Spawned { at, .. }
            | Self::MutationStarted { at, .. }
            | Self::LivenessSampled { at, .. }
            | Self::DeadlineReached { at, .. }
            | Self::DeadlineClockStarted { at, .. }
            | Self::DeadlineClockSampled { at, .. }
            | Self::Terminal { at, .. } => *at,
        }
    }
}

#[derive(Debug, Default)]
struct OwnedProcessObservationState {
    events: Vec<OwnedProcessLifecycleEvent>,
    liveness_request: Option<OwnedProcessOperationId>,
    owner_waker: Option<Waker>,
    deadline_basis: HashMap<OwnedProcessOperationId, DeadlineClockBasis>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OwnedProcessObservationHandle {
    state: Arc<Mutex<OwnedProcessObservationState>>,
}

impl OwnedProcessObservationHandle {
    #[cfg(test)]
    pub(crate) fn events(&self) -> Vec<OwnedProcessLifecycleEvent> {
        self.state
            .lock()
            .map(|state| state.events.clone())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn wait_for_mutation(
        &self,
        operation: ProcessOperation,
        timeout: Duration,
    ) -> Result<OwnedProcessOperationId, String> {
        let started = Instant::now();
        loop {
            let result = self
                .state
                .lock()
                .map_err(|_| "owned-process observation state is unavailable".to_string())?
                .events
                .iter()
                .find_map(|event| match event {
                    OwnedProcessLifecycleEvent::MutationStarted {
                        operation_id,
                        operation: observed,
                        ..
                    } if *observed == operation => Some(*operation_id),
                    _ => None,
                });
            if let Some(operation_id) = result {
                return Ok(operation_id);
            }
            if started.elapsed() >= timeout {
                return Err("owned-process mutation observation timed out".to_string());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(test)]
    pub(crate) fn request_liveness_sample(
        &self,
        operation_id: OwnedProcessOperationId,
    ) -> Result<(), String> {
        let waker = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "owned-process observation state is unavailable".to_string())?;
            let terminal_operation = state.events.iter().find_map(|event| match event {
                OwnedProcessLifecycleEvent::Terminal {
                    operation_id: observed,
                    operation,
                    ..
                } if *observed == operation_id => Some(*operation),
                _ => None,
            });
            if let Some(operation) = terminal_operation {
                state
                    .events
                    .push(OwnedProcessLifecycleEvent::LivenessSampled {
                        operation_id,
                        operation,
                        at: SystemTime::now(),
                        alive: Some(false),
                        terminal_reported: true,
                    });
                None
            } else {
                if state.liveness_request.replace(operation_id).is_some() {
                    return Err("owned-process liveness request is already pending".to_string());
                }
                state.owner_waker.clone()
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn wait_for_liveness(
        &self,
        operation_id: OwnedProcessOperationId,
        timeout: Duration,
    ) -> Result<OwnedProcessLivenessSample, String> {
        let started = Instant::now();
        loop {
            let sample = self
                .state
                .lock()
                .map_err(|_| "owned-process observation state is unavailable".to_string())?
                .events
                .iter()
                .find_map(|event| match event {
                    OwnedProcessLifecycleEvent::LivenessSampled {
                        operation_id: observed,
                        at,
                        alive,
                        terminal_reported,
                        ..
                    } if *observed == operation_id => Some(OwnedProcessLivenessSample {
                        operation_id,
                        at: *at,
                        alive: *alive,
                        terminal_reported: *terminal_reported,
                    }),
                    _ => None,
                });
            if let Some(sample) = sample {
                return Ok(sample);
            }
            if started.elapsed() >= timeout {
                return Err("owned-process liveness observation timed out".to_string());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn record(&self, event: OwnedProcessLifecycleEvent) {
        if let Ok(mut state) = self.state.lock() {
            state.events.push(event);
        }
    }

    /// Install the exact deadline basis for one owned-process invocation.
    /// The owner calls this once, at the same instant the timer is
    /// constructed, so the timer and every later clock sample share the same
    /// monotonic start/deadline pair.
    fn install_deadline_basis(
        &self,
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        basis: Option<DeadlineClockBasis>,
    ) {
        let Some(basis) = basis else {
            return;
        };
        if let Ok(mut state) = self.state.lock() {
            if state.deadline_basis.contains_key(&operation_id) {
                return;
            }
            state.deadline_basis.insert(operation_id, basis);
            state
                .events
                .push(OwnedProcessLifecycleEvent::DeadlineClockStarted {
                    operation_id,
                    operation,
                    deadline_clock_start_ns: 0,
                    deadline_ns: u64::try_from(basis.deadline.as_nanos()).unwrap_or(u64::MAX),
                    at: basis.start_wall,
                });
        }
    }

    /// Derive a sanitized sample from the exact retained deadline basis.
    ///
    /// The sample is computed from `Instant::now()` against the shared basis,
    /// so it remains available even after the owned-process owner has observed
    /// terminal state (for example immediately after host wake).
    ///
    /// The operation is atomic and fail closed: one successful state lock
    /// resolves the exact basis, captures the monotonic sample and its
    /// corresponding wall timestamp at the same sampling point, appends
    /// exactly one `DeadlineClockSampled` event, and returns `Ok(())` only
    /// after that event is present. A poisoned or unavailable observation
    /// state returns `Err` and never reports sampling success.
    #[cfg(test)]
    pub(crate) fn request_deadline_clock_sample(
        &self,
        operation_id: OwnedProcessOperationId,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "owned-process observation state is unavailable".to_string())?;
        let basis = state
            .deadline_basis
            .get(&operation_id)
            .copied()
            .ok_or_else(|| "owned-process deadline-clock basis is unavailable".to_string())?;
        let operation = state.events.iter().find_map(|event| match event {
            OwnedProcessLifecycleEvent::Spawned {
                operation_id: observed,
                operation,
                ..
            } if *observed == operation_id => Some(*operation),
            _ => None,
        });
        let operation = operation
            .ok_or_else(|| "owned-process deadline-clock operation is unavailable".to_string())?;
        let sample = basis.sample(Instant::now());
        let at = SystemTime::now();
        state
            .events
            .push(OwnedProcessLifecycleEvent::DeadlineClockSampled {
                operation_id,
                operation,
                at,
                deadline_clock_ns: sample.deadline_clock_ns,
                remaining_ns: sample.remaining_ns,
                deadline_reached: sample.deadline_reached,
                owner_reported: false,
            });
        let published = matches!(
            state.events.last(),
            Some(OwnedProcessLifecycleEvent::DeadlineClockSampled {
                operation_id: observed,
                owner_reported: false,
                ..
            }) if *observed == operation_id
        );
        if published {
            Ok(())
        } else {
            Err("owned-process deadline-clock sample was not published".to_string())
        }
    }

    fn record_terminal(&self, operation_id: OwnedProcessOperationId, operation: ProcessOperation) {
        if let Ok(mut state) = self.state.lock() {
            let at = SystemTime::now();
            state.events.push(OwnedProcessLifecycleEvent::Terminal {
                operation_id,
                operation,
                at,
            });
            if let Some(basis) = state.deadline_basis.get(&operation_id).copied() {
                let sample = basis.sample(Instant::now());
                state
                    .events
                    .push(OwnedProcessLifecycleEvent::DeadlineClockSampled {
                        operation_id,
                        operation,
                        at,
                        deadline_clock_ns: sample.deadline_clock_ns,
                        remaining_ns: sample.remaining_ns,
                        deadline_reached: sample.deadline_reached,
                        owner_reported: true,
                    });
            }
            if state.liveness_request == Some(operation_id) {
                state.liveness_request = None;
                state
                    .events
                    .push(OwnedProcessLifecycleEvent::LivenessSampled {
                        operation_id,
                        operation,
                        at,
                        alive: Some(false),
                        terminal_reported: true,
                    });
            }
        }
    }

    fn register_owner_waker(&self, waker: &Waker) {
        if let Ok(mut state) = self.state.lock() {
            state.owner_waker = Some(waker.clone());
        }
    }

    fn take_liveness_request(&self, operation_id: OwnedProcessOperationId) -> bool {
        self.state
            .lock()
            .map(|mut state| {
                if state.liveness_request == Some(operation_id) {
                    state.liveness_request = None;
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false)
    }

    fn terminal_reported(&self, operation_id: OwnedProcessOperationId) -> bool {
        self.state
            .lock()
            .map(|state| {
                state.events.iter().any(|event| {
                    matches!(
                        event,
                        OwnedProcessLifecycleEvent::Terminal {
                            operation_id: observed,
                            ..
                        } if *observed == operation_id
                    )
                })
            })
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug)]
struct OwnedProcessObservationContext {
    handle: OwnedProcessObservationHandle,
    operation_id: OwnedProcessOperationId,
    operation: ProcessOperation,
}

impl OwnedProcessObservationContext {
    fn begin(handle: OwnedProcessObservationHandle, operation: ProcessOperation) -> Self {
        let operation_id = OwnedProcessOperationId(
            NEXT_OBSERVED_OPERATION_ID.fetch_add(1, AtomicOrdering::Relaxed),
        );
        let context = Self {
            handle,
            operation_id,
            operation,
        };
        context.handle.record(OwnedProcessLifecycleEvent::Spawned {
            operation_id,
            operation,
            at: SystemTime::now(),
        });
        context
    }

    fn mutation_started(&self) {
        if self.operation.is_mutating() {
            self.handle
                .record(OwnedProcessLifecycleEvent::MutationStarted {
                    operation_id: self.operation_id,
                    operation: self.operation,
                    at: SystemTime::now(),
                });
        }
    }

    fn sample_liveness(&self, alive: Option<bool>) {
        self.handle
            .record(OwnedProcessLifecycleEvent::LivenessSampled {
                operation_id: self.operation_id,
                operation: self.operation,
                at: SystemTime::now(),
                alive,
                terminal_reported: self.handle.terminal_reported(self.operation_id),
            });
    }

    fn deadline_reached(&self, deadline: Duration) {
        self.handle
            .record(OwnedProcessLifecycleEvent::DeadlineReached {
                operation_id: self.operation_id,
                operation: self.operation,
                deadline,
                at: SystemTime::now(),
            });
    }

    fn install_deadline_basis(&self, basis: Option<DeadlineClockBasis>) {
        self.handle
            .install_deadline_basis(self.operation_id, self.operation, basis);
    }

    fn terminal(&self) {
        self.handle
            .record_terminal(self.operation_id, self.operation);
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
    observation: Option<OwnedProcessObservationContext>,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    // Capture the wall observation adjacent to the authoritative monotonic
    // start so DeadlineClockStarted.at correlates with actual timer-basis
    // construction rather than later observer installation.
    let start_wall = SystemTime::now();
    let deadline_clock_start = Instant::now();
    let deadline_basis = deadline_basis(deadline_clock_start, start_wall, deadline);
    let timer = deadline_basis.map_or_else(Timer::never, |basis| Timer::at(basis.deadline_at));
    if let Some(observation) = observation.as_ref() {
        observation.install_deadline_basis(deadline_basis);
    }
    run_child_with_deadline_signal(
        child,
        stdout,
        stderr,
        timer,
        Some(deadline),
        process_delay,
        observation,
    )
    .await
}

async fn run_child_with_deadline_signal<D: Future>(
    mut child: Child,
    stdout: ChildStdout,
    stderr: ChildStderr,
    deadline_signal: D,
    deadline: Option<Duration>,
    process_delay: Option<Duration>,
    observation: Option<OwnedProcessObservationContext>,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    let mut status_future = Box::pin(child.status());
    let mut output_future = Box::pin(read_streams(stdout, stderr));
    let mut timer = Box::pin(deadline_signal);
    let mut process_delay = process_delay.map(|delay| Box::pin(Timer::after(delay)));
    let mut process_delay_ready = process_delay.is_none();
    let mut sampled_status = None;

    let event = poll_fn(|context| {
        if !process_delay_ready {
            if let Some(delay) = process_delay.as_mut() {
                if delay.as_mut().poll(context).is_ready() {
                    process_delay_ready = true;
                }
            }
        }
        if let Some(observation) = observation.as_ref() {
            observation.handle.register_owner_waker(context.waker());
            if observation
                .handle
                .take_liveness_request(observation.operation_id)
            {
                match status_future.as_mut().poll(context) {
                    Poll::Ready(result) => {
                        observation.sample_liveness(Some(false));
                        sampled_status = Some(
                            result
                                .map(|status| status.code().unwrap_or(-1))
                                .map_err(|_| ()),
                        );
                    }
                    Poll::Pending => observation.sample_liveness(Some(true)),
                }
            }
        }
        // Output is checked first so an already-observed overflow/read failure
        // retains precedence over a status or deadline observed in the same poll.
        if let Poll::Ready(result) = output_future.as_mut().poll(context) {
            return Poll::Ready(ProcessEvent::Output(result));
        }
        if let Some(result) = sampled_status.take() {
            return Poll::Ready(ProcessEvent::Exited(result));
        }
        if let Poll::Ready(result) = status_future.as_mut().poll(context) {
            return Poll::Ready(ProcessEvent::Exited(
                result
                    .map(|status| status.code().unwrap_or(-1))
                    .map_err(|_| ()),
            ));
        }
        if timer.as_mut().poll(context).is_ready() {
            if let (Some(deadline), Some(observation)) = (deadline, observation.as_ref()) {
                observation.deadline_reached(deadline);
            }
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
                    let settled_status = if let Some(result) = sampled_status.take() {
                        drop(status_future);
                        SettledStatus::Complete(result)
                    } else {
                        settle_status(status_future).await
                    };
                    match settled_status {
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
    run_owned_process_with_deadline_and_observer(program, args, deadline, operation, None)
}

fn run_owned_process_with_deadline_and_observer(
    program: &str,
    args: &[String],
    deadline: Duration,
    operation: ProcessOperation,
    observer: Option<OwnedProcessObservationHandle>,
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
    let observation =
        observer.map(|observer| OwnedProcessObservationContext::begin(observer, operation));
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let error = cleanup_child(&mut child, ProcessFailureKind::Spawn);
            if let Some(observation) = observation.as_ref() {
                observation.terminal();
            }
            return Err(error);
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let error = cleanup_child(&mut child, ProcessFailureKind::Spawn);
            if let Some(observation) = observation.as_ref() {
                observation.terminal();
            }
            return Err(error);
        }
    };
    if let Some(observation) = observation.as_ref() {
        observation.mutation_started();
    }
    let result = block_on(run_child(
        child,
        stdout,
        stderr,
        deadline,
        take_test_process_delay(operation),
        observation.clone(),
    ));
    if let Some(observation) = observation.as_ref() {
        observation.terminal();
    }
    result
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
        None,
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
    let deadline = take_test_process_deadline(operation).unwrap_or_else(|| operation.deadline());
    run_owned_process_with_deadline(program, args, deadline, operation)
}

#[cfg(test)]
pub(crate) fn run_owned_process_observed(
    program: &str,
    args: &[String],
    operation: ProcessOperation,
    observer: OwnedProcessObservationHandle,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    let deadline = take_test_process_deadline(operation).unwrap_or_else(|| operation.deadline());
    run_owned_process_with_deadline_and_observer(program, args, deadline, operation, Some(observer))
}

#[cfg(test)]
fn run_owned_process_observed_for_test(
    program: &str,
    args: &[String],
    deadline: Duration,
    operation: ProcessOperation,
    observer: OwnedProcessObservationHandle,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    run_owned_process_with_deadline_and_observer(program, args, deadline, operation, Some(observer))
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
    fn scoped_deadline_override_is_consumed_once_for_matching_operation() {
        let _deadline =
            arm_test_process_deadline(ProcessOperation::DeviceCopy, Duration::from_millis(10));
        let executable = std::env::current_exe().expect("test executable should be available");
        let args = vec![
            "--exact".to_string(),
            "owned_process::tests::short_helper".to_string(),
            "--ignored".to_string(),
            "--nocapture".to_string(),
        ];

        let first = run_owned_process(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &args,
            ProcessOperation::DeviceCopy,
        )
        .expect_err("the matching scoped deadline should time out the first child");
        assert_eq!(first.kind, ProcessFailureKind::TimedOut);

        let second = run_owned_process(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &args,
            ProcessOperation::DeviceCopy,
        )
        .expect("the consumed override must not affect a later copy");
        assert_eq!(second.status_code, Some(0));
    }

    #[test]
    fn scoped_deadline_override_does_not_affect_nonmatching_operation() {
        let _deadline =
            arm_test_process_deadline(ProcessOperation::DeviceCopy, Duration::from_millis(10));
        let executable = std::env::current_exe().expect("test executable should be available");
        let args = vec![
            "--exact".to_string(),
            "owned_process::tests::short_helper".to_string(),
            "--ignored".to_string(),
            "--nocapture".to_string(),
        ];

        let probe = run_owned_process(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &args,
            ProcessOperation::Probe,
        )
        .expect("a nonmatching operation must retain the armed override");
        assert_eq!(probe.status_code, Some(0));

        let copy = run_owned_process(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &args,
            ProcessOperation::DeviceCopy,
        )
        .expect_err("the matching operation should consume the retained override");
        assert_eq!(copy.kind, ProcessFailureKind::TimedOut);
    }

    #[test]
    fn dropping_unused_deadline_override_removes_it() {
        {
            let _deadline =
                arm_test_process_deadline(ProcessOperation::DeviceCopy, Duration::from_millis(10));
        }
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = run_owned_process(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::short_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            ProcessOperation::DeviceCopy,
        )
        .expect("dropping an unused arm must restore the production deadline");
        assert_eq!(output.status_code, Some(0));
    }

    #[test]
    fn deadline_override_is_thread_local() {
        let executable = std::env::current_exe().expect("test executable should be available");
        let args = vec![
            "--exact".to_string(),
            "owned_process::tests::short_helper".to_string(),
            "--ignored".to_string(),
            "--nocapture".to_string(),
        ];
        let worker_executable = executable.clone();
        let worker_args = args.clone();
        let worker = std::thread::spawn(move || {
            let _deadline =
                arm_test_process_deadline(ProcessOperation::DeviceCopy, Duration::from_millis(10));
            run_owned_process(
                worker_executable
                    .to_str()
                    .expect("worker executable should be utf-8"),
                &worker_args,
                ProcessOperation::DeviceCopy,
            )
            .expect_err("the worker's scoped deadline should apply on its thread")
            .kind
        });
        let main_result = run_owned_process(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &args,
            ProcessOperation::DeviceCopy,
        )
        .expect("the worker's scoped deadline must not cross thread boundaries");
        assert_eq!(main_result.status_code, Some(0));
        assert_eq!(
            worker.join().expect("worker should finish"),
            ProcessFailureKind::TimedOut
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

    #[test]
    fn observed_child_preserves_one_identity_and_event_order() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let worker_observer = observer.clone();
        let worker = std::thread::spawn(move || {
            run_owned_process_observed(
                executable
                    .to_str()
                    .expect("test executable should be utf-8"),
                &[
                    "--exact".to_string(),
                    "owned_process::tests::observed_helper".to_string(),
                    "--ignored".to_string(),
                    "--nocapture".to_string(),
                ],
                ProcessOperation::DeviceCopy,
                worker_observer,
            )
        });

        let operation_id = observer
            .wait_for_mutation(ProcessOperation::DeviceCopy, Duration::from_secs(2))
            .expect("the observed mutation should start");
        observer
            .request_liveness_sample(operation_id)
            .expect("the exact child liveness sample should be requested");
        let sample = observer
            .wait_for_liveness(operation_id, Duration::from_secs(2))
            .expect("the exact child liveness sample should complete");
        assert_eq!(sample.operation_id, operation_id);
        assert_eq!(sample.alive, Some(true));
        assert!(!sample.terminal_reported);

        let result = worker
            .join()
            .expect("observed helper thread should finish")
            .expect("observed helper should complete");
        assert_eq!(result.status_code, Some(0));

        let events = observer.events();
        let matching = events
            .iter()
            .filter(|event| event.operation_id() == operation_id)
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 6);
        assert!(matches!(
            matching[0],
            OwnedProcessLifecycleEvent::Spawned { .. }
        ));
        assert!(matches!(
            matching[1],
            OwnedProcessLifecycleEvent::MutationStarted { .. }
        ));
        assert!(matches!(
            matching[2],
            OwnedProcessLifecycleEvent::DeadlineClockStarted { .. }
        ));
        assert!(matches!(
            matching[3],
            OwnedProcessLifecycleEvent::LivenessSampled {
                alive: Some(true),
                ..
            }
        ));
        assert!(matches!(
            matching[4],
            OwnedProcessLifecycleEvent::Terminal { .. }
        ));
        assert!(matches!(
            matching[5],
            OwnedProcessLifecycleEvent::DeadlineClockSampled {
                owner_reported: true,
                ..
            }
        ));
        assert!(matching.windows(2).all(|pair| pair[0].at() <= pair[1].at()));
    }

    #[test]
    fn completed_child_cannot_be_sampled_as_alive() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = run_owned_process_observed_for_test(
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
            ProcessOperation::DeviceCopy,
            observer.clone(),
        )
        .expect("the observed helper should complete");
        assert_eq!(output.status_code, Some(0));

        let operation_id = observer
            .wait_for_mutation(ProcessOperation::DeviceCopy, Duration::from_secs(1))
            .expect("the completed mutation should remain observable");
        observer
            .request_liveness_sample(operation_id)
            .expect("the terminal sample should be recorded");
        let sample = observer
            .wait_for_liveness(operation_id, Duration::from_secs(1))
            .expect("the terminal sample should be available");
        assert_eq!(sample.alive, Some(false));
        assert!(sample.terminal_reported);
        assert!(!observer
            .events()
            .iter()
            .any(|event| { matches!(event, OwnedProcessLifecycleEvent::DeadlineReached { .. }) }));
    }

    #[test]
    fn observed_timeout_preserves_cleanup_and_emits_terminal() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let _deadline =
            arm_test_process_deadline(ProcessOperation::DeviceCopy, Duration::from_millis(100));
        let error = run_owned_process_observed(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::timeout_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            ProcessOperation::DeviceCopy,
            observer.clone(),
        )
        .expect_err("the observed helper must exceed the test deadline");

        assert_eq!(error.kind, ProcessFailureKind::TimedOut);
        assert!(matches!(
            error.cleanup,
            ProcessCleanup::Confirmed | ProcessCleanup::Uncertain
        ));
        assert!(observer
            .events()
            .iter()
            .any(|event| matches!(event, OwnedProcessLifecycleEvent::Terminal { .. })));
        let events = observer.events();
        let timeout_index = events
            .iter()
            .position(|event| matches!(event, OwnedProcessLifecycleEvent::DeadlineReached { .. }))
            .expect("the actual timer transition should be observed");
        let terminal_index = events
            .iter()
            .position(|event| matches!(event, OwnedProcessLifecycleEvent::Terminal { .. }))
            .expect("terminal cleanup should be observed");
        assert!(timeout_index < terminal_index);
        assert_eq!(
            events[timeout_index].operation_id(),
            events[terminal_index].operation_id()
        );
    }

    #[test]
    fn observation_debug_output_contains_no_command_or_pid_data() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = run_owned_process_observed_for_test(
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
            ProcessOperation::DeviceCopy,
            observer.clone(),
        )
        .expect("the observed helper should complete");
        assert_eq!(output.status_code, Some(0));

        let debug = format!("{:?}", observer.events()).to_ascii_lowercase();
        assert!(!debug.contains("--exact"));
        assert!(!debug.contains("normal_helper"));
        assert!(!debug.contains("pid"));
    }

    #[test]
    fn deadline_basis_captures_one_exact_start_deadline_pair_and_never_fires_on_overflow() {
        let start = Instant::now();
        let start_wall = SystemTime::now();
        let basis = deadline_basis(start, start_wall, Duration::from_secs(7))
            .expect("a finite deadline must produce an exact basis");
        assert_eq!(basis.start, start);
        assert_eq!(basis.start_wall, start_wall);
        assert_eq!(basis.deadline, Duration::from_secs(7));
        assert_eq!(basis.deadline_at, start + Duration::from_secs(7));
        assert!(
            deadline_basis(Instant::now(), SystemTime::now(), Duration::MAX).is_none(),
            "an overflowing deadline must preserve the never-fire timer fallback"
        );
    }

    #[test]
    fn observed_process_records_one_deadline_clock_basis_and_consistent_samples() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let worker_observer = observer.clone();
        let worker = std::thread::spawn(move || {
            run_owned_process_observed(
                executable
                    .to_str()
                    .expect("test executable should be utf-8"),
                &[
                    "--exact".to_string(),
                    "owned_process::tests::observed_helper".to_string(),
                    "--ignored".to_string(),
                    "--nocapture".to_string(),
                ],
                ProcessOperation::DeviceCopy,
                worker_observer,
            )
        });

        let operation_id = observer
            .wait_for_mutation(ProcessOperation::DeviceCopy, Duration::from_secs(2))
            .expect("the observed mutation should start");
        observer
            .request_liveness_sample(operation_id)
            .expect("the exact child liveness sample should be requested");
        observer
            .wait_for_liveness(operation_id, Duration::from_secs(2))
            .expect("the exact child liveness sample should complete");
        observer
            .request_deadline_clock_sample(operation_id)
            .expect("the exact deadline-clock sample should be requested");

        worker
            .join()
            .expect("observed helper thread should finish")
            .expect("observed helper should complete");

        let events = observer.events();
        let starts = events
            .iter()
            .filter_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockStarted {
                    operation_id: observed,
                    operation,
                    deadline_clock_start_ns,
                    deadline_ns,
                    ..
                } if *observed == operation_id => {
                    Some((*operation, *deadline_clock_start_ns, *deadline_ns))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(starts.len(), 1);
        let (operation, start_ns, deadline_ns) = starts[0];
        assert_eq!(operation, ProcessOperation::DeviceCopy);
        assert_eq!(start_ns, 0, "the run-local origin must be zero");
        assert_eq!(
            deadline_ns,
            u64::try_from(ProcessOperation::DeviceCopy.deadline().as_nanos())
                .expect("the production deadline must fit u64 nanoseconds")
        );

        let samples = events
            .iter()
            .filter_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockSampled {
                    operation_id: observed,
                    deadline_clock_ns,
                    remaining_ns,
                    deadline_reached,
                    owner_reported,
                    ..
                } if *observed == operation_id => Some((
                    *deadline_clock_ns,
                    *remaining_ns,
                    *deadline_reached,
                    *owner_reported,
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            samples.len() >= 2,
            "a requested and an owner terminal sample are required"
        );
        let requested = samples
            .iter()
            .find(|(_, _, _, owner_reported)| !*owner_reported)
            .expect("a watcher-requested sample must be recorded");
        let terminal = samples
            .iter()
            .find(|(_, _, _, owner_reported)| *owner_reported)
            .expect("an owner terminal sample must be recorded");
        assert!(
            requested.0 <= terminal.0,
            "both samples must advance on the same exact deadline basis"
        );
        assert!(
            requested.1 >= terminal.1,
            "remaining budget must be non-increasing while the observed clock advances"
        );
        assert!(
            !terminal.2,
            "a completed child must not claim the deadline was reached"
        );
    }

    #[test]
    fn deadline_clock_sample_survives_owner_terminal_for_the_post_wake_race() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let _deadline =
            arm_test_process_deadline(ProcessOperation::DeviceCopy, Duration::from_millis(100));
        let error = run_owned_process_observed(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::timeout_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            ProcessOperation::DeviceCopy,
            observer.clone(),
        )
        .expect_err("the observed helper must exceed the test deadline");
        assert_eq!(error.kind, ProcessFailureKind::TimedOut);

        let operation_id = observer
            .events()
            .iter()
            .find_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockStarted { operation_id, .. } => {
                    Some(*operation_id)
                }
                _ => None,
            })
            .expect("a deadline-clock basis should have been installed");
        observer
            .request_deadline_clock_sample(operation_id)
            .expect("a post-terminal sample must still derive from the retained exact basis");

        let events = observer.events();
        let terminal_index = events
            .iter()
            .position(|event| matches!(event, OwnedProcessLifecycleEvent::Terminal { .. }))
            .expect("the owner terminal event must be recorded");
        let sample_index = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    OwnedProcessLifecycleEvent::DeadlineClockSampled {
                        owner_reported: false,
                        ..
                    }
                )
            })
            .expect("the post-terminal requested sample must be recorded");
        assert!(
            terminal_index < sample_index,
            "the retained basis must allow sampling after the owner terminal"
        );
        let (clock_ns, reached) = match &events[sample_index] {
            OwnedProcessLifecycleEvent::DeadlineClockSampled {
                deadline_clock_ns,
                deadline_reached,
                ..
            } => (*deadline_clock_ns, *deadline_reached),
            _ => unreachable!("sample_index must select a sampled event"),
        };
        assert!(
            clock_ns >= 100_000_000,
            "the post-terminal sample must be at or past the deadline"
        );
        assert!(
            reached,
            "the post-terminal sample must observe the deadline as reached"
        );
    }

    #[test]
    fn observed_completion_at_the_deadline_boundary_keeps_completion_without_deadline_reached() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = run_owned_process_observed_for_test(
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
            observer.clone(),
        )
        .expect("a completed exact child must remain completed at the deadline boundary");
        assert_eq!(output.status_code, Some(0));
        assert!(
            !observer
                .events()
                .iter()
                .any(|event| matches!(event, OwnedProcessLifecycleEvent::DeadlineReached { .. })),
            "completion observed by the owner before the timer must win without a deadline event"
        );
        assert!(
            observer.events().iter().any(|event| {
                matches!(
                    event,
                    OwnedProcessLifecycleEvent::DeadlineClockSampled {
                        owner_reported: true,
                        ..
                    }
                )
            }),
            "the owner terminal deadline-clock sample must be recorded"
        );
    }

    #[test]
    fn observed_timeout_records_owner_deadline_and_terminal_clock_sample() {
        let observer = OwnedProcessObservationHandle::default();
        let executable = std::env::current_exe().expect("test executable should be available");
        let _deadline =
            arm_test_process_deadline(ProcessOperation::DeviceCopy, Duration::from_millis(100));
        let error = run_owned_process_observed(
            executable
                .to_str()
                .expect("test executable should be utf-8"),
            &[
                "--exact".to_string(),
                "owned_process::tests::timeout_helper".to_string(),
                "--ignored".to_string(),
                "--nocapture".to_string(),
            ],
            ProcessOperation::DeviceCopy,
            observer.clone(),
        )
        .expect_err("the observed helper must exceed the test deadline");
        assert_eq!(error.kind, ProcessFailureKind::TimedOut);

        let events = observer.events();
        let deadline_index = events
            .iter()
            .position(|event| matches!(event, OwnedProcessLifecycleEvent::DeadlineReached { .. }))
            .expect("the owner-recorded deadline event must be present");
        let terminal_sample_index = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    OwnedProcessLifecycleEvent::DeadlineClockSampled {
                        owner_reported: true,
                        ..
                    }
                )
            })
            .expect("the owner terminal deadline-clock sample must be present");
        let terminal_index = events
            .iter()
            .position(|event| matches!(event, OwnedProcessLifecycleEvent::Terminal { .. }))
            .expect("the terminal event must be present");
        assert!(
            deadline_index < terminal_index,
            "DeadlineReached must precede terminal cleanup"
        );
        let (clock_ns, reached) = match &events[terminal_sample_index] {
            OwnedProcessLifecycleEvent::DeadlineClockSampled {
                deadline_clock_ns,
                deadline_reached,
                ..
            } => (*deadline_clock_ns, *deadline_reached),
            _ => unreachable!("terminal_sample_index must select a sampled event"),
        };
        assert!(
            clock_ns >= 100_000_000,
            "the timeout terminal sample must be at or past the deadline"
        );
        assert!(
            reached,
            "the timeout terminal sample must observe the deadline as reached"
        );
    }

    #[test]
    fn deadline_clock_sample_without_an_exact_basis_is_rejected() {
        let observer = OwnedProcessObservationHandle::default();
        let error = observer
            .request_deadline_clock_sample(OwnedProcessOperationId::from_raw_for_test(1))
            .expect_err("a sample without the exact shared deadline basis must be rejected");
        assert!(
            error.contains("basis"),
            "the rejection must name the missing exact basis: {error}"
        );
    }

    #[test]
    fn deadline_clock_sample_request_publishes_exactly_one_event() {
        let observer = OwnedProcessObservationHandle::default();
        let operation_id = OwnedProcessOperationId::from_raw_for_test(510);
        observer.record(OwnedProcessLifecycleEvent::Spawned {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            at: SystemTime::now(),
        });
        let basis = deadline_basis(Instant::now(), SystemTime::now(), Duration::from_secs(120))
            .expect("a finite deadline must produce a basis");
        observer.install_deadline_basis(operation_id, ProcessOperation::DeviceCopy, Some(basis));
        observer
            .request_deadline_clock_sample(operation_id)
            .expect("a sample with an exact basis must publish");
        let samples = observer
            .events()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    OwnedProcessLifecycleEvent::DeadlineClockSampled {
                        owner_reported: false,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            samples, 1,
            "one successful sampling request must publish exactly one sampled event"
        );
    }

    #[test]
    fn deadline_clock_sample_pairs_wall_and_monotonic_values_at_one_sampling_point() {
        let observer = OwnedProcessObservationHandle::default();
        let operation_id = OwnedProcessOperationId::from_raw_for_test(511);
        observer.record(OwnedProcessLifecycleEvent::Spawned {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            at: SystemTime::now(),
        });
        let start = Instant::now();
        let basis = deadline_basis(start, SystemTime::now(), Duration::from_secs(120))
            .expect("a finite deadline must produce a basis");
        observer.install_deadline_basis(operation_id, ProcessOperation::DeviceCopy, Some(basis));
        let wall_before = SystemTime::now();
        let instant_before = Instant::now();
        observer
            .request_deadline_clock_sample(operation_id)
            .expect("a sample with an exact basis must publish");
        let wall_after = SystemTime::now();
        let instant_after = Instant::now();
        let (at, deadline_clock_ns) = observer
            .events()
            .iter()
            .find_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockSampled {
                    operation_id: observed,
                    at,
                    deadline_clock_ns,
                    ..
                } if *observed == operation_id => Some((*at, *deadline_clock_ns)),
                _ => None,
            })
            .expect("the sampling request must publish its event");
        assert!(
            wall_before <= at && at <= wall_after,
            "the published wall timestamp must fall inside the sampling operation"
        );
        let ns_before = u64::try_from(instant_before.duration_since(start).as_nanos())
            .expect("bounded elapsed nanoseconds");
        let ns_after = u64::try_from(instant_after.duration_since(start).as_nanos())
            .expect("bounded elapsed nanoseconds");
        assert!(
            ns_before <= deadline_clock_ns && deadline_clock_ns <= ns_after,
            "the published monotonic value must fall inside the same sampling operation"
        );
    }

    #[test]
    fn deadline_clock_sample_with_unavailable_state_fails_closed() {
        let observer = OwnedProcessObservationHandle::default();
        let operation_id = OwnedProcessOperationId::from_raw_for_test(512);
        observer.record(OwnedProcessLifecycleEvent::Spawned {
            operation_id,
            operation: ProcessOperation::DeviceCopy,
            at: SystemTime::now(),
        });
        let basis = deadline_basis(Instant::now(), SystemTime::now(), Duration::from_secs(120))
            .expect("a finite deadline must produce a basis");
        observer.install_deadline_basis(operation_id, ProcessOperation::DeviceCopy, Some(basis));
        let _ = std::panic::catch_unwind(|| {
            let _guard = observer
                .state
                .lock()
                .expect("state should lock for poisoning");
            panic!("poison the observation state");
        });
        let error = observer
            .request_deadline_clock_sample(operation_id)
            .expect_err("a poisoned observation state must not report sampling success");
        assert!(
            error.contains("unavailable"),
            "the rejection must name the unavailable state: {error}"
        );
        assert_eq!(
            observer
                .events()
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        OwnedProcessLifecycleEvent::DeadlineClockSampled {
                            owner_reported: false,
                            ..
                        }
                    )
                })
                .count(),
            0,
            "a failed sampling request must not publish a sample"
        );
    }

    #[test]
    fn deadline_clock_started_at_uses_the_retained_basis_wall_observation() {
        let observer = OwnedProcessObservationHandle::default();
        let operation_id = OwnedProcessOperationId::from_raw_for_test(513);
        let start_wall = SystemTime::now();
        let start = Instant::now();
        let basis = deadline_basis(start, start_wall, Duration::from_secs(120))
            .expect("a finite deadline must produce a basis");
        // Deliberately separate basis construction from observer installation
        // so a later SystemTime::now() inside install would be distinguishable.
        std::thread::sleep(Duration::from_millis(25));
        observer.install_deadline_basis(operation_id, ProcessOperation::DeviceCopy, Some(basis));
        let (origin_ns, deadline_ns, at) = observer
            .events()
            .iter()
            .find_map(|event| match event {
                OwnedProcessLifecycleEvent::DeadlineClockStarted {
                    operation_id: observed,
                    deadline_clock_start_ns,
                    deadline_ns,
                    at,
                    ..
                } if *observed == operation_id => {
                    Some((*deadline_clock_start_ns, *deadline_ns, *at))
                }
                _ => None,
            })
            .expect("the clock start event must be published");
        assert_eq!(
            at, start_wall,
            "DeadlineClockStarted.at must be the wall observation retained with basis construction"
        );
        assert_eq!(
            origin_ns, 0,
            "the run-local monotonic origin must remain zero"
        );
        assert_eq!(
            deadline_ns,
            u64::try_from(Duration::from_secs(120).as_nanos())
                .expect("the deadline must fit u64 nanoseconds")
        );
    }

    #[test]
    fn poisoned_observation_state_does_not_alter_the_owned_process_result() {
        let observer = OwnedProcessObservationHandle::default();
        let _ = std::panic::catch_unwind(|| {
            let _guard = observer
                .state
                .lock()
                .expect("state should lock for poisoning");
            panic!("poison the observation state");
        });
        let executable = std::env::current_exe().expect("test executable should be available");
        let output = run_owned_process_observed_for_test(
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
            ProcessOperation::DeviceCopy,
            observer.clone(),
        )
        .expect("a poisoned observer must never replace the owned-process result");
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
    fn observed_helper() {
        std::thread::sleep(Duration::from_millis(500));
        println!("observed-helper");
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
    fn short_helper() {
        std::thread::sleep(Duration::from_millis(50));
        println!("short-helper");
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
