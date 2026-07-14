# EmuChef Context

This document is the standalone current-state contract for the repository.

## Product

EmuChef provisions Android emulation handhelds from authored YAML. A device
plan selects recipes, the planner emits an execution plan, and the executor
applies that plan through local filesystem operations and ADB.

Rust is the sole product runtime. The Cargo package
`crates/emuchef-rust-backend` builds the `emuchef` binary and owns:

- `plan`, `validate`, and `apply` CLI commands;
- authored catalog loading and validation;
- device profile matching and live `adb shell getprop` probing;
- execution-plan generation and single-threaded execution;
- dry-run and real-ADB adapters;
- the one-shot and JSONL editor protocols;
- the additive Phase 0 end-user catalog and execution-session protocol;
- read-only end-user device inventory, probing, deterministic matching, and
  product configuration operations;
- document sessions, command application, undo/redo, save, YAML emission, and
  reference indexing.

The Python packages under `src/` are frozen reference code pending deletion.
They expose no console script or `python -m emuchef` entrypoint. Product builds,
runtime checks, packaging, and the editor do not execute Python.

## Authored Data and Planning

Authored source lives under `authored/`:

- `apps/` defines app selections;
- `recipes/` defines inputs, artifacts, groups, and ordered steps;
- `device_profiles/` defines match criteria and capability defaults;
- `device_plans/` selects profiles and recipes.

Product catalog operations consume a resolved local snapshot rather than a
repository-specific global root. A snapshot carries `bundled` or
`local_directory` source kind, source id, optional version and cache key, and an
independent optional SHA-256 content digest. `cached_remote` is reserved but
has no resolver, networking, synchronization, signature, or update behavior.
The legacy `authoredRoot` request field remains a compatibility adapter for
existing configuration operations; new product catalog operations require a
resolved `catalog` object.

`emuchef plan` accepts an authored root; either a device-plan id or a saved
user-configuration ID/path; an optional configuration-root override; optional
recipe and input-binding overrides; optional explicit device context; optional
ADB path/serial; an output path; and verbose output. Supplying `--adb` or
`--serial` enables live device probing. Generated execution plans use
`plan.<device-plan>.001` identifiers. No `--recipe` and no `--clear-recipes`
leaves request selection absent, one or more `--recipe` values replace saved or
device-plan selection, and `--clear-recipes` supplies an explicit empty
replacement. The clear flag conflicts with any recipe occurrence.

Each `--bind <recipe-id>/<input-id>=<JSON_OR_STRING>` key may occur once.
Values parse as JSON when valid and otherwise remain strings, so list and
`multiple: true` inputs use JSON arrays. Duplicate explicit keys are CLI errors
and are never converted into arrays.

Raw one-shot and sidecar requests for `describeConfiguration` and
`planConfiguration` reject duplicate qualified keys in a `bindings` object
before JSON object entries are collapsed into a map. The failure keeps the
existing `invalid_request` code and reports `duplicate_binding_key`, the
`bindings` field, and the qualified key in structured details without either
supplied value. CLI duplicate handling remains a separate argument-parsing
contract.

Step parameter specifications declare their accepted value sources and runtime
value types. Authored recipe refs remain recipe-local (`inputs.<id>`,
`artifacts.<id>.<field>`, and `steps.<id>[.outputs.<name>]`). Validation rejects
literal or ref namespaces that a parameter does not accept and rejects refs
whose declared runtime type is incompatible. `copy_files.dest` and
`copy_files.copy_policy` accept either literals or compatible input refs, while
`copy_files.source` remains limited to compatible input, artifact, and step
output refs. Canonical recipe YAML continues emitting direct literal values and
single-field `{ ref: ... }` mappings.

Recipe input declarations support `string`, `integer`, `boolean`, `enum`,
`file`, `directory`, `path`, `device_path`, `string_list`, `path_list`, and
`object`. Each declaration carries semantic label, description, and role
metadata; required and multiple flags; a JSON-compatible default; structured
enum options; sensitive and advanced presentation flags; extensible metadata;
and validation for existence, extensions, path kind, and allowed device-path
prefixes. Enum option values are unique, defaults match the declared type and
enum options, and canonical YAML preserves declaration and option order.

