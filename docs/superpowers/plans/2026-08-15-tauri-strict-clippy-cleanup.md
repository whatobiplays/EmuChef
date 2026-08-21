# Tauri Strict-Clippy Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `apps/emuchef-app/src-tauri` pass strict Clippy with zero denied warnings under both default and `real-execution` feature sets while preserving all existing runtime and Phase 6D.6 behavior.

**Architecture:** Treat Clippy as the failing gate and repair diagnostics at their source with minimal idiomatic Rust transformations. Keep qualification, process lifecycle, sidecar, execution, and UI-smoke authority contracts unchanged; if `execution.rs` changes, regenerate the derived UI binding index because that file is source-digested by the index.

**Tech Stack:** Rust, Tauri 2, Cargo/Clippy, async-io, async-process, futures-lite, Node.js Phase 6D.6 validator, React/Vitest frontend gates.

## Global Constraints

- Work from current `main`; preserve the committed Phase 6D.6 UI-smoke implementation.
- Fix all Tauri strict-Clippy warnings exposed during this pass, not only the original 12 lib-target + 9 lib-test-target diagnostics.
- Preserve executor state transitions, operation deadlines, timeout classification, child-process ownership/cleanup, ADB behavior, device/root qualification, recovery policy, sidecar framing, and UI-smoke gating/projection/capture semantics.
- Do not weaken `-D warnings`, Clippy configuration, or crate/module lint policy.
- Do not add broad `#[allow]`/`#[expect]`; use a narrowly scoped lint allowance only when an idiomatic behavior-preserving fix is unreasonable and document why.
- Do not modify accepted evidence/traces, `scenario-manifest.json`, `evidence-schema.json`, UI capture evidence, or `.serena/**`.
- If `apps/emuchef-app/src-tauri/src/execution.rs` changes, regenerate `docs/testing/phase-6d6/ui-binding-index.json` with the repository tool; never hand-edit the index.
- Do not run ignored physical tests, identity-replacement qualification, host-sleep qualification, or manual/UI-smoke qualification.
- Phase 6D remains **In progress** after this cleanup; Phase 6E remains **Planned**.

---

## File Structure

Primary files expected to change based on the reproduced baseline diagnostics:

- `apps/emuchef-app/src-tauri/src/commands.rs` — native command/helper lint cleanup.
- `apps/emuchef-app/src-tauri/src/device_qualification.rs` — qualification predicates, references, conversions, and test idioms.
- `apps/emuchef-app/src-tauri/src/adb.rs` — bounded process polling idioms without timeout/cleanup semantic changes.
- `apps/emuchef-app/src-tauri/src/sidecar.rs` — bounded request timeout polling idiom without protocol semantic changes.
- `apps/emuchef-app/src-tauri/src/execution.rs` — the remaining reproduced execution warning and any newly exposed local Clippy findings.

Derived/current-state files that may need updates after source cleanup:

- `docs/testing/phase-6d6/ui-binding-index.json` — regenerate only if a source-digested file changes; `execution.rs` is one of those sources.
- `CONTEXT.md` — replace the stale statement that Tauri strict Clippy is red once both gates pass.
- `docs/product/phase-6d6-physical-interruption-qualification.md` — update only the current strict-Clippy gate status.
- `docs/product/product-roadmap.md` — remove the Clippy blocker from the next-action/current-state wording while retaining the remaining evidence blockers.

Do not restructure these large modules solely for lint cleanup.

---

### Task 1: Establish the fresh current-HEAD Clippy baseline

**Files:**
- Inspect: `apps/emuchef-app/src-tauri/src/commands.rs`
- Inspect: `apps/emuchef-app/src-tauri/src/device_qualification.rs`
- Inspect: `apps/emuchef-app/src-tauri/src/adb.rs`
- Inspect: `apps/emuchef-app/src-tauri/src/execution.rs`
- Inspect: `apps/emuchef-app/src-tauri/src/sidecar.rs`

**Interfaces:**
- Consumes: current committed Tauri crate at the task-start HEAD.
- Produces: exact default-feature and `real-execution` Clippy diagnostic sets that drive Tasks 2–4.

- [ ] **Step 1: Confirm worktree boundaries before lint work**

Run:

```bash
git status --short --branch
```

Expected: current task starts from `main`; any pre-existing `.serena/**` dirt is noted and left untouched. Do not stash/reset it.

- [ ] **Step 2: Run the default strict-Clippy gate**

Run:

```bash
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --all-targets -- -D warnings
```

Expected at task start: non-zero if the known baseline debt remains. Record every lint name, file, line/symbol, and whether it is lib or test-target.

- [ ] **Step 3: Run the `real-execution` strict-Clippy gate**

