# Qualification Session Device Reassociation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a restored qualification session validate and associate a newly discovered process-local device handle without changing its persisted historical handle.

**Architecture:** `SessionHandles` owns the current-process session-to-device association. Qualification refresh establishes that association only after trusted production observations match every immutable target fact. React keeps qualification intent locking separate from device-selection locking.

**Tech Stack:** Rust, Tauri 2, React 19, TypeScript 6, Vitest, Testing Library

## Global Constraints

- Do not rewrite the persisted qualification-session `deviceHandle`.
- Do not create a qualification-only discovery, probe, selection, review, or execution path.
- Do not weaken target fact comparison or accept a device by model or display label alone.
- Do not bind review or execution before successful current-process association.
- Do not create or alter physical evidence, targets, recipes, profiles, or matrix state.
- Do not commit or push.

---

### Task 1: Process-local qualification device authority

**Files:**

- Modify: `apps/emuchef-app/src-tauri/src/handles.rs`
- Modify: `apps/emuchef-app/src-tauri/src/qualification_mode.rs`

**Interfaces:**

- Produces: `SessionHandles::qualification_session_device_handle(&self, session_handle: &str) -> Option<&str>`
- Produces: `SessionHandles::associate_qualification_session_device(&mut self, session_handle: &str, device_handle: &str) -> bool`
- Consumes: trusted `QualificationDeviceObservation` values from `observe_device_from_source`

- [ ] **Step 1: Add failing `SessionHandles` tests**

Add tests proving a session can claim one device, repeat the same claim, cannot
switch to another handle, and loses the association when runtime authority is
invalidated.

```rust
#[test]
fn qualification_session_device_associations_are_process_local_and_single_device() {
    let mut handles = SessionHandles::default();

    assert!(handles.associate_qualification_session_device("session-one", "device-live"));
    assert_eq!(
        handles.qualification_session_device_handle("session-one"),
        Some("device-live")
    );
    assert!(handles.associate_qualification_session_device("session-one", "device-live"));
    assert!(!handles.associate_qualification_session_device("session-one", "device-other"));

    handles.invalidate_runtime_authority_preserving_identities();
    assert_eq!(handles.qualification_session_device_handle("session-one"), None);
}
```

- [ ] **Step 2: Run the handle test and verify RED**

Run:

```sh
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_session_device_associations_are_process_local_and_single_device
```

Expected: compilation fails because the association methods do not exist.

- [ ] **Step 3: Implement the bounded association map**

Add `qualification_device_by_session: HashMap<String, String>` to
`SessionHandles`. Implement the two methods above with insert-if-empty,
same-value idempotence, and different-value rejection. Clear the map from
`invalidate_runtime_authority_preserving_identities`.

- [ ] **Step 4: Run the handle test and verify GREEN**

Run the command from Step 2. Expected: one passing test.

- [ ] **Step 5: Add failing qualification session comparison tests**

Add tests that distinguish historical handle checking from resumed target
checking:

```rust
#[test]
fn resumed_observation_accepts_a_new_handle_only_when_all_target_facts_match() {
    let mut session = test_session();
    let mut observation = test_observation();
    observation.device_identity = "device-new-process".to_string();

    session.observe_matching_target(observation);

    assert_eq!(session.run_validity(), RunValidity::Valid);
    assert_eq!(session.to_persisted().device_handle, "device-test");
}

#[test]
fn resumed_observation_invalidates_material_target_drift() {
    let mut session = test_session();
    let mut observation = test_observation();
    observation.device_identity = "device-new-process".to_string();
    observation.firmware_build = "different-build".to_string();

    session.observe_matching_target(observation);

    assert_eq!(session.run_validity(), RunValidity::Invalid);
    assert_eq!(
        session.invalid_reason().as_deref(),
        Some("firmware_build_changed")
    );
}
```

- [ ] **Step 6: Run the resumed comparison tests and verify RED**

Run:

```sh
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml resumed_observation
```

Expected: compilation fails because `observe_matching_target` does not exist.

- [ ] **Step 7: Separate historical handle validation from target fact validation**

Keep `observe_matching_device` as the session-start check. Extract the existing
profile, manufacturer, model, Android version/API, ABI/SoC, firmware, and root
comparisons into `observe_matching_target`, which deliberately does not compare
the process-local handle.

- [ ] **Step 8: Run the resumed comparison tests and verify GREEN**

Run the command from Step 6. Expected: both tests pass.

- [ ] **Step 9: Add failing refresh and binding tests**

Exercise the qualification refresh helper with a restored session and fake
production observation source. Assert that matching material facts establish
the supplied live association while keeping `session.to_persisted().device_handle`
unchanged. Assert that material drift invalidates the session and leaves the
association absent. Extend review and execution matching tests so they require
the associated live handle and reject a missing or different association.

- [ ] **Step 10: Run the focused Rust tests and verify RED**

Run:

```sh
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode::tests
```

Expected: the new restored-session and binding assertions fail against stale
persisted-handle comparisons.

- [ ] **Step 11: Route commands through the runtime association**

After a new session is successfully persisted, associate its session handle with
the request device handle. During refresh:

```rust
let associated_device_handle = state
    .handles
    .lock()
    .map_err(|_| safe_qualification_error("qualification_refresh_failed"))?
    .qualification_session_device_handle(&session_handle)
    .map(str::to_string);
```

