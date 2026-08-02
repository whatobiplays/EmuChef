# Phase 6D.3 ADB Transport Failures

## 1. Scope and status

**Owner:** EmuChef proper / Shared Runtime  
**Status:** Automated implementation complete; physical interruption and device qualification pending  
**Date:** 2026-08-02

This slice classifies stable, completed ADB transport responses during the
existing fixed-serial real-execution path. It extends the Phase 6D.2 private
error path without changing process ownership, deadlines, output bounds,
sidecar framing, cleanup evidence, public schemas, execution statuses, or
recovery workflow. Real execution remains compile-time gated.

The evidence is deterministic host automation only. No physical Android
device, packaged GUI, release artifact, or production qualification is claimed.

## 2. Classification boundary

The private ADB adapter classifies only after an ADB process has completed and
returned bounded stdout/stderr and an exit status. The same centralized helper
is applied to:

- checked command paths;
- unchecked predicate, package, path, and launcher-resolution paths; and
- the direct root-probe executor path before root-result parsing.

Matching is case-insensitive and line-anchored to ADB-owned `error:` or
`adb: error:` prefixes, plus narrowly recognized daemon/server-owned lines.
Accepted forms include device offline, device unauthorized, serial-specific
device-not-found, no-device, daemon connection/startup, connection-reset,
ADB transport-error, response-read, and exact closed responses. A serial is
used only to recognize the form and is never retained. Remote shell text that
merely contains `connection reset`, `transport error`, `closed`, `cannot
connect`, or `failed to connect` remains ordinary command output/failure.

Timeout, spawn/resolution, wait, read, and output-overflow failures are typed
by the owned-process layer and bypass text classification. Successful output
remains available to trusted root, predicate, and activity parsing. Ordinary
completed nonzero exits become a sanitized `CommandFailed` kind without
commands, paths, exit codes, stdout, or stderr.

## 3. Private kinds and stable issue codes

The private adapter, executor, and session layers carry these typed kinds:

| Private kind | Stable issue code |
|---|---|
| `DeviceOffline` | `device_offline` |
| `DeviceUnauthorized` | `device_unauthorized` |
| `DeviceDisconnected` | `device_disconnected` |
| `AdbServerUnavailable` | `adb_server_unavailable` |
| `TransportReset` | `device_transport_lost` |
| `TransportFailure` | `device_transport_lost` |

Root denial, unavailable `su`, unknown root responses, ordinary command
failure, process/output failure, verification failure, and timeout remain
distinct private classifications. No failure-kind field is serialized.

## 4. Precedence and fail-stop behavior

Classification precedence is:

1. timeout remains timeout, including when partial text was captured;
2. output/read/wait/process failures remain process failures;
3. spawn/resolution remains host/process failure;
4. a completed nonzero recognized ADB response becomes a typed transport kind;
5. other root-probe failures retain root denial/unavailability/check-failure
   semantics; and
6. other completed nonzero exits remain ordinary command failures.

Timeout and every typed transport kind use one private device-safety fail-stop
predicate. At root preflight, skip predicates, operations, verification, and
permission actions, a matching failure fails the active step, preserves prior
evidence and active-operation outputs, stops permission actions, performs no
later device or host work, leaves later report steps `pending`, terminalizes
the run as failed, and releases the active execution slot. A failed real
atomic operation keeps `partialChangesPossible` true. Terminal pending work is
projected as **Not attempted**; no blocked/cancelled records are synthesized.

The result is not resumed, retried, reconnected, replayed, rolled back, or
checkpointed automatically. Another real run requires fresh device
qualification, a newly generated plan, fresh review, and a new execution
identity.

## 5. Sanitized Tauri and frontend projection

Tauri allowlists only the stable issue codes and authors the user guidance:

- offline: reconnect the reviewed device;
- unauthorized: authorize USB debugging for the intended reviewed device;
- disconnected/missing: reconnect the intended reviewed device;
- ADB server unavailable: repair the local ADB/Platform-Tools service; and
- transport lost/reset: the device connection was lost during execution.

Every remediation states that fresh qualification, plan generation, and plan
review are required and that reconnecting or repairing does not resume the old
execution. `reconnect_device` is used for device conditions; the server case
uses `repair_platform_tools`. Existing React issue cards, partial-change
warning, repair action, and terminal **Not attempted** rendering remain the
authoritative presentation. Raw serials, commands, paths, output, OS errors,
exit details, and arbitrary backend text are excluded from snapshots, events,
reports, diagnostics, and UI copy.

## 6. Explicit boundaries and follow-ups

This slice does not implement same-serial physical identity replacement
detection (Phase 6D.4), repeated root-authority revalidation or revocation
policy (Phase 6D.5), or physical offline/unauthorized/disconnect qualification
(Phase 6D.6). It also does not add ADB-server restart, reconnect loops,
automatic retry, resume, rollback, replay, checkpointing, public configuration,
protocol fields, or dependency changes.

Physical interruption, representative-device, host-sleep, low-storage,
packaged-GUI, release, and production qualification remain open work. Overall
Phase 6D remains **In progress**.
