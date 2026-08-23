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
   reauthorization never resumes the old execution. Host-sleep cases use the
   deterministic pre-suspend handshake described below: `sleep-requested` and
   `sleep-entered` are created while the host is still awake (`sleep-entered`
   is the final operator handoff immediately before physical suspension, not
   an OS sleep-entry event), the harness creates `sleep-ready` only after
   proving the exact `DeviceCopy` child alive and sampling the exact
   owned-process deadline clock, and `wake` is created immediately after
   resume as the first post-resume acknowledgement. Root revocation uses the same
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

For `cancellation_active`, `usb_disconnect_active`, and `device_offline`, the
harness creates a unique run-owned temporary host
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
→ terminal-ready for disconnect/offline
→ restore and verify the selected device
→ cleanup-ready
→ fixture-only cleanup
```

A stale action, same-second action/terminal pair, missing exact-child event,
non-live sample, or source calibration outside the committed bounds blocks the
attempt. The private process-delay seam and host process-table inspection are
not physical evidence.

## Host-sleep stimulus and pre-suspend handshake

Both host-sleep scenarios reuse the exact private pseudo-device copy stimulus
`/dev/zero -> /dev/null` through the production `RealAdbDevice::copy_on_device`
path, exactly like `operation_timeout`. The copy does not normally complete on
its own and remains live until timeout, transport failure, or explicit process
cleanup; it creates no persistent device payload and never modifies `/dev/zero`
or `/dev/null`. A
private, thread-local, one-shot `#[cfg(test)]` override selects a 120-second
qualification deadline for this ignored qualification entry point; production
`ProcessOperation::DeviceCopy` remains 300 seconds. 120 seconds gives the
operator a practical window to create the pre-suspend markers, physically
sleep and wake the host, and (for `host_sleep_before_deadline`) wake before the
deadline, while keeping the after-deadline repetition bounded; the 15-second
operation-timeout value is too short for reliable manual sleep qualification.

The host-sleep sequence is deterministic and every operator-created marker is
created while the host is awake:

```text
operation-started
→ operator creates sleep-requested (awake)
→ watcher samples the exact DeviceCopy child alive
→ watcher samples the exact owned-process deadline clock
→ watcher creates sleep-ready
→ operator observes sleep-ready
→ operator creates sleep-entered within four seconds (final awake handoff)
→ operator immediately initiates physical host sleep
→ host resumes
→ operator immediately creates wake (first post-resume acknowledgement)
→ watcher samples the deadline clock again from the retained exact basis
→ exact child terminal result
→ fixture-only cleanup
```

`sleep-ready` is an internal qualification-only handshake marker; it never
enters the strict evidence schema and is removed during sentinel cleanup. The
harness never creates `sleep-ready` unless the exact child is alive, terminal
has not been reported, and the deadline-clock sample succeeded. `sleep-entered`
must follow `sleep-ready` within four seconds, which keeps the exact-child
liveness sample within the same bounded freshness window used by the active
scenarios. The documented measurement tolerance is 8,000 ms, covering the
enforced handoff window, the sample-to-`sleep-ready` latency budget,
canonical-second marker quantization on both the sleep and wake boundaries,
and scheduler margin. Keep the host suspended long enough (at least about
15 seconds) that the included and excluded branches are distinguishable within
that tolerance.

The host-sleep deadline phase is anchored to the exact owned-process
deadline-clock start (the selected `DeadlineClockStarted.at` wall timestamp),
not the earlier `operation-started` progress marker. The serialized
`hostSleep.operationStartedAt` therefore represents the actual 120-second
timer start; `sentinel.operationStartedAt` remains the independent
progress-marker observation and is not the deadline threshold authority.
That wall timestamp is the wall observation retained alongside construction
of the exact monotonic timer basis; it correlates with timer creation and is
not a separate observer-install timestamp. The final host-sleep lifecycle
snapshot is taken only after the bounded watcher has finished publishing its
retained-basis post-wake sample, so the owner may reach terminal immediately
after resume and terminal may precede that post-wake sample.

Every host-sleep attempt persists the sanitized owned-process lifecycle
observations in `trace.lifecycle` before the final `activeProcess` and
`hostSleep` projections are derived. A blocked or partial attempt therefore
never discards the source observations that explain it, and a blocked record
also retains the exact watcher gate that failed (for example the four-second
`sleep-ready` to `sleep-entered` handoff window, the five-second liveness
freshness bound, or the `ack` marker content) in its notes instead of
reporting only a generic handshake failure. Operators must create
`sleep-entered` within four seconds of observing `sleep-ready`; a later
handoff cannot truthfully claim the exact child was alive immediately before
physical suspension.

