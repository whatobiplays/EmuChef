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
semantics.

Tauri resolves each named Setup Location from its own current T04 volume
evidence and retains the binding independently of T04's single presentation
selection. Planning includes only bindings used by selected Recipe writers.
The immutable plan digest covers each active trusted binding, including the
volume evidence, root, and relative path. The retained review binds its
canonical destination scope, tree snapshot, and any directory-creation effects.
Overlapping named locations are permitted when unused. Active Setup Location
and legacy shared-path writers are compared by canonical resolved scope and
reviewed effects. Conflicting file writes, destructive same-path/ancestor
effects, and overlapping directory creations that would invalidate another
writer's reviewed missing-path precondition fail before execution. An
idempotent directory effect may coexist when the directory already exists at
review time and no other effect conflicts.

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
reported under T03 copy semantics; failures after an authorized mutation retain
truthful partial outcomes.

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

Portable schema-v1 configurations store only an authored device-plan
reference, recipe selection, and user bindings. They never store a generated
execution plan or its digest. Reopening a configuration therefore always
re-probes the selected current device and runs current validation, description,
planning, and review before an execution can start.

Crash recovery uses the same portable-intent boundary without becoming a
planner or executor input source. Tauri restores selected authored references
and schema-permitted bindings into a fresh document session, after which the
ordinary current-catalog description and planning path runs. Authored input
`sensitive` metadata alone controls whether a binding value can enter the
recovery record; omitted bindings produce a re-entry diagnostic before a fresh
review can be created. No generated plan, digest, target, review, or execution
state is reconstructed.

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
