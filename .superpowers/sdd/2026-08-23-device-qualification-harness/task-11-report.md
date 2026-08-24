# Task 11 Implementation Report

## Scope and outcome

Task 11 was implemented in the isolated worktree
`/Users/daniel/Projects/EmuChef/.worktrees/device-qualification-harness`.
The production-bound qualification workflow, security/name regression guards,
operator documentation, roadmap state, canonical matrix projection, Make/CI
validation, and current functionality documentation are updated.

The implementation adds no physical device target, physical evidence bundle,
or physical qualification claim. `tools/device-qualification.mjs` remains the
sole canonical authority for qualification validation and matrix generation.
The production EmuChef workflow remains the system under test; no
qualification-only ADB or planner/executor authority was added.

## Changed files

1. `.github/workflows/emuchef-execution-feature-matrix.yml` — renamed the active
   device-qualification validation step.
2. `CONTEXT.md` — documented the current production-bound operator semantics
   and the unqualified physical state.
3. `Makefile` — updated the device-qualification help text.
4. `apps/emuchef-app/tests/security-policy.test.mjs` — added the opaque React
   API guard and corrected the existing source-slice boundary so opaque
   qualification handles are not mistaken for launch authority.
5. `docs/manual/device-qualification-operator.md` — replaced obsolete manual
   identity instructions with the implemented nine-step production workflow.
6. `docs/product/product-roadmap.md` — records harness availability without
   claiming physical qualification; the matrix remains in progress and
   Daijisho/ES-DE remain deferred.
7. `docs/qualification/device-qualification-matrix.md` — regenerated only by
   the canonical tool with domain-oriented output.
8. `docs/testing/device-qualification/evidence/README.md` — removed the active
   Phase 6F label while preserving the empty physical-evidence boundary.
9. `tests/fixtures/device-qualification/matrix/expected-qualified-limited.md`
   — updated the synthetic matrix heading.
10. `tools/device-qualification.mjs` — updated generated matrix and CLI output
    labels without changing canonical authority or evidence semantics.
11. `tools/device-qualification.test.mjs` — added opaque API, active naming,
    runbook, Make/CI, roadmap, and generated-matrix regression guards. The
    active source scan inspects `tools/device-qualification*` while narrowly
    excluding only the two pre-existing Phase 6D.6 material-exclusion path
    literals for historical UI-smoke artifacts.
12. `.superpowers/sdd/2026-08-23-device-qualification-harness/task-11-report.md`
    — this validation and handoff record.

## TDD and focused validation

1. Red phase before implementation:
   - `rtk node --test tools/device-qualification.test.mjs` — exit 1; 70
     passed, 3 failed for the stale runbook, Make/CI naming, and active-name
     expectations.
   - `rtk npm --prefix apps/emuchef-app run test:security` — exit 1; 28
     passed, 1 failed because the pre-existing source slice incorrectly
     treated valid opaque `plan`, `reviewHandle`, and `executionHandle` fields
     as launch authority.
2. Green phase after implementation and before the final scan tightening:
   - `rtk node --test tools/device-qualification.test.mjs` — exit 0; 74
     passed, 0 failed.
   - `rtk npm --prefix apps/emuchef-app run test:security` — exit 0; 29
     passed, plus the nested runtime-retirement suite's 12 passed tests.

## Canonical matrix validation

1. `rtk node tools/device-qualification.mjs --write-matrix` — exit 0;
   `Device qualification check passed.`
2. `rtk node tools/device-qualification.mjs --check` — exit 0;
   `Device qualification check passed.`

The generated matrix reports no registered physical-device targets and makes
no physical qualification claim.

## Full validation battery

Every shell command below uses the repository-required `rtk` wrapper. Results
are exact for the commands completed in this worktree.

1. `rtk node --test tools/device-qualification.test.mjs` — exit 0; 74
   passed, 0 failed.
2. `rtk make device-qualification-check` — exit 0; the device-qualification
   Node tests and canonical `--check` completed successfully. RTK truncated
   the verbose test listing, but the process exit was 0.
