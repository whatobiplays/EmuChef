# Task 7 Report: Trusted Candidate Persistence and Bounded Node Invocation

## Outcome

Task 7 is implemented in the isolated worktree. The Rust repository boundary
uses the trusted compile-time repository root, persists non-authoritative
qualification candidates under `.emuchef_runtime/qualification-candidates/`,
recovers candidates across repository instances, and exposes only the bounded
`--describe`, `--register-target <handle>`, and `--record-run <handle>` Node
operations. The canonical Node tool remains responsible for semantic
validation, canonicalization, digests, promotion, projection, and matrix
rendering.

No physical device was used or qualified. No canonical target, evidence bundle,
matrix, or external system was mutated by this task.

## TDD evidence

### RED

Command:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_repository
```

Result: expected compilation failure because the repository module and its
interfaces did not exist. The run reported 18 missing-symbol errors, including
`QualificationRepository`, `CandidateKind`, the runner trait, and the required
opaque-handle prefix.

### GREEN

The same focused command passed after the implementation:

```text
cargo test: 13 passed, 263 filtered out (2 suites, 0.01s)
```

The tests cover opaque handles, fixed roots, restart recovery, path rejection,
report-byte persistence and presence checks, stale-build inspection,
allowlisted argument vectors, candidate-kind binding, malformed tool output,
description decoding, and discard behavior.

## Validation

| Check | Result |
| --- | --- |
| `cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check` | PASS |
| `cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_repository` | PASS: 13 tests |
| `cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution` | PASS |
| Full default Tauri tests | PASS: 274 passed, 2 ignored |
| Feature-enabled Tauri tests excluding the stale historical UI-smoke module | PASS: 266 passed, 2 ignored, 24 filtered |
| `node --test tools/device-qualification.test.mjs` | PASS: 69 tests |
| `node tools/device-qualification.mjs --check` | PASS |

Strict feature-enabled Clippy was also run. The Task 7 findings were fixed;
the remaining three `-D warnings` diagnostics are existing needless-borrow
findings in `apps/emuchef-app/src-tauri/src/commands.rs`.

The unfiltered feature-enabled Tauri suite remains blocked by 12 pre-existing
UI-smoke failures. The checked-in UI binding index records
`execution.rs` as `sha256:9ea1...`, while the current HEAD source hashes to
`sha256:e6c092...`; the latest Task 6 commit changed that production file
without refreshing the historical index. This task does not alter that
unrelated evidence/index contract.

## Changed files

- `apps/emuchef-app/src-tauri/src/qualification_repository.rs`
- `apps/emuchef-app/src-tauri/src/lib.rs`
- `CONTEXT.md`
- this report