Planner runtime types map host files and directories to `file_path` and
`directory_path`, device paths to `device_path`, enums to strings, and declared
list types to typed arrays. Device paths retain device location and are not
expanded or rewritten as host paths. The authored `feature.copy_roms` recipe
demonstrates a host directory, constrained device destination, and enum policy
consumed through recipe-local input refs.

Schema-v1 user configurations persist recipe selection and direct runtime input
bindings independently from authored recipes. A document requires a non-empty
`device_plan`, keeps fully qualified `<recipe-id>/<input-id>` binding keys, and
preserves unknown top-level extension fields deterministically. Structural
loading is catalog-independent: malformed YAML, unsupported schema identity,
invalid required field shapes, malformed binding keys, duplicate mappings, and
ambiguous binding value sources prevent loading. Recipe and input existence,
dependency-expanded selection membership, value compatibility, enum and path
constraints, device-plan existence, and missing required inputs are semantic
diagnostics, so a structurally valid document remains editable and canonically
emittable when its catalog is unavailable or its saved values are invalid.

User-configuration identifiers resolve as `<configuration-root>/<id>.yaml`.
The configuration root is the directory containing configuration files and may
be set explicitly. Otherwise EmuChef uses the platform configuration directory:
`~/Library/Application Support/EmuChef/user-configurations` on macOS,
`%APPDATA%\EmuChef\user-configurations` on Windows, and
`$XDG_CONFIG_HOME/emuchef/user-configurations` or
`~/.config/emuchef/user-configurations` on Linux. Absolute values, values with a
slash or backslash, and case-insensitive `.yaml` or `.yml` suffixes are always
paths, including when missing. Other values must be valid configuration IDs;
resolution never searches the authored root or current working directory.

Planning keeps explicit request bindings, persisted user-configuration
bindings, and device-plan input overrides as separate maps until the planner
resolves them. For every input in the dependency-expanded selected recipe set,
the winning value is chosen in this order: explicit request, saved user
configuration, device plan, recipe default, then unbound. Only the winning
value is validated for a planning request; a valid higher-precedence value may
therefore shadow an invalid saved value. An invalid winner produces a
diagnostic and never falls back. Resolved records retain `explicit`,
`user_configuration`, `device_plan`, or `recipe_default` provenance. Unknown
keys and bindings outside the effective recipe set are errors, optional inputs
may remain explicitly unbound, and required inputs without a winner are
reported together. The execution plan receives only normalized effective input
values; the executor does not merge layers or interpret provenance.

The side-effect-free `describeConfiguration` JSON operation accepts an authored
root plus a request or saved device plan, optional saved configuration,
syntax-resolved configuration root, optional selected-recipe replacement,
explicit bindings, and device context. Request-level `devicePlan` and
`selectedRecipes` fields replace saved values when present, including an
explicit empty recipe list. Discovery expands dependencies, applies the central
binding resolver, and returns every effective input with declaration metadata,
partial values, provenance, per-input diagnostics, and aggregate diagnostics.
It does not require a complete valid plan and performs no ADB, network,
extraction, copy, device-write, or persistence operations.

For both discovery and planning, the camelCase protocol field
`userConfiguration` accepts either an ID/path string or an inline schema-v1
document object. Inline documents use the persisted schema's snake_case fields,
including `schema_version`, `device_plan`, and `selected_recipes`; camelCase
aliases inside the document are not accepted. Inline objects use the same
structural parser and model as file-backed documents and perform no
configuration-file lookup.

The `planConfiguration` JSON operation accepts the same context and performs
complete catalog, dependency, winning-binding, ref, constraint, and required
value validation before returning an in-memory normalized execution plan,
resolved inputs, and diagnostics. It performs no ADB commands, downloads,
extraction, host or device copies, writes, or plan-file persistence. A
request-level `devicePlan` replaces the saved required `device_plan`; the saved
document itself remains unchanged and self-contained. The desktop editor can
request and inspect this structured plan result without shelling out through
the CLI.