3. `rtk cargo fmt --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --check`
   — exit 0; no output.
4. `rtk cargo fmt --manifest-path apps/emuchef-app/src-tauri/Cargo.toml -- --check`
   — exit 0; no output.
5. `rtk cargo check --manifest-path crates/emuchef-rust-backend/Cargo.toml --all-targets`
   — exit 0.
6. `rtk cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml` —
   exit 0; 857 passed, 17 ignored across 23 suites.
7. `rtk cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --all-targets`
   — exit 0; 0 errors and 2 existing dead-code warnings in
   `qualification_repository.rs` (`report_bytes`, `candidate_root`, and
   `list_candidates`).
8. `rtk cargo check --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --all-targets --features real-execution`
   — exit 0; 0 errors and the same 2 existing warnings.
9. `rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml` —
   exit 0; 315 passed, 2 ignored.
10. `rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution`
    — exit 101; 319 passed, 12 failed, 2 ignored. See the historical failure
    record below.
11. `rtk npm --prefix apps/emuchef-app run test` — exit 0; 83 Node logic tests
    passed and 9 Vitest files passed with 94 tests passed.
12. `rtk npm --prefix apps/emuchef-app run test:security` — exit 0; 29
    security tests and 12 runtime-retirement tests passed; runtime-retirement
    coverage was 100.00% lines, 97.92% branches, and 100.00% functions.
13. `rtk npm --prefix apps/emuchef-app run typecheck` — exit 0; `ok`.
14. `rtk npm --prefix apps/emuchef-app run lint` — exit 0; `ok`.
15. `rtk npm --prefix apps/emuchef-app run build` — exit 0; Vite transformed
    45 modules and produced the production bundle.
16. `rtk make test` — exit 0. The target rebuilt and ran the backend, Tauri,
    frontend, security, typecheck, lint, configuration-editor, and
    device-qualification checks. Its normal local cleanup removed 8,914
    ignored generated files (2.7 GiB); no tracked evidence or source was
    removed.

## Security and naming inspection

1. `rtk git grep -nE 'phase[-_]?6f|Phase6f|PHASE_6F' -- tools apps/emuchef-app/src apps/emuchef-app/src-tauri/src tests/fixtures/device-qualification docs/testing/device-qualification docs/manual/device-qualification-operator.md docs/qualification/device-qualification-matrix.md Makefile .github/workflows/emuchef-execution-feature-matrix.yml`
   — exit 1 with no output, meaning no matches.
2. `rtk git grep -nE 'candidatePath|repositoryPath|evidencePath|toolPath|executablePath' -- apps/emuchef-app/src`
   — exit 1 with no output, meaning no matches.
3. The unslop-code high-severity scans for
   `tools/device-qualification.mjs`, `tools/device-qualification.test.mjs`,
   and `apps/emuchef-app/tests/security-policy.test.mjs` each exited 0 with
   zero findings.
4. `rtk git diff --check` — exit 0; no whitespace errors.

## Known failures and incomplete checks

1. The only full-battery failure was
   `rtk cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml --features real-execution`.
   All 12 failures are in the pre-existing historical
   `apps/emuchef-app/src-tauri/src/phase6d6_ui_smoke.rs` tests; no Task 11
   implementation file is in those failures, and the historical UI-smoke
   source/evidence was not rewritten. The exact failing tests and reported
   locations were:

   - `candidate_labels_are_unique_within_subcase_and_sanitized` — line 2096;
     expected a cancellation label containing `active interruption`.
   - `capture_fails_closed_when_projection_state_violates_the_ui_contract` —
     line 1301:14; `candidate` assertion.
   - `capture_ui_state_matches_manifest_contract_for_every_eligible_subcase` —
     line 1301:14; `candidate` assertion.
   - `capture_rejects_a_symlinked_capture_root` — line 1301:14; `candidate`
     assertion.
   - `capture_writes_canonical_artifact_and_returns_trusted_bindings` — line
     1301:14; `candidate` assertion.
   - `evidence_symlink_source_is_rejected` — line 1956; expected an error
     message containing `evidence`.
   - `load_projection_projects_through_production_path_without_execution_authority`
     — line 1301:14; `candidate` assertion.
   - `load_projection_rejects_tampered_evidence_bytes` — line 1519:66;
     index out of bounds with length 0.
   - `load_projection_rejects_tampered_trace_bytes` — line 1921:66; index out
     of bounds with length 0.
   - `status_lists_only_eligible_sanitized_candidates` — line 1383; left
     `false`, right `true`.
   - `store_is_bounded_and_sessions_invalidate_stale_handles` — line 2043:77;
     index out of bounds with length 0.
   - `trace_symlink_source_is_rejected` — line 1976; expected an error message
     containing `evidence`.

