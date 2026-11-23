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

## Capability-restricted qualification

Restriction is an explicit launcher option for clean development
qualification builds. Use this command for an authorized restricted run.

```sh
npm --prefix apps/emuchef-app run device-qualification -- --restrict-privileged-execution
```

Without the option, the launcher uses the standard qualification build and
sidecar preparation and starts the app without a restriction argument. Unknown
options fail before the launcher starts a build. With the option, the launcher
prepares a restricted sidecar and passes the restriction argument to the
native app. The native app requires debug mode, the `real-execution` feature,
runtime opt-in and the embedded clean-build gate. The sidecar checks its own
debug and `real-execution` gates, verifies the embedded qualification source
identity, and confirms the effective policy during its private handshake.
Restriction stays fixed for the process lifetime. Changing it requires a
relaunch and fresh profile resolution, planning and review.

The Rust planner suppresses effective `root_shell` and `app_data_write` while
preserving the device's observed root state. Execution uses ordinary
unprivileged ADB. Operations that require privilege or lack a verifiable
unprivileged route are blocked. Restricted CopyCommit streams staged files
and its helper through ADB shell. The adapter checks the daemon, then verifies
the shell UID and security context in the same command that writes the stream.
It checks the digest derived from the Android ID and build fingerprint before
opening the staging destination. Raw sync or directory transfers can fail if
the adapter cannot establish a guarded ordinary route.

A rooted target can provide evidence for the tested unprivileged path when
the accepted review and terminal report establish the restriction, target,
profile and plan, source and build identity, reviewed digest, required
operations and outcomes. OEM-assisted execution needs separate evidence.

Physical use still requires separate authorization for the target, build,
actions, paths and run scope. No capability-restricted physical run has been
performed. Physical acceptance is `NOT RUN`.

## Future KONKR Pocket Advance ROM Merge candidate

A future, separately authorized run may qualify only the ROM-copy operation on
the KONKR Pocket Advance. The bounded scenario selects `feature.copy_roms`,
retains the required `copy_rom_library` step, and merges ROM files into the
reviewed internal shared-storage location. It does not qualify RetroArch's
full provisioning journey or other recipes.

### Candidate source and capability assumptions

Build a future candidate from a clean, isolated source snapshot. Record its
source commit and material build digest, the effective policy revision, and
the exact capability values used for planning and execution. Production device
profile and plan defaults remain unchanged. The host-only planner check may
provide `shared_storage_write` in memory to prove that the normal KONKR plan
retains the required ROM-copy step and reviewed destination binding. That
synthetic planning input does not establish that a physical device supports
the route.

If a future candidate needs a capability value that production does not
advertise, keep that candidate source isolated from production and obtain
separate review and owner authorization before building or running it. The
capability assumption must be explicit in the candidate evidence. Do not
silently promote a production capability or change a required step to make the
plan pass.

### Restricted ordinary execution path

Use the restricted qualification launch command above. The planned ROM-copy
operation must remain `unprivileged` and use the reviewed internal storage
binding through the existing guarded ADB shell CopyCommit path described above.
That path must pass the device/build identity digest check described above
before opening the staging destination. If it cannot establish the guarded
route, the operation must fail closed.

Record the device's observed root state separately from `root_authority_used`.
A rooted device may still use the restricted unprivileged route. The evidence
must show that execution used no root authority, OEM compiler, or OEM stage.

### Required evidence and approval checkpoints

Before any physical run, separately review and authorize the candidate source,
capability assumption, exact target, build, operation, destination path, ROM
inputs, and run scope. Identify the preinstalled RetroArch package and compare
its package identity and installed APK SHA-256 with the pinned artifact.
Approve any RetroArch launch, force-stop, or cleanup separately if the run
needs it. The evidence must identify:

1. The device identity and session, selected profile `konkr.pocket_advance`,
   profile-match resolution, Android version and API level, and build
   fingerprint.
2. The candidate source commit and digest, qualification build identity, pinned
   APK SHA-256, artifact source and version or build ID, and preinstalled
   package identity.
3. The effective policy revision and execution strategy ID and digest. Record
   the observed device root state separately from `root_authority_used`, and
   show that no root authority, OEM compiler, or OEM stage was used.
4. The selected plan `konkr.pocket_advance.base`, selected recipe and required
   step, Merge policy, and the reviewed destination path and volume identity.
   The canonical validator requires the executed plan to have the same
   immutable ID and digest as the reviewed plan. It also requires a successful
   terminal result for `feature.copy_roms/copy_rom_library`. A skipped,
   missing, partial, cancelled, or unsuccessful required step is not qualified.
   This step result does not establish emulator usability.
5. Capture the source and destination in three independent manifests. For
   each file, record its relative path and SHA-256.
   - `S` records the reviewed ROM source snapshot before copying.
   - `B` records the reviewed destination snapshot before mutation.
   - `A` records the same destination after the operation.

   Bind all three to the same reviewed source identity, destination and volume
   identity, immutable reviewed-plan ID and digest, and terminal result for
   `feature.copy_roms/copy_rom_library`. For every path in `S`, require the
   same path and SHA-256 in `A`, including paths that also appear in `B`. For
   every path in `B` absent from `S`, require the same path and SHA-256 in `A`.
   Every path in `A` must also appear in `S` or `B`.
   Identify the destination sentinel by relative path and expected SHA-256.
   Require the sentinel to be absent from `S` and to match its expected path
   and hash in both `B` and `A`. The full `S`/`B`/`A` comparison and sentinel
   check must each pass. Missing entries, unexpected paths, wrong hashes, or
   partial or unverifiable manifests fail the corresponding postcondition. If
   any postcondition fails, do not report a passing ROM Merge result.