## Authorization boundary and reconnect handshake

`device_unauthorized` is a safe-boundary scenario. Revoking stored USB-debugging
trust does not reliably invalidate an already authenticated ADB transport, so
qualification forces a new handshake after the first reviewed operation has
completed. Use only the selected device and keep unrelated ADB state untouched.

The authorization sequence is:

```text
first operation finishes
→ boundary-ready
→ revoke USB debugging authorizations on the selected device
→ authorization-revoked with exact ack content
→ disconnect the selected USB device
→ prove the exact serial is absent for at least one canonical second
→ reconnect the same device without accepting the prompt
→ harness observes the exact serial as unauthorized
→ unauthorized-observed
→ operator-action releases the second operation
→ second operation fails before mutation as device_unauthorized or device_identity_unverified
→ terminal-ready
→ accept the selected device's authorization prompt
→ prove the exact serial is device
→ cleanup-ready
→ fixture-only cleanup and final authorized observation
```

Do not create `operator-action` before `unauthorized-observed`. The terminal
issue may be `device_unauthorized`, or `device_identity_unverified` when the
production pre-operation identity guard cannot collect complete evidence from
the independently observed unauthorized device. The latter never qualifies by
itself: a missing revocation marker, no selected-serial absence interval, an
authorized reconnect, mismatched transition/terminal issues, changed identity,
or any disconnect/offline issue does not qualify.

## Low-storage safety protocol

Low-storage is the only case that allocates device capacity. It is blocked
unless the selected device is disposable, the initial free space is between
4 GiB and 5,308,416 KiB, and the exact run-scoped fixture directory is absent.
When the selected device has more than the accepted maximum, use the reviewed
profile-based preflight utility instead of manually adding or deleting files:

```sh
node tools/device-storage-preflight.mjs status \
  --serial <one-exact-serial> \
  --profile phase-6d6-low-storage

node tools/device-storage-preflight.mjs prepare \
  --serial <one-exact-serial> \
  --profile phase-6d6-low-storage \
  --dry-run

node tools/device-storage-preflight.mjs prepare \
  --serial <one-exact-serial> \
  --profile phase-6d6-low-storage \
  --yes
```

The utility requires exactly one authorized selected device, verifies the
fixture package, and proves that Downloads and the qualification destination
are on the same reported filesystem and mount. It writes non-sparse zero-filled
chunks directly on the device under only:

```text
/sdcard/Download/EmuChefStoragePreflight/phase-6d6-low-storage
```

That directory requires the exact EmuChef ownership marker and may contain only
`chunk-NNNN.bin` files. Preparation remeasures available storage after every
chunk, stops inside the committed 4,194,304–5,308,416 KiB window, and can resume
an interrupted owned allocation. A missing marker, unknown entry, symlink,
filesystem mismatch, another ADB device, or unexplained storage delta fails
closed. `status` and `prepare --dry-run` never mutate the device.

The Downloads allocation is operator preflight support, not physical evidence
and not part of the run-scoped fixture cleanup. Keep it in place for both
`low_storage` repetitions. The harness still creates and verifies its own
fixture-owned 1 GiB recovery-reserve file before allocating a bounded
fixture-owned filler. The filler is capped at 4 GiB and leaves 64 MiB of cleanup
headroom; the first reviewed copy uses a 128 MiB host fixture payload so the
destination receives genuine ENOSPC while the headroom remains available for
cleanup. Harness cleanup removes payloads, filler, sentinel state, and the
reserve in that order, then verifies the run directory is absent and restores
capacity to the preflight baseline.

After both repetitions, or when abandoning the storage qualification, remove
only the exact owned preflight directory:

```sh
node tools/device-storage-preflight.mjs cleanup \
  --serial <one-exact-serial> \
  --profile phase-6d6-low-storage
```

