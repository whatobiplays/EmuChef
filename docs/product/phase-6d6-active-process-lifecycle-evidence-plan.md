# Phase 6D.6 Active-Process Lifecycle Evidence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add exact owned-child lifecycle evidence and a bounded device-side copy stimulus so the four active Phase 6D.6 scenarios can produce valid `activeProcess` evidence without changing production deadlines, ownership, or error semantics.

**Architecture:** The existing owned-process boundary remains the sole child owner and emits test-only, sanitized lifecycle observations through an invocation-scoped handle. `ProcessAdbCommandExecutor` forwards that optional handle to the real owned process, and the physical harness correlates the first `DeviceCopy` child with the current execution-slot run scope, an `active-ready` handshake, the operator action, terminal recovery, and strict cleanup evidence.

**Tech Stack:** Rust, `async_process`, `async_io`, `futures_lite`, `serde_json`, existing Phase 6D.6 Node validators, Cargo/libtest.

## Global Constraints

- Preserve the fixed production deadlines exactly: 30 seconds for probes, 120 seconds for shell mutations, and 300 seconds for install/push/device-copy/generic fallback.
- Do not add a test delay, sleep seam, host PID scan, child-handle escape, detached task, background worker, or ownership transfer.
- Do not expose program names, arguments, serials, paths, stdout, stderr, or PIDs in lifecycle observations or evidence.
- Keep ordinary production execution behavior identical when no observer is installed.
- Keep `operation_timeout` and both host-sleep scenarios blocked after this slice.
- Do not modify the scenario manifest or evidence schema unless an unavoidable mismatch is proven.
- Do not edit, regenerate, stage, or commit the six accepted physical evidence records or their traces.
- Active stimulus constants remain: 256 MiB calibration source, 30-second target, 15-second minimum predicted window, 240-second maximum predicted copy, 512 MiB minimum source, 8 GiB maximum source, 1 GiB cleanup headroom, and 5-second liveness freshness.
- Transport/offline/authorization operators create `operator-action` first, wait 1.1–3 seconds, then perform the transition; cleanup begins only after `terminal-ready`, recovery, and a fresh `cleanup-ready` acknowledgement.

---

## File map

- `crates/emuchef-rust-backend/src/owned_process.rs`: exact-child lifecycle authority, observation handle, liveness request servicing, and owned-process regressions.
- `crates/emuchef-rust-backend/src/executor/adb.rs`: optional observer forwarding through `ProcessAdbCommandExecutor` and `RealAdbDevice`.
- `crates/emuchef-rust-backend/src/end_user_runtime.rs`: retain ordinary root-probe construction through `ProcessAdbCommandExecutor::default()` after the executor becomes observer-capable.
- `crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`: bounded active stimulus, `active-ready` handshake, capture binding, recovery checkpoint, strict authorization chronology, evidence serialization, and harness tests.
- `docs/manual/phase-6d6-physical-interruption-qualification.md`: operator protocol and safety limits.
- `docs/product/phase-6d6-active-process-lifecycle-evidence-design.md`: clarified terminal-recovery design record.
- `docs/product/phase-6d6-active-process-lifecycle-evidence-plan.md`: this execution plan.

---

### Task 1: Add exact owned-child lifecycle observation

**Files:**
- Modify: `crates/emuchef-rust-backend/src/owned_process.rs`

**Interfaces:**
- Produces `OwnedProcessObservationHandle`, `OwnedProcessOperationId`, `OwnedProcessLivenessSample`, `OwnedProcessLifecycleEvent`, and `run_owned_process_observed` under `#[cfg(test)]`.
- `OwnedProcessLifecycleEvent::operation()` and `operation_id()` expose only the typed class and opaque ID.
- `OwnedProcessObservationHandle::events()` returns a cloned ordered event list.
- `OwnedProcessObservationHandle::wait_for_mutation(operation, timeout)` returns the first matching opaque ID.
- `OwnedProcessObservationHandle::request_liveness_sample(operation_id)` wakes the owner poll loop and requests one exact-child status-future sample.
- `OwnedProcessObservationHandle::wait_for_liveness(operation_id, timeout)` returns `OwnedProcessLivenessSample { at, alive, terminal_reported }`.
- `run_owned_process` remains unchanged for ordinary callers.

- [ ] **Step 1: Write failing lifecycle tests**

Add tests that exercise the real current test executable and assert one opaque identity across all events:

