# Device-Unauthorized Boundary Qualification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the non-portable active-session authorization-revocation qualification with a safe-boundary reconnect-and-unauthorized qualification.

**Architecture:** Keep runtime classification and execution behavior unchanged. Reclassify only the physical harness scenario, capture a selected-serial absence and reconnect chronology around the boundary checkpoint, and update the shared manifest/validator so qualification requires the second reviewed operation to fail before mutation as `device_unauthorized`.

**Tech Stack:** Rust, Serde/serde_json, Node.js test runner, JSON evidence manifest, Markdown runbook.

## Global Constraints

- Preserve the failed `device_unauthorized` record and trace unchanged as audit evidence.
- Preserve its exact former scenario contract as a non-passing-only legacy audit snapshot; stale contracts can never qualify.
- Keep `device_unauthorized` mandatory with two clean repetitions.
- Keep `EMUCHEF_PHASE_6D6_AUTHORIZATION_RESET=1` mandatory.
- Do not change runtime issue codes, ADB classifiers, Tauri projection, public APIs, schema version, retry/resume policy, or mandatory counts.
- Require exact selected-serial chronology: authorized, boundary, absent interval, reconnected unauthorized, terminal, final authorized.
- Do not stage, commit, or push.

---

### Task 1: Lock the shared evidence contract to safe-boundary authorization

**Files:**
- Modify: `docs/testing/phase-6d6/scenario-manifest.json`
- Modify: `tools/phase-6d6-evidence.test.mjs`
- Modify: `tools/phase-6d6-evidence-regression.test.mjs`

**Interfaces:**
- Consumes: existing `device_unauthorized` scenario record builder and `validateEvidenceRecord`.
- Produces: manifest fields and tests requiring one completed first step, no active-process evidence, and reconnect chronology.

- [ ] **Step 1: Add failing Node assertions**

Update authorization tests so a passing record requires:

```js
{
  executed: 1,
  skipped: 0,
  failed: 1,
  cancelled: 0,
  blocked: 0,
  notAttempted: 0,
}
```

Set `activeProcess` to `null`, set `scenarioFacts.activeCheckpoint` to `false`, set `scenarioFacts.boundaryCheckpoint` to `true`, and add transition fields:

```js
originalDisconnectedAt
serialAbsentFrom
serialAbsentUntil
reconnectedAt
```

Add negative assertions for missing absence, zero-length absence, unauthorized before reconnect, and active-process relabeling.

- [ ] **Step 2: Run the focused Node test and confirm failure**

```sh
node --test --test-name-pattern="authorization|supported active" tools/phase-6d6-evidence.test.mjs tools/phase-6d6-evidence-regression.test.mjs
```

Expected: FAIL against the active-operation manifest/validator contract.

- [ ] **Step 3: Update the manifest minimally**

For `device_unauthorized`:

- keep `allowedIssueCodes: ["device_unauthorized"]`;
- keep only the `executed: 1, failed: 1, notAttempted: 0` state;
- remove `activeProcess`;
- set facts to `activeCheckpoint: false`, `boundaryCheckpoint: true`;
- require the new reconnect chronology in `authorizationTransition`.

- [ ] **Step 4: Leave validator field enforcement for Task 3**

The tests should continue failing specifically because the validator does not yet require the new chronology.

---

### Task 2: Reclassify the Rust harness scenario and plan stimulus

**Files:**
- Modify: `crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`

**Interfaces:**
- Consumes: `Scenario::is_active_checkpoint`, `Scenario::is_boundary_checkpoint`, `Scenario::supports_active_process_capture`, `prepare_active_stimulus`, and `reviewed_plan`.
- Produces: `DeviceUnauthorized` as an ordinary two-step boundary scenario without host-push stimulus.

- [ ] **Step 1: Add failing Rust inventory assertions**

Extend `scenario_inventory_is_exact_and_requires_two_repetitions` or add a focused test asserting:

```rust
assert!(!Scenario::DeviceUnauthorized.is_active_checkpoint());
assert!(Scenario::DeviceUnauthorized.is_boundary_checkpoint());
assert!(!Scenario::DeviceUnauthorized.supports_active_process_capture());
assert_eq!(active_process_operation(Scenario::DeviceUnauthorized), ProcessOperation::DeviceCopy);
```

Also assert the manifest contract has no `active_process` and only the boundary step-state shape.

- [ ] **Step 2: Run the focused Rust test and confirm failure**

```sh
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution --lib executor_real_adb_tests::physical_interruption_qualification::tests::device_unauthorized_uses_a_safe_boundary_without_active_process_capture -- --exact --nocapture
```

Expected: FAIL because the scenario is still classified as active.

- [ ] **Step 3: Implement the minimal classification change**

- Remove `DeviceUnauthorized` from `is_active_checkpoint`.
- Add it to `is_boundary_checkpoint`.
- Remove it from `supports_active_process_capture`.
- Keep `requires_terminal_recovery` and the authorization-reset opt-in.
- Let existing active-stimulus selection skip it automatically.