Product planning returns canonical JSON SHA-256 `planDigest` data and captures
resolved catalog identity, a reviewed target binding, ordered recipe
name/description snapshots, and normalized step notes. Canonical plan JSON
recursively sorts object keys, emits no insignificant whitespace, and preserves
array order. Target serial compares exactly after trimming; manufacturer and
model compare case-insensitively after whitespace normalization; Android API
level compares exactly. Missing or different actual values are hard failures
when the reviewed plan contains the fact.

Authored steps may include optional `progress_note` presentation text. Runtime
fallback order is progress note, step name, humanized step type, then step id.
This metadata does not change planner selection, dependencies, parameters,
execution, or verification. Existing recipes remain valid without it.

## Execution

Execution is single-threaded and dependency-aware. A failed step blocks
dependent steps while unrelated steps may continue. Skip conditions produce
skipped steps according to the execution-plan contract. Verification runs after
the step action and can fail the step.

The JSONL sidecar's additive `phase0_end_user_runtime` extension is explicitly
negotiated. It supports `describeCatalog`, `startExecution`, `getExecution`,
`getExecutionEvents`, `cancelExecution`, `listAdbDevices`, `probeDevice`, and
`matchDevice` while retaining the existing configuration operations. One
execution may be active in a sidecar process;
terminal attempts remain inspectable in memory until process exit.

## End-User Support and Artifact Cache

The end-user Tauri application owns a dedicated artifact cache at
`<app-data>/artifact-cache`. Tauri derives and injects that root when it starts
the sidecar. React, environment variables, and current-working-directory policy
cannot redirect it. The backend default remains `.emuchef_cache/artifacts` when
no explicit cache root is supplied, so CLI, config-editor, tests, and other
embedders retain their existing behavior. Legacy caches are neither migrated
nor deleted.

Each managed cache entry consists of one payload and an optional schema-v1
metadata sidecar. The sidecar contains a safe artifact label, source-kind enum,
SHA-256 source fingerprint, expected payload size, and internal payload
filename plus the payload modification fingerprint captured before promotion;
it contains no raw URL or path and provides no execution authority.
Metadata publication failure leaves a usable unindexed payload and does not
fail execution. Inventory count and size treat payload and metadata as one
logical entry. Orphan metadata and unrecognized files remain unmanaged and
non-removable.

React receives generation-scoped opaque cache-entry handles plus safe category,
label, source kind, structural integrity state, combined size, age bucket,
in-use state, and removability. Tauri owns selective, unused, and all-removable
cleanup. It requires exact count/size confirmation, blocks cleanup while an
execution is starting or active, and revalidates root confinement, symlink
status, logical association, and both component fingerprints immediately before
deletion. Cleanup returns stable sanitized outcomes and a fresh inventory.

The Support & Storage panel exports a deterministic schema-v1 ZIP through a
native save dialog. Tauri owns the destination and bundle bytes. The 2 MiB
bundle includes only app/runtime status, OS class, feature gates, public catalog
identity, aggregate saved-configuration and retained-execution state, and
aggregate cache counts/sizes. It excludes names, configuration contents,
bindings, handles, paths, serials, environment values, URLs, credentials, logs,
process or ADB output, reviewed plans, files, and crash data.

`startExecution` requires a complete plan and its canonical digest, recomputes
the digest before apply, and rejects `runtimeRoot` and `cacheRoot`. Those roots
are configured at sidecar startup and each attempt derives its own runtime
directory. Real execution requires reviewed target facts and performs ADB
preflight. Dry-run performs fake-device execution only and reports
`simulated: true` with `simulated_only` verification scope; it is not
real-device verification.

After digest validation and real-target preflight, `startExecution` performs
canonical admission for every retained artifact before committing an execution
number, report, event, active record, cancellation state, or worker. Admission
runs under the execution-state lock without creating persistent reservation
state or reacquiring execution state. It shares the resolver's artifact type,
cache mode, destination, URL, local-source, and sandbox policy; performs no
network or filesystem/device mutation; and leaves the execution number and
single-active slot unused on failure. Safe failures use
`execution_start_failed`, `artifact_not_ready`, and a stable artifact cause code
without artifact identifiers, URLs, credentials, paths, roots, or raw sandbox
details.

