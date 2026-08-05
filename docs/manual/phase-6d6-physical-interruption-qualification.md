# Phase 6D.6 physical interruption qualification runbook

This runbook is for a deliberate development-build qualification only. It does
not enable ordinary production real execution, and it does not qualify signing,
notarization, packaged-GUI behavior, release artifacts, or Phase 6E recipes.
The ignored Rust harness refuses to touch ADB until every gate below is valid.

## Safety gates

1. Start from the clean `main` checkout and verify the expected Phase 6D.5
   baseline. Do not reset, stash, clean, stage, commit, or push anything.
2. Use AC power. Disable automatic host sleep for normal qualification. The
   two host-sleep cases below are the deliberate exception and require an
   operator at the host.
3. Use one supported, disposable or deliberately prepared Android device. No
   second device may be attached. `adb devices -l` must contain exactly one
   `device` row matching `EMUCHEF_TEST_DEVICE_SERIAL` byte-for-byte.
4. Allowlist only `com.emuchef.fixture`. Use only the committed Phase 6C
   fixture package and its four manifest-owned roots. No user data, system
   path, production package, or unrelated device is in scope.
5. Create an empty temporary host directory and export it as
   `EMUCHEF_PHASE_6D6_SENTINEL_DIR`. The harness writes fixed markers there;
   operator acknowledgements use exactly `ack` followed by a newline. For the
   four supported active cases, wait for `active-ready`. That marker is created
   only after the production runner lifecycle observer samples the exact
   `Push` child alive.
   Create `operator-action` within five seconds. For cancellation, that marker
   is the cancellation request. For USB disconnect, device-offline, and
   authorization revocation, create the marker first, wait 1.1–3 seconds, then
   perform the prepared physical transition. After the typed terminal result,
   wait for `terminal-ready`, restore the same selected device to its online and
   authorized state, verify it, then create `cleanup-ready`. Reconnection or
   reauthorization never resumes the old execution. Host-sleep cases continue
   to use `sleep-requested`, `sleep-entered`, and `wake`; their deadline-clock
   measurement remains a separate blocker. Root revocation uses the same
   `terminal-ready` and `cleanup-ready` recovery boundary. Creating `abort`
   stops a checkpoint. Every checkpoint expires after ten minutes (`600`
   seconds); a missing, stale, or out-of-order marker is blocked.
6. Select exactly one scenario and one repetition (`1` or `2`) per invocation.
   Run both repetitions from a freshly cleaned fixture state; the harness
   creates a unique run-scoped fixture directory, refuses any pre-existing
   residual, and never reuses another run's payload. A deterministic host test
   or a simulated transition is not physical evidence.

The required gate family is:

```text
EMUCHEF_RUN_REAL_ADB_TESTS=1
EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS=1
EMUCHEF_PHASE_6D6_SCENARIO=<one exact scenario>
EMUCHEF_PHASE_6D6_REPETITION=1|2
EMUCHEF_TEST_DEVICE_SERIAL=<one exact selected serial>
EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture
EMUCHEF_PHASE_6D6_SENTINEL_DIR=<empty test-owned directory>
```

Root revocation additionally requires `EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1`,
`EMUCHEF_RUN_REAL_ADB_ROOT_DESTRUCTIVE_TESTS=1`, and the exact committed
`EMUCHEF_TEST_ROOT_PATH_PREFIX_ALLOWLIST`. Low storage additionally requires
`EMUCHEF_PHASE_6D6_STORAGE_DESTRUCTIVE=1`; authorization reset,
same-serial replacement, and host sleep require their corresponding
`EMUCHEF_PHASE_6D6_AUTHORIZATION_RESET=1`,
`EMUCHEF_PHASE_6D6_IDENTITY_REPLACEMENT=1`, or
`EMUCHEF_PHASE_6D6_HOST_SLEEP=1` opt-in.

## Invocation

Run this command once for each scenario/repetition, replacing only the
explicit gate values. The serial is supplied to the trusted Rust harness; it
is never copied into evidence, UI output, or support data.