```rust
#[test]
fn observed_child_preserves_one_identity_and_event_order() {
    let observer = OwnedProcessObservationHandle::default();
    let executable = std::env::current_exe().unwrap();
    let worker = observer.clone();
    let operation = std::thread::spawn(move || {
        run_owned_process_observed(
            executable.to_str().unwrap(),
            &[
                "--exact".into(),
                "owned_process::tests::observed_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            ProcessOperation::DeviceCopy,
            worker,
        )
    });

    let operation_id = observer.wait_for_mutation(ProcessOperation::DeviceCopy, Duration::from_secs(2)).unwrap();
    observer.request_liveness_sample(operation_id).unwrap();
    let sample = observer.wait_for_liveness(operation_id, Duration::from_secs(2)).unwrap();
    assert_eq!(sample.alive, Some(true));
    let result = operation.join().expect("observed helper thread should finish");
    assert_eq!(result.unwrap().status_code, Some(0));
}
```

Also add focused tests for:

Add an ignored `observed_helper` that sleeps for 500 ms and then prints one bounded line; the test must join the helper thread before returning.

```rust
fn completed_child_cannot_be_sampled_as_alive()
fn timeout_preserves_failure_and_emits_terminal_after_cleanup()
fn no_observer_preserves_existing_result_shape()
fn observation_payload_contains_no_command_or_pid_fields()
```

The test helper may use a short observer-only deadline helper, but physical evidence must never use it.

- [ ] **Step 2: Run the focused tests and confirm failure**

Run:

```bash
cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  owned_process::tests::observed_child_preserves_one_identity_and_event_order \
  -- --nocapture
```

Expected: compile failure because the observation types and observed entry point do not exist.

- [ ] **Step 3: Implement the observation types**

Add test-only types with no raw process details:

```rust
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct OwnedProcessOperationId(u64);

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OwnedProcessLifecycleEvent {
    Spawned { operation_id: OwnedProcessOperationId, operation: ProcessOperation, at: SystemTime },
    MutationStarted { operation_id: OwnedProcessOperationId, operation: ProcessOperation, at: SystemTime },
    LivenessSampled {
        operation_id: OwnedProcessOperationId,
        operation: ProcessOperation,
        at: SystemTime,
        alive: Option<bool>,
        terminal_reported: bool,
    },
    Terminal { operation_id: OwnedProcessOperationId, operation: ProcessOperation, at: SystemTime },
}
```

Back the handle with `Arc<Mutex<State>>`. The state contains only events, one keyed sample request, one keyed sample result, and an optional registered `Waker`. `request_liveness_sample` stores the request and wakes the registered owner waker.

- [ ] **Step 4: Integrate observation into the owner loop**

Refactor the private owned-process path so:

```rust
pub(crate) fn run_owned_process(...) -> Result<...> {
    run_owned_process_with_observer(program, args, operation, None)
}

#[cfg(test)]
pub(crate) fn run_owned_process_observed(
    program: &str,
    args: &[String],
    operation: ProcessOperation,
    observer: OwnedProcessObservationHandle,
) -> Result<CapturedProcessOutput, OwnedProcessError> {
    run_owned_process_with_observer(program, args, operation, Some(observer))
}
```

Required sequencing:

1. Spawn child.
2. Allocate opaque ID from a process-local atomic counter.
3. Emit `Spawned`.
4. Acquire both pipes.
5. Emit `MutationStarted` for mutating operation classes, including `DeviceCopy`.
6. In every poll, register the current waker, service a matching liveness request by polling the exact owner-held status future, emit `LivenessSampled`, consume any ready status exactly once, then preserve the existing output/status/deadline precedence.
7. Emit exactly one `Terminal` only after the existing terminal result and cleanup classification are known.

Do not move or weaken the current output-first polling precedence.

- [ ] **Step 5: Run all owned-process tests**

```bash
cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  owned_process::tests \
  -- --nocapture
```

Expected: all owned-process tests pass, including timeout, overflow, wait, scoped-delay, and new observation tests.

- [ ] **Step 6: Review Task 1 diff**

Confirm no production command, deadline, process ownership, or cleanup behavior changed for the `None` observer path.

---

### Task 2: Forward observations through the real ADB adapter

**Files:**
- Modify: `crates/emuchef-rust-backend/src/executor/adb.rs`