Reject a different already-associated handle through
`DeviceIdentityChanged`. Otherwise observe the requested live handle, call
`observe_matching_target`, persist the canonical validity result, and establish
the association only when the session remains valid. If a concurrent different
claim wins, invalidate and persist through the same canonical path.

Change review, execution, and finalization matching to require the associated
handle and compare production bindings with it. Pass that handle into
`review_matches_session`; do not fall back to the persisted handle.

- [ ] **Step 12: Run focused Rust tests and verify GREEN**

Run the command from Step 10. Expected: all qualification-mode tests pass.

### Task 2: Restored-session hook coordination

**Files:**

- Modify: `apps/emuchef-app/src/useDeviceQualificationMode.ts`
- Modify: `apps/emuchef-app/tests/useDeviceQualificationMode.dom.test.tsx`

**Interfaces:**

- Produces: `DeviceQualificationModeController.deviceSelectionLocked: boolean`
- Consumes: ordinary workflow `deviceHandle`, probed `facts`, and `devicePlan`

- [ ] **Step 1: Add failing hook tests**

Extend the harness to render `deviceSelectionLocked` and `runValidity`.
Add cases proving:

1. A restored session has an intent lock but no device-selection lock.
2. Selecting and probing `device-new-process` calls
   `refreshQualificationSession("session-opaque", "device-new-process")`.
3. A valid response preserves the original session, plan, recipes, checkpoint,
   and valid run state without calling `beginQualificationSession`.
4. An invalid response exposes canonical invalid state and still does not begin
   a new session.
5. A session begun in the current process keeps device selection locked.

- [ ] **Step 2: Run hook tests and verify RED**

Run:

```sh
cd apps/emuchef-app
rtk npm exec vitest -- run --config tests/vitest.config.ts tests/useDeviceQualificationMode.dom.test.tsx
```

Expected: `deviceSelectionLocked` is absent and restored workflow facts do not
start refresh.

- [ ] **Step 3: Implement restored association coordination**

Add `deviceSelectionLocked` to the controller. It is true only when a session
has a non-null `sessionDeviceHandleRef`.

In the refresh effect, when a session exists without a current-process handle,
wait until the ordinary workflow has both a selected handle and probed facts.
Invoke the existing refresh API with that handle, then set the ref only when the
backend returns a valid session after establishing its trusted association. Keep
`intentLock` derived from every session so device-plan and recipe locking does
not change.

- [ ] **Step 4: Run hook tests and verify GREEN**

Run the command from Step 2. Expected: all hook tests pass.

### Task 3: Connect row selection policy

**Files:**

- Modify: `apps/emuchef-app/src/App.tsx`
- Modify: `apps/emuchef-app/tests/App.dom.test.tsx`

**Interfaces:**

- Consumes: `qualification.intentLock`
- Consumes: `qualification.deviceSelectionLocked`

- [ ] **Step 1: Add failing DOM tests**

Add a restored-session fixture with a historical handle hidden from the status
DTO and a currently discovered connected device with a fresh handle. Assert the
Connect row is enabled and selecting it calls the ordinary production probe
path. Add an active same-process fixture asserting its row remains disabled with
an explicit accessible reason. Retain or add a no-session assertion proving
ordinary selection remains enabled.

- [ ] **Step 2: Run App DOM tests and verify RED**

Run:

```sh
cd apps/emuchef-app
rtk npm exec vitest -- run --config tests/vitest.config.ts tests/App.dom.test.tsx
```

Expected: the restored-session connected row is disabled by `intentLock`.

- [ ] **Step 3: Split device selection from intent locking**

Use `qualification.deviceSelectionLocked` in the `selectDevice` guard and the
Connect-row disabled predicate. Continue using `qualificationLocksIntent` for
plan, recipe, saved-configuration, repair, and runtime-restart policy.

Include the active session lock in `aria-describedby` and render:

```tsx
{qualification.deviceSelectionLocked && (
  <p className="disabled-reason" id={stableDomId("device-reason", device.deviceHandle)}>
    This qualification session is already associated with its current device.
  </p>
)}
```

Compose this with existing device-state reasons so each row has one stable
reason element.

- [ ] **Step 4: Run App DOM tests and verify GREEN**

Run the command from Step 2. Expected: all App DOM tests pass.

### Task 4: Documentation and validation

**Files:**

- Modify: `CONTEXT.md`
- Review: every changed file

- [ ] **Step 1: Update current product facts**

Document that the persisted session handle is historical capture metadata,
`SessionHandles` owns the validated current-process association, restored
sessions allow ordinary selection before refresh, and review/execution binding
requires that association.

- [ ] **Step 2: Run focused affected tests**

```sh
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode::tests
cd apps/emuchef-app
rtk npm exec vitest -- run --config tests/vitest.config.ts tests/useDeviceQualificationMode.dom.test.tsx tests/App.dom.test.tsx
```

- [ ] **Step 3: Run full requested validation**

```sh
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml
rtk npm --prefix apps/emuchef-app run test
rtk npm --prefix apps/emuchef-app run typecheck
rtk npm --prefix apps/emuchef-app run lint
rtk make test
rtk git diff --check
```

- [ ] **Step 4: Audit the patch**

Run the code scanner on changed Rust and TypeScript files, inspect
`rtk git diff --stat`, `rtk git diff`, and `rtk git status --short`, and
confirm no target, recipe, profile, evidence, matrix, runtime session, or Git
history file changed.
