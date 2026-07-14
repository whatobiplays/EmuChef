# Phase 2B Guarded Real-Device Execution

## 1. Status and product boundary

Phase 2B defines the planned trust boundary and product behavior for guarded
real-device execution in `apps/emuchef-app`. Canonical backend artifact
admission is implemented as prerequisite groundwork, but guarded real-device
execution is not an implemented product capability. The shipped end-user
application remains limited to the Phase 2A Simulated Run workflow and does not
expose real execution.

The future product implementation reuses the Phase 0 `startExecution`,
`getExecution`, `getExecutionEvents`, and `cancelExecution` sidecar operations.
It adds no sidecar protocol extension. React communicates only with trusted
Tauri commands and never calls those sidecar operations directly.

Phase 2B is disabled in ordinary builds until the rollout gates in this
document are satisfied for a specific platform and a separate release decision
enables it. Detecting that the sidecar supports `mode: real` is not sufficient
to enable the product surface.

## 2. User workflow and confirmation

Real execution is a separate action from Simulated Run. It is presented only
after a complete target-bound review and only when the platform-specific
real-execution gate is enabled. Simulation remains available under its Phase 2A
contract and is never described as equivalent to real execution.

Before a real start request, the confirmation view shows:

1. a generic connected-device label;
2. public target facts already available to the review UI, such as
   manufacturer, model, Android version, and Android API level;
3. the reviewed recipes and human-readable action summary;
4. a warning that execution can install software, transfer or replace files,
   change permissions and app operations, launch or stop applications, and
   otherwise mutate the connected device;
5. a warning that cancellation is cooperative and cannot undo work that has
   completed;
6. a warning that EmuChef provides no rollback, restore, automatic backup, or
   prior-state recovery; and
7. an instruction to keep the intended device connected and stable until the
   attempt becomes terminal.

The confirmation view does not show an exact serial, a shortened serial, a
serial suffix, or any other serial-derived display value. Tauri retains the
exact serial internally and uses it for target validation.

The user must type the exact phrase `APPLY TO DEVICE` and affirm all three
acknowledgments:

```json
{
  "phrase": "APPLY TO DEVICE",
  "irreversibleChangesAcknowledged": true,
  "noRollbackAcknowledged": true,
  "keepDeviceConnectedAcknowledged": true
}
```

Tauri trims leading and trailing whitespace from `phrase` and then requires a
case-sensitive match. React may disable the start control until its local
checks pass, but Tauri performs the authoritative validation. Confirmation is
used for one start request only, is not persisted, and is not converted into a
reusable token. Any rejected or failed start requires the user to reopen the
confirmation and confirm again.

## 3. Public Tauri contract

The later implementation adds a real-execution command family without
changing the Phase 2A command behavior:

1. `start_real_execution` accepts an opaque `reviewHandle` and the confirmation
   object from Section 2.
2. `get_real_execution` accepts only an opaque `executionHandle`.
3. `get_real_execution_events` accepts an opaque `executionHandle` and a
   non-negative `afterSequence`.
4. `cancel_real_execution` accepts only an opaque `executionHandle`.

React cannot submit an execution mode, complete plan, plan digest, target
binding, device serial, ADB executable path, catalog root, runtime root, cache
root, sidecar execution id, or artifact location through these commands.
Supplying unexpected trusted fields is rejected rather than ignored.

The trusted start payload sent from Tauri to the existing sidecar operation is:

```json
{
  "plan": "<exact retained reviewed plan>",
  "planDigest": "<retained canonical digest>",
  "mode": "real",
  "targetDevice": "<exact retained target binding>"
}
```

Only Tauri constructs this payload and selects `mode=real` (`"mode": "real"`
on the wire). The sidecar
continues to reject request-level ADB, runtime-root, and cache-root policy.

## 4. Trusted start sequence

The trusted real-start path performs the following sequence. Tauri owns the
client-side gate, confirmation, reservation, retained-state checks, and sidecar
request; `startExecution` owns the sidecar processing identified below.

1. Check the platform-specific real-execution rollout gate. If it is disabled,
   return `real_execution_disabled` without inspecting or reserving execution
   store capacity.
2. Validate the complete confirmation object before reservation where
   practical. Invalid confirmation returns
   `real_execution_confirmation_invalid` and consumes no store capacity.
3. Reserve the single shared simulated/real start slot. A reservation or active
   run of either mode returns `execution_in_progress`.
4. Resolve the retained review and enforce its existing idle, absolute,
   tombstone, stale, expired, and unknown-handle rules.
