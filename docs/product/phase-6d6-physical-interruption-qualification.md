# Phase 6D.6 Physical Interruption Qualification

**Owner:** EmuChef proper / Shared Runtime
**Status:** In progress. Automated contract remediation is implemented and physical evidence collection is underway, but the mandatory matrix, UI-smoke pair, and denied-warning Clippy gates remain incomplete. The validator is authoritative for accepted and missing repetitions.

## Contract

Phase 6D.6 closes the remaining interruption-policy slice without introducing
checkpointing, resume, replay, rollback, automatic retry/reconnect, persistent
execution, a public timeout, a new serialized status, or a host sleep plugin.
Real execution remains compile-time gated and disabled in ordinary production
builds. The production path under qualification is a reviewed
`ExecutionPlan` executed by `ExecutorRunner<RealAdbDevice>`.

The ignored harness is fail-closed. It validates the global and phase opt-ins,
one exact scenario, repetition `1` or `2`, one exact selected serial, the
`com.emuchef.fixture` package allowlist, the committed fixture roots, and an
empty test-owned sentinel directory before querying ADB. Root, storage,
authorization-reset, same-serial replacement, and host-sleep cases have extra
opt-ins. The runner lifecycle callback alone is not proof that an active
mutation child is alive. Active interruption records require exact run,
operation, child, spawn, mutation-start, pre-action liveness, action, and
terminal evidence. The harness binds a production-owned observation to the
first reviewed host `Push` and exposes `active-ready` only after sampling the
exact child alive. Same-serial replacement polls successful ADB inventory
observations rather than treating an operator marker as transition evidence.
Authorization qualification runs at a safe boundary: after the first reviewed
operation completes, the operator revokes trust and forces a same-serial
reconnect; the harness requires a real absence interval and a genuine
`unauthorized` row before releasing the second operation. The prior active
attempt remains non-passing audit evidence under its exact historical contract.
Every operator checkpoint is bounded to ten minutes; a missing, stale,
malformed, or aborted marker is blocked rather than passed.

## Automated contract evidence

The production storage classifier recognizes only bounded completed ADB output
with anchored ENOSPC evidence. It accepts the stable issue code
`device_storage_exhausted` for stdout or stderr and either completed exit code,
while preserving timeout, process/output, transport, identity, and root
precedence. Storage failure enters the existing device fail-stop path: prior
evidence and conservative possible-partial-change reporting remain, later
device/host work remains pending for terminal **Not attempted** projection, the
active execution slot is released, and no deletion, retry, or continuation is
attempted. Tauri projects only:

> The device ran out of storage during execution.

Recovery requires freeing device storage, fresh qualification, a newly
generated and reviewed plan, and a new execution; the old execution cannot
resume.

The private owned-process seams provide deterministic tests for a controlled
deadline signal and a one-shot process-delay regression bound to exactly one
`DeviceCopy` invocation. Delayed polling cannot turn a child that already
exited into an active operation or timeout. Tests retain timeout precedence,
child kill/reap, output bounds, panic cleanup, and parallel isolation. Identity
probes and every other operation class remain unaffected. Production still uses the existing fixed `async_io::Timer`
deadlines and no test delay. Existing cancellation, transport, identity, root,
partial-result, slot, projection, export, and sidecar-loss tests remain the
automated evidence for their invariants. Sidecar loss stays terminal
`runtime_session_lost`/`execution_unavailable`; it never creates a second owner
or an automatic resume path.

## Controlled physical-path harness

`crates/emuchef-rust-backend/src/executor_real_adb_tests/physical_interruption_qualification.rs`
contains one ignored entry point supporting thirteen physical scenarios.
Twelve are mandatory closure cases and `device_offline` is conditional
diagnostic evidence:

- cancellation during an active atomic operation and at a safe boundary;
- USB disconnect during an operation and at a boundary;
- a mandatory safe-boundary authorization-reset/reconnect transition and a
  conditional real-offline transition when a reviewed device-specific procedure
  exists;
- stable identity/reconnect and same-serial replacement (replacement remains
  explicitly unqualified if appropriate hardware is unavailable);
- root revocation between two privileged atomic commands, followed by a
  bounded `cleanup-ready` checkpoint after root authority is restored;
- deterministic owned-process timeout regression evidence, which is not a
  physical repetition and cannot be promoted by relabeling;
- low storage using only a fixture-owned destination and a one-GiB reserve;
- host sleep before and after the fixed deadline; and
- development-build Tauri/React recovery smoke for cancellation, transport,
  root, storage, and host sleep.