Run:

```bash
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --all-targets --features real-execution -- -D warnings
```

Expected at task start: non-zero if the known baseline debt remains. Diff the diagnostic set against Step 2; shared diagnostics should be fixed once at source.

- [ ] **Step 4: Classify diagnostics before editing**

Use the actual Clippy messages as authority. Group them into:

```text
A. Mechanical value/reference/iterator/test idioms
B. Async poll/readiness idioms in adb.rs / sidecar.rs
C. Execution-specific diagnostic(s)
D. Newly exposed warnings after A-C are fixed
```

Do not make speculative refactors beyond the diagnostics.

---

### Task 2: Clean qualification and command idioms

**Files:**
- Modify: `apps/emuchef-app/src-tauri/src/device_qualification.rs`
- Modify only if its fresh diagnostic remains: `apps/emuchef-app/src-tauri/src/commands.rs`
- Test: existing unit tests in those modules

**Interfaces:**
- Consumes: `DeviceQualificationState`, `CapabilityOutcome`, `QualificationContextKey`, `RootQualificationKey`, `SessionHandles` with their current signatures.
- Produces: behavior-identical qualification/command code with the corresponding Clippy diagnostics removed.

- [ ] **Step 1: Convert repeated capability scans to direct membership checks when Clippy reports `manual_contains`**

Current pattern:

```rust
let state = if capabilities
    .iter()
    .any(|capability| *capability == CapabilityOutcome::Unsupported)
{
    DeviceQualificationState::Unsupported
} else if capabilities
    .iter()
    .any(|capability| *capability == CapabilityOutcome::Unknown)
{
    DeviceQualificationState::InsufficientlyQualified
} else {
    DeviceQualificationState::Supported
};
```

Use the behavior-equivalent form:

```rust
let state = if capabilities.contains(&CapabilityOutcome::Unsupported) {
    DeviceQualificationState::Unsupported
} else if capabilities.contains(&CapabilityOutcome::Unknown) {
    DeviceQualificationState::InsufficientlyQualified
} else {
    DeviceQualificationState::Supported
};
```

Do not alter classification precedence: explicit `Unsupported` must still win over `Unknown`.

- [ ] **Step 2: Apply only Clippy-proven needless-reference/borrow simplifications**

For example, if the fresh diagnostic identifies an already-borrowed `&AppState` passed as `&state`, change only:

```rust
let adb_path = current_adb_path(&state)?;
```

to:

```rust
let adb_path = current_adb_path(state)?;
```

Likewise, if a `QualificationContextKey` is already borrowed, remove only the redundant extra borrow rather than changing the helper signature.

- [ ] **Step 3: Replace cloned singleton slices with `std::slice::from_ref` when reported**

Current test idiom:

```rust
let result = classify_complete(true, 8, 13, &[supported.clone()]);
```

Use:

```rust
let result = classify_complete(true, 8, 13, std::slice::from_ref(&supported));
```

Preserve later mutation by keeping the owned `supported` value available.

- [ ] **Step 4: Apply the fresh `commands.rs` suggestion only if it is still emitted**

Do not rewrite command flow preemptively. The accepted shape of root-review invalidation remains:

```rust
if let Some(device_handle) = invalidation.device_handle.as_deref() {
    handles.invalidate_reviews_for_device(device_handle, "root_qualification_changed");
}
```

If Clippy flags a different exact expression in this file, make the smallest equivalent local rewrite and retain the same call ordering and error strings.

- [ ] **Step 5: Run focused qualification/command tests**

Run:

```bash
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  device_qualification::tests
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  commands::tests
```

Expected: all selected tests pass; do not run ignored physical tests.

- [ ] **Step 6: Rerun both strict-Clippy gates and inspect what remains**

Run both commands from Task 1. Expected: Task 2 diagnostics are gone; remaining diagnostics are limited to other source locations or newly exposed warnings.

- [ ] **Step 7: Commit this independently reviewable mechanical cleanup**

```bash
git add \
  apps/emuchef-app/src-tauri/src/device_qualification.rs \
  apps/emuchef-app/src-tauri/src/commands.rs
git commit -m "Fix Tauri qualification Clippy warnings"
```

If `commands.rs` was not changed, omit it from `git add`. Never stage `.serena/**`.

---

### Task 3: Clean bounded async polling without changing lifecycle semantics

**Files:**
- Modify: `apps/emuchef-app/src-tauri/src/adb.rs`
- Modify: `apps/emuchef-app/src-tauri/src/sidecar.rs`
- Test: existing process/sidecar lifecycle tests in those modules

