# Phase 6D.2 Operation Deadlines

## 1. Scope and status

**Owner:** EmuChef proper / Shared Runtime  
**Status:** Automated implementation complete; physical interruption and device qualification pending  
**Date:** 2026-08-02

This slice makes one-shot host processes, ADB operations, and the EmuChef
proper sidecar bounded and explicitly owned. It preserves the existing review,
execution, recovery, and public DTO contracts. Real execution remains
feature-gated and is not enabled by this work.

The implementation proves behavior with deterministic host tests. It does not
claim physical-device, packaged-GUI, signing, notarization, release, or
production qualification.

## 2. Process ownership and fixed budgets

`crates/emuchef-rust-backend/src/owned_process.rs` is the private ownership
boundary for one-shot ADB/process commands. Each invocation owns its child,
stdin policy, stdout, stderr, status future, deadline, and cleanup future in
one locally driven future tree. There are no reader/watchdog threads, detached
executor tasks, channels, or global-executor work. A pending local read future
may be dropped only after the owning operation has decided its terminal cause.

The fixed internal operation classes are:

| Operation class | Deadline |
|---|---:|
| Probe, predicate, root preflight, launch, force-stop | 30 seconds |
| Shell mutation | 120 seconds |
| Install, push, device copy, defensive generic fallback | 300 seconds |

Both output streams retain at most 4 MiB. The owner drains incrementally with a
fixed buffer and stops the child on overflow; overflow is a process failure,
not a timeout. Spawn, wait, stream-read, overflow, timeout, and cleanup
uncertainty remain separate internal evidence.

On timeout or fatal process failure the owner drops/closes stdin where
applicable, reconciles an already-exited child, terminates the exact child,
attempts a bounded reap, and settles or explicitly drops only locally owned
read futures. `TimedOut` remains the primary cause even when cleanup evidence is
uncertain.

## 3. Typed executor and fail-stop behavior

The backend executor carries `DeviceOperationError` and a private failure-kind
marker through `RealAdbDevice`, `ExecutorRunner`, and the execution session.
Timeout is never inferred by parsing presentation text. It becomes the stable
internal issue code `operation_timed_out`; failure-kind and cleanup evidence are
skipped from serialized `StepRunRecord` data, so the public schema is unchanged.

After a real device timeout:

1. the current step is failed and retains its completed outputs where available;
2. no later step resolves parameters, performs a device mutation, or verifies;
3. later steps remain in their initialized `pending` state and are projected as
   **Not attempted** only after terminalization;
4. the report conservatively sets `partialChangesPossible` for a real failed
   operation; and
5. the execution worker reaches a terminal report and releases its active
   execution slot through the existing session lifecycle.

Ordinary command failures, verification failures, cancellation, root denial,
transport/process failure, and timeout remain distinct internally. The Tauri
projection allowlists `operation_timed_out` and emits authored remediation text;
raw commands, serials, paths, stdout, stderr, and OS errors do not reach React.

## 4. Sidecar transport boundary

The EmuChef proper sidecar uses `async-process`, `async-io`, and `futures-lite`
directly. A request and its response are handled serially in one caller-owned
future tree with a fixed 300-second deadline. Responses are read by a bounded,
incremental JSONL framer with a 16 MiB maximum frame. EOF before a newline,
malformed JSON, an ID mismatch, a missing result/error envelope, transport
failure, and deadline expiry are fatal protocol loss. A valid structured backend
error remains nonfatal.

Fatal loss drops the local transport, closes stdin, terminates and boundedly
reaps the exact process generation, clears the process, persists
`runtime_session_lost`, and rejects all later requests for that generation.
Replacing or restarting a generation drops the old pipes before a new process is
created, so a late frame cannot be consumed by the new generation. Startup
failures remain externally projected as `runtime_start_failed`; valid capability
incompatibility remains `runtime_unsupported`.

## 5. Platform-Tools lifecycle repair

Tauri Platform-Tools validation now uses the same local-future ownership shape
without changing its existing `PATH=/usr/bin:/bin`, environment clearing,
working-directory, five-second validation deadline, 64 KiB retained-output
limit, or public error codes. Output overflow, timeout, read failure, wait
failure, and cleanup uncertainty are handled without background readers.

## 6. Automated evidence

Coverage includes fixed deadline inventory, normal completion, timeout and
cleanup evidence, bounded output and overflow precedence, typed timeout
propagation, fail-stop downstream scheduling, pending/not-attempted projection,
sidecar framing and frame overflow, partial EOF, exact response IDs, repeated
session-loss rejection, valid backend errors, sanitized timeout projection, and
cross-platform process-helper tests that do not invoke a shell, PowerShell,
`cmd`, or platform signals.

The full verification commands and results for this recovery run are recorded
in the run-specific `RESULT.md`.

## 7. Deferred Phase 6D work

Physical cancellation, disconnect, offline/unauthorized transitions, same-
serial replacement, root revocation, low storage, host sleep, and packaged-GUI
interruption qualification remain open. This slice adds no checkpointing,
resume, rollback, replay, automatic retry, persistent execution, new public
timeout controls, or production feature enablement.
