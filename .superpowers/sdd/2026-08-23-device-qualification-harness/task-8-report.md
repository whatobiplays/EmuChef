# Task 8 Review-Fix Report

Date: 2026-08-23
Worktree: `device-qualification-harness`
Base Task 8 commit: `9652d928538f4250916dc14dcc825735cdf16543`

## 1. Result

The bounded review-fix round is complete. Qualification repository access is now lazy and optional, disabled builds return a sanitized unavailable projection without initializing Node or repository state, and trusted source/build checks fail closed. Registration leaves the trusted lifecycle dirty until a clean commit and rebuild are observed. Candidate previews recompute promotability from current source state, and repository filesystem work, Node status, candidate projection, and canonical mutation share one operation gate.

The production EmuChef workflow remains the system under test. React receives only opaque handles and plans plus sanitized DTOs; `tools/device-qualification.mjs` remains the sole canonical schema, digest, identity, mutation, and projection authority. No physical device was used and no physical qualification or evidence was created.

## 2. TDD evidence

The review fixes followed the required RED/GREEN sequence.

### RED 1: missing lazy provider, source state, and command seams

Before implementation, the focused Rust command failed at compile time:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
FAIL, exit 101
unresolved/missing: QualificationRepositoryProvider,
QualificationObservationSource, QualificationSourceState,
qualification_mode_status, and capture_target_registration_payload_from
```

### GREEN 1

After adding the lazy provider, fail-closed status projection, trusted source checks, observation seam, strict fact handling, and operation gate:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
PASS: 14 tests
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_repository
PASS: 27 tests
```

### RED 2: command-boundary lifecycle regression

The added registration lifecycle test was then run before its command helper/provider injection was implemented:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
FAIL, exit 101
missing: QualificationRepositoryProvider::for_test and
register_qualification_target_with_repository
```

### GREEN 2

The command-level registration guard and test provider were implemented and verified:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
PASS: 15 tests
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_repository
PASS: 27 tests
```

The focused tests cover disabled Node suppression, sanitized unavailable status, production observation order, strict manufacturer/model/firmware types, numeric Android-version normalization, every root outcome, stale source/build classification, lifecycle blocking, candidate immutability, and registration/discard/status serialization.

### Review fix round 2 RED: exported Tauri command boundary

The new tests were then run through the exported command functions themselves. The first run failed because Tauri's managed-state test module was not enabled and the injected call counter assertion used the wrong type:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
FAIL, exit 101
tauri::test was unavailable and the registration call counter had no len method
```

After the test dependency was enabled and the harness assertion corrected, the tests reached the exported commands and failed for the intended missing seam:

```text
FAIL: 17 passed, 2 failed
exported registration and discard stopped at qualification_mode_disabled
```

### Review fix round 2 GREEN

The provider now supplies a build identity only under `cfg(test)` so the exported commands can run against injected state without weakening production gates. The complete exported boundary suite passes:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
PASS: 19 tests
```

These tests directly call `get_device_qualification_mode_status`, `create_qualification_target_candidate`, `register_qualification_target`, and `discard_qualification_candidate`. The registration test calls the exported registration command twice and verifies the second action returns sanitized `qualification_source_changed` after the first successful registration.

## 3. Review fixes implemented

1. `QualificationRepositoryProvider` uses `OnceLock<Option<_>>`; ordinary and packaged builds do not resolve or canonicalize a source checkout during startup, and unavailable repositories produce empty sanitized status.
2. Registration requires trusted embedded build identity, matching clean Git `HEAD`, and a clean tracked worktree. Successful canonical registration marks trusted lifecycle state dirty; subsequent recordable operations fail until a clean commit/rebuild identity is present. Stale candidates remain inspectable and explicitly discardable.
3. A repository-wide operation gate serializes candidate filesystem work, Node `--describe`, candidate status/projection, discard, and canonical registration. The concurrency regression proves status and discard cannot observe or delete a candidate during status or matrix replacement.
4. Candidate promotability is recomputed from current source/build facts on load/list and preserves the stored target values and provenance when stale.
5. Manufacturer, model, and firmware observations require non-empty strings. Only numeric Android version values are normalized. Granted and denied root checks map to rooted/non-root; unavailable, failed, and identity-mismatched root checks reject the candidate.
6. `CONTEXT.md` documents the lazy provider, trusted lifecycle, serialization gate, stale classification, and strict observation semantics.
7. The exported-command tests use a real Tauri test app with injected `AppState` and repository provider state. The test-only provider identity override is unavailable to ordinary and packaged builds.

## 4. Validation

All validation completed without connected hardware:

1. `rtk cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check` passed.
2. Focused mode tests passed: 19, including direct exported-command calls.
3. Focused repository tests passed: 27.
4. Full default Tauri Rust tests passed: 306 passed, 2 ignored.
5. `rtk cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution` passed with 0 errors; two existing dead-code warnings remain for future run-record repository APIs.
6. App typecheck and lint passed.
7. Full app tests passed: 82 logic tests and 76 Vitest tests. A parallel invocation produced one non-reproducible call-count assertion; the isolated file and complete suite both passed on rerun.
8. Focused opaque API Vitest tests passed: 2.
9. App security tests passed: 28 policy tests and 12 Python-retirement tests.
10. App production build passed.
11. Canonical Node qualification tests passed: 72.
12. `rtk node tools/device-qualification.mjs --check` and `rtk make device-qualification-check` passed.
13. `rtk git diff --check` passed. New Task 8 source/test additions contain no active phase/slice identifiers, React qualification inputs remain opaque, and no qualification path authority is exposed.

## 5. Review-fix-round-2 changed files

1. `apps/emuchef-app/src-tauri/Cargo.toml`
2. `apps/emuchef-app/src-tauri/src/qualification_mode.rs`
3. `apps/emuchef-app/src-tauri/src/qualification_repository.rs`
4. `.superpowers/sdd/2026-08-23-device-qualification-harness/task-8-report.md`

`CONTEXT.md` was not changed in this round because the changes are test-only infrastructure and do not alter production semantics. No `.chatgpt` files, SDD ledgers, canonical qualification tool/schema files, physical-device evidence, or external systems were modified.