6. On the device, proof that RetroArch can see and load the copied ROM. A
   successful host plan or copy result alone does not establish emulator
   usability.

Host-only planning evidence does not authorize device connection or writes.
Physical operation requires its own approval checkpoints before the target is
connected and before the reviewed storage write. Release qualification needs
separate review of the release scope and its required workflows; this ROM-only
result cannot support a full-setup or release claim. Physical status is
`NOT RUN`, and emulator-side usability is unverified.

## Operator flow

1. Launch a clean qualification build with `npm --prefix apps/emuchef-app run device-qualification`.
2. If the device is unregistered: connect/probe/match it, choose usb2/usb3, review the captured facts, Register device target, stop, commit the registry/matrix, and rebuild.
3. On the new clean build: begin a qualification attempt by choosing the registered target and canonical workflow.
4. Complete the workflow-declared checkpoints that must pass before the run. A required prerequisite checkpoint that is missing, failed, or unable to verify invalidates the attempt.
5. Complete normal EmuChef inputs, review, and explicit real-execution confirmation. The attempt observes these product transitions automatically.
6. After terminal execution, complete any remaining workflow-declared checkpoints. An attempt may remain pending while required non-prerequisite checkpoint evidence is still obtainable. Recording `unable_to_verify` for any required checkpoint immediately invalidates and closes the attempt; later evidence cannot repair it.

   Even after checkpoint evidence is complete, candidate materialization may
   be deferred if the retained terminal report is unavailable or inconsistent
   with the product's retained terminal classification, if required authored
   recipe source differs from the immutable session-start digest, or if the
   running qualification build differs from the build that captured the
   attempt. Restore the exact required recipe source and the complete captured
   source/build identity, including its commit and material build digest, then
   run that matching qualification build and refresh qualification status.
   Status may safely retry materialization from the execution report and
   checkpoint evidence already retained; it does not rerun or re-observe the
   product execution. Recovery first verifies that the previous process ended
   with a proven clean handoff. If that proof is absent, the attempt is
   invalidated and retained as invalid/not_observed under the current build,
   even when the attempt was captured by another build. Only a cleanly handed-off
   valid attempt is deferred unchanged when the running build differs from its
   captured build. An already-invalid attempt may likewise be recovered under
   another build only as an invalid/not_observed, non-promotable audit candidate;
   operators do not need to restore the captured build solely to preserve that
   invalid result.
   There is no operator finalization step. If the report remains unavailable
   or inconsistent after the source/build conditions are restored, treat the
   terminal result as unproven and abandon the attempt.
7. Inspect the automatically materialized candidate classification. Explicitly
   Record qualification run only for promotable candidates. Invalid/not_observed
   recovery candidates remain available as non-promotable audit records and may
   be discarded when they are no longer needed; they cannot be recorded as
   canonical evidence. A passing KONKR `rom-library-sync` candidate also needs
   its `rom-merge-evidence.json` attachment. Use the qualification-only
   attachment control to supply the raw JSON bytes before recording the run.
   The attachment digest protects the stored bytes. Capture origin and
   collection method remain unverified.
8. If the attempt cannot continue, use Abandon qualification attempt to close it as an invalid candidate. Abandoning never changes the product execution.
9. Stop and commit the resulting immutable evidence bundle and matrix before another recordable promotion from a fresh build.
10. Run `make device-qualification-check` and repository tests before committing/shipping evidence.

## Evidence record rules

The ROM Merge artifact binds the candidate handle and session to the retained
execution report, target, build, profile, reviewed plan, and terminal copy step.
Its S, B, and A manifests contain unique relative paths and SHA-256 digests.
Every source path must appear unchanged in A. Destination-only paths from B
must remain unchanged in A, and A cannot contain paths absent from both S and
B. The sentinel must be absent from S and retain its reviewed path and digest
in B and A.

The artifact also records the preinstalled RetroArch package, installed APK
digest, and the fixed source, version, and digest from the repository pin file.
The checked-in `rom-merge-apk-pin.json` currently has no approved pin. The
attachment gate therefore rejects promotion until a separately reviewed fixed
pin is present. Operator-supplied pin values do not establish that authority.
The ROM Merge result qualifies only the copy workflow. KONKR support remains
capped at `limited` until separate physical evidence shows RetroArch can see
and load the copied ROM.

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
- Reading an execution or exporting a report is a pure product projection.
  Qualification status does not infer new product transitions, but it may
  recover persisted session state or retry candidate materialization when the
  terminal report and checkpoint evidence are retained and the exact authored
  source again matches the session-start fingerprint.
- A terminal execution that is still missing required non-prerequisite
  checkpoints remains in terminal-awaiting-evidence only while the required
  evidence can still be obtained. A completed attempt also remains pending if
  its retained terminal report does not match the terminal status or authored
  source no longer matches the session-start fingerprint. After required
  checkpoints are complete and the exact source is restored, refreshing
  qualification status retries materialization from retained evidence. If the
  attempt remains pending then, report/status integrity is still unproven and
  the operator should abandon it. The immutable candidate is otherwise
  materialized automatically once required evidence is complete or the attempt
  becomes invalid; there is no operator finalization step. A required checkpoint
  marked unable to verify immediately invalidates and closes the attempt.
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
- A valid new-version attempt resumes only after a proven clean shutdown and a
  matching captured build identity. A valid attempt captured by another build
  remains deferred and unchanged. An unproven shutdown or incompatible persisted
  attempt becomes invalid/not_observed and remains as a non-promotable audit
  candidate.
- An already-invalid attempt may be recovered under another build only as a
  non-promotable invalid/not_observed audit candidate; restoring its captured
  build is not required to preserve that audit result.
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