**Interfaces:**
- Consumes `OwnedProcessObservationHandle` and `run_owned_process_observed` from Task 1.
- Produces `ProcessAdbCommandExecutor::with_process_observer` and `RealAdbDevice::new_with_process_observer` under `#[cfg(test)]`.

- [ ] **Step 1: Write failing adapter tests**

Add a test that executes the current test binary through `ProcessAdbCommandExecutor` with `ProcessOperation::DeviceCopy` and confirms the observer receives `DeviceCopy` events without changing the command result:

```rust
#[test]
fn process_executor_forwards_device_copy_observation_without_changing_result() {
    let observer = OwnedProcessObservationHandle::default();
    let mut executor = ProcessAdbCommandExecutor::with_process_observer(observer.clone());
    let executable = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    let result = executor.run_for(
        &[
            executable,
            "--exact".into(),
            "owned_process::tests::normal_helper".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        ProcessOperation::DeviceCopy,
    ).unwrap();
    assert_eq!(result.returncode, 0);
    assert!(observer.events().iter().any(|event| event.operation() == ProcessOperation::DeviceCopy));
}
```

Also retain the existing exhaustive operation-class test.

- [ ] **Step 2: Run the focused adapter test and confirm failure**

```bash
cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor::adb::tests::process_executor_forwards_device_copy_observation_without_changing_result \
  -- --nocapture
```

Expected: compile failure because observer constructors do not exist.

- [ ] **Step 3: Convert the unit executor into an observer-capable struct**

Use this shape:

```rust
#[derive(Debug, Default)]
pub struct ProcessAdbCommandExecutor {
    #[cfg(test)]
    process_observer: Option<OwnedProcessObservationHandle>,
}
```

`run_for` calls `run_owned_process` when no observer exists and `run_owned_process_observed` only when the test-only observer is installed. Keep transport, storage, timeout, and process-failure mapping exactly where they are now.

- [ ] **Step 4: Add the physical-harness constructor**

Add:

```rust
#[cfg(test)]
pub(crate) fn new_with_process_observer(
    executable: impl Into<String>,
    serial: Option<String>,
    observer: OwnedProcessObservationHandle,
) -> Self
```

It constructs the same identity and root-authority guards as `RealAdbDevice::new`, replacing only the executor instance.

- [ ] **Step 5: Run ADB adapter tests**

```bash
cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor::adb::tests \
  -- --nocapture
```

Expected: all adapter tests pass; completed transport/storage classification and fake executor behavior remain unchanged.

- [ ] **Step 6: Review Task 2 diff**

Confirm the ordinary `RealAdbDevice::new` path still installs no observer and has no new public production authority surface.

---

### Task 3: Add bounded active stimulus and exact-child physical capture

**Files:**
- Modify: `crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`

**Interfaces:**
- Consumes `OwnedProcessObservationHandle`, `OwnedProcessOperationId`, `OwnedProcessLifecycleEvent`, and `RealAdbDevice::new_with_process_observer`.
- Produces `ActiveStimulus`, `ActiveProcessCapture`, `derive_active_stimulus`, `active_process_evidence`, and deterministic recovery helpers.

- [ ] **Step 1: Write failing pure calibration tests**

Add tests around a pure integer derivation helper:

```rust
#[test]
fn active_stimulus_derivation_targets_the_bounded_operator_window() {
    let derived = derive_active_stimulus(ACTIVE_CALIBRATION_KIB, 1_000).unwrap();
    assert!((ACTIVE_MIN_KIB..=ACTIVE_MAX_KIB).contains(&derived.payload_kib));
    assert!((ACTIVE_MIN_PREDICTED_MS..=ACTIVE_MAX_PREDICTED_MS).contains(&derived.predicted_ms));
}

#[test]
fn active_stimulus_derivation_rejects_zero_or_unusable_throughput() {
    assert!(derive_active_stimulus(ACTIVE_CALIBRATION_KIB, 0).is_err());
    assert!(derive_active_stimulus(ACTIVE_CALIBRATION_KIB, 1).is_err());
}
```

Add tests for insufficient free space, fixed fixture-owned paths, and cleanup inventory.

- [ ] **Step 2: Write failing active-process evidence tests**

Construct synthetic ordered owner events and assert the existing schema is produced only when all bindings are valid:

```rust
#[test]
fn exact_child_capture_serializes_the_existing_active_process_schema() { ... }

#[test]
fn same_second_action_and_terminal_remain_blocked() { ... }

#[test]
fn mismatched_operation_identity_or_non_live_sample_is_rejected() { ... }

#[test]
fn timeout_and_host_sleep_remain_without_physical_active_process_capture() { ... }
```