5. Recompute and compare the current catalog source identity, version, and
   content digest with the retained review.
6. Resolve the currently validated managed Platform-Tools installation and
   verify that it is still the installation associated with the review.
7. Refresh device inventory, resolve the retained opaque device handle, and
   require that its internally retained exact serial is connected and
   available.
8. Probe fresh device facts and compare the exact serial, normalized
   manufacturer and model, and exact Android API level with the retained target
   binding under the Phase 0 matching rules.
9. Recompute the canonical JSON SHA-256 of the exact retained plan and compare
   it with the retained plan digest.
10. Where the retained review contains user-supplied local file or directory
    input values already owned by Tauri, verify that they still exist, have the
    expected kind, and are readable. This is a limited trusted-state check, not
    canonical artifact admission.
11. Send the exact retained plan, digest, and target to `startExecution` while
    trusted code supplies `mode: real`. The sidecar repeats canonical digest
    validation and real-device target preflight at its authoritative boundary,
    then performs the canonical artifact admission defined in Section 5.
12. Only after target preflight and canonical artifact admission succeed may
    the sidecar allocate, accept, and start the execution attempt.
13. Create and bind a random public execution handle only after the sidecar has
    returned a successful execution record.

Tauri releases the reservation after every failure following reservation,
including lock errors, review failures, catalog drift, Platform-Tools failure,
inventory or probe failure, target mismatch, digest mismatch, artifact
input failure, sidecar transport failure, and every rejected sidecar start,
including canonical artifact-admission failure. No public execution handle is
allocated for a failed start.

Start revalidation does not repeat profile matching, select another device
plan, merge new inputs, or replan. Any changed reviewed state requires a fresh
configuration description and review.

## 5. Artifact readiness and ownership

Artifact readiness is an admission check, not a pre-download phase and not a
guarantee that a later transfer will succeed. The real-start request accepts no
artifact path, URL, cache root, runtime root, or admission override.

Tauri may validate only user-supplied local file and directory input values
that are already retained trusted review state. It may check their current
existence, expected file/directory kind, and readability so an obviously stale
BYO input returns `artifact_not_ready` before the sidecar request. Tauri does
not interpret `file://` sources, inspect artifact cache eligibility, validate
HTTP(S) source policy, make sandbox decisions, or duplicate canonical resolver
rules.

Canonical artifact admission is trusted processing inside the existing
`startExecution` sidecar operation. It is not a new public preflight or
admission operation and adds no request field. After canonical digest
validation and authoritative real-device target preflight, but before an
execution id, report, active-session record, or worker is allocated, the
sidecar evaluates the exact retained plan against its configured runtime,
cache, and read-only sandbox roots.

Sidecar admission directly reuses the canonical ArtifactResolver URL, cache,
destination, local-source, and sandbox rules. The resolver factors one
non-mutating admission path from those rules rather than reproducing them in
Tauri or maintaining a second policy. Admission determines whether:

1. an authoritative complete cache hit is structurally admissible;
2. a `file://` source is absolute, permitted by the read-only sandbox, and
   presently a readable source of the required kind;
3. an HTTP(S) source has a supported, structurally valid source URL;
4. the selected runtime/cache destination is permitted by the authoritative
   sandbox policy; and
5. the retained artifact definition is supported by the current executor
   contract.

Admission performs no DNS lookup, connection attempt, HTTP request, download,
extraction, directory creation, partial-file creation or cleanup, cache
mutation or publication, ADB operation, staging, transfer, or device write.
Cold HTTP(S) sources may therefore pass admission and later fail during
execution. A filesystem or cache race after admission is also reported as a
runtime failure; the executor remains authoritative when it acts.

A canonical admission failure rejects `startExecution` before attempt
allocation. Admission runs under the execution-state lock so the prospective
execution number and single-active invariant remain atomic, but it creates no
persistent reservation or session state and does not reacquire execution
state. The sidecar returns `execution_start_failed` with the safe discriminator
`artifact_not_ready` and a stable artifact cause code; Tauri maps that failure,
releases its shared start reservation, and allocates no public handle. Raw
artifact identifiers, paths, URLs, credentials, sandbox roots, and internal
details are absent from the rejection.

Only after admission succeeds does the existing executor own artifact
resolution, strict TLS, redirect and timeout limits, cache hits and
publication, unique partial files, partial cleanup, archive extraction, host
staging, device transfer, and device mutation. Phase 2B does not download,
redistribute, mirror, proxy, or update catalog content or Android
Platform-Tools outside that existing execution ownership.

## 6. Execution store and handle lifecycle