**Interfaces:**
- Consumes: `Timer`, pinned futures, existing `ProcessFailure`, `SidecarCleanup`, `REQUEST_TIMEOUT`, `PROCESS_CLEANUP_TIMEOUT`.
- Produces: identical timeout and cleanup decisions with Clippy-clean readiness polling.

- [ ] **Step 1: Replace only redundant `Poll::Ready(_)` pattern checks**

Where Clippy reports redundant pattern matching such as:

```rust
if let std::task::Poll::Ready(_) = timer.as_mut().poll(context) {
    return std::task::Poll::Ready(Err(ProcessFailure::CleanupUncertain));
}
```

use:

```rust
if timer.as_mut().poll(context).is_ready() {
    return std::task::Poll::Ready(Err(ProcessFailure::CleanupUncertain));
}
```

Apply the same shape to the timeout branch in `settle_process_status`, `run_process`, and the sidecar request timeout closure when those are the emitted diagnostics.

- [ ] **Step 2: Preserve poll ordering exactly**

For `adb.rs`, keep output/status polling before timeout polling exactly as today. For `sidecar.rs`, keep exchange polling before timer polling. Do not replace the custom polling with `race`, `select`, detached tasks, or a new timer abstraction.

The intended shape remains:

```rust
poll_fn(|context| {
    if let std::task::Poll::Ready(result) = exchange.as_mut().poll(context) {
        return std::task::Poll::Ready(result);
    }
    if timer.as_mut().poll(context).is_ready() {
        return std::task::Poll::Ready(Err(
            "Rust runtime response timed out.".to_string(),
        ));
    }
    std::task::Poll::Pending
})
.await
```

- [ ] **Step 3: Run focused ADB process lifecycle tests**

Run:

```bash
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  adb::tests
```

Expected: all non-ignored ADB tests pass; ignored timeout/output helper bodies are not manually invoked.

- [ ] **Step 4: Run focused sidecar lifecycle tests**

Run:

```bash
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  sidecar::tests
```

Expected: all selected sidecar tests pass, including runtime-session-loss and cleanup ownership assertions.

- [ ] **Step 5: Rerun both strict-Clippy gates**

Run both Task 1 Clippy commands. Expected: no remaining diagnostic in the polling sites; capture any newly exposed warnings for Task 4.

- [ ] **Step 6: Commit the lifecycle-preserving lint cleanup**

```bash
git add \
  apps/emuchef-app/src-tauri/src/adb.rs \
  apps/emuchef-app/src-tauri/src/sidecar.rs
git commit -m "Fix Tauri process polling Clippy warnings"
```

Never stage `.serena/**`.

---

### Task 4: Clear execution and all newly exposed Tauri warnings

**Files:**
- Modify: `apps/emuchef-app/src-tauri/src/execution.rs`
- Modify as dictated by fresh Clippy only: other files under `apps/emuchef-app/src-tauri/src/`
- Regenerate if `execution.rs` changes: `docs/testing/phase-6d6/ui-binding-index.json`
- Test: existing Tauri tests under both feature sets

**Interfaces:**
- Consumes: current real/simulated execution projection and terminal-policy contracts.
- Produces: zero-warning strict Clippy under both feature sets without altering serialized execution/UI-smoke behavior.

- [ ] **Step 1: Fix the reproduced execution diagnostic using its exact current Clippy suggestion as the starting point**

Do not refactor `execution.rs` broadly. Preserve public projection shape, error strings, `terminalPolicy`, cancellation guidance, and real-execution authority behavior.

If the warning is a local redundant borrow/closure/pattern, use the direct equivalent. For example:

```rust
// before, when Clippy proves the closure is redundant
.map(|value| helper(value))

// after
.map(helper)
```

Only apply this form when the current diagnostic points to that exact expression.

- [ ] **Step 2: Rerun default strict Clippy immediately**

```bash
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --all-targets -- -D warnings
```

If another warning appears, fix it in the same pass with the smallest behavior-preserving source change. Repeat until exit 0.

- [ ] **Step 3: Rerun `real-execution` strict Clippy and clear feature-only warnings**

```bash
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --all-targets --features real-execution -- -D warnings
```

Repeat fix → rerun until exit 0. Do not stop after matching the original 21 findings; this task ends only at zero denied warnings for both commands.

- [ ] **Step 4: Run Tauri tests under both feature sets before regenerating derived metadata**

```bash
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --features real-execution
```

Expected: zero failures; only pre-existing ignored tests remain ignored.

- [ ] **Step 5: Regenerate the UI binding index if any source-digested file changed**

Because `execution.rs` is a declared source digest, if it changed run:

```bash
node tools/phase-6d6-evidence.mjs --regenerate-ui-binding-index
node tools/phase-6d6-evidence.mjs
```

