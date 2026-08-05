# Phase 6D.6 Active Host-Push Stimulus Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the unusably fast device-side active-copy calibration with a calibrated, non-compressible, run-owned host push while preserving exact-child evidence, existing timing bounds, and all six accepted physical records.

**Architecture:** Keep the lifecycle observer in `owned_process.rs` unchanged. The physical harness creates a deterministic non-compressible host fixture in a unique temporary workspace, calibrates through the production `RealAdbDevice::push` path without an observer, then runs the reviewed `copy_files` step with a host `file_path` source and observes the exact `ProcessOperation::Push` child. The shared evidence contract gains `host_push` only for the four supported active scenarios; existing `device_copy` records remain valid.

**Tech Stack:** Rust, `tempfile`, `serde_json`, existing executor/ADB abstractions, JSON Schema, Node.js test runner.

## Global Constraints

- Keep the target duration at 30 seconds.
- Keep the accepted predicted range at 15–240 seconds.
- Keep the active payload range at 512 MiB–8 GiB.
- Keep the operator-action freshness window at five seconds.
- Do not change production deadlines, cancellation semantics, process ownership, cleanup classification, issue precedence, or slot ownership.
- Do not modify the six accepted physical evidence records or their traces.
- Do not add shell sleeps, private delay evidence, retries, resume, reconnect, replay, or ownership transfer.
- Host and device fixtures must be unique, run-owned, sanitized from evidence, and removed on all success and error paths.

---

## File Map

- `crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`
  - Owns deterministic host-fixture generation, push calibration, reviewed active plan construction, exact-operation selection, cleanup, evidence serialization, and Rust contract tests.
- `docs/testing/phase-6d6/scenario-manifest.json`
  - Declares `host_push` for the four supported active scenarios while retaining `device_copy` elsewhere.
- `docs/testing/phase-6d6/evidence-schema.json`
  - Allows `device_copy` and `host_push` and keeps the record internally consistent with the selected scenario contract.
- `docs/testing/phase-6d6/evidence-template.json`
  - Shows a `host_push` active example without changing historical evidence.
- `tools/phase-6d6-evidence.mjs`
  - Validates operation class against the selected scenario contract rather than a global `device_copy` constant.
- `tools/phase-6d6-evidence.test.mjs`
  - Covers active `host_push`, historical `device_copy`, and cross-operation relabeling failures.
- `tools/phase-6d6-evidence-regression.test.mjs`
  - Locks the four active scenario contracts to `host_push` and verifies accepted records remain valid.
- `docs/manual/phase-6d6-physical-interruption-qualification.md`
  - Documents host-push calibration, storage requirements, cleanup, and unchanged operator protocol.
- `docs/product/phase-6d6-active-process-lifecycle-evidence-design.md`
  - Records the operation-class evolution from the original device-copy assumption.

---

### Task 1: Deterministic Non-Compressible Host Fixture

**Files:**
- Modify: `crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`

**Interfaces:**
- Produces: `fn write_active_host_fixture(path: &Path, byte_len: u64, seed: u64) -> Result<(), String>`
- Produces: `fn splitmix64_next(state: &mut u64) -> u64`
- Consumes: a unique run-owned temporary workspace path and a seed derived from the run-scope digest.

- [ ] **Step 1: Add failing generator tests**

Add tests that create small files in a `tempfile::tempdir()` and assert:

```rust
#[test]
fn active_host_fixture_is_exact_deterministic_and_nontrivial() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.bin");
    let second = temp.path().join("second.bin");
    write_active_host_fixture(&first, 131_089, 7).unwrap();
    write_active_host_fixture(&second, 131_089, 7).unwrap();
    assert_eq!(std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap());
    assert_eq!(std::fs::metadata(&first).unwrap().len(), 131_089);
    let bytes = std::fs::read(&first).unwrap();
    assert!(bytes.iter().any(|byte| *byte != 0));
    assert_ne!(&bytes[..64], &bytes[64..128]);
}

#[test]
fn active_host_fixture_changes_with_run_seed() {
    // Same length, different seed, different prefix.
}
```

Add one injected writer-failure test through a small internal helper that accepts `impl Write` and verifies the path-level wrapper removes a partial file.

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml active_host_fixture -- --nocapture
```

Expected: compilation failure because the generator functions do not exist.

- [ ] **Step 3: Implement the bounded-memory generator**

Implement SplitMix64-style output:

```rust
fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
    value ^ (value >> 31)
}
```

Stream little-endian words through a fixed 1 MiB buffer. Use `OpenOptions::create_new(true)` so pre-existing paths fail closed. Flush, sync, and verify the exact file size. On any write, flush, sync, or size-verification failure, remove the partial path before returning a sanitized error.

- [ ] **Step 4: Run the focused tests and confirm GREEN**

Run the same focused command and require zero failures.

- [ ] **Step 5: Review the diff for secret/path leakage**

Confirm no host path, seed, fixture bytes, raw serial, or device path is added to evidence or error output.

---

### Task 2: Production Push Calibration and Run-Owned Workspace

**Files:**
- Modify: `crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`

**Interfaces:**
- Replaces the device-source `ActiveStimulus` with a non-`Clone` owner type:

```rust
#[derive(Debug)]
struct ActiveStimulus {
    host_workspace: tempfile::TempDir,
    host_source_path: PathBuf,
    device_destination_path: String,
    payload_kib: u64,
    predicted_ms: u64,
}
```

The stimulus is passed by shared reference through plan construction and execution. The `TempDir` remains owned by the invocation until cleanup and is never cloned or converted into a persistent path.

- Produces: `fn active_operation_class(scenario: Scenario) -> &'static str`
- Produces: `fn active_process_operation(scenario: Scenario) -> ProcessOperation`
- Consumes: `RealAdbDevice::new("adb", Some(serial))` and the ordinary `ExecutorDevice::push` method for calibration.

