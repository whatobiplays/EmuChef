# Phase 0 End-User Runtime Contracts

## 1. Scope

Phase 0 is the backend contract for a future guided EmuChef application. It
adds product-facing catalog discovery, reviewed-plan integrity, target binding,
execution sessions, snapshots, and incremental events to the Rust sidecar. It
does not add an end-user UI, multi-device execution, catalog networking,
rollback, undo, or automatic state restoration.

The existing configuration editor operations remain available. New end-user
operations do not expose editor document ids, dirty state, YAML text, file
paths, undo/redo state, or recipe-authoring commands.

## 2. Capability negotiation

`hello` remains protocol version 1 and advertises the additive extension:

```json
{
  "protocolExtensions": [
    {"id": "phase0_end_user_runtime", "version": 1}
  ]
}
```

Clients call sidecar-only `negotiateCapabilities` with ordered
`requiredCapabilities` and `optionalCapabilities` arrays. The response reports
enabled and unsupported entries separately. Any unsupported required entry
makes `compatible` false; an unsupported optional entry does not. Phase 0 adds:

1. `describeCatalog`
2. `negotiateCapabilities`
3. `startExecution`
4. `getExecution`
5. `getExecutionEvents`
6. `cancelExecution`

## 3. Resolved catalog snapshots

Product discovery and planning receive a resolved local `catalog` snapshot:

```json
{
  "root": "/materialized/catalog",
  "sourceKind": "bundled",
  "sourceId": "emuchef.bundled",
  "version": "2026.07",
  "cacheKey": "bundled-2026.07",
  "contentDigest": {
    "algorithm": "sha256",
    "value": "<64 lowercase or uppercase hexadecimal characters>"
  }
}
```

`sourceKind` is `bundled` or `local_directory`. `cached_remote` is reserved so
a future component can materialize a remote catalog into a verified local
cache, but Phase 0 rejects it and performs no networking, update checks, or
signature verification. `sourceId`, `version`, and `cacheKey` describe catalog
identity and caching. The optional content digest is an independent integrity
claim and is never used as catalog identity or version.

When present, content integrity is SHA-256 over top-level YAML files under
`apps`, `recipes`, `device_profiles`, and `device_plans`. Files are sorted by
normalized relative path. Each input record is framed as
`<path-length>:<path><content-length>:<raw-content>`.

`describeCatalog` returns identity, ordered device-plan inventory, ordered
device-profile inventory, and recipe display/input/capability metadata. It
does not return the local snapshot root. `authoredRoot` remains supported only
as a legacy compatibility adapter for the pre-Phase-0 configuration operations;
new product operations require `catalog`.

## 4. Reviewed plans and digest

Product planning accepts optional `targetDevice` facts and returns a normalized
plan plus `planDigest`. Product plans retain catalog identity, ordered recipe
snapshots (`id`, `name`, and optional `description`), target binding, and steps
in normalized planner order.

The plan digest is lowercase SHA-256 over canonical UTF-8 JSON:

1. Serialize the complete execution plan as JSON.
2. Recursively sort every object key lexicographically.
3. Preserve array order exactly, including recipe and step order.
4. Emit no insignificant whitespace.
5. Hash the resulting UTF-8 bytes and encode lowercase hexadecimal.

`startExecution` requires both the complete reviewed plan and its digest. The
sidecar recomputes the digest immediately before accepting the apply attempt.
A mismatch fails with `plan_digest_mismatch`; no step is scheduled. Every
execution report contains the digest and a full immutable `reviewedPlan`
snapshot, making a terminal report self-contained.

## 5. Target binding and preflight

A target binding contains `serial` and optional reviewed `manufacturer`,
`model`, and `androidApiLevel`. Real execution requires a plan target binding
and performs an ADB `getprop` preflight before creating an active attempt.

Matching rules are:

1. Serial values are trimmed and compared exactly, including case.
2. Manufacturer and model values collapse internal whitespace, trim leading
   and trailing whitespace, and compare case-insensitively.
3. Android API levels compare as exact integers.
4. If a reviewed optional fact exists but the actual device omits it, matching
   fails.
5. A serial, manufacturer, model, or API mismatch fails with
   `target_device_mismatch` and identifies only the mismatched field.

