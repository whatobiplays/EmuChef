# Task 7 Report: Trusted Candidate Persistence and Bounded Node Invocation

## Outcome

Task 7 and the bounded review fix rounds are implemented in the isolated
worktree. Rust now stores each non-authoritative candidate as a strict local
envelope around an opaque Node-owned payload, binds optional report metadata to
exact bytes, stages complete candidate directories under the fixed runtime
root, and refuses symlinked roots, directories, or files. Publication uses the
macOS OS-level no-replace rename primitive, so a destination created after the
absence probe remains untouched while owned staging is removed. Promotion
checks candidate build identity before invoking Node, and Node independently
guards candidate paths and emits strict operation-result envelopes. The
canonical Node tool remains responsible for semantic validation,
canonicalization, digests, promotion, projection, and matrix rendering.

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

The review-fix RED run used the same focused command after adding the new
regressions. It failed to compile with four expected missing-symbol/setup
errors for the new embedded-build test constructor and staged-publish helper
(plus the test-only digest import), confirming that the new behaviors were not
being exercised by the pre-fix implementation.

The round-2 RED run added the deterministic competing-destination regression
before changing publication:

```text
rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml competing_destination_after_absence_check_is_not_replaced_and_staging_is_cleaned
```

It failed with the expected `cannot find function
publish_staged_candidate_with_hook` compilation error, confirming that the
interleaving behavior was not present in the replacement-capable publisher.

### GREEN

The original implementation's focused command passed with 13 tests. After the
first review fixes and their regressions, the focused command passed with 23
tests. After the round-2 no-replace publication fix and its regression, the
same focused command passes:

```text
cargo test: 24 passed, 263 filtered out (2 suites, 0.16s)
```

The fix-round tests cover strict candidate and operation envelopes, wrong-shaped
valid JSON, missing/stale build identity with zero runner calls, symlinked root,
candidate-directory, candidate-file, and report-file rejection, report
metadata/byte mismatch, atomic staging residue, duplicate-handle preservation,
restart recovery, the competing-destination interleaving with staging cleanup,
and the existing bounded invocation contract.

## Validation

| Check | Result |
| --- | --- |
| `cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check` | PASS |
| `cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml qualification_repository` | PASS: 24 tests |
| `cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution` | PASS |
| Full default Tauri tests | PASS: 285 passed, 2 ignored |
| Feature-enabled Tauri tests excluding the stale historical UI-smoke module | PASS: 277 passed, 2 ignored, 24 filtered |
| Unfiltered feature-enabled Tauri tests | 289 passed, 12 pre-existing UI-smoke failures, 2 ignored |
| `node --check tools/device-qualification.mjs` | PASS |
| `node --test tools/device-qualification.test.mjs` | PASS: 72 tests |
| `node tools/device-qualification.mjs --check` | PASS |
| `make device-qualification-check` | PASS: Node suite and repository check |
| `git diff --check` | PASS |

Strict feature-enabled Clippy was also run. The Task 7 findings were fixed;
the remaining three `-D warnings` diagnostics are existing needless-borrow
findings in `apps/emuchef-app/src-tauri/src/commands.rs`.

The unfiltered feature-enabled Tauri suite remains blocked by 12 pre-existing
UI-smoke failures (`289 passed, 12 failed, 2 ignored`). The checked-in UI
binding index records
`execution.rs` as `sha256:9ea1...`, while the current HEAD source hashes to
`sha256:e6c092...`; the latest Task 6 commit changed that production file
without refreshing the historical index. This task does not alter that
unrelated evidence/index contract.

## Changed files

- `apps/emuchef-app/src-tauri/src/qualification_repository.rs`
- `tools/device-qualification.mjs`
- `tools/device-qualification.test.mjs`
- `CONTEXT.md`
- this report

Round 2 changed only `apps/emuchef-app/src-tauri/src/qualification_repository.rs`
and this report.