Execution reports contain the full reviewed plan, digest, target, RFC 3339 UTC
timestamps, overall status, structured issues, ordered recipe groups with
captured names/descriptions, and ordered step results with recipe ownership,
notes, messages, and outputs. Overall status is `running`, `succeeded`,
`succeeded_with_warnings`, `failed`, or `cancelled`. Blocked required work makes
the overall attempt failed; `blocked` is primarily a step or recipe-group
state. Incremental events use per-execution sequence numbers and
`getExecutionEvents(afterSequence)` while `getExecution` remains the complete
snapshot recovery surface.

Cancellation is cooperative between atomic operations. It stops new step
scheduling, may allow the current operation to finish, preserves completed
results, marks unscheduled work cancelled, and performs no rollback. A worker
panic yields `execution_worker_panicked`, leaves a terminal inspectable report,
and releases the active slot. Retry or repair means generating and reviewing a
fresh plan and starting a new execution id. There is no runtime rollback,
device-state undo, reverse-step generation, automatic backup, or restoration
promise.

Supported steps include artifact resolution/extraction, file copy, APK install,
launch, force-stop, permission/app-op grants, and waits. Device-target archive
extraction happens in the host staging area before files are pushed through
ADB; Android `unzip` is not required.

Artifact resolution supports absolute `file://`, HTTP, and HTTPS URLs. The
single-threaded resolver uses strict Rustls certificate and hostname validation,
a 15-second connect timeout, one five-minute transfer deadline, at most five
redirects, and no automatic retries. HTTPS-to-HTTP redirects are rejected.

Cache keys continue to hash the original URL bytes, including query and
fragment. Complete default-cache files are authoritative and bypass URL parsing
and network setup after their artifact type, cache mode, destination, regular
file kind, and sandbox policy pass. Without a valid cache hit, malformed and
unsupported source URLs fail admission. New bytes are streamed to unique
same-directory partial files, flushed, synced, and published without
clobbering. `cache: none` always transfers and uses a unique runtime path on
collision. Failures are typed and redacted, remove partial files when cleanup
succeeds, and block dependent steps without preventing unrelated work.

## Editor

`apps/config-editor` is a React/Tauri application. Tauri builds and packages the
Rust `emuchef` binary as an external sidecar and launches it with `--sidecar`.
The JSONL sidecar owns persistent document sessions and in-memory Phase 0
execution reports/events. Sidecar startup may set `--runtime-root`,
`--cache-root`, and `--adb`; defaults are working-directory-local runtime/cache
directories and `adb`.

Release builds use a local-only production CSP with no Vite development URL.
Frontend sources are limited to the packaged application, while Tauri IPC uses
only `ipc:` and `http://ipc.localhost`. Development-only Vite and HMR settings
live in `tauri.dev.conf.json` and are selected by the maintained Tauri command
wrapper only for `tauri dev`.

Product-facing Tauri commands use non-prefixed document names:
`list_step_specs`, `open_recipe`, `get_document`, `apply_recipe_command`,
`undo`, `redo`, `save_recipe`, `save_recipe_as`, `validate`, `emit_yaml`,
`get_ref_index`, and `set_document_authored_root`. `sidecar_status`,
`sidecar_ping`, and `sidecar_restart` retain the prefix because they manage the
sidecar process. Transport request identifiers are removed before responses
reach frontend code.

The editor and JSONL protocol also expose catalog-independent user-configuration
open, create, inspect, edit, validate, canonical-emission, save, authored-root,
and close operations. Supplying an authored root adds semantic diagnostics but
is not required to open or emit a structurally valid document. The desktop
editor provides an initial user-configuration surface for selected recipes,
direct binding edits, diagnostics, canonical YAML, and saving; recipe authoring
continues to use its existing document surface.

When a user-configuration document has an authored root, the editor uses
`describeConfiguration` to group inputs by recipe and choose controls from
semantic types. Enums use declared options, booleans use checkboxes, host files
and directories use native pickers, device paths use text entry, and structured
or multiple values use JSON entry. The screen shows required, optional,
advanced, sensitive, and effective-source state without hard-coded recipe or
input identifiers.

## End-User App

