# Phase 6D.4 Same-Serial Identity Replacement

## 1. Scope and status

**Owner:** EmuChef proper / Shared Runtime  
**Status:** Automated implementation complete; physical interruption and device qualification pending  
**Date:** 2026-08-02

This slice detects a conservative class of same-serial device replacement at
the existing real-execution safe boundaries. ADB remains bound to the exact
serial retained by the reviewed plan, while a private non-serial fingerprint
checks that the connected device still matches the reviewed target. The
fingerprint, probe output, mismatch details, and timing remain private Rust
state. No public protocol, execution status, DTO, schema, manifest, or
configuration field changes.

Evidence in this document is deterministic host automation only. No physical
Android device, packaged GUI, release artifact, production build, or hardware
qualification is claimed.

## 2. Threat and safety model

ADB serial continuity is necessary for a reviewed command but is not sufficient
proof that the same physical device is still connected. A device can be
replaced while a serial is reused. The guard therefore establishes private
read-only evidence before the first device-bound operation and revalidates it
at every executor-visible device boundary. A mismatch or inability to obtain
sufficient evidence fails closed, stops the real run at the current atomic
step, and never reconnects, retries, resumes, replays, rolls back, or performs
automatic recovery.

The guard does not claim cryptographic hardware identity or attestation. It is
an automated conservative detector for same-serial replacement between safe
boundaries; sub-command races and platform-specific identity changes remain
possible and are treated conservatively.

## 3. Private fingerprint and sufficiency

The private `DeviceIdentityFingerprint` contains the normalized:

- manufacturer;
- brand;
- model;
- product name;
- product device;
- board;
- hardware;
- optional hardware SKU;
- Android SDK level;
- ordered ABI list;
- build fingerprint; and
- secure Android ID as the required non-serial per-device discriminator.

The selected ADB serial and transport ID are not fingerprint fields. Volatile
properties such as uptime, boot completion, battery state, timestamps, process
IDs, and connection IDs are ignored. The raw property text, Android ID, build
fingerprint, commands, and any derived hash are never logged or projected.

Text properties are trimmed and whitespace-canonicalized. Stable properties
and Android ID compare lowercase; meaningful build-fingerprint case is
preserved. The Android ID accepts one through sixteen hexadecimal characters
after normalization, is compared lowercase, and rejects missing or empty
values, non-hex text, values longer than sixteen characters, numeric zero with
any zero padding, `null`, `unknown`, and the known unusable
`9774d56d682e549c` placeholder.

All of the following are required for a complete sample: manufacturer, brand,
model, product, device, board, hardware, SDK level, a non-empty ordered ABI
list, build fingerprint, and a valid Android ID. Hardware SKU is the only
optional field and is represented as `None` when absent. A partial sample can
never establish or validate a baseline. Two complete samples must have the
same normalized field presence and values; a required-field disagreement is
unverified, and hardware-SKU appearance or disappearance between complete
samples is also unverified. A complete baseline is established only after the
reviewed serial and every reviewed manufacturer, model, and API-level field
that is present in the plan matches the observed sample.

Each identity check uses exactly four bounded, fixed-serial, non-root probe
commands: two `shell getprop` samples and two `shell settings get secure
android_id` samples. They use the existing `ProcessOperation::Probe` process
deadline and output bounds. Identity probes call the unguarded bounded process
seam directly and are never recursively identity-checked.

## 4. Safe-boundary coverage

The current `ExecutorDevice` interface has thirteen executor-visible
operations implemented by `RealAdbDevice`: root revalidation, install, push,
mkdir, remove-file, remove-tree, device copy, package predicate, path-exists
predicate, path-is-directory predicate, plan command, launch, and force-stop.
The identity configuration hook and fake-filesystem selector are lifecycle
hooks, not operations. Every real operation receives one pre-check and one
post-check around the complete executor-visible operation. Compound launcher
resolution is included inside that single boundary. Root preflight is
identity-guarded without adding repeated root authorization checks. Dry-run
devices use the no-op configuration hook and issue zero identity probes.

## 5. Precedence and typed outcomes

Existing Phase 6D.2 timeout/process/output and Phase 6D.3 ADB transport
classification remains authoritative. Identity collection propagates timeout,
spawn/resolution, output/process, offline, unauthorized, disconnected, ADB
server, reset, and transport failures without relabeling them. No post-probe
runs after an untrustworthy completion. A completed ordinary command or root
failure may retain its original typed result after a successful post-probe; a
completed post-probe that detects a changed or unverified identity may
supersede that result.

Private identity kinds map to stable internal issue codes without adding a
serialized failure-kind field:

| Private kind | Stable issue code | Meaning |
|---|---|---|
| `DeviceIdentityChanged` | `device_identity_changed` | Complete evidence differs from the reviewed baseline or target binding. |
| `DeviceIdentityUnverified` | `device_identity_unverified` | Evidence is missing, partial, internally inconsistent, or cannot be safely read. |

Both kinds use the existing device fail-stop predicate. Pre-operation failures
use the ordinary sanitized identity-failure message and prevent the intended
operation. Post-operation failures use one exact private marker:

> The device identity could not be verified after the operation may have run.

Tauri consumes that marker before authored projection. It enables
`partialChangesPossible` from the marker only for one of the two identity issue
codes on the terminal real-execution mapping path. Arbitrary backend messages
never enable the flag.

## 6. Fail-stop and partial results

An identity failure fails the active real step, preserves earlier records and
outputs, preserves outputs already assembled by a compound permission action,
stops further permission actions, performs no later device or unrelated host
mutation, and leaves later report steps in the existing serialized `pending`
state. Terminal Tauri/React projection presents those steps as **Not attempted**
without inventing a new status. The terminal result remains failed. Possible
partial changes are reported when earlier real work completed or when the
exact post-operation marker proves that the current operation may have run.
The active slot is released and a subsequent run receives a new execution
identity; no in-place retry or resume exists.

## 7. Tauri authority invalidation

When the first terminal real snapshot is retained and its report contains
either identity issue code, Tauri performs one focused invalidation operation.
It removes the affected live device record and cached facts, clears its
qualification context, invalidates only reviews for that opaque device handle,
advances its session epoch and device generation, and invalidates matching
completed or in-flight root evidence. The root attempt generation is bumped so
a late qualification completion is rejected. The existing serial-to-opaque-
handle inventory reconciliation map and order remain authoritative; this slice
does not promise that a future inventory receives or retains any particular
opaque handle.

The terminal execution mapping and sanitized report remain retained for display
and export. Repeated snapshot reads do not repeat invalidation because the
active-to-terminal transition is consumed once. Export is a pure projection and
does not invalidate authority. Another real run requires a fresh inventory
connection, probe, qualification, generated plan, review, and execution
identity.

## 8. Sanitization and product boundaries

Snapshots, event batches, reports, support summaries, and React issue cards
receive only allowlisted statuses, authored identity guidance, existing opaque
handles, and existing safe device presentation. Android ID, build fingerprint,
property names and values, serials, transport IDs, fingerprint hashes,
commands, paths, stdout/stderr, and arbitrary backend messages are excluded.
Changed and unverified guidance is distinct and requires reconnection, a fresh
identity probe and qualification, a new plan, and a new review; the old run
cannot resume.

This slice does not implement Phase 6D.5 repeated root-authority revalidation
or root revocation, Phase 6D.6 physical disconnect/replacement qualification,
hardware attestation, persistent identity databases, cross-run tracking,
automatic reconnect, retry, recovery, rollback, replay, checkpointing, public
configuration, or production enablement. Overall Phase 6D remains **In
progress**.
