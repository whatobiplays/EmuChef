# Planner and Executor

The Rust package `crates/emuchef-rust-backend` is the sole planner and executor
implementation. There is no alternate backend, compatibility executable, or
runtime fallback.

The planner loads an authored device plan and profile, selects recipes, applies
input bindings, expands dependencies, normalizes steps, and emits a typed
execution plan. Optional live ADB probing supplies detected facts before
explicit CLI context overrides are applied to the emitted context.

Runtime input declarations, saved user configurations, binding precedence,
provenance, discovery, and side-effect-free plan creation are specified in
[Runtime Recipe Configuration](runtime-recipe-configuration.md). The planner is
the only layer that merges those sources. The execution plan contains normalized
effective inputs; the executor does not load saved configuration or apply
precedence.

The executor processes normalized steps on one thread. Failed or blocked
dependencies make downstream work `blocked`; blocked work does not resolve
parameters, execute, or verify. Skip conditions produce `skipped` results, and
verification can fail a completed action. Unrelated work may continue.

Product plans retain resolved catalog identity, reviewed target facts, ordered
recipe name/description snapshots, recipe ownership for every step, and
human-readable notes. Notes use `progress_note`, step name, humanized step type,
then step id. The plan SHA-256 digest is calculated from recursively key-sorted,
whitespace-free canonical JSON while preserving every array's normalized order.

## Setup Location destinations

A Recipe may declare a typed `setup_location_ref` input for `copy_files.dest`.
The supported writers copy a host directory to a device directory using
`merge` (Add or Update), `replace`, or directory-style `sync` (Mirror). A
Setup Location reference in any other operation or parameter fails validation
or planning closed. Existing Recipe parsing and validation, App Target
references, and legacy device-path destinations retain their established
semantics. Setup Location writers cannot use runtime step-output sources
because review cannot bind an immutable source snapshot before earlier steps
run. Planning and snapshot-coverage validation reject that combination before
execution.

Device `extract_archive` steps remain legacy writers. Their device-side effects
are not part of the reviewed CopyCommit projection. If an active copy overlaps
an extraction whose resulting destination state cannot be reviewed, planning
continues to fail closed and requires a separately prepared review. Host
extraction does not write to device storage.

Config Editor input definitions do not include `setup_location_ref`; that
Recipe type is accepted only for `copy_files.dest`.

Tauri resolves each named Setup Location from its own current T04 volume
evidence and retains the binding independently of T04's single presentation
selection. Planning includes only bindings used by selected Recipe writers.
Portable saved configurations may omit these session-bound values; planning a
selected Recipe with a required location still needs a current session binding.
A saved literal path is not converted into a location reference and must be
reselected if it no longer resolves.
The immutable plan digest covers each active trusted binding, including the
volume evidence, root, and relative path. Each reviewed copy snapshot also binds
its initial destination manifest, ordered predecessor step IDs, projected
before and after states, exact deletion set, and projection digests.

## Capability and operation eligibility

Capability constraints can make an operation ineligible for the current
device. A missing capability blocks a selected non-toggleable state-changing
step (`copy_files`, `extract_archive`, `force_stop_app`, `grant_permissions`,
`install_apk`, or `launch_app`) instead of silently removing it. Other
known capability-ineligible work may be pruned. An unclassified non-toggleable
step is treated as required until its capability behavior is defined. A
required step whose dependency is unavailable blocks planning. The planner
reports missing capabilities through the same typed plan path used for
execution.

An elevated permission operation is omitted on an unprivileged path when its
typed permission set has actions that are inapplicable without `root_shell`.
The planner omits the missing `root_shell` requirement for that operation and
still checks its other required capabilities. `policy.on_failure` controls how
applicable permission-action failures are reported; it does not determine
operation selection.

## Qualification-only capability restriction

The qualification restriction is a trusted policy set at launch. The clean
qualification launcher parses and forwards `--restrict-privileged-execution`
when requested. It uses standard qualification by default.
The native app must accept that request only when its debug,
`real-execution`, runtime opt-in and embedded clean-build gates pass. The
sidecar must independently require debug, `real-execution` and matching
embedded qualification source identity, and verify the effective policy during
the private handshake. The setting is fixed for the process lifetime. Changing
it requires a relaunch and fresh current-session profile resolution, planning
and review.