`apps/emuchef-app` is the separate React/Tauri application for guided
end-user setup. It packages the Rust `emuchef --sidecar` runtime and a bundled
catalog snapshot but shares no frontend modules or runtime state with
`apps/config-editor`. The workflow is Connect Device, Confirm Device, Choose
Setup, Provide Inputs, Review Plan, and Simulated Run. The execution stage is a
fake-device dry run of the exact retained reviewed plan. It performs no real
apply, device writes, artifact resolution, catalog networking, wireless
onboarding, rollback, resume, or parallel-device work, and its result is not
real-device evidence.

The React frontend is keyboard and screen-reader operable across the workflow,
saved configurations, execution results, and Support & Storage. It provides a
skip link, stable landmarks and headings, semantic collections, associated
input diagnostics, focusable error summaries, bounded polite/assertive live
regions, and native determinate execution progress. Status and availability
always have text in addition to color. Reduced-motion, forced-colors, 200% zoom,
and narrow desktop windows are supported without changing the workflow
architecture.

Custom prompts and confirmations use exactly-once controllers. Each pending
request has one safe cancellation result, teardown never implies a destructive
or execution-confirming choice, and an overlapping request cannot replace the
live resolver. Runtime restart, configuration replacement, app reset, unmount,
and the top-level frontend error boundary safely cancel pending prompts.
Support & Storage owns and cancels its nested cleanup confirmation before the
parent closes.

Modal and native-dialog focus restoration first validates the recorded invoker,
then uses a transition-specific workflow or error destination, the current
workflow fallback, main content, and finally the header Support & Storage
action. Disconnected, hidden, disabled, or inert invokers are skipped. Focus
never silently falls to the document body, and generation checks prevent stale
restoration from stealing focus from a newer modal or workflow transition.

The app creates, opens, edits, saves, saves under a new identity, and reuses
named schema-v1 portable configurations. A saved document contains its
generated configuration identity and name, one authored device-plan reference,
selected recipe IDs, user bindings, and safe schema extensions. It never
contains a generated execution plan, plan digest, review or execution handle,
real-execution confirmation, launch action, target serial, probed device facts,
catalog root, ADB path, or runtime session state.

Tauri owns native configuration dialogs, absolute configuration paths, sidecar
document identifiers, opaque configuration handles, and a private ten-entry
recent-file index. React receives safe names, portable intent, dirty state, and
sanitized diagnostics. Missing recent files can be removed or relinked only to
a file with the same embedded configuration identity. Save As creates a new
name and generated identity while leaving the original file unchanged.

Phase 2B guarded real-device execution is implemented behind the
default-disabled Cargo feature `real-execution`. The compile-time feature is the
only product gate, and a policy-only Tauri query reports its boolean state.
Ordinary builds remain simulation-only. Trusted Tauri code owns confirmation,
mode selection, exact target identity, reviewed plans and digests,
Platform-Tools revalidation, unambiguous retained BYO file/directory checks,
and sidecar identifiers. Platform-specific packaged-device evidence, privacy
and security approval, an operator runbook, and a separate release decision are
required before a release build opts into the feature.

Tauri launches the sidecar and negotiates the `phase0_end_user_runtime`
extension plus `describeCatalog`, `listAdbDevices`, `probeDevice`,
`matchDevice`, `describeConfiguration`, `planConfiguration`, `startExecution`,
`getExecution`, `getExecutionEvents`, `cancelExecution`, and the schema-v1
user-configuration document operations before ADB is needed. Runtime and
catalog startup are independent from Platform-Tools setup.
A missing ADB installation blocks only device discovery and displays the
Platform-Tools setup flow.

Opening or reusing a portable configuration invalidates every prior generated
plan, digest, review, execution, confirmation, launch action, target binding,
and probed device fact. The user must select and freshly probe a current device,
validate against the current catalog and device capabilities, generate a fresh
description and plan, and complete a fresh review before execution. Stale plan
references, recipes, and bindings remain visible for correction and are never
silently substituted.

Save/Discard/Cancel protection applies before actions that would lose, replace,
close, reload, or invalidate dirty portable edits. Platform-Tools removal keeps
the active document and dirty edits intact while invalidating device, review,
execution, confirmation, and launch authority, so removal itself does not
require the dirty prompt.

