# Phase 2A Simulated End-User Execution

## 1. Product boundary

`apps/emuchef-app` extends its guided review workflow with a Simulated Run
stage. The stage executes the exact retained reviewed plan through the Phase 0
execution-session API in `dry_run` mode. It uses fake-device adapters and makes
no real device changes. A simulated result is not real-device verification or
release evidence.

Phase 2A does not expose real execution, artifact transfer, execution history,
persistence, resume, rollback, device restoration, parallel execution, remote
catalog synchronization, or configuration authoring.

## 2. Trusted start and revalidation

React starts a simulation by submitting only an opaque `reviewHandle`. It
cannot submit a plan, digest, target serial, catalog path, execution mode, ADB
path, runtime root, or cache root. Trusted Tauri code resolves the retained
immutable review and performs these checks before calling `startExecution`:

1. the review remains live under its normal stale and expiry lifecycle;
2. the bundled catalog source identity, version, and content digest match the
   reviewed catalog;
3. the reviewed device is still connected and available;
4. the retained serial, manufacturer, model, and Android API level match a
   fresh probe using the Phase 0 target-preflight normalization rules; and
5. a newly computed canonical JSON SHA-256 digest matches the retained plan
   digest.

Target revalidation does not repeat device-profile confidence matching and
does not replan. Tauri passes the exact retained plan, digest, and target to the
existing `startExecution` operation and hard-codes `mode: dry_run`. The sidecar
performs its own canonical digest verification as the authoritative execution
boundary.

A disconnected or changed target blocks start with a sanitized stale-review
error. After a dry run starts, it no longer depends on the real device. A later
disconnect neither cancels the simulation nor removes its report.

## 3. Execution handles and lifetime

Tauri maps random, session-scoped public execution handles to sidecar execution
identifiers. Handles are never reused and are lost on restart. The store is
bounded to one start reservation or active mapping plus the latest terminal
mapping. Replacing the terminal mapping permanently drops the older handle.

The active slot is reserved before preflight and released after every preflight
or `startExecution` failure. A public execution handle is created and bound to
the sidecar identifier only after `startExecution` succeeds. The retained
review remains independently available during and after simulation until its
ordinary expiry, stale, discard, or capacity lifecycle invalidates it.

There is no execution resume after an app or sidecar restart. If a sidecar loses
its in-memory session while the UI remains open, the app reports the run as
unavailable and offers a return to the retained review when it remains valid.
Otherwise the user must reconnect and generate a new review.

## 4. Progress and cancellation

`getExecution` snapshots are authoritative. Each accepted snapshot replaces
the displayed recipe, step, warning, error, timestamp, and terminal state. The
frontend rejects snapshots from an older generation, another handle, or a
lower sequence. After accepting a snapshot, event polling resumes after that
snapshot's `latestSequence`.

`getExecutionEvents(afterSequence)` supplies incremental presentation events
only. Events are sorted and deduplicated by their monotonic sequence and never
override snapshot state. A missed event does not lose progress because the
next complete snapshot restores the current report. Polling is non-overlapping,
stops at a terminal snapshot, and clears its timer when the execution view is
disposed.

Progress is grouped in retained recipe order and shows human-readable recipe
and step names, normalized notes, status, safe messages, warnings, timestamps,
and duration. Reports handle `running`, `succeeded`,
`succeeded_with_warnings`, `failed`, and `cancelled`; recipe and step reports
also show blocked work. Blocked required work remains an overall failure.

Cancellation is cooperative. Completed simulated steps remain visible in the
report, no new simulated steps start, the current simulated atomic step may
finish, and no real device changes or rollback exist. Polling continues until
the authoritative snapshot becomes terminal.

## 5. React trust boundary

React receives only opaque review/execution handles and projected execution
DTOs. The projection omits the full reviewed plan, target binding, exact serial,
catalog root, sidecar execution identifier, step outputs, arbitrary filesystem
paths, and raw sidecar errors. Every execution screen and terminal result is
labeled Simulated / Dry Run and explicitly disclaims real-device evidence.