Simulated and real attempts use one shared Tauri execution store:

1. at most one start reservation or active mapping exists across both modes;
2. at most one latest-terminal mapping exists across both modes;
3. the existing latest terminal may remain inspectable while another attempt
   is active;
4. when the active attempt becomes terminal, it replaces the previous terminal
   mapping regardless of mode; and
5. dropped, evicted, or restarted handles are never reused.

Each mapping records its execution kind internally. A real command rejects a
simulation handle, and a simulation command retains its existing behavior.
Public execution handles are random and session-scoped. The sidecar execution
id remains trusted-only.

The retained review has an independent lifecycle after a successful start.
Expiry, discard, or later staleness of the original review does not cancel an
active attempt or erase its report. Retry and repair always require a fresh
plan, digest, target validation, confirmation, and execution handle.

## 7. Snapshots, events, and result projection

`getExecution` snapshots are authoritative. Every accepted snapshot replaces
the public recipe, step, warning, error, timestamp, sequence, and terminal
state for that execution. `getExecutionEvents(afterSequence)` is
presentation-only delivery data. Events are sorted and deduplicated by
sequence, never override snapshot state, and are recoverable through the next
snapshot.

The public real-execution snapshot contains only:

1. the public execution and review handles;
2. `simulated: false` and `verificationScope: real_device`;
3. overall and grouped recipe/step status;
4. public recipe and step names, normalized notes, and allowlisted safe
   messages;
5. sanitized warnings and errors;
6. RFC 3339 start and finish timestamps, latest sequence, and terminal state;
   and
7. public target facts that do not derive from the device serial.

The projection omits the complete reviewed plan, exact target binding, every
serial representation, catalog root, sidecar execution id, step outputs,
arbitrary filesystem paths, source URLs containing credentials/query/fragment,
raw ADB or process output, and raw sidecar errors. Release builds do not log
trusted raw errors. Debug logging follows the existing trusted-only redaction
policy and never writes forbidden values to the frontend console.

Polling is non-overlapping, resumes after an accepted snapshot's
`latestSequence`, stops after a terminal snapshot, and clears timers when the
view is disposed. Ordinary snapshot and event refresh failures both use
`execution_status_failed`. A missing sidecar in-memory execution session uses
`execution_unavailable`.

## 8. Disconnection, cancellation, and failure

### 8.1 Device disconnection

Disconnection before the sidecar accepts the start blocks execution with
`device_disconnected`, releases the reservation, and allocates no execution
handle.

After start, Phase 2B adds no device-monitor-driven continuation or automatic
cancellation policy. Device operations fail under the existing executor
semantics. Independent steps may proceed only when the canonical plan and its
existing dependency graph already permit them; the UI must not promise that
unrelated work generally continues. Reconnecting does not resume a failed or
blocked step and does not retry an ADB operation in place.

A disconnect after terminal completion does not alter or erase the retained
terminal report. Device inventory may separately invalidate the originating
review and return the guided workflow to its safe connection state.

### 8.2 Cooperative cancellation

Cancellation requests use the existing sidecar operation. Cancellation is
cooperative between atomic operations:

1. completed steps and device mutations remain visible and are not reversed;
2. the current ADB command, transfer, extraction, or other atomic operation may
   finish before cancellation is observed;
3. no new step is scheduled after cancellation is observed;
4. unscheduled work becomes cancelled under the Phase 0 report contract; and
5. polling continues until an authoritative terminal snapshot is received.

Cancellation does not promise rollback, restoration, backup recovery, removal
of transferred files, package uninstall, permission reversal, or termination
of the current atomic operation.

### 8.3 Dependency and worker failure

Failed work blocks its dependents while independent work follows the existing
normalized plan and dependency graph. Required blocked work produces an
overall failed attempt. A caught worker panic produces the existing terminal
`execution_worker_panicked` issue, leaves the report inspectable, and releases
the sidecar active slot.

## 9. Restart and lost-session behavior

Whole-application restart and sidecar execution-session loss are distinct:

1. **Whole-application restart:** all device, review, and execution handles are
   memory-only and are lost. The app does not infer whether a previously active
   real attempt succeeded, failed, or partially changed the device. The user
   must reconnect, re-probe, recreate the configuration state, replan, and
   generate a fresh review before another real start.
2. **Sidecar execution-session loss while Tauri remains alive:** the public run
   becomes unavailable with `execution_unavailable`. Tauri invalidates the
   originating review before allowing another real start. The UI states that
   the device may have been partially changed and that the outcome is unknown.
   It never synthesizes a failed, cancelled, or successful terminal result.