Packaged catalog data is materialized under the application resource directory.
The trusted backend requires the four product directories, ignores only regular
`.gitkeep` placeholders, rejects symlinks and every other unsupported entry,
computes the canonical catalog SHA-256, and passes a
resolved snapshot to the sidecar. React receives source identity, version, and
digest without a catalog filesystem path. The catalog remains a bundled MVP
source; no network or update implementation exists.

Exact ADB serials and executable paths exist only inside trusted Rust/Tauri
communication. Tauri assigns stable opaque device handles while a serial
remains present during one application session and invalidates them on device
disappearance. React receives the opaque handle, masked serial, display facts,
connection state, and actionable errors. Exact serials never enter React
payloads, state, logs, storage, or markup.

The backend owns exact/high/low/none device matching. A unique exact/high match
may recommend a plan. Low/no matches are never auto-selected. Backend-approved
safe generic plans may be offered for explicit user choice, and the workflow
blocks only when the backend reports no safe candidate.

`describeConfiguration` is authoritative for recommended/default, optional,
dependency-required, and unavailable recipes; effective dependency expansion;
input declarations, values, provenance, and diagnostics; and review readiness.
React does not reconstruct planner rules. Native Tauri dialogs supply host file
and directory recipe inputs without blocking the IPC executor. Single-file,
multi-file, and directory cancellation leaves the input unchanged. Input-level
diagnostics render only with their field; aggregate diagnostics remain at page
level only when no input diagnostic represents the same binding key and code,
or the same code and message when no binding key is available. React and Tauri
send camelCase product fields;
probe facts remain snake_case inside the trusted inventory DTO and are converted
to camelCase `deviceContext` and `targetDevice` objects for the sidecar.
`selectedRecipes: null` selects device-plan defaults, while an explicit empty
array selects no recipes. Missing required input values remain successful
description results with `binding_missing` diagnostics instead of transport
failures.

The trusted configuration-description response retains the exact target device
binding and verified catalog identity/digest. The React projection omits the
target binding, catalog root, raw sidecar payload, and all exact serials.
Sidecar failures map to stable sanitized configuration error codes and useful
recovery messages. Debug builds log the complete internal error to the Rust
terminal only after redacting exact serials and absolute paths; release builds
do not expose raw internal errors.

Tauri retains the complete immutable reviewed-plan result, exact target binding,
catalog identity and digest, and canonical plan digest behind an opaque review
handle. React receives only a serial-free human review. At most 16 live reviews
are kept in memory, with a 30-minute idle lifetime, two-hour absolute lifetime,
and 64 bounded tombstones. Device disappearance, changed facts, catalog change,
Platform-Tools replacement/removal, discard, or capacity eviction returns
`review_stale`; time expiry returns `review_expired`; an unrecognized handle
returns `review_unknown`.

Simulation start accepts only the opaque review handle at the React boundary.
Tauri revalidates review lifetime, current catalog identity/digest, connected
device presence, the retained serial/manufacturer/model/API target facts, and a
new canonical plan digest without repeating profile matching or planning. It
then sends the exact retained plan, digest, and target to `startExecution` with
trusted code forcing `dry_run`. A device disconnect blocks start but cannot
cancel or erase a dry run after it has started.

Random execution handles are session-scoped, never reused, and lost on restart.
The shared simulated/real store retains one kind-aware start reservation or
active mapping and at most the latest terminal mapping. Every failed preflight
or start releases the reservation; a public handle is bound only after the
sidecar returns a successful execution identifier. Wrong-kind lookups are
indistinguishable from unknown handles. A lost real sidecar session removes
only the matching active or latest-terminal mapping, invalidates its originating
review, and reports an unknown outcome without inferring terminal status.
Reviews otherwise retain their independent stale, expiry, discard, and capacity
lifecycle.

`getExecution` snapshots are authoritative for recipe-grouped progress and
terminal state. Incremental events are presentation data only; after accepting
a snapshot the UI resumes event polling after its `latestSequence`. Polling
stops on a terminal snapshot and ignores stale handles or generations.
Cancellation is cooperative: completed steps remain visible, no new work starts
after cancellation is observed, and the current atomic operation may finish.
Real execution provides no rollback or restoration. New real projections are
serial-free and allowlist messages, target facts, and report fields; Android
version is omitted unless an existing trusted string supplies it and is never
derived from API level. React projections omit the sidecar execution id, full
plan, target binding, catalog root, step outputs, arbitrary paths, and raw
sidecar errors.