```sh
SENTINEL_DIR="$(mktemp -d -t emuchef-phase-6d6)"
export EMUCHEF_RUN_REAL_ADB_TESTS=1
export EMUCHEF_RUN_PHASE_6D6_PHYSICAL_TESTS=1
export EMUCHEF_PHASE_6D6_SCENARIO=cancellation_active
export EMUCHEF_PHASE_6D6_REPETITION=1
export EMUCHEF_TEST_DEVICE_SERIAL=<one-exact-serial>
export EMUCHEF_TEST_PACKAGE_ALLOWLIST=com.emuchef.fixture
export EMUCHEF_PHASE_6D6_SENTINEL_DIR="$SENTINEL_DIR"
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml --features real-execution --lib executor_real_adb_tests::physical_interruption_qualification::manual_phase_6d6_physical_interruption_qualification -- --ignored --exact --nocapture
```

The operator must stay at the checkpoint. For one of the four supported active
cases, wait for `active-ready`; do not use `operation-started` as liveness
proof. Create `operator-action` within five seconds and follow the scenario's
ordering below. For a boundary case, wait for `boundary-ready`, then perform
the documented transition and create the marker. The marker protocol is
bounded to `600` seconds; do not leave a test waiting forever. After every
attempt, confirm that the sentinel directory is empty and that fixture-owned
device paths have no residual state.

## Active-operation stimulus and handshake

For `cancellation_active`, `usb_disconnect_active`, `device_offline`, and
`device_unauthorized`, the harness creates a unique run-owned temporary host
workspace and writes a deterministic, non-secret, non-compressible calibration
file. It times a 256 MiB transfer through the ordinary production ADB push
adapter, derives a host source between 512 MiB and 8 GiB targeting approximately
30 seconds, and blocks unless the predicted push duration is 15–240 seconds.
Device free space must cover the active destination plus 1 GiB of cleanup
headroom. The temporary host workspace is the only additional executor read
root for that invocation. Calibration and active host/device files are removed
during fixture-only cleanup; their raw paths never enter evidence.

The active sequence is:

```text
operation-started
→ exact Push child sampled alive
→ active-ready
→ operator-action within five seconds
→ cancellation request, or wait 1.1–3 seconds and perform the physical transition
→ exact child terminal result
→ terminal-ready for disconnect/offline/authorization
→ restore and verify the selected device
→ cleanup-ready
→ fixture-only cleanup
```

A stale action, same-second action/terminal pair, missing exact-child event,
non-live sample, or source calibration outside the committed bounds blocks the
attempt. The private process-delay seam and host process-table inspection are
not physical evidence.

## Low-storage safety protocol

Low-storage is the only case that allocates device capacity. It is blocked
unless the selected device is disposable, the initial free space is between
4 GiB and 5,308,416 KiB, and the exact run-scoped fixture directory is absent.
The harness then creates and verifies a fixture-owned 1 GiB recovery-reserve
file before allocating a bounded fixture-owned filler. The filler is capped at
4 GiB and leaves 64 MiB of cleanup headroom; the first reviewed copy uses a
128 MiB host fixture payload so the destination receives genuine ENOSPC while
the headroom remains available for cleanup. No user, system, or non-fixture
path is ever deleted. Cleanup removes payloads, filler, sentinel state, and the
reserve in that order, then verifies the run directory is absent, final free
space is recorded, and the recovery capacity is restored. A reserve,
ownership, allocation-bound, cleanup, or restoration proof that is missing
blocks the repetition and cannot be counted as a pass.

## Physical scenario matrix

Twelve scenarios are mandatory closure evidence. `device_offline` is
conditional diagnostic evidence because ADB does not provide a general,
reliable operator-controlled transition into a stable offline state. Run it
only when a reviewed device-specific procedure can prove
`device → offline → device`; inability to produce that transition does not
block Phase 6D.6 closure. Never relabel disconnect, unauthorized, or ADB-server
failure as offline evidence.