Rust remains the authority for the plan. Restriction monotonically removes
`root_shell` and `app_data_write` from effective capabilities and prevents
direct-root or OEM-assisted strategies. Device observations retain actual
root and OEM eligibility facts. Ordinary unprivileged ADB remains the only
execution strategy. Required operation and dependency checks continue to
apply, including the typed omission rule for optional elevated permissions.

Restricted review must use a bounded, one-use sidecar authority for the retained
Rust planning result. It binds the plan digest, mode, exact target, sidecar
generation, restriction and ordinary ADB strategy. Tauri retains it with the
current profile selection, catalog, device epoch and Platform-Tools identity.
Start must consume the authority and pass the same policy to the worker. The
worker and ADB adapter must reject privileged routes and require the ordinary
shell identity where device operations need it. Missing, stale, mismatched or
lost authority fails closed. Existing start-reservation, uncertain-outcome,
mandatory-operation, copy-review, configuration and cancellation protections
continue to govern the restricted run.

Restricted CopyCommit streams ordinary staged files and the helper through
ADB shell. The adapter gates the route on the ADB daemon and checks the shell
UID and security context in the same command that writes the stream. It also
checks the digest derived from the Android ID and build fingerprint before
opening the staging destination. Raw sync or directory transfers can fail
when the adapter cannot establish a guarded ordinary route.

Qualification evidence must record the accepted review and terminal report with
source/build identity, target, profile and plan, observed root, effective
capabilities, strategy, reviewed digest, required operation identities and
outcomes. The bundle validator must accept a restricted-unprivileged claim only
when these facts agree. Historical bundles remain readable and do not acquire
that classification without the attestation. Physical acceptance is `NOT RUN`.

Distinct Recipes may write to overlapping App Targets, shared paths, and Setup
Locations. Overlap alone does not make the plan invalid or trigger a semantic
compatibility check. Overlapping writers from the same Recipe remain blocked;
ordered predecessor links are valid only between distinct Recipes. The existing
serial plan order determines which copy is reviewed first. Each later CopyCommit snapshot is prepared against the projected
result of earlier overlapping reviewed copies. Its own `merge`, `replace`, or
`sync` policy determines its writes and exact deletion set, including whether
it replaces or removes files written earlier in the run. The planner adds no
cross-Recipe dependency edge or concurrency.

Before a later overlapping copy mutates its destination, the executor requires
every reviewed predecessor to have a complete successful report matching its
projected result. It replays those reports and requires the current destination
manifest to equal that state, then rechecks the operation's normal source,
destination, device, volume, confinement, and safe-write preconditions. A
skipped, failed, partial, uncertain, or mismatched predecessor leaves that
later overlapping snapshot unusable until a fresh review. Unexpected drift
also fails the affected operation. Disjoint destinations remain independently
eligible when another Recipe fails.

Setup Location bindings for unused named locations do not enter the plan and
may overlap. Shared directory creation still fails when it would invalidate a
later writer's reviewed missing-path precondition; an already-existing
directory effect may be shared when the remaining preconditions hold. Implicit
parent-directory creation by a legacy `file_path` copy targeting a directory
remains part of its reviewed effects. Unreviewable device-side extraction
effects do not join the ordered projection and keep their existing fail-closed
review boundary.

T04's ten-second observation lifetime governs whether volume evidence is fresh
enough to resolve or refresh a Setup Location. It does not expire an immutable
review or change its digest. Before the first selected-destination mutation,
execution requires fresh matching volume identity and continuity evidence,
current write access, canonical confinement, and the reviewed destination
preconditions. A missing destination suffix is allowed only beneath an
existing verified directory; each absent component is an explicit reviewed
creation effect and is created one at a time through the no-follow directory
helper. If verification or that safe helper is unavailable, execution fails
closed before destination mutation. Staging and its cleanup remain separately
reported under T03 copy semantics. A failed mutation is `noChange` only when
fresh post-inspection confirms the reviewed destination tree is unchanged and
no destination directories were created. `partial` requires observed
destination effects; `uncertain` reports an outcome that cannot be verified.

