# Planner and Executor

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
Retry or repair creates a new attempt from a freshly validated and reviewed
plan. Execution has no undo, rollback, inverse-step, backup, or restoration
contract. The complete product protocol is documented in
[Phase 0 End-User Runtime Contracts](../product/phase-0-runtime-contracts.md).

Artifact resolution supports absolute `file://`, HTTP, and HTTPS URLs inside the
runtime/cache sandbox. The resolver owns compatible URL-based filenames,
destination selection, partial-file cleanup, and no-clobber publication. The
transport owns serial blocking HTTP requests and fixed-size response streaming.

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