| Scenario | Operator transition | Required proof |
| --- | --- | --- |
| `cancellation_active` | Wait for `active-ready`, then immediately create `operator-action`; this is the cancellation request. | The exact `Push` child was alive before the request, the first atomic operation settles truthfully, later work is not scheduled, and no hidden owner remains. |
| `cancellation_boundary` | Wait for `boundary-ready`, then request cancellation. | Safe-boundary cancellation preserves completed evidence and leaves later work Not attempted. |
| `usb_disconnect_active` | Wait for `active-ready`, create `operator-action`, wait 1.1–3 seconds, disconnect the selected cable, wait for `terminal-ready`, reconnect the same device, verify its exact serial is `device`, then create `cleanup-ready`. | Exact live-child binding, stable disconnected/transport issue, conservative partial state, cleanup outcome, and no automatic resume. |
| `usb_disconnect_boundary` | Wait for `boundary-ready`, disconnect the selected cable, verify the selected serial is absent, then create `operator-action` to release the second operation while the device is disconnected. Wait for `terminal-ready`, reconnect the same device, verify its exact serial is `device`, then create `cleanup-ready`. | The first operation remains completed, the second operation fails before mutation with a typed disconnect/transport issue, no later work runs, and cleanup occurs only after explicit recovery authorization. |
| `device_offline` | **Conditional only.** When a reviewed reversible procedure exists, wait for `active-ready`, create `operator-action`, wait 1.1–3 seconds, enter ADB offline, wait for `terminal-ready`, restore the same device online, verify it, then create `cleanup-ready`. | Opportunistic exact live-child `device_offline` evidence remains valid and auditable, but this scenario is not required for closure. |
| `device_unauthorized` | With the authorization-reset opt-in, wait for `active-ready`, create `operator-action`, wait 1.1–3 seconds, revoke only the selected device's debugging authorization, wait for `terminal-ready`, reauthorize the same device, verify its row is `device`, then create `cleanup-ready`. Do not reset unrelated ADB state. | Exact live-child binding plus ordered initial authorized, genuine `unauthorized`, terminal, cleanup, and final authorized observations; `device_unauthorized`; no automatic resume. |
| `identity_stability` | Perform one controlled reconnect at `boundary-ready` with the same device. | Repeated complete identity samples remain stable and the second operation succeeds. |
| `identity_replacement` | With explicit owner-approved hardware, disconnect the first target before attaching the same-serial replacement at `boundary-ready`. The harness polls successful ADB inventory samples and stable fingerprints, proving original attachment, serial absence, replacement attachment, and no simultaneous target. | `device_identity_changed` or `device_identity_unverified` before later mutation. If hardware is unavailable, record this exact case unqualified. |
| `root_revocation` | On a prepared rooted device, revoke EmuChef's adb-shell root authority after `boundary-ready`, acknowledge `operator-action`, wait for `terminal-ready`, restore root authority, then acknowledge `cleanup-ready`. | The second privileged command does not run, root failure is primary, prior mutation is retained, cleanup is separate and verified, and identity precedence is unchanged. |
| `operation_timeout` | Use only a genuinely bounded operation that reaches the fixed Rust-owned deadline while the exact child remains owned. The private delay regression seam is automated evidence only and cannot qualify this repetition. | `operation_timed_out`, kill/reap or uncertainty evidence, no descendant, no later scheduling. Block the case if a safe genuine deadline cannot be exercised. |
| `low_storage` | With the separate destructive opt-in, verify at least 4 GiB free, create/verify the unique fixture-owned 1-GiB recovery reserve, allocate bounded filler, then acknowledge the checkpoint. | Genuine ENOSPC maps to `device_storage_exhausted`; no deletion/retry; cleanup removes only run-scoped payload/filler/sentinel/reserve and proves restored free capacity. Block the case if any proof is unavailable. |
| `host_sleep_before_deadline` | Begin the long fixture operation, create `sleep-requested`, manually sleep the host before the fixed deadline, create `sleep-entered` after entry, then create `wake` immediately after resuming. | Record ordered sleep/wake times, measured executor and wall elapsed time, timer behavior, child result, identity, terminal state, slot release, and no second owner. |
| `host_sleep_after_deadline` | Repeat after enough active elapsed time to cross the fixed deadline, using the same three physical markers. | Record whether the timer observes active-host or suspended time; a measured completion or timeout is valid only when the branch is internally consistent. Transport loss, indeterminate timing, and contradictory timestamps remain blocked. |