When the owned directory exists, cleanup requires its exact marker and rejects
mismatched markers or unknown entries. An already absent directory is a clean
no-op. Cleanup deletes no other Downloads content, synchronizes the device,
verifies the owned directory is absent, and reports available space before and
after.
A reserve, ownership, allocation-bound, cleanup, or restoration proof that is
missing blocks the repetition and cannot be counted as a pass.

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
| `device_unauthorized` | With the authorization-reset opt-in, wait for `boundary-ready`; revoke only the selected device's USB-debugging authorizations and create `authorization-revoked`; disconnect the selected cable, prove the exact serial absent for at least one canonical second, reconnect the same device without accepting the prompt, wait for `unauthorized-observed`, then create `operator-action`. After `terminal-ready`, authorize the same device, verify its exact row is `device`, then create `cleanup-ready`. Do not reset unrelated ADB state. | The first operation is completed before revocation; a real absent interval and same-serial unauthorized reconnect precede the second operation; the second operation fails before mutation with `device_unauthorized`, or with `device_identity_unverified` when the production identity guard fails first. Either branch requires the same independent authorization chronology; authority is invalidated, the slot is released, no automatic resume occurs, and final authorized cleanup is proven. |
| `identity_stability` | Perform one controlled reconnect at `boundary-ready` with the same device. | Repeated complete identity samples remain stable and the second operation succeeds. |
| `identity_replacement` | With explicit owner-approved hardware, disconnect the first target before attaching the same-serial replacement at `boundary-ready`. The harness polls successful ADB inventory samples and stable fingerprints, proving original attachment, serial absence, replacement attachment, and no simultaneous target. | `device_identity_changed` or `device_identity_unverified` before later mutation. If hardware is unavailable, record this exact case unqualified. |
| `root_revocation` | On a prepared rooted device, revoke EmuChef's adb-shell root authority after `boundary-ready`, acknowledge `operator-action`, wait for `terminal-ready`, restore root authority, then acknowledge `cleanup-ready`. | The second privileged command does not run, root failure is primary, prior mutation is retained, cleanup is separate and verified, and identity precedence is unchanged. |
| `operation_timeout` | No operator marker is required. The harness uses the exact hard-coded private pseudo-device paths `/dev/zero` (source) and `/dev/null` (destination) from the ignored qualification module and runs the reviewed first copy with `source.location == "device"` through `RealAdbDevice::copy_on_device`. No FIFO or other special file is created on the device; the copy is a real non-root device-side `cp /dev/zero /dev/null` that stays live until the owned-process timer wins. A private `#[cfg(test)]` scoped override selects a fixed 15-second qualification deadline; production `DeviceCopy` remains 300 seconds. | The same exact child is sampled alive about 12 seconds after its matching `DeviceCopy` mutation, then the real timer wins, kill/reap cleanup is confirmed, and `DeadlineReached` precedes `Terminal`. The result is `operation_timed_out` with the second step Not attempted, clean run-scope cleanup, and explicit timeout metadata. Any missing stimulus, liveness, deadline, confirmed cleanup, or residual proof blocks the repetition. |
| `low_storage` | With the separate destructive opt-in, first use the reviewed storage-preflight profile when the device is above the accepted free-space window. Keep that exact owned Downloads allocation for both repetitions. The harness then verifies at least 4 GiB free, creates/verifies the unique fixture-owned 1-GiB recovery reserve, allocates bounded filler, and waits for the checkpoint. | Genuine ENOSPC maps to `device_storage_exhausted`; no deletion/retry; harness cleanup removes only run-scoped payload/filler/sentinel/reserve and restores the preflight baseline. The separate profile-owned Downloads allocation is removed explicitly after both repetitions. Block the case if any ownership, bound, or restoration proof is unavailable. |
| `host_sleep_before_deadline` | Use the private `/dev/zero -> /dev/null` copy with the 120-second scoped qualification deadline. After `operation-started`, create `sleep-requested`, wait for `sleep-ready`, create `sleep-entered` within four seconds, immediately suspend the host, then create `wake` immediately after resume while the measured wake chronology is still before the 120-second deadline window. | The exact child was alive immediately before the `sleep-entered` handoff; before-sleep, after-wake, and owner-terminal deadline-clock samples derive from one exact basis; wall and executor elapsed time, remaining budget, tolerance, and timer classification are internally consistent; the terminal result agrees with owner lifecycle events; no second owner. |
| `host_sleep_after_deadline` | Use the same stimulus and handshake, but remain suspended long enough that wall time crosses the 120-second deadline before `wake`. | Same measured clock and consistency requirements. If suspended time is included, the deadline may become ready immediately on resume and terminal may precede the post-wake sample; if suspended time is excluded, executor budget may remain after wake and timeout may occur only after additional active-host time. Transport loss, missing samples, and indeterminate or contradictory measurements remain blocked. |