For a real Setup Location writer, Tauri issues a lease bound to the current
device handle, session epoch, runtime generation, reviewed plan identity and
digest, and execution attempt. It travels only in the existing trusted Rust
`startExecution` request. The backend validates and registers that lease before
spawning the worker. Its local mutation gate enforces the Tauri-issued lease and
does not derive or refresh device-session authority. Each destination mutation
revalidates the trusted binding and acquires a permit covering that operation.
Tauri assigns attempts strictly increasing sequences that survive execution
store resets. The sidecar keeps one runtime-local high-water mark and rejects
replayed or older sequences without retaining every completed attempt.
Device inventory transitions revoke matching leases before advancing a session
epoch; revocation closes the gate and waits for an active permit to finish. The
worker does not request permission from Tauri, so start admission does not wait
on a worker-to-Tauri permission round trip. Runtime loss permits execution-authority reset only after sidecar cleanup is confirmed and no T05 lease remains and no real execution is active or unresolved. A rejected start retires its matching attempt and leaves the session epoch current for a later attempt sequence. Device and runtime transitions revoke matching leases before publishing invalidated authority. Tauri releases local authority after the corresponding retirement or revocation is acknowledged. An unknown execution mapping or uncertain terminal copy-commit outcome keeps the lease closed until matching trusted terminal evidence resolves it. If a real-start response is lost or unusable, the durable operation fence blocks restart and new real writes across store reset and app relaunch. T05 accepts this fail-closed state; automatic recovery and an Android-side operation-identity protocol are outside the requirement. A new device, session, runtime generation, or review does not resolve the old operation. Runtime-process cleanup alone cannot clear the lease or fence or reset execution authority while mutation status is unresolved.
Revocation prevents later
writes but does not roll back completed destination mutations; execution reports
preserve staging, cleanup, and partial-result facts.

The Setup Location list distinguishes loading, unavailable, and successfully
empty results. Tauri propagates list failures instead of projecting them as an
empty list. A save rejected for stale storage evidence immediately invalidates
the current readiness and queues a same-context refresh. Existing device, plan,
runtime, and request-generation fences prevent a late result from an earlier
context from restoring stale readiness.

Public review shows the Setup Location name, volume category, relative path, and
reviewed effects. Legacy shared-path destinations are presented with a
sanitized shared-storage label and relative suffix when that suffix is known;
otherwise review uses a generic shared-storage label. Physical mount roots,
device identity, volume UUIDs, and continuity evidence remain in trusted
Rust/Tauri state and the canonical plan binding; they are excluded from every
public review projection.

The sidecar execution manager is outside the pure planner/executor boundary. It
owns attempt ids, one-active-attempt coordination, RFC 3339 UTC timestamps,
full-plan report snapshots, recipe-grouped status, ordered sequence-numbered
events, target preflight, and cooperative cancellation. Cancellation is checked
between atomic steps, schedules no later work, preserves completed results, and
never rolls back. A caught worker panic produces `execution_worker_panicked`, a
terminal inspectable report, and releases the active slot.

After plan-digest validation and real-target preflight, `startExecution`
admits every retained artifact before committing an attempt id, report, event,
active record, cancellation token, or worker. Admission remains under the
execution-state lock so the prospective attempt number and one-active-attempt
invariant are atomic; it neither creates persistent reservation state nor
reacquires execution state. Rejection leaves the attempt number and active slot
unused.

Dry-run and real execution use the same normalized plan and report shape.
Dry-run reports are explicitly simulated and are not real-device verification.
The end-user Tauri app exposes guarded real execution only when compiled with
the default-disabled `real-execution` feature. Tauri retains the reviewed plan,
digest, target, Platform-Tools identity, and existing trusted `resolvedInputs`;
it does not add a sidecar metadata operation or derive Android version from API
level. File/directory prechecks apply only to resolved inputs whose existing
authoritative type is explicitly `file` or `directory`.
Retry or repair creates a new attempt from a freshly validated and reviewed
plan. Execution has no undo, rollback, inverse-step, backup, or restoration
contract. The complete product protocol is documented in
[Phase 0 End-User Runtime Contracts](../product/phase-0-runtime-contracts.md).

Portable schema-v1 configurations store the authored device-plan reference,
recipe selection, and reusable user bindings. They exclude `setup_location_ref`
values because each named location belongs to one device session and plan.
Saved-configuration validation permits a required location value to be absent;
runtime planning blocks until the selected workflow has a current binding.
Previously saved literal paths remain unchanged and are not migrated into
location IDs.
Configurations never store a generated execution plan or
its digest; reopening one always re-probes the selected device and runs current
validation, description, planning, and review before execution starts.

