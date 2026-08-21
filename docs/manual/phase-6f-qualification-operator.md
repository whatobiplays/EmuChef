# Phase 6F Physical Qualification Operator Runbook

## Purpose

This runbook is the operator procedure for a future physical-device
qualification run under the Phase 6F evidence model. It does not qualify any
device today. The repository currently contains no physical evidence records,
and the Phase 6F foundation does not claim that any device or workflow is
supported.

Production EmuChef remains the system under test. The qualification harness
observes the production workflow; it does not replace planner, executor,
device-probe, or Tauri authority. A matching authored device profile does not
itself imply support.

## Before you start

1. Choose one canonical workflow from `docs/testing/phase-6f/workflow-catalog.json`.
2. Verify the device target is registered in `docs/testing/phase-6f/device-targets.json`.
3. Set `EMUCHEF_PHASE_6F_BUILD_IDENTITY` to the exact EmuChef build identity under test.
4. Set `EMUCHEF_PHASE_6F_RUNTIME_CONTRACT` to the execution or runtime contract version under test.
5. Confirm the workflow's required capabilities and prerequisites apply to the target.

## Qualification sequence

1. Choose an existing canonical workflow ID.
2. Register or capture a device target from observed facts and an existing authored profile ID.
3. Verify prerequisites and production capability applicability.
4. Capture the current EmuChef build identity, workflow version, exact relevant authored recipe SHA-256 digests, runtime contract version, device facts, root state, and connection type.
5. Execute the real production EmuChef workflow through its ordinary reviewed execution boundary.
6. Collect the required automated observations from product outputs and device observations.
7. Collect only declared human checkpoints using `pass`, `fail`, or `unable_to_verify`.
8. Distinguish an invalid harness or infrastructure run from a valid product failure.
9. Create a new immutable evidence JSON record without overwriting an older run.
10. Run `node tools/phase-6f-qualification.mjs --check` after adding evidence.
11. Regenerate with `node tools/phase-6f-qualification.mjs --write-matrix` only after validation succeeds.
12. Rerun `--check` and repository tests before committing evidence.

## Evidence record rules

Each evidence record is one physical run for one device target and one
canonical workflow. The record is immutable after it is committed. Never edit
or delete a completed record because a later run succeeds.

The record binds the run to the workflow version and to the registered device
target. It carries a structured compatibility fingerprint and a derived
`fingerprintDigest` over that fingerprint. It also carries a `recordDigest`
over the canonical record content. Both digests are recomputed by the
validator, and either mismatch rejects the record.

Human checkpoints are typed evidence, not free-form operator prose. Every
checkpoint ID and allowed outcome must come from the workflow definition:

- `pass` means the operator verified the fact the checkpoint establishes.
- `fail` means the operator observed a product failure for that fact. The run
  may be valid with `qualificationOutcome: "failed"`.
- `unable_to_verify` means the operator could not establish the fact. The run
  must use `runValidity: "invalid"` and `qualificationOutcome:
  "not_observed"`. The record may remain historical audit evidence, but it is
  never selected as current qualification evidence and never derives a
  product failure.

A missing required human-checkpoint result makes the evidence record invalid
and the validator rejects it. A missing required automated observation makes
the record invalid for the same reason.

An invalid run is an infrastructure or harness failure, not a product
qualification failure. It must use `qualificationOutcome: "not_observed"`.
A valid product failure uses `qualificationOutcome: "failed"` and must show a
failed automated observation, a failed human checkpoint, or a modeled
target-wide prerequisite or safety failure.

Run IDs use the immutable form `phase-6f-run-sha256:<64 hex characters>`.
Records live under `docs/testing/phase-6f/evidence/`. Synthetic fixtures
belong only under `tests/fixtures/phase-6f/` and must never be copied into
the production evidence directory.

## Current state derivation

Workflow state and device support tier are derived, never authored. Evidence
does not contain a support tier. The projector selects the newest compatible
valid record for each workflow, classifies compatibility only on the
dimensions the workflow declares, and derives `qualified`, `failed`, `stale`,
`deferred`, `missing`, or `not_applicable` for each applicable workflow.

A device target is `Qualified` only when every required workflow is currently
`qualified` and no modeled target-wide failure applies. It is `Limited` when
some required workflow is `failed`, `stale`, `deferred`, or `missing` while
meaningful qualified functionality remains. It is `Unqualified` when no
required workflow is qualified or a modeled target-wide prerequisite or
safety failure invalidates the target as a whole.

## Harness boundary

The foundation does not yet automate steps 2 through 8 end to end. Future
harness work must call production boundaries rather than introducing
qualification-only planner or executor behavior.

## Repository validation

Run the full Phase 6F validation before and after adding evidence:

```sh
node --test tools/phase-6f-qualification.test.mjs
node tools/phase-6f-qualification.mjs --check
make phase-6f-qualification-check
```

`--check` validates the production definitions and evidence, renders the
expected matrix in memory, and compares it byte-for-byte with
`docs/qualification/phase-6f-device-matrix.md`. `--write-matrix` writes only
the generated matrix, and only after all inputs validate. The generated
matrix is a projection, not an independent source of truth.