Each mutating attempt begins from clean fixture state, creates a unique
run-scoped fixture directory, and runs under the same RAII guard that owns the
production execution-session slot. Evidence carries unique run, scope,
sentinel, nonce, slot, record path, trace path, canonical record digest, and
trace digest identities; create-new writes prevent overwriting an earlier
attempt. The harness records a terminal step summary and performs bounded
cleanup independently from the operation outcome. The low-storage case proves at least 4 GiB initial free
space (bounded to 5,308,416 KiB so the filler remains safe), creates and
verifies a fixture-owned 1 GiB recovery reserve before its bounded 4 GiB
maximum filler, and leaves 64 MiB of cleanup headroom. The first reviewed copy
uses a generated 128 MiB host fixture payload so the destination receives
genuine ENOSPC without consuming the recovery reserve. Payload, filler,
sentinel, and reserve are removed only within that run scope. Cleanup proves
the reserve and final free capacity were restored; inability to prove ownership
or restoration blocks the repetition.
Raw serials, paths, command output, credentials, and payloads never enter
evidence.

ADB offline is normally a transient transport initialization or failure state
rather than a generally controllable physical condition. The harness, schema,
and validator retain full support for truthful offline attempts, but absence of
a reproducible `device → offline → device` procedure does not block closure.
The two USB-disconnect scenarios provide the mandatory active and boundary
physical transport evidence.

## Evidence contract

`docs/testing/phase-6d6/evidence-schema.json` is the strict machine-readable
record contract. `evidence-template.json` is illustrative only and is never
counted as a physical repetition. The dependency-free
`tools/phase-6d6-evidence.mjs` validator checks the authoritative per-scenario
contract, exact gates, expected issue and step/Not-attempted accounting,
partial-change and authority disposition, observable active-slot release,
sentinel chronology/uniqueness, low-storage reserve facts, cleanup ownership,
residual state, sanitization, duplicate repetitions, and matrix completeness.
`tools/phase-6d6-result.mjs` parses the run-local RESULT schema and rejects
missing tests, unauthorized changed files, or a false Phase 6D closure. The
evidence validator intentionally exits successfully with an explicit “valid
but incomplete” result when the evidence directory is empty so CI can verify
the harness without touching ADB.

Host-sleep evidence can pass only when it records the deadline clock at start,
before sleep, after wake, and terminal; remaining budget on both sides of the
suspension; wall suspension; tolerance and rationale; phase; terminal outcome;
and host/toolchain facts. Classification is derived from clock advancement and
budget consumption, never from completion or timeout. The current physical
harness cannot observe that exact production deadline clock, emits a null
measurement, and blocks both host-sleep paths. A
transport failure, indeterminate/contradictory timing, or missing marker is
blocked rather than treated as timer evidence. Development-build UI smoke is a
mandatory pair of composite records; each repetition contains cancellation,
transport, root, storage, and host-sleep/runtime-loss subcases with distinct
backend run/trace bindings, sub-run identities, canonical UI-state artifacts,
artifact digests, exact authored projections, and sanitized observations. The
transport subcase must bind to a passing active or boundary USB-disconnect
record; conditional offline evidence cannot satisfy the mandatory UI binding.

Every genuine attempt must record scenario/repetition, timestamp, commit, host
OS/version/architecture, Platform-Tools revision, sanitized identity and
device facts, root version only for root cases, fixture checksum, exact opt-ins
with the serial redacted, the fixed harness command, preparation/operator
action, observed execution success, the contract snapshot, scenario facts,
sentinel chronology, observed issue code, exact step-state counts, partial-
change possibility, authority invalidation, an observable active-slot release,
cleanup ownership digests/outcome, residual-state result, storage reserve facts
when applicable, outcome, and sanitized notes. Each mandatory case is complete
only after two clean passing records. A failed, skipped, blocked, or absent
mandatory record keeps Phase 6D open. Conditional offline records remain valid
audit evidence but do not affect completeness. Failed or blocked records remain
valid audit evidence when they truthfully report non-clean cleanup or residual
state; they are never counted as passing repetitions.

## Current disposition

Physical evidence collection is underway, but the twelve-scenario mandatory
matrix and two UI-smoke repetitions remain incomplete. `device_offline` is
conditional diagnostic evidence and may remain unqualified without blocking
closure; any attempted offline record must still satisfy the full evidence
contract. The exact backend and Tauri `clippy -D warnings` commands also retain
pre-existing findings outside the authorized Phase 6D.6 paths. Same-serial
replacement still requires suitable hardware or explicit owner acceptance.
Phase 6D remains **In progress** and Phase 6E has not started. Signing,
notarization, packaged-GUI, and release qualification remain outside this slice.
