# Task 4 Report: Candidate Validation, Evidence Bundles, and Atomic Canonical Recording

## Summary

Task 4 is implemented in the canonical Node authority at
`tools/device-qualification.mjs`.

The tool now:

- validates and seals qualification run records with canonical `runId`,
  `fingerprintDigest`, and `recordDigest`;
- loads and validates bundle-shaped evidence as `{ record, reportBytes }`;
- enforces digest-bound `execution-report.json` artifacts for valid runs and
  permits report omission only for invalid `not_observed` audit bundles that do
  not reference the artifact;
- enforces the `retroarch-plus-bios` version-2 required human checkpoint
  `clean_or_deliberately_reset_device`;
- validates bounded qualification candidates by
  `qualification-candidate-<32 lowercase hex>`;
- promotes target-registration and qualification-run candidates through
  create-new canonical mutations only;
- revalidates current repository state, workflow catalog, target registry,
  authored recipe digests, and fingerprints at promotion time; and
- performs rollback-safe matrix replacement for both target registration and run
  recording.

## TDD Evidence

### RED

Focused command:

```sh
node --test --test-name-pattern="valid evidence requires|invalid audit evidence|changing a bound report|same target or run twice" tools/device-qualification.test.mjs
```

Initial failure:

```text
SyntaxError: The requested module './device-qualification.mjs' does not provide an export named 'loadEvidenceBundle'
```

That confirmed the expected missing surface before implementation:

- no bundle loader/export;
- no bundle validator/export; and
- no candidate-promotion mutation entry points.

### GREEN

Focused command after implementation:

```sh
node --test --test-name-pattern="production catalog binds|valid evidence requires|invalid audit evidence|changing a bound report|required human checkpoint|same target or run twice|rolls back|failed required human checkpoint" tools/device-qualification.test.mjs
```

Result:

```text
8 tests, 8 passed, 0 failed
```

Full command:

```sh
node --test tools/device-qualification.test.mjs
```

Result:

```text
65 tests, 65 passed, 0 failed
```

Matrix/rendering commands:

```sh
node tools/device-qualification.mjs --write-matrix
node tools/device-qualification.mjs --check
```

Result for both:

```text
Phase 6F qualification foundation check passed.
```

## Contract and Fixture Changes

- Updated production workflow catalog to make `retroarch-plus-bios` version 2
  with the required `clean_or_deliberately_reset_device` checkpoint.
- Extended the production evidence schema with top-level `artifacts` and the
  strict `production_execution_report` artifact contract.
- Updated the evidence README to describe immutable bundle directories instead
  of standalone JSON files.
- Migrated synthetic evidence fixtures to bundle directories with
  `evidence.json` and `execution-report.json` where applicable.
- Updated projection fixture loading to consume bundle directories and validate
  each bundle before projection.
- Updated `CONTEXT.md` with the new bundle, checkpoint, candidate-promotion,
  and current-state projection contracts.

## Files Changed

- `tools/device-qualification.mjs`
- `tools/device-qualification.test.mjs`
- `docs/testing/device-qualification/evidence-schema.json`
- `docs/testing/device-qualification/workflow-catalog.json`
- `docs/testing/device-qualification/evidence/README.md`
- `tests/fixtures/device-qualification/**`
- `docs/qualification/device-qualification-matrix.md`
- `CONTEXT.md`

## Commit

Required commit message:

```text
feat: record immutable qualification bundles
```

## Fix Round 1 — Review Corrections

Review required three behavior corrections and one coverage expansion:

- make run-bundle creation truly create-new/no-clobber atomic;
- validate every existing evidence bundle, including bound execution-report
  bytes, before either promotion path projects or mutates canonical state;
- add focused regressions for reservation races, tampered existing bundles,
  and target-registration rollback symmetry.

### RED

Focused regression command before the fix:

```sh
node --test --test-name-pattern="refuses a destination reserved|tampered existing evidence bundle report blocks|target registration restores registry" tools/device-qualification.test.mjs
```

Result:

```text
3 tests, 1 passed, 2 failed
```

Expected failures before implementation:

- run recording deleted a competing destination created after the absence check;
- promotions did not validate existing bundle report bytes before mutation.

### GREEN

Focused command after the fix:

```sh
node --test --test-name-pattern="recording the same target or run twice|refuses a destination reserved|tampered existing evidence bundle report blocks|target registration restores registry|recording a run rolls back the newly created evidence bundle" tools/device-qualification.test.mjs
```

Result:

```text
5 tests, 5 passed, 0 failed
```

Full command after the fix:

```sh
node --test tools/device-qualification.test.mjs
```

Result:

```text
68 tests, 68 passed, 0 failed
```

Sequential CLI verification after the fix:

```sh
node tools/device-qualification.mjs --write-matrix
node tools/device-qualification.mjs --check
```

Result for both:

```text
Phase 6F qualification foundation check passed.
```

### Fix-Round Delta

- Replaced the run-bundle check-then-rename flow with an atomic create-new
  directory reservation that cannot clobber a destination created by another
  invocation and only cleans up directories reserved by the current process.
- Introduced validated existing-evidence loading for canonical projection so
  target registration, run recording, and production projection all verify
  bundle bytes before using historical evidence.
- Added regression coverage for competing destination reservation, tampered
  existing execution-report artifacts on both promotion paths, and
  target-registration rollback when matrix replacement fails.
