# Task 8 Implementation Report

Date: 2026-08-23
Worktree: `device-qualification-harness`

## 1. Result

Task 8 is implemented and committed locally. The change adds the development-only target-registration surface without replacing the production EmuChef workflow. Target facts come from the existing probe, match, qualification, and root-check boundaries. Node remains the authority for schema validation, target IDs, canonical mutation, and repository projection.

No physical device was used. No physical target or evidence record was created.

## 2. TDD evidence

The required Rust serialization test was written against the camelCase contract before the serialization implementation was corrected.

RED:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
FAIL, exit 101
camelCase qualification DTO should deserialize: missing field `production_recipes`
```

The TypeScript contract typecheck passed after the interfaces were added. GREEN was then verified with:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_mode
cargo test: 9 passed
```

The Rust tests cover camelCase round-tripping, disabled-mode status, sanitized mode guards, capability projection, production plan/profile resolution, schema-v2 target projection, stored provenance preservation, and fail-closed fact handling. TypeScript tests cover DTO shape and the four opaque API commands.

## 3. Implementation

1. Added `qualification_mode.rs` with:

   - strict camelCase Tauri DTOs for build identity, workflows, target summaries, provenance previews, candidate summaries, and mode status;
   - four-gate mode status with no Node invocation when disabled;
   - target capture from production probe, match, qualification, and root-check helpers;
   - exact capability mapping for APK installation and shared-storage writes;
   - schema-v2 fact provenance for observations, root checks, and USB connection attestation;
   - opaque candidate persistence and read-only preview projection;
   - guarded registration delegation to `--register-target`; and
   - validated opaque candidate discard.

2. Extended `commands.rs` and the existing root-qualification module so public product commands and qualification capture share the same observation authorities.

3. Registered the repository state and four Tauri commands in `lib.rs`.

4. Added the frontend DTO contract and opaque API wrappers. React receives no repository, candidate, evidence, executable, or process paths.

5. Added the required project-context documentation for the target-registration semantics.

## 4. Validation

All validation completed without connected hardware:

1. `cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check` passed.
2. Focused Rust qualification tests passed: 9 tests.
3. Repository candidate tests passed: 24 tests.
4. Full default Tauri Rust tests passed: 293 passed, 2 ignored.
5. `cargo check --features real-execution` passed with 0 errors. Two existing dead-code warnings remain in the future run-record repository API.
6. App typecheck and lint passed.
7. Full app tests passed: 82 logic tests and 76 Vitest tests.
8. The focused opaque API Vitest suite passed: 2 tests.
9. App security tests passed: 28 policy tests and 12 Python-retirement tests.
10. App production build passed.
11. Canonical Node qualification tests passed: 72 tests.
12. `node tools/device-qualification.mjs --check` and `make device-qualification-check` passed.
13. `git diff --check` passed. New Task 8 source and tests contain no phase/slice terminology, and the React source contains none of the forbidden path-authority fields.

The focused Vitest command was run from `apps/emuchef-app`, because Vitest resolves `tests/vitest.config.ts` relative to its working directory.

## 5. Changed files

1. `CONTEXT.md`
2. `.superpowers/sdd/2026-08-23-device-qualification-harness/task-8-report.md`
3. `apps/emuchef-app/src-tauri/src/qualification_mode.rs`
4. `apps/emuchef-app/src-tauri/src/commands.rs`
5. `apps/emuchef-app/src-tauri/src/device_qualification.rs`
6. `apps/emuchef-app/src-tauri/src/lib.rs`
7. `apps/emuchef-app/src/types.ts`
8. `apps/emuchef-app/src/api.ts`
9. `apps/emuchef-app/tests/deviceQualificationContract.test.ts`
10. `apps/emuchef-app/tests/deviceQualificationApi.dom.test.tsx`