For every mutating case, the harness cleans only the fixture-owned destination
files and reports residual state independently from the operation result. Do
not modify boot images, modules, SELinux policy, system partitions, root
manager configuration, user data, or unrelated devices.

The timeout repetition is the one exception to the operator-action protocol:
the blocking source is the exact hard-coded private device copy
`/dev/zero -> /dev/null` executed through the production
`RealAdbDevice::copy_on_device` path, and the existing owned-process timer
performs the timeout, kill/reap, and terminal cleanup sequence. The copy
creates no persistent payload and requires no special-file creation on the
device. The unique run scope remains authoritative for run identity, evidence
binding, second-step fixture state, residual verification, and cleanup proof;
no attempt is made to delete, replace, or modify `/dev/zero` or `/dev/null`.
The 15-second value is a thread-local, one-shot test seam used only by this
ignored qualification entry point; ordinary production execution still
resolves `ProcessOperation::DeviceCopy` to its 300-second deadline. No
operator-marker or Terminal 2 procedure is required. The host-sleep
repetitions use the same private `/dev/zero -> /dev/null` copy with a
120-second scoped qualification deadline (see
Host-sleep stimulus and pre-suspend handshake).

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
still time out after enough later active time. The owned-process timer now
shares one exact monotonic start/deadline basis with the qualification
observations: before-sleep, after-wake, and owner-terminal samples derive from
that basis, and the retained basis keeps post-wake sampling truthful even when
the owner selected terminal immediately after resume. `sleepEnteredAt` is the
last operator/harness handoff immediately before physical suspension, not an
OS-level sleep-entry event; `wakeAt` is the first post-resume operator
acknowledgement, not an exact OS wake instant. The implementation blocker is
removed, but no physical host-sleep repetition is qualified until the operator
deliberately runs it against a physical device. `transport_loss` requires zero
owner-emitted `DeadlineReached` events: the owner event is the only authority
for whether the timeout branch won, and a monotonic clock sample at or beyond
the nominal deadline never converts a transport failure into a timeout.

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

## Development UI-smoke binding and capture

The development build provides a gated, evidence-bound terminal projection so
an operator can later run the mandatory UI smoke without replaying a physical
fault through the GUI. The checked-in
`docs/testing/phase-6d6/ui-binding-index.json` is derived metadata: it lists
only UI-contract-compatible passing physical bindings plus raw evidence/trace
and source digests. It is not qualification evidence and contains no authored
UI strings.

Validate the repository read-only, or regenerate the index explicitly after
the base evidence contract passes:

```sh
node tools/phase-6d6-evidence.mjs
node tools/phase-6d6-evidence.mjs --regenerate-ui-binding-index
npm --prefix apps/emuchef-app run phase6d6:ui-smoke:preflight
```

The qualification development app requires a debug build, the
`real-execution` Cargo feature, `EMUCHEF_RUN_REAL_ADB_TESTS=1`, and the exact
`EMUCHEF_PHASE_6D6_UI_SMOKE=1` opt-in:

```sh
npm --prefix apps/emuchef-app run tauri:dev:phase6d6-ui-smoke
```

In the qualification shell, select a subcase and physical repetition, load the
projection (no device or ADB is required), verify the normal terminal UI, and
capture the canonical sanitized `ui_state_capture` under
`docs/testing/phase-6d6/evidence/ui/`. Capture returns the artifact path and
digest, the backend run/trace binding, the trusted development-build identity,
and a deterministic `ui-subrun-sha256:*` identity. The operator observation and
the final `ui_smoke_composite` record remain deliberate manual evidence steps
and are never created by the application. No UI-smoke repetition has been run
or counted yet, and no compatible host-sleep binding exists, so host sleep is
shown unavailable.

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
node --test tools/device-storage-preflight.test.mjs
```

Phase 6D remains **In progress** until all twelve mandatory scenarios have two clean
passing repetitions, both composite UI-smoke repetitions pass, the automated
matrix passes, and any named mandatory hardware limitation has explicit owner
acceptance. Conditional `device_offline` evidence may remain unrun or blocked
without preventing closure; any attempted record must still report its outcome
truthfully. A blocked or unrun mandatory scenario is never reported as passing,
and Phase 6E must not start from this run.