2. After tightening the active source scan to inspect the canonical tool, the
   user interrupted `rtk node --test tools/device-qualification.test.mjs`
   after approximately 3.1 seconds. That invocation has no exit status and is
   recorded as incomplete; the preceding version of the same focused suite
   was green with 74 passed tests. A process check found no surviving test
   process. No further broad validation was run after the interruption.
3. An earlier parallel diagnostic attempt ran matrix write and check together;
   the check exited 1 with `generated device qualification matrix is out of
   date; run --write-matrix` while the write exited 0. The required sequential
   write/check commands immediately afterward both exited 0, so this was an
   ordering race rather than a matrix defect.

## Commit and physical-qualification confirmation

1. Implementation/docs/tests commit:
   `47fa261e14e7a266c402b95cadcb0182da0023e6`
2. The report is committed separately after this implementation commit so the
   implementation hash can be recorded without a self-referential commit
   hash. The report commit hash is returned in the handoff.
3. Confirmed: no physical qualification was performed, no physical evidence
   was added, no physical target was registered, and no evidence was rewritten.
   No push, merge, publish, deploy, connected-hardware action, or other
   external side effect was performed.

## Review follow-up: active-name scan self-test exclusion

The review finding was reproduced: the original
`rtk node --test tools/device-qualification.test.mjs` run exited 1 with 73
passed and 1 failed because the active-name scan included its own test source,
whose intentional forbidden-term regex literals matched the scan.

The fix changes only the active implementation file set and its narrowly
scoped source selection:

1. `tools/device-qualification.mjs` remains included; only
   `tools/device-qualification.test.mjs` is excluded.
2. The active set retains qualification files under
   `apps/emuchef-app/src`, `apps/emuchef-app/src-tauri/src`,
   `tests/fixtures/device-qualification`, and
   `docs/testing/device-qualification`, plus the operator runbook, generated
   matrix, `Makefile`, and the workflow file.
3. The workflow scan is limited to its active `Validate device qualification
   foundation` step so unrelated historical Phase 6C/6D6 workflow entries are
   not rewritten or broadly exempted.
4. A regression asserts the self-test is absent, the canonical tool and
   production paths remain present, and an injected `phase-7` term in
   `apps/emuchef-app/src/DeviceQualificationOverlay.tsx` is rejected.
   Opaque React API guards were not weakened or changed.

Follow-up TDD and validation results:

1. `rtk node --test tools/device-qualification.test.mjs` after adding the
   regression but before the file-set fix — exit 1; 73 passed, 2 failed (the
   original self-test scan failure and the new expected self-test exclusion
   assertion).
2. `rtk node --test tools/device-qualification.test.mjs` after the fix — exit
   0; 75 passed, 0 failed.
3. `rtk npm --prefix apps/emuchef-app run test:security` — exit 0; 29
   security tests and 12 runtime-retirement tests passed; coverage remained
   100.00% lines, 97.92% branches, and 100.00% functions.
4. `rtk node tools/device-qualification.mjs --check` — exit 0;
   `Device qualification check passed.`
5. `rtk git diff --check` — exit 0; no whitespace errors.

The review-fix implementation commit is
`38c69bfcdb0628d835b9e671837b6dadc4723d63`. The separate report commit is
returned in the final handoff.