- [ ] **Step 1: Add failing derivation and cleanup tests**

Add or update tests to assert:

```rust
let derived = derive_active_stimulus(256 * 1024, 6_880).unwrap();
assert!((1_100 * 1024..=1_130 * 1024).contains(&derived.payload_kib));
assert!((29_000..=31_000).contains(&derived.predicted_ms));
```

Add tests proving active scenarios map to `ProcessOperation::Push` / `host_push`, while boundary, timeout, and host-sleep scenarios retain `device_copy`.

Add a cleanup test that creates a run-owned host workspace and verifies cleanup removes calibration and active files without touching a sibling path.

- [ ] **Step 2: Run focused tests and confirm RED**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution physical_interruption_qualification::tests::active_ -- --nocapture
```

Expected: failures because active scenarios still map to `DeviceCopy` and the stimulus has no host workspace.

- [ ] **Step 3: Implement host workspace and calibration**

Create the temporary workspace only after all ordinary invocation/device gates pass. Derive the seed from the already-sanitized run-scope digest. Generate a 256 MiB calibration file with `write_active_host_fixture`.

Use an unobserved `RealAdbDevice` and the production `push` method:

```rust
let mut calibration_device = RealAdbDevice::new("adb", Some(facts.serial.clone()));
let started = Instant::now();
calibration_device.push(&calibration_path, &calibration_destination, false)?;
let elapsed_ms = u64::try_from(started.elapsed().as_millis())?;
```

Verify the device destination size, remove it, derive the active payload, require device free space for `payload + 1 GiB`, and require host free-space/creation success by generating the final file. Do not retain a device-side source.

- [ ] **Step 4: Integrate exact cleanup ownership**

Ensure every preparation error removes:

1. calibration device destination;
2. active device destination if created;
3. calibration host file;
4. active host file;
5. the temporary host workspace;
6. the run-scoped device directory.

Preserve cleanup outcome reporting and do not hide a primary calibration or execution error.

- [ ] **Step 5: Run focused tests and confirm GREEN**

Require all active-stimulus, derivation, and cleanup tests to pass.

---

### Task 3: Reviewed Host Push and Exact Push-Child Evidence

**Files:**
- Modify: `crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`

**Interfaces:**
- `reviewed_plan(...)` consumes `ActiveStimulus::host_source_path` as a host `file_path` and `device_destination_path` as the destination.
- `run_reviewed_plan(...)` adds only `host_workspace.path()` to `read_only_roots` for active scenarios.
- The observer waits for `active_process_operation(scenario)` and serializes `active_operation_class(scenario)`.

- [ ] **Step 1: Add failing exact-operation tests**

Add tests that build lifecycle events for `ProcessOperation::Push` and assert the evidence is `host_push`. Add negative tests:

```rust
assert!(active_process_evidence(
    &device_copy_events,
    action_time,
    run_scope,
    ProcessOperation::Push,
    "host_push",
).is_none());
```

Also reject `Push` events when the expected class is `device_copy`.

- [ ] **Step 2: Run focused tests and confirm RED**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution exact_child_capture -- --nocapture
```

Expected: current serializer hard-codes `device_copy` and accepts only `DeviceCopy` events.

- [ ] **Step 3: Make lifecycle extraction operation-aware**

Change `active_process_evidence` to take the expected `ProcessOperation` and semantic class. Require all `Spawned`, `MutationStarted`, `LivenessSampled`, and `Terminal` events to match the same operation ID and expected operation before serializing.

Do not infer a class from arbitrary strings; call it only with values returned by `active_process_operation` and `active_operation_class`.

- [ ] **Step 4: Route the reviewed first step through host push**

For the four active scenarios, emit a host `file_path` runtime value. Keep all other scenarios unchanged. Add the host workspace to the executor sandbox `read_only_roots` only for that invocation:

```rust
let mut read_only_roots = vec![fixture_root()];
if let Some(stimulus) = active_stimulus {
    read_only_roots.push(stimulus.host_workspace.path().to_path_buf());
}
```

The ordinary `copy_files` path must call `device.push`, creating an owned `ProcessOperation::Push` child.

- [ ] **Step 5: Update watcher binding**

Replace the fixed `wait_for_mutation(ProcessOperation::DeviceCopy, ...)` with the scenario-selected operation. Preserve the exact five-second freshness and canonical-second chronology checks.