Expected: regeneration succeeds and normal validation reports the evidence contract valid but still incomplete. Inspect the index diff: only source/self digest fields may change unless the accepted physical evidence set itself changed, which this task forbids.

Do not hand-edit `ui-binding-index.json`.

- [ ] **Step 6: Verify no evidence/provenance files changed**

Run:

```bash
git diff --name-only
```

Reject any task-produced changes under:

```text
docs/testing/phase-6d6/evidence/
docs/testing/phase-6d6/scenario-manifest.json
docs/testing/phase-6d6/evidence-schema.json
.serena/
```

The pre-existing `.serena` deletion may remain visible but must not be modified or staged by this task.

- [ ] **Step 7: Commit the remaining source cleanup and derived index update**

Stage only files actually changed by this task, for example:

```bash
git add \
  apps/emuchef-app/src-tauri/src/execution.rs \
  docs/testing/phase-6d6/ui-binding-index.json
git commit -m "Finish Tauri strict Clippy cleanup"
```

Include any additional Tauri Rust files only if fresh Clippy required them. Never stage `.serena/**`.

---

### Task 5: Update current-state documentation and run the complete gate matrix

**Files:**
- Modify: `CONTEXT.md`
- Modify: `docs/product/phase-6d6-physical-interruption-qualification.md`
- Modify: `docs/product/product-roadmap.md`
- Verify: all files changed in Tasks 2–4

**Interfaces:**
- Consumes: verified zero-warning Tauri Clippy state.
- Produces: truthful current-state documentation and final automated verification evidence.

- [ ] **Step 1: Replace only stale Clippy-blocker wording**

Update the three current-state docs so they say:

```text
- backend strict Clippy passes;
- Tauri strict Clippy now passes under both default and real-execution feature sets;
- the automated Clippy blocker is cleared;
- Phase 6D remains In progress because identity_replacement, host-sleep physical repetitions, and both ui_smoke_composite repetitions remain missing;
- manual UI-smoke qualification remains deferred until the required compatible host-sleep physical binding exists and the operator chooses to perform the deferred manual work;
- Phase 6E remains Planned.
```

Remove statements that the current Tauri gate is red or that the next priority is to clear Clippy. Do not rewrite historical evidence descriptions stating that the diagnostics previously reproduced at `b8bf14a`.

- [ ] **Step 2: Run formatting and both Tauri check/test/Clippy matrices**

```bash
cargo fmt --check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml

cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --all-targets -- -D warnings

cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --features real-execution
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --features real-execution
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --all-targets --features real-execution -- -D warnings
```

Expected: every command exits 0; record observed test counts rather than hard-coding them.

- [ ] **Step 3: Reconfirm backend strict Clippy**

```bash
cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  --all-targets --all-features -- -D warnings
```

Expected: exit 0.

- [ ] **Step 4: Run the Phase 6D.6 Node contract gates**

```bash
node tools/phase-6d6-evidence.mjs
node --test tools/phase-6d6-evidence.test.mjs
node --test tools/phase-6d6-result.test.mjs
```

Expected: validator exits 0 and truthfully reports the remaining missing physical/UI-smoke repetitions; both Node test suites pass.

- [ ] **Step 5: Run the frontend protection matrix**

```bash
npm --prefix apps/emuchef-app test
npm --prefix apps/emuchef-app run test:security
npm --prefix apps/emuchef-app run typecheck
npm --prefix apps/emuchef-app run lint
npm --prefix apps/emuchef-app run build
```

Expected: all commands exit 0.

- [ ] **Step 6: Confirm final diff boundaries**

Run:

```bash
git status --short
git diff --check
git diff --name-only HEAD~3..HEAD
```

Inspect the actual task commits rather than relying solely on `HEAD~3` if the number of commits differs. Confirm `.serena/**` was never staged/committed and no physical/manual evidence changed.

- [ ] **Step 7: Commit truthful current-state documentation**

```bash
git add \
  CONTEXT.md \
  docs/product/phase-6d6-physical-interruption-qualification.md \
  docs/product/product-roadmap.md
git commit -m "Record cleared Tauri Clippy gate"
```

- [ ] **Step 8: Produce the final implementation result**

Report exact observed outcomes for:

```text
- default Tauri clippy -D warnings
- real-execution Tauri clippy -D warnings
- Tauri default tests
- Tauri real-execution tests
- backend strict Clippy
- Node evidence/RESULT tests
- frontend test/security/typecheck/lint/build
- binding-index regeneration/validation if execution.rs changed
- final changed-file set
```

Final disposition must state that the Tauri strict-Clippy blocker is cleared only if both strict commands actually exit 0. Phase 6D remains In progress and Phase 6E remains Planned; no physical/manual qualification is counted by this cleanup.