- [ ] **Step 4: Run the focused Rust test**

Expected: PASS.

---

### Task 3: Capture and validate reconnect authorization chronology

**Files:**
- Modify: `crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`
- Modify: `tools/phase-6d6-evidence.mjs`
- Modify: `tools/phase-6d6-evidence.test.mjs`

**Interfaces:**
- Consumes: `AuthorizationTransitionCapture`, `spawn_authorization_transition_observer`, `authorization_transition_evidence`, and the Node authorization validation branch.
- Produces: exact timestamp fields `originalDisconnectedAt`, `serialAbsentFrom`, `serialAbsentUntil`, and `reconnectedAt`.

- [ ] **Step 1: Add a failing Rust chronology test**

Add a pure helper test or serialized evidence test proving the required order:

```text
initialObservedAt
< operationStartedAt
< revocationCheckpointAt
< originalDisconnectedAt
<= serialAbsentFrom
< serialAbsentUntil
<= reconnectedAt
<= observedAt
<= terminalDetectedAt
< cleanupStartedAt
<= cleanupCompletedAt
< finalStateObservedAt
```

Reject no absence, an open-ended absence, and unauthorized observed before reconnect.

- [ ] **Step 2: Extend `AuthorizationTransitionCapture`**

Add:

```rust
original_disconnected_at: Option<SystemTime>,
serial_absent_from: Option<SystemTime>,
serial_absent_until: Option<SystemTime>,
reconnected_at: Option<SystemTime>,
```

- [ ] **Step 3: Update the observer state machine**

Wait for `authorization-revoked` in a later canonical second than `boundary-ready`, then poll `selected_serial_observation` and record:

1. first `Absent` as disconnect and absence start;
2. continued absence without changing the start;
3. first later `Unauthorized` as absence end, reconnect, and unauthorized observation;
4. create harness-owned `unauthorized-observed` only after that capture;
5. ignore `Attached` or `Unauthorized` before a proven absence and do not self-attest a transition;
6. release the second operation only when `operator-action` arrives after `unauthorized-observed`.

- [ ] **Step 4: Serialize the new evidence fields**

Include all four fields in `authorization_transition_evidence`. Return `null` unless the terminal issue is `device_unauthorized` and the capture contains the required chronology.

- [ ] **Step 5: Enforce the chronology in Node validation**

Add the four fields to exact authorization keys and timestamp parsing. Require the order above and the same run/device scope.

- [ ] **Step 6: Run focused Rust and Node tests**

```sh
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution --lib executor_real_adb_tests::physical_interruption_qualification::tests -- --nocapture
node --test --test-name-pattern="authorization" tools/phase-6d6-evidence.test.mjs
```

Expected: PASS.

---

### Task 4: Align operator choreography and current-state documentation

**Files:**
- Modify: `docs/manual/phase-6d6-physical-interruption-qualification.md`
- Modify: `docs/product/phase-6d6-physical-interruption-qualification.md`
- Modify: `docs/product/phase-6d1-execution-safety-audit.md`
- Modify: `docs/product/product-roadmap.md`
- Modify: `CONTEXT.md`

**Interfaces:**
- Consumes: the final manifest and harness behavior.
- Produces: exact operator instructions and accurate current-state claims.

- [ ] **Step 1: Update the runbook sequence**

Document:

```text
first operation completes
→ boundary-ready
→ wait into a later canonical second
→ revoke authorizations
→ authorization-revoked
→ disconnect selected USB device
→ verify exact serial absent
→ reconnect same device
→ verify exact serial unauthorized
→ unauthorized-observed
→ operator-action
→ second operation fails device_unauthorized
→ terminal-ready
→ reauthorize
→ verify exact serial device
→ cleanup-ready
```

Explicitly prohibit creating `authorization-revoked` in the same canonical second as `boundary-ready` or creating `operator-action` before harness-owned `unauthorized-observed`.

- [ ] **Step 2: Update current-state docs**

Replace active-operation authorization wording with safe-boundary reconnect qualification. State that the prior blocked attempt remains audit evidence and does not qualify.

- [ ] **Step 3: Ensure runbook validator markers remain present**

Keep all existing required gate, timeout, sentinel, and UI-smoke phrases.

---

### Task 5: Full verification and diff review

**Files:**
- Review all changed files.
- Do not modify the two failed evidence files.

- [ ] **Step 1: Run formatting and automated checks**

```sh
cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml --all -- --check
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution --lib executor_real_adb_tests::physical_interruption_qualification::tests -- --nocapture
node tools/phase-6d6-evidence.mjs
node --test tools/phase-6d6-evidence.test.mjs
node --test tools/phase-6d6-evidence-regression.test.mjs
git diff --check
```

- [ ] **Step 2: Confirm expected evidence baseline**

The failed active attempt remains non-qualifying, so the validator must still report:

```text
12 mandatory physical repetitions and 2 UI-smoke repetitions missing
```

- [ ] **Step 3: Review Git status**

Confirm the failed record and trace are unchanged and untracked, and no files are staged or committed.
