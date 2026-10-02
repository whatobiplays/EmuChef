# Device Qualification Operator Runbook

## Purpose

This runbook describes the production-bound workflow for a future
physical-device qualification run. The repository currently contains a
registered physical target but no physical workflow qualification evidence.
Target registration alone does not qualify the device or any workflow.

Production EmuChef remains the system under test. The qualification harness
observes the production workflow; it does not replace planner, executor,
device-probe, or Tauri authority. A matching authored device profile does not
itself imply support.

## Operator flow

1. Launch a clean qualification build with `npm --prefix apps/emuchef-app run device-qualification`.
2. If the device is unregistered: connect/probe/match it, choose usb2/usb3, review the captured facts, Register device target, stop, commit the registry/matrix, and rebuild.
3. On the new clean build: begin a qualification attempt by choosing the registered target and canonical workflow.
4. Complete the workflow-declared checkpoints that must pass before the run. A required prerequisite checkpoint that is missing, failed, or unable to verify invalidates the attempt.
5. Complete normal EmuChef inputs, review, and explicit real-execution confirmation. The attempt observes these product transitions automatically.
6. Complete any remaining workflow-declared checkpoints after the terminal execution. Until they are complete the attempt stays in terminal-awaiting-evidence.
7. Inspect the automatically materialized candidate classification, then explicitly Record qualification run, including invalid/not_observed audit runs only when intentionally preserving harness history.
8. If the attempt cannot continue, use Abandon qualification attempt to close it as an invalid candidate. Abandoning never changes the product execution.
9. Stop and commit the resulting immutable evidence bundle and matrix before another recordable promotion from a fresh build.
10. Run `make device-qualification-check` and repository tests before committing/shipping evidence.

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
candidate persistence, checkpoint declaration, terminal classification, and
explicit recording over the normal production EmuChef workflow. It does not
add a qualification-only planner, executor, device command, or ADB authority.
The operator remains responsible for physical observations and must not treat
the harness being available as physical qualification evidence.

The harness does not reconstruct lifecycle ordering. The product observes its
own committed transitions and the harness captures them, so the operator
interface never decides when a review or an execution belongs to an attempt and
never finalizes a candidate. See "Automatic lifecycle capture" below.

## Automatic lifecycle capture

Qualification evidence lifecycle is owned by the Rust/Tauri product, not by the
operator interface. The operator interface loads sanitized status, declares
checkpoints, and records or discards candidates. It never binds a review or an
execution, never finalizes a candidate, and never retries a trusted transition.

- Every committed product transition is fed to the active attempt
  synchronously, after the product committed it and before the product result is
  returned. Observed transitions are trusted device observation, explicit root
  check, review creation, real-execution admission, and real-execution terminal.
- Device observation is committed at the single authoritative seam, so an
  attempt sees exactly the observation the product committed, with the same
  typed facts, profile match, and root state.
- Every real execution is watched by the product terminal monitor, whether or
  not qualification mode is active. The monitor retains the authoritative
  terminal transition - status, authority invalidation, launch action, and
  report bytes - and only then notifies qualification.
- The monitor never gives up on a running execution. It keeps observing through
  transient failures and resolves the execution through the existing
  authoritative runtime-loss semantics when the runtime session that owned it is
  gone, so a product execution is never left active because observation stopped.
- Reading an execution, exporting a report, or refreshing status is a pure
  projection. None of them advances product or qualification lifecycle state.
- A terminal execution that is still missing required non-prerequisite
  checkpoints enters terminal-awaiting-evidence. The immutable candidate is
  materialized automatically once the required evidence is complete or the
  attempt becomes invalid; there is no operator finalization step.
- A recorded checkpoint is immutable. A second submission for the same
  checkpoint is rejected and never replaces the retained outcome or timestamp.
  While an attempt is in terminal-awaiting-evidence, only unrecorded required
  checkpoints are accepted.
- A terminal execution that invalidated device identity or root authority can
  never produce valid evidence, so the attempt materializes as
  invalid/not_observed.
- Abandoning an attempt closes it immediately as an invalid/not_observed
  candidate. Abandonment never changes product execution state.

## Restart and fail-closed recovery

- At most one attempt is active in a process.
- A new-version attempt resumes only after a proven clean shutdown. An unproven
  shutdown or an incompatible persisted attempt invalidates the attempt instead
  of producing evidence.
- After a restart, the resumed attempt reassociates with a device on the first
  trusted device observation the product commits, not on the first status
  query. If the attempt can no longer prove it ran against the same device, it
  fails closed.
- A resumed attempt never trusts the previous process's device, review, or
  execution handles. Device and review associations are re-established by new
  authoritative product observations in the current process, and an attempt
  that admitted a real execution without retaining its terminal transition
  fails closed as invalid/not_observed.
- If an attempt cannot be persisted, it is poisoned in memory: it is reported as
  unavailable and can never claim evidence. Ordinary product operations are
  never failed by qualification persistence.

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