- [ ] **Step 6: Run focused Rust tests and confirm GREEN**

Run:

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml executor::adb::tests -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution physical_interruption_qualification::tests -- --nocapture
```

Require zero failures.

---

### Task 4: Shared Contract and Backward-Compatible Validators

**Files:**
- Modify: `docs/testing/phase-6d6/scenario-manifest.json`
- Modify: `docs/testing/phase-6d6/evidence-schema.json`
- Modify: `docs/testing/phase-6d6/evidence-template.json`
- Modify: `tools/phase-6d6-evidence.mjs`
- Modify: `tools/phase-6d6-evidence.test.mjs`
- Modify: `tools/phase-6d6-evidence-regression.test.mjs`

**Interfaces:**
- The four active scenario contracts require `host_push`.
- Historical and non-active contracts continue to require `device_copy`.
- `scenarioFacts.operationClass` must equal `scenarioContract.activeProcess.operationClass` when an active-process contract exists, otherwise the scenario’s declared reviewed operation class.

- [ ] **Step 1: Add failing Node contract tests**

Add tests that assert:

```js
for (const scenario of [
  "cancellation_active",
  "usb_disconnect_active",
  "device_offline",
  "device_unauthorized",
]) {
  assert.equal(scenarioContractFor(scenario).activeProcess.operationClass, "host_push");
}
```

Keep assertions that accepted boundary/identity/root records remain `device_copy`. Add relabeling tests that mutate `activeProcess.operationClass` or `scenarioFacts.operationClass` independently and require rejection.

- [ ] **Step 2: Run Node tests and confirm RED**

Run:

```bash
node --test tools/phase-6d6-evidence.test.mjs
node --test tools/phase-6d6-evidence-regression.test.mjs
```

Expected: failures because the manifest/schema/validator still impose `device_copy` globally.

- [ ] **Step 3: Update manifest, schema, and template**

Change only the four supported active contracts to `host_push`. In the schema, replace the `const` values with:

```json
{ "enum": ["device_copy", "host_push"] }
```

Update the template’s active example and scenario facts to `host_push`.

- [ ] **Step 4: Replace the global Node assertion**

Change:

```js
if (record.scenarioFacts.operationClass !== "device_copy") ...
```

into a comparison against the selected scenario contract. Keep the existing `activeProcess.operationClass === rule.operationClass` check. Fail if scenario facts and active-process evidence disagree.

- [ ] **Step 5: Run Node tests and confirm GREEN**

Require 26 evidence tests, 4 regression tests, and all accepted physical records to validate unchanged.

---

### Task 5: Runbook, Design Reconciliation, and Full Verification

**Files:**
- Modify: `docs/manual/phase-6d6-physical-interruption-qualification.md`
- Modify: `docs/product/phase-6d6-active-process-lifecycle-evidence-design.md`
- Modify: `.chatgpt/codex-runs/2026-08-03T213321Z-phase-6d-6-physical-interruption-qualification/run.json` only if the local RESULT validator requires the newly changed tracked paths to be authorized.
- Modify: `.chatgpt/codex-runs/2026-08-03T213321Z-phase-6d-6-physical-interruption-qualification/RESULT.md` only after fresh verification output exists.

**Interfaces:**
- Runbook describes a 256 MiB non-compressible host-push calibration and one device destination plus 1 GiB headroom.
- Existing operator markers and recovery sequences remain unchanged.

- [ ] **Step 1: Update documentation**

Replace device-side source/copy language for the four active scenarios with host-push language. State that the host fixture is deterministic, non-secret, run-owned, and deleted. Keep the literal validator marker `production runner lifecycle`.

Reconcile the earlier lifecycle design so it no longer claims every active scenario is `DeviceCopy`; state that exact-child evidence is operation-aware and the current physical active stimulus is `Push` / `host_push`.

- [ ] **Step 2: Run formatting and focused verification**

```bash
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all -- --check
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml owned_process::tests -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml executor::adb::tests -- --nocapture
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution physical_interruption_qualification::tests -- --nocapture
```

- [ ] **Step 3: Run the complete backend suite**

```bash
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution
```

- [ ] **Step 4: Run all Phase 6D.6 validators**

```bash
node tools/phase-6d6-evidence.mjs
node --test tools/phase-6d6-evidence.test.mjs
node --test tools/phase-6d6-evidence-regression.test.mjs
node tools/phase-6d6-result.mjs
node --test tools/phase-6d6-result.test.mjs
```

Expected evidence status before the new physical run: contract valid but incomplete with 20 physical and 2 UI-smoke repetitions missing.

- [ ] **Step 5: Review repository state**

Confirm:

- no accepted evidence or trace file changed;
- only planned source, contract, test, and documentation files changed;
- no host diagnostic file remains;
- no device fixture path remains from automated tests;
- no raw serial or host/device path appears in evidence fixtures.

- [ ] **Step 6: Commit the implementation after verification**

Stage only the reviewed tracked files. Do not regenerate or rewrite the six accepted evidence records.