Dry-run may validate caller-supplied target facts against the reviewed binding,
but it never probes ADB. Its target result is therefore simulated context, not
real-device identity evidence.

## 6. Sidecar startup policy

Filesystem roots are sidecar process policy, not execution-request input:

```text
emuchef --sidecar \
  --runtime-root /private/runtime \
  --cache-root /shared/artifacts \
  --adb /path/to/adb
```

Defaults are `.emuchef_runtime`, `.emuchef_cache/artifacts`, and `adb` beneath
the sidecar working directory. `startExecution` rejects `runtimeRoot` and
`cacheRoot`. Each attempt derives
`<runtime-root>/executions/<execution-id>` internally; dry-run fake-device state
is nested under that attempt directory. The cache root may be shared by
attempts. Host input and `file://` sources remain read-only sandbox roots.

## 7. Execution sessions and reports

One sidecar process permits one active execution and retains all completed
reports in memory until exit. Retained reports include:

1. execution id, plan id, digest, and full reviewed plan;
2. `real` or `dry_run` mode;
3. explicit `simulated` and `verificationScope` fields;
4. selected/actual target identity;
5. RFC 3339 UTC `startedAt` and terminal `finishedAt` strings;
6. overall status, warnings, and structured errors;
7. recipe groups in normalized plan order with name and description snapshots;
8. steps in normalized plan order with recipe ownership, technical name/id,
   human-readable note, status, message, and completed outputs;
9. the latest event sequence number.

Overall status is `running`, `succeeded`, `succeeded_with_warnings`, `failed`,
or `cancelled`. Step status is `pending`, `running`, `succeeded`, `skipped`,
`failed`, `blocked`, or `cancelled`. Recipe groups additionally use `blocked`
as a derived presentation state. Required blocked work always makes the overall
attempt `failed`; `blocked` is not an overall success alternative.

A dry-run uses fake-device adapters and is always marked `simulated: true` with
`verificationScope: simulated_only`. Its messages explicitly state that it
does not establish real-device execution or verification. Real attempts use
`verificationScope: real_device`.

## 8. Incremental events and snapshots

`getExecution` returns the current complete snapshot. `getExecutionEvents`
returns ordered events with a per-execution sequence starting at 1. Passing
`afterSequence` returns only events with a greater sequence, plus
`latestSequence` and `terminal`. Event timestamps are RFC 3339 UTC strings.

Events cover execution start, step progress phases, cancellation requests,
terminal completion, and worker panic. Step events carry recipe id, step id,
human-readable note, phase, optional status, and a stable message. Snapshots
remain authoritative and events are incremental delivery data; clients can
recover from a missed event by reading the snapshot and resuming after its
`latestSequence`.

## 9. Notes, failure, cancellation, and panic

Recipe steps may declare `progress_note`. The deterministic report/event note
fallback is:

1. non-blank `progress_note`;
2. non-blank step `name`;
3. humanized step `type` with `_` and `-` converted to spaces;
4. step id.

Failed and blocked dependencies mark downstream steps `blocked`. Blocked steps
do not resolve parameters, execute, or verify. Unrelated work may continue.
Artifact failures retain stable codes including
`artifact_tls_verification_failed`; GUI-facing issues associate codes with the
owning recipe and step without exposing volatile process output.

`cancelExecution` is cooperative. It records `cancel_requested`, allows the
current atomic operation to finish, schedules no new steps after cancellation
is observed, retains completed outputs, marks unscheduled work `cancelled`, and
performs no rollback.

The worker boundary catches panics. A panic produces a terminal failed report
and event with `execution_worker_panicked`, retains the report for inspection,
and releases the active execution slot.

## 10. Retry, repair, and no undo

Retry and repair mean resolving current catalog/configuration state, generating
and reviewing a fresh plan and digest, rerunning target preflight, and calling
`startExecution` to create a new execution id. Completed device changes from an
earlier attempt are not reversed. Skip conditions and idempotent steps may
avoid repeating already-satisfied work.

There is no execution rollback, undo, inverse-step generation, automatic
backup, or promise to restore prior device state. Editor document undo/redo is
unrelated and does not apply to device execution.