Neither case supports execution persistence, session recovery, resume,
reattachment, automatic retry, or retry in place.

## 10. Public error taxonomy

Tauri returns stable codes and fixed recovery-oriented messages. Raw sidecar,
ADB, filesystem, URL, serial, and process details never appear in public error
strings.

| Code | Class and required behavior |
| --- | --- |
| `real_execution_disabled` | The platform/build rollout gate is off; no capacity or trusted state is consumed. |
| `real_execution_confirmation_invalid` | The phrase or any acknowledgment is missing or invalid; no capacity is consumed. |
| `execution_in_progress` | A simulated or real start reservation/active mapping already occupies the shared slot. |
| `review_stale` | Catalog, Platform-Tools, plan digest, retained target, or other reviewed state changed; generate a fresh review. |
| `review_expired` | The review exceeded its idle or absolute lifetime; generate a fresh review. |
| `review_unknown` | The handle was never known or its bounded tombstone aged out; generate a fresh review. |
| `platform_tools_unavailable` | The validated managed ADB installation cannot be used; repair Platform-Tools and generate a fresh review. |
| `device_disconnected` | The retained target is not connected and available before start. |
| `artifact_not_ready` | A limited retained BYO input check in Tauri or canonical sidecar artifact admission failed before attempt allocation. |
| `real_execution_start_failed` | The trusted sidecar start failed without a more specific safe mapping. |
| `execution_state_unavailable` | Tauri could not access its trusted execution store. |
| `execution_unavailable` | The public handle was evicted/lost or the sidecar no longer retains the execution session; never infer an outcome. |
| `execution_status_failed` | An ordinary snapshot or event refresh failed while the session may still exist. |
| `execution_cancel_failed` | The cancellation request could not be delivered or accepted. |

Terminal report issues use only curated stable codes, including current
artifact, dependency, verification, capability, conflict, generic step, and
worker-panic codes. Messages are allowlisted or sanitized before projection.
An unfamiliar internal code maps to a generic safe issue instead of passing
through raw details.

## 11. Terminal presentation and evidence boundary

The terminal view remains explicitly labeled Real Device. It distinguishes
`succeeded`, `succeeded_with_warnings`, `failed`, and `cancelled`, shows blocked
required work as failure, and states that cancellation or failure may leave
completed changes on the device. It offers no Resume, Retry in Place, Undo,
Restore, or Roll Back action. A new attempt begins from a fresh review.

A terminal real-device result is product feedback, not release evidence by
itself. It does not prove the identity of the packaged application, source
commit, host, target, catalog, cache conditions, or post-run device state.

Release evidence requires a separate maintained manual procedure that records
the exact tested commit and packaged build, uses a disposable device, verifies
the intended cold/warm/offline and post-run scenarios, and records only a
sanitized repository summary. Raw logs, device identity, operator identity,
screenshots, exact host details, and user-specific paths remain operator-held
outside the repository. Evidence for one build, platform, or scenario does not
establish another.

## 12. Non-goals

Phase 2B does not define or add:

1. rollback, restore, reverse steps, execution undo, automatic backup, or a
   promise to return the device to its prior state;
2. resume, persistence, reattachment, automatic retry, or retry in place;
3. multiple active attempts, parallel devices, or a second execution slot;
4. wireless ADB discovery or onboarding;
5. remote catalog synchronization, catalog updating, or catalog networking;
6. Android Platform-Tools download, redistribution, mirroring, proxying, or
   automated license acceptance;
7. a sidecar protocol extension or request-supplied runtime policy;
8. a React-visible serial representation, sidecar id, full plan, path, output,
   or raw error; or
9. any change to Phase 2A simulation behavior or its evidence disclaimer.

## 13. Rollout gates

The later implementation must keep real execution disabled in ordinary builds
until all of these gates pass:

1. the complete automated trust-boundary and lifecycle matrix in Section 15;
2. near-complete branch coverage for new confirmation, preflight, store,
   projection, redaction, cancellation, and lost-session code;
3. packaged-application validation on a disposable device for the specific
   platform, including successful, failed, cancelled, disconnected, cold-cache,
   warm-cache, and offline warm-cache scenarios;
4. privacy and security review confirming that forbidden trusted values do not
   enter React payloads, frontend state, logs, storage, or markup;
5. a maintained operator runbook and sanitized evidence format; and
6. an explicit release decision enabling real execution for that platform.

Enablement is platform-specific. Passing macOS evidence does not enable Windows
or Linux. The enabled state must be explicit product policy; capability
negotiation, a connected device, or successful simulation cannot enable it.