EmuChef never bundles, vendors, redistributes, mirrors, proxies, or downloads
Android SDK Platform-Tools. The setup UI opens only Google's official
Platform-Tools page and imports a macOS ZIP through a backend-owned native
picker. React cannot submit an archive path. Validation limits archive size and
expansion, rejects encryption, traversal, symlinks, special entries, and
case-colliding names, and extracts only `adb`, `NOTICE.txt`, and
`source.properties` to private application data. The backend records and
rechecks SHA-256 for all three retained files, native Mach-O compatibility,
Google signer Team Identifier `EQHXZ8M8AV`, supported version, and controlled
`adb version` execution.

Platform-Tools 35.0.0 is the minimum supported release and 37.0.0 is the tested
upper bound. Valid newer releases are accepted with an untested-version warning.
Release builds resolve only the validated managed installation and never depend
on `PATH`. Debug builds may use an explicit `EMUCHEF_ADB_PATH` override first or
deliberately enable system ADB lookup with `EMUCHEF_ALLOW_SYSTEM_ADB=1` after
managed lookup. Failed replacements preserve the prior active installation and
settings; removal and cleanup are restricted to the managed application-data
root.

## Testing and Compatibility

Rust unit and integration tests are the product behavior authority. Frontend
typechecking, linting, logic tests, and Tauri tests cover editor behavior and
sidecar packaging.
macOS packaging automation inspects a caller-supplied `.app` rather than a
developer-specific path. It verifies Info.plist identity, host architecture,
the main executable and bundled `emuchef` sidecar, embedded frontend markers,
local signing state, dynamic dependencies, and the absence of Python, shadow,
legacy, and development-server remnants. Separate smoke commands exercise
direct JSONL hello/ping and application launch against that exact bundle.

The network CLI integration test can target the exact packaged `emuchef`
executable through the test-only `EMUCHEF_TEST_BINARY` environment variable.
This test seam is not read by product code. It proves local HTTP cold, warm,
and offline cache behavior plus self-signed HTTPS rejection without changing
the macOS trust store.

`crates/emuchef-rust-backend/tests/fixtures/compatibility_goldens_v1` contains
frozen v1 compatibility fixtures. They are immutable evidence, are not
regenerated from Python, and may change only through an explicit compatibility
contract decision. New behavior uses Rust-native fixtures and tests.

Normal automated verification does not run real-device apply. Device evidence
is collected manually using
`docs/manual/real-device-retroarch-validation.md`. The evidence record at
`docs/release/evidence/real-device-retroarch-2026-07-11.md` confirms a successful
local-artifact baseline, real-device apply and idempotent rerun, clean-cache
HTTP(S) resolution, warm-cache and offline warm-cache reruns, matching cache
manifests, and no leaked partial files on commit
`5dca50603cf3a4831867c229157a94906151cbb7`.

## Release State

Host-target Tauri builds and simulated bundled-sidecar tests are available.
The canonical operator procedure for macOS release bundles is
`docs/manual/macos-packaged-gui-validation.md`; it separates static bundle,
packaged sidecar, packaged runtime, and interactive editor evidence. The editor
exposes side-effect-free planning, but not apply, through its Tauri and JSONL
command surfaces.
Public release readiness still requires real packaged GUI evidence on supported
targets, updater support, and cross-platform release automation.

The macOS `0.1.0` application and disk image built from commit
`93f816fc1ea59cd034a40432e4e2a269e11eead7` have completed Developer ID signing,
hardened-runtime validation, Apple notarization, ticket stapling, local
Gatekeeper assessment, and installed-application assessment. Sensitive Apple
identifiers and raw notarization output remain external. This is local-Mac
evidence; separate clean-Mac validation remains pending.

Release tooling validates the Apple environment without printing credential
values, verifies Developer ID signatures, Gatekeeper notarization, and stapled
tickets independently, and generates a path-safe SHA-256 manifest. The
maintained macOS release verification command supports exact Tauri artifact
discovery or explicit artifact paths, runs the existing bundle and packaged
runtime smokes, and never submits, staples, uploads, or publishes artifacts.