Crash recovery uses the same portable-intent boundary and never becomes a
planner or executor input source. Tauri restores authored references and
permitted bindings into a fresh document session, then runs the current
description and planning path. Sensitive and unclassified values remain
omitted; `setup_location_ref` values are also omitted and must be selected again
when a required location input is validated. Recovery re-entry diagnostics are
limited to sensitive or unclassified values recorded as required re-entry.
Omitting a Setup Location reference uses ordinary input validation and does not
create that recovery diagnostic. No generated plan, digest, target, review, or execution state is
reconstructed.

Artifact resolution supports absolute `file://`, HTTP, and HTTPS URLs inside the
runtime/cache sandbox. The resolver owns compatible URL-based filenames,
destination selection, partial-file cleanup, and no-clobber publication. The
transport owns serial blocking HTTP requests and fixed-size response streaming.

For default-cache publication, the resolver also prepares an optional schema-v1
metadata sidecar containing only a safe artifact label, source kind, source
fingerprint, payload filename, and expected size. The payload remains execution
authority. The sidecar also binds those fields to the payload modification
fingerprint captured before promotion. Metadata failure leaves a usable
unindexed payload, and stale metadata cannot validate a replaced payload; it
never changes execution success into failure. The end-user Tauri app alone
interprets this payload-plus-sidecar pair as one manageable cache entry under
its explicitly injected application-data root.

The resolver also owns side-effect-free artifact admission. It validates
supported artifact and cache definitions, the selected destination and sandbox
policy, readable regular `file://` sources, and structurally valid HTTP(S)
sources. It performs no DNS, network, directory creation, partial-file,
publication, cache, staging, ADB, or device work. A complete authoritative
default-cache regular file still bypasses original-source URL parsing and
contact after its definition, destination, file kind, and sandbox policy pass.

HTTPS uses strict Rustls certificate and hostname validation. The client has a
15-second connect timeout, one five-minute deadline across headers, redirects,
and body, a five-redirect limit, and no retries. HTTPS-to-HTTP redirects fail
before the downgraded request. Transparent decompression, resume, freshness
checking, and authored checksums are absent. Standard system proxy discovery is
enabled without an EmuChef-specific configuration surface.

Complete default-cache regular files are authoritative and bypass URL parsing
and client construction. New files use unique partials in the destination
directory, `sync_all`, and `persist_noclobber`; the exact underlying
no-overwrite primitive is platform-dependent. `cache: none` always transfers
and uses a unique runtime filename when the compatible base path already exists.
## Phase 2C completion actions

The end-user Tauri application derives completion summaries and report exports
from retained authoritative execution snapshots. Fresh repair returns through
configuration description, planning, and review; it never replays an execution
or treats the old review as current authority.

Final app launch uses split responsibility without split authority. Tauri owns
and atomically consumes each opaque launch-action handle. The sidecar's internal
`launchExecutionApp` request remains stateless with respect to one-shot action
consumption: it rederives the single eligible candidate from the retained plan
and report and invokes the existing typed `launch_app` adapter. This permits
Tauri to issue a new handle after a failed attempt while preventing duplicate
use of any individual handle.

## Future accepted product direction, not implemented
The normative future contract is [EmuChef requirements](../product/emuchef-requirements.md): a reviewed OEM bundle freezes content, destination, backup mode and strategy across separate invocation attempts; an actual invocation start is different from staging/acknowledgement, and unknown start evidence blocks changed-preference authority. The required implementation must stop before an unapproved overwrite on enabled-backup failure, reconcile all started attempts under a serial OEM barrier, retain Recovery Originals independently of ZIPs/staging, and treat an individual verified-owned cleanup failure as a warning with pending retry. This paragraph does not describe existing executor behavior.


The [EmuChef product realignment proposal](../product/emuchef-product-realignment-proposal.md) records accepted owner requirements for app-generated configuration, per-file backups/restoration, and staged review. T05 Setup Locations provide logical destinations only for the supported T03 `copy_files` writers described above; they do not add backup, restoration, or transactional setup rollback. Until the corresponding broader requirements are implemented and qualified, the current no-backup/no-restoration statements remain factual. Any file-level restoration must be separately bounded and must not imply a transactional setup rollback or reactivation of old execution authority.
