# Tauri Strict-Clippy Cleanup Design

## Purpose

Clear the Phase 6D blocking denied-warning Clippy gate for the end-user Tauri
crate without changing runtime behavior, qualification semantics, execution
ownership, or Phase 6D.6 evidence contracts.

The cleanup covers the currently reproduced Tauri diagnostics and any
additional Clippy warnings exposed after those findings are fixed. Completion
requires the Tauri crate to pass strict Clippy under both the default feature
set and `real-execution`.

## Current state

The Phase 6D.6 UI-smoke binding/capture slice is now committed on `main` at
`7f715624342a4e43aa5686d2052d619979e4aaac`. Its final verification run
established that:

- the Tauri default test suite passed with 254 passed, 0 failed, 2 ignored;
- the Tauri `real-execution` test suite passed with 270 passed, 0 failed, 2
  ignored;
- backend strict Clippy passed;
- Tauri strict Clippy failed with 12 lib-target and 9 lib-test-target errors
  under both feature sets;
- the normalized Tauri diagnostic set reproduced identically in an isolated
  clean checkout at the prior baseline HEAD
  `b8bf14a876c0931d04d55efdfdeb42f1652cf167`, proving the UI-smoke slice did
  not introduce those diagnostics;
- the failures were concentrated in existing Tauri source including
  `commands.rs`, `device_qualification.rs`, `adb.rs`, `execution.rs`, and
  `sidecar.rs`.

The cleanup is therefore a bounded quality-gate repair, not a feature or
qualification change. Before editing, implementation must rerun both strict
Tauri Clippy commands at the current HEAD and use those fresh diagnostics as
the working set.

## Approach

Use idiomatic source fixes until both strict Tauri Clippy invocations are clean.
Each diagnostic should be resolved at its source using the smallest
behavior-preserving transformation available.

Preferred changes include:

- iterator and predicate simplification;
- removal of redundant closures, borrows, references, clones, conversions, or
  temporary bindings;
- equivalent conditional/control-flow simplification;
- equivalent future/polling expression cleanup;
- test-only assertion or fixture idiom cleanup;
- local helper/signature cleanup where callers remain behaviorally identical.

After each group of fixes, rerun strict Clippy and include newly exposed
warnings in the same cleanup until no denied warning remains under either
feature set.

## Lint policy

The gate must be cleared by improving the source rather than weakening policy.

Do not:

- remove or relax `-D warnings`;
- change Clippy configuration to disable the failing lints;
- add crate-wide or module-wide `#[allow(clippy::...)]`;
- add broad `#[allow]` or `#[expect]` annotations as a substitute for a normal
  idiomatic fix.

A narrowly scoped lint allowance is acceptable only if the construct is
intentional, a behavior-preserving idiomatic alternative is not reasonable,
and the reason is documented directly at the allowance. This is expected to be
exceptional rather than the normal cleanup mechanism.

## Behavioral invariants

The cleanup must not change observable EmuChef behavior. In particular, it
must preserve:

- executor state transitions and terminal projection semantics;
- operation deadlines, timeout classification, and host-sleep deadline-clock
  behavior;
- owned child-process lifecycle, cleanup, and failure precedence;
- ADB transport and command behavior;
- device inventory reconciliation and qualification classification;
- identity continuity and same-serial replacement protections;
- root qualification, invalidation, and requalification behavior;
- sidecar request/response framing, timeout, and cleanup behavior;
- recovery policy and authority invalidation rules;
- Phase 6D.6 UI-smoke gating, trusted binding resolution, projection, and
  capture behavior;
- public Tauri command/API contracts unless a purely mechanical signature
  simplification leaves all callers and serialized behavior equivalent.

If an automatic Clippy suggestion could change observable behavior, do not use
it verbatim. Apply a behavior-preserving alternative or leave the construct
unchanged and document a narrowly justified lint exception.

## Repository boundaries

The cleanup may modify Rust source and Rust tests under
`apps/emuchef-app/src-tauri/` as needed to reach a clean strict-Clippy result.
Documentation may be updated only if the final gate state makes an existing
current-state statement stale.

Do not modify:

- `docs/testing/phase-6d6/evidence/**/*.json`;
- `docs/testing/phase-6d6/evidence/traces/**/*.json`;
- `docs/testing/phase-6d6/evidence/ui/**`;
- `docs/testing/phase-6d6/scenario-manifest.json`;
- `docs/testing/phase-6d6/evidence-schema.json`;
- accepted physical qualification provenance;
- `.serena/**`.

The committed Phase 6D.6 UI-smoke implementation must be preserved and not
reverted, rewritten, or conflated with the lint cleanup.

Because `docs/testing/phase-6d6/ui-binding-index.json` source-digests
`apps/emuchef-app/src-tauri/src/execution.rs`, any Clippy cleanup that changes
`execution.rs` must regenerate the binding index with
`node tools/phase-6d6-evidence.mjs --regenerate-ui-binding-index` and verify the
result with the normal validator. That derived metadata update is allowed and
must never be hand-edited; accepted evidence and traces remain unchanged.

No physical qualification, host-sleep qualification, identity-replacement
qualification, or manual GUI/UI-smoke qualification is part of this task.

## Verification

First establish the fresh current-HEAD lint baseline:

```bash
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --all-targets -- -D warnings
cargo clippy --manifest-path apps/emuchef-app/src-tauri/Cargo.toml \
  --all-targets --features real-execution -- -D warnings
```

After the final source fix, run the complete Rust gate:

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

cargo clippy --manifest-path crates/emuchef-rust-backend/Cargo.toml \
  --all-targets --all-features -- -D warnings
```

Then rerun the Phase 6D.6 protection matrix:

```bash
node tools/phase-6d6-evidence.mjs
node --test tools/phase-6d6-evidence.test.mjs
node --test tools/phase-6d6-result.test.mjs

npm --prefix apps/emuchef-app test
npm --prefix apps/emuchef-app run test:security
npm --prefix apps/emuchef-app run typecheck
npm --prefix apps/emuchef-app run lint
npm --prefix apps/emuchef-app run build
```

Do not run ignored physical tests or any manual qualification path as part of
verification.

## Completion criteria

This cleanup is complete only when:

1. strict Tauri Clippy passes with zero denied warnings for the default feature
   set;
2. strict Tauri Clippy passes with zero denied warnings with
   `--features real-execution`;
3. Tauri checks and tests pass under both feature sets;
4. backend strict Clippy still passes;
5. the Phase 6D.6 Node/frontend protection matrix still passes;
6. no forbidden evidence, schema, manifest, manual qualification, or `.serena`
   state is changed.

Clearing this gate removes the current automated Clippy blocker, but it does
not complete Phase 6D. Phase 6D remains In progress until the remaining
required physical evidence and both deferred UI-smoke composite repetitions
are collected and accepted. Phase 6E remains Planned.