## 14. Remaining implementation checklist

The canonical backend admission prerequisite is implemented inside
`startExecution`. The remaining product work is:

1. Add the default-off platform/build gate and enforce it before store access.
2. Add the real confirmation DTO and trusted validation without persisting it.
3. Generalize the internal execution mapping with an execution-kind field while
   preserving every Phase 2A command and response contract.
4. Add the four real-execution Tauri commands from Section 3.
5. Implement Tauri's exact ordered retained-state preflight and reservation
   cleanup from Section 4, limiting local input checks to trusted retained BYO
   values.
6. Build a separate real projection that exposes only the allowlisted fields in
   Section 7 and sanitizes every issue and message.
7. Implement the public error mapping in Section 10 and distinguish ordinary
   refresh failure from session loss.
8. Invalidate the originating review on sidecar real-session loss and implement
   the distinct whole-app restart UX.
9. Add the confirmation, progress, cancellation, terminal, unknown-outcome,
    and fresh-review UX without displaying any serial-derived value.
10. Add unit, integration, frontend logic, security, packaged-app, and manual
    disposable-device coverage from Section 15.
11. Complete the rollout evidence and make enablement a separate reviewed
    release change.

## 15. Phase 2B test matrix

| Area | Required scenarios |
| --- | --- |
| Gate and confirmation | Disabled gate touches no store; wrong phrase, case, omitted field, and each false acknowledgment consume no capacity; valid confirmation is single-use and not persisted. |
| Start payload | React can submit only the opaque review handle and confirmation; Tauri alone supplies the retained plan, digest, target, and real mode; unexpected trusted fields are rejected. |
| Reservation | Simulation blocks real and real blocks simulation; every Tauri or sidecar rejection, including canonical admission failure, releases Tauri capacity; neither sidecar attempt state nor a public handle exists before successful admission and start. |
| Review and catalog | Live, stale, expired, unknown, evicted, discarded, catalog-changed, and digest-changed reviews return the specified safe code and recovery. |
| Target and ADB | Missing/replaced Platform-Tools, disconnected/offline/unauthorized target, exact-serial mismatch, normalized manufacturer/model mismatch, API mismatch, and sidecar target-preflight failure allocate no handle and leak no identity. |
| Tauri BYO input checks | Retained user-selected file/directory exists, has the expected kind, and is readable; missing, wrong-kind, and unreadable values fail safely; Tauri performs no URL, cache, or sandbox admission. |
| Sidecar artifact admission | Existing `startExecution` performs canonical cache-hit, `file://`, HTTP(S), destination, and sandbox admission before attempt allocation; valid cold HTTP(S) performs no request; failures are credential-safe and map to `artifact_not_ready`; no new public operation exists. |
| Artifact runtime | Successful cold/warm/offline cache paths plus TLS, redirect, timeout, HTTP, partial-cleanup, extraction, and cache-publication failures use stable sanitized issues without leaking URLs or paths. |
| Shared store | One reservation/active mapping plus one terminal mapping across both modes; old terminal remains during an active run; new terminal replaces it; evicted handles fail safely; handles are never reused. |
| Snapshots and events | Authoritative replacement, monotonic sequence, sorting, deduplication, missed-event recovery, stale-generation rejection, non-overlapping polling, terminal stop, and ordinary refresh failure mapping. |
| Disconnection | Before-start block; current/next device-operation failure during execution; only dependency-graph-permitted independent work proceeds; reconnect does not resume; terminal report survives later disconnect. |
| Cancellation | Request before and during atomic work, current operation allowed to finish, no later scheduling, completed mutations retained, terminal cancellation, repeated cancellation, and delivery failure. |
| Restart and session loss | Whole-app restart loses all handles and requires a fresh workflow; sidecar session loss marks outcome unknown, invalidates the originating review, and prevents resume or same-review restart. |
| Projection and privacy | No serial representation, target binding, plan, sidecar id, catalog root, arbitrary path, step output, credential-bearing URL, raw process output, or raw error appears in IPC, React state, logs, storage, or rendered markup. |
| Terminal and evidence | Every terminal status, warnings, blocked failure, partial-change warning, and evidence disclaimer renders correctly; no rollback, restore, resume, or retry-in-place control exists. |
| Rollout | Ordinary builds remain disabled; enablement is explicit and platform-specific; packaged safe-device evidence and privacy/security approval are required before release enablement. |

The implementation is acceptable only when these scenarios pass without
regressing the complete Phase 2A simulation suite.