- [ ] **Step 3: Run the focused harness tests and confirm failure**

```bash
cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  --features real-execution \
  physical_interruption_qualification::tests::active_stimulus_derivation_targets_the_bounded_operator_window \
  -- --nocapture
```

Expected: compile failure because the calibration and capture helpers do not exist.

- [ ] **Step 4: Add exact active-scenario and stimulus constants**

Add:

```rust
const ACTIVE_CALIBRATION_KIB: u64 = 256 * 1024;
const ACTIVE_TARGET_MS: u64 = 30_000;
const ACTIVE_MIN_PREDICTED_MS: u64 = 15_000;
const ACTIVE_MAX_PREDICTED_MS: u64 = 240_000;
const ACTIVE_MIN_KIB: u64 = 512 * 1024;
const ACTIVE_MAX_KIB: u64 = 8 * 1024 * 1024;
const ACTIVE_CLEANUP_HEADROOM_KIB: u64 = 1024 * 1024;
const ACTIVE_SAMPLE_FRESHNESS: Duration = Duration::from_secs(5);
```

Define:

```rust
fn supports_active_process_capture(scenario: Scenario) -> bool {
    matches!(scenario,
        Scenario::CancellationActive |
        Scenario::UsbDisconnectActive |
        Scenario::DeviceOffline |
        Scenario::DeviceUnauthorized)
}
```

Do not enable capture for `OperationTimeout` or host-sleep.

- [ ] **Step 5: Implement bounded device-side calibration**

Use integer `u128` arithmetic:

```rust
payload_kib = clamp(calibration_kib * ACTIVE_TARGET_MS / elapsed_ms, ACTIVE_MIN_KIB, ACTIVE_MAX_KIB)
predicted_ms = payload_kib * elapsed_ms / calibration_kib
```

Reject zero elapsed time, overflow, predicted duration outside 15–240 seconds, and free space below:

```text
2 × payload_kib + 1 GiB
```

Preparation sequence:

1. Create unique run scope.
2. Create 256 MiB calibration source.
3. Time one device-side `cp` to a calibration destination.
4. Remove calibration destination.
5. Derive bounded active source size.
6. Verify free space.
7. Create and verify active source.

Use fixed run-scoped filenames and ensure `cleanup_fixture` removes calibration destination, calibration source, active source, first destination, and second destination before verifying the run scope is absent.

- [ ] **Step 6: Build the first reviewed step from a device source**

For the four supported active scenarios, create the first `copy_files` source as:

```json
{"type":"file_path","value":"<fixture-owned-active-source>","location":"device"}
```

The ordinary executor must therefore call `copy_on_device` and spawn `ProcessOperation::DeviceCopy`. Keep all other scenario plans unchanged.

- [ ] **Step 7: Add the `active-ready` exact-child handshake**

Install one observer on the physical `RealAdbDevice`. In the active watcher:

1. Wait for the first `MutationStarted` event with `ProcessOperation::DeviceCopy`.
2. Request a liveness sample for that exact opaque ID.
3. Wait for `LivenessSampled { alive: Some(true), terminal_reported: false }`.
4. Create `active-ready`.
5. Wait for a fresh `operator-action` no more than five seconds after the sample.
6. For cancellation, set the existing cancellation flag.
7. For transport/offline/authorization, leave the operation running so the physical transition determines the typed terminal result.

Add `active-ready` to sentinel cleanup, but do not add it to the strict evidence schema.

- [ ] **Step 8: Build and serialize `activeProcess`**

Correlate exactly one operation ID and require:

```text
spawned <= mutationStarted <= checkedAlive <= action < terminal
```

Use actual `SystemTime` for the five-second freshness check and canonical Unix seconds for final contract ordering. Produce separate domain-separated hashes:

```rust
operationId = digest("phase6d6-operation:<opaque-id>")
childIdentity = digest("phase6d6-child:<opaque-id>")
```

Insert the resulting value into both the contract-evaluation object and final evidence record. Keep it null for unsupported scenarios.

- [ ] **Step 9: Add deterministic terminal recovery before cleanup**

For `UsbDisconnectActive`, `DeviceOffline`, and `DeviceUnauthorized`:

1. Wait for runner terminal result.
2. Create `terminal-ready`.
3. Wait for a fresh `cleanup-ready` after the terminal marker.
4. Begin cleanup only after the operator has restored the selected device.

