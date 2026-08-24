# Device Qualification Operator Runbook

## Purpose

This runbook describes the production-bound workflow for a future
physical-device qualification run. Implementing the harness does not qualify
any device or workflow. The repository currently contains no physical target
or evidence records.

Production EmuChef remains the system under test. The qualification harness
observes the production workflow; it does not replace planner, executor,
device-probe, or Tauri authority. A matching authored device profile does not
itself imply support.

## Operator flow

1. Launch a clean qualification build with `npm --prefix apps/emuchef-app run device-qualification`.
2. If the device is unregistered: connect/probe/match it, choose usb2/usb3, review the captured facts, Register device target, stop, commit the registry/matrix, and rebuild.
3. On the new clean build: choose the registered target and canonical workflow.
4. Complete normal EmuChef inputs, review, and explicit real-execution confirmation.
5. Complete only workflow-declared human checkpoints.
6. Inspect terminal candidate classification.
7. Explicitly Record qualification run, including invalid/not_observed audit runs only when intentionally preserving harness history.
8. Stop and commit the resulting immutable evidence bundle and matrix before another recordable promotion from a fresh build.
9. Run `make device-qualification-check` and repository tests before committing/shipping evidence.

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

Run IDs use the immutable form `qualification-run-sha256:<64 hex characters>`.
Records live under `docs/testing/device-qualification/evidence/`. Synthetic fixtures
belong only under `tests/fixtures/device-qualification/` and must never be copied into
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

The harness implements the operator flow by layering target registration,
candidate persistence, checkpoint capture, terminal classification, and
explicit recording over the normal production EmuChef workflow. It does not
add a qualification-only planner, executor, device command, or ADB authority.
The operator remains responsible for physical observations and must not treat
the harness being available as physical qualification evidence.

## Repository validation

Run the repository validation before committing or shipping evidence:

```sh
npm --prefix apps/emuchef-app run device-qualification
node --test tools/device-qualification.test.mjs
node tools/device-qualification.mjs --check
make device-qualification-check
```

`--check` validates the production definitions and evidence, renders the
expected matrix in memory, and compares it byte-for-byte with
`docs/qualification/device-qualification-matrix.md`. `node tools/device-qualification.mjs --write-matrix` writes only the generated matrix, and only after all inputs validate. The generated matrix is a projection, not an independent source of truth.