For every mutating case, the harness cleans only the fixture-owned destination
files and reports residual state independently from the operation result. Do
not modify boot images, modules, SELinux policy, system partitions, root
manager configuration, user data, or unrelated devices.

## Host-sleep policy

The Rust owned-process timer remains the existing `async_io::Timer`; no sleep
inhibitor, OS event plugin, checkpoint, resume token, or replay path is added.
The same local child/future tree remains authoritative if the process survives
sleep. After wake it may report a trustworthy completion, a typed transport
failure, or `operation_timed_out`. If the sidecar generation is lost, the
terminal result is `runtime_session_lost`/`execution_unavailable` with an
indeterminate device outcome. Application restart never resumes the old
execution. Record the measured timer behavior on each qualified host instead
of assuming that suspension counts as elapsed executor time. Classification
comes only from production deadline-clock samples and remaining budget
immediately before sleep and after wake, within a documented tolerance.
Terminal outcome is a separate consistency check: excluded suspension may
still time out after enough later active time. The current harness has no exact
deadline-clock observation seam, so both host-sleep scenarios remain blocked.

## Development UI smoke

UI smoke is mandatory closure evidence. Preserve the 24-record mandatory
physical matrix
(12 scenarios × 2 repetitions) and add exactly two composite development-build
UI-smoke records, one per repetition. Conditional `device_offline` records may
be retained in addition to that matrix but do not affect completeness. Each
composite record separately covers cancellation, one USB-disconnect transport
failure, root revocation, low storage, and host sleep/runtime loss. The
transport subcase must bind to passing `usb_disconnect_active` or
`usb_disconnect_boundary` evidence, not conditional offline evidence. For
every subcase record the development-build identity,
version, and digest; a unique sub-run; the physical backend run and trace
digest; exact authored title, issue, and remediation; terminal and **Not
attempted** projection; partial-change and authority/recovery state; available
controls; a sanitized UI-state artifact with a content digest; and an
artifact-bound operator observation. Missing, duplicated, copied, or
inconsistent subcases keep closure blocked. This is not packaged-GUI or release
qualification.

## Evidence and closure

The harness writes only sanitized JSON under
`docs/testing/phase-6d6/evidence/`. Serial values are SHA-256 digests, device
paths and raw ADB output are omitted, the active-slot record comes from the
`production-execution-session-slot` lifecycle seam, and cleanup/residual outcomes
are required. Every attempt uses unique run, scope, sentinel, nonce, slot,
evidence-path, trace-path, record-digest, and trace-digest identities; evidence
files use create-new semantics and cannot overwrite prior attempts. Host-sleep records are passable only with internally consistent
sleep/wake/timer measurements; transport loss, indeterminate, or contradictory
timing is blocked. Validate the matrix without touching ADB:

```sh
node tools/phase-6d6-evidence.mjs
node --test tools/phase-6d6-evidence.test.mjs
node tools/phase-6d6-result.mjs
node --test tools/phase-6d6-result.test.mjs
```

Phase 6D remains **In progress** until all twelve mandatory scenarios have two clean
passing repetitions, both composite UI-smoke repetitions pass, the automated
matrix passes, and any named mandatory hardware limitation has explicit owner
acceptance. Conditional `device_offline` evidence may remain unrun or blocked
without preventing closure; any attempted record must still report its outcome
truthfully. A blocked or unrun mandatory scenario is never reported as passing,
and Phase 6E must not start from this run.