Do not apply this checkpoint to cancellation.

- [ ] **Step 10: Make authorization chronology exact**

For `DeviceUnauthorized`:

- ensure the initial authorized observation serializes in an earlier Unix second than `operation-started`;
- use the exact owner `activeProcess.terminalAt` as `authorizationTransition.terminalDetectedAt`;
- require the unauthorized inventory observation after `operator-action` and no later than terminal;
- after cleanup, wait until a later canonical Unix second, observe the selected serial as authorized, and store that timestamp as `finalStateObservedAt`;
- never query or alter unrelated ADB authorization state.

- [ ] **Step 11: Run the complete physical-harness unit tests**

```bash
cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  --features real-execution \
  physical_interruption_qualification::tests \
  -- --nocapture
```

Expected: all existing and new harness tests pass; `operation_timeout` and host-sleep still block without their separate measurements.

- [ ] **Step 12: Review Task 3 diff**

Confirm no accepted evidence file changed and no active stimulus path escapes the unique fixture run scope.

---

### Task 4: Update the runbook and run the regression matrix

**Files:**
- Modify: `docs/manual/phase-6d6-physical-interruption-qualification.md`
- Modify: `docs/product/phase-6d6-active-process-lifecycle-evidence-design.md`
- Preserve: `docs/testing/phase-6d6/evidence/**`

**Interfaces:**
- Documents the exact operator protocol implemented in Task 3.

- [ ] **Step 1: Update active-case safety gates**

Document:

```text
operation-started
→ owner confirms exact DeviceCopy child alive
→ active-ready
→ operator-action within 5 seconds
→ cancellation request OR wait 1.1–3 seconds and perform transition
→ exact child terminal result
→ terminal-ready for transport/offline/authorization
→ restore selected device
→ cleanup-ready
→ fixture-only cleanup
```

State that calibration may allocate 512 MiB–8 GiB for the source plus an equal destination and 1 GiB cleanup headroom, and that the harness blocks before mutation when the bounds cannot be satisfied.

- [ ] **Step 2: Update each active scenario row**

Specify:

- `cancellation_active`: create `operator-action` immediately after `active-ready`.
- `usb_disconnect_active`: create action, wait 1.1–3 seconds, disconnect, wait `terminal-ready`, reconnect the same device, verify online, create `cleanup-ready`.
- `device_offline`: create action, wait 1.1–3 seconds, enter the prepared reversible offline state, wait `terminal-ready`, restore online state, create `cleanup-ready`.
- `device_unauthorized`: create action, wait 1.1–3 seconds, revoke only the selected device authorization, wait `terminal-ready`, reauthorize the same device, verify `device`, create `cleanup-ready`.

- [ ] **Step 3: Run formatting and focused Rust verification**

```bash
cargo fmt \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  --all \
  -- --check

cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  owned_process::tests \
  -- --nocapture

cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  executor::adb::tests \
  -- --nocapture

cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  --features real-execution \
  physical_interruption_qualification::tests \
  -- --nocapture
```

- [ ] **Step 4: Run full Rust verification**

```bash
cargo test \
  --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  --features real-execution
```

- [ ] **Step 5: Run Phase 6D.6 Node verification**

```bash
node tools/phase-6d6-evidence.mjs
node --test tools/phase-6d6-evidence.test.mjs
node --test tools/phase-6d6-evidence-regression.test.mjs
node tools/phase-6d6-result.mjs
node --test tools/phase-6d6-result.test.mjs
```

Expected current state after implementation but before more physical runs:

```text
Phase 6D.6 evidence contract valid but incomplete (20 physical repetitions and 2 UI-smoke repetitions missing).
```

- [ ] **Step 6: Review the final diff**

Confirm only these tracked files changed:

```text
crates/emuchef-rust-backend/src/owned_process.rs
crates/emuchef-rust-backend/src/executor/adb.rs
crates/emuchef-rust-backend/src/end_user_runtime.rs
crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs
docs/manual/phase-6d6-physical-interruption-qualification.md
docs/product/phase-6d6-active-process-lifecycle-evidence-design.md
docs/product/phase-6d6-active-process-lifecycle-evidence-plan.md
```

The twelve existing untracked evidence/trace files remain untouched.

- [ ] **Step 7: Commit the reviewed implementation**

Stage only the six tracked paths above and create a local commit such as:

```text
feat: add active process qualification evidence
```
