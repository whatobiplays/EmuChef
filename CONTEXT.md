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

The retired Python implementation, tests, dependencies, package metadata, and
entrypoints are absent from the repository. Python is not required for product
builds, development, tests, packaging, release qualification, or either Tauri
application. Repository policy rejects reintroduced Python product/runtime
code and alternate-backend selection paths.
The canonical retirement contract and evidence checklist are in
`docs/product/phase-4a-python-runtime-retirement.md`.

## GPT Repo MCP Product Contract

The repository-owned machine-readable product contract is
docs/product-contract.json. It is a bounded projection of PRODUCT.md and
the canonical product and qualification documents used for product-grounded
planning. Product decisions remain authoritative in the human-readable
documents; update the contract when those decisions change.

## Config Editor Authored Generation

The Config Editor provides guided generation workflows for a starter app
definition and installation recipe from a local APK, public GitHub repository,
public GitHub release, or direct public HTTPS APK URL, and for a starter device
profile from one connected ADB device. Rust owns APK inspection contracts, typed authored models,
validation, canonical draft generation, collision detection, evidence
classification, and sidecar operations. Tauri owns native paths, configured
external tools, exact ADB serials, native save destinations, final collision
revalidation, and trusted writes. React owns presentation and explicit author
choices.

Generation produces reviewable drafts and performs no authored-data writes until explicit save. The generated recipe opens through the existing recipe document session. App definitions and device profiles initially use dedicated draft forms and canonical YAML previews rather than new persistent editor-session types. App definitions remain catalog and tracking metadata; generated recipes remain execution authority.

Android package facts come from APK inspection rather than filenames. The
APK-inspection contract uses a separately configured user-supplied
`apkanalyzer` or `aapt2`; EmuChef does not bundle Android SDK build tools.
Local APK generation uses a required user-provided APK recipe input. Generated
app definitions use `user_provided_apk` install-source metadata with resolver
`none`, `local_apk` tracking metadata, `artifacts.apk.required: false`, and
`artifacts.byo_apk.required: true`. The selected APK path is session-only. The
last validated analyzer executable and authored root are persisted by the
trusted Tauri layer so later generator sessions can restore them. Verified APK
facts remain review evidence unless the author explicitly enters metadata.

The local APK wizard accepts regular `.apk` files no larger than 2 GiB and a
regular executable whose basename matches the selected analyzer adapter.
Analyzer commands use direct arguments, a 30-second timeout, and a 4 MiB bound
per output stream. The current analyzer command surfaces do not provide a
signing-certificate SHA-256 fingerprint, so that fact remains missing with a
deterministic warning. Split and non-base APKs are rejected.

Remote source analysis runs only in the trusted Tauri layer. GitHub modes use
the GitHub REST API and never scrape rendered HTML. Draft releases are always
excluded; repository mode excludes prereleases by default, while exact
prerelease selections require explicit confirmation. Candidate assets must be
non-empty `.apk` files no larger than 2 GiB. Direct URL mode requires a public
HTTPS address without credentials, fragments, or query parameters. Metadata
responses are limited to 2 MiB, redirects to five HTTPS hops, connection time
to 10 seconds, and complete requests to 30 seconds. APK downloads stream into
a generator-session temporary directory and are removed when that session is
cancelled, completed, restarted, or dropped.

Remote sources may generate either a pinned-download recipe or the same
user-provided APK recipe shape used by local generation. Pinned recipes declare
a `remote_file` artifact, resolve it, and install its `local_path`; their app
metadata records normalized GitHub repository/release/asset identity or the
direct HTTPS APK URL. The user-provided strategy retains the Phase 3
`user_provided_apk` plus `local_apk` source shape and does not persist remote
identity. Credentials, temporary paths, response bodies, and inspected APK
facts are not persisted.

App-generator sessions retain local and downloaded APKs, remote source and
asset selections, analyzer paths, temporary workspaces, and authored-root
paths behind opaque process-memory handles. The Config Editor also maintains one validated,
persistent app-wide authored-root selection. Selecting a root in app generation
updates that shared selection, applies it to an open recipe document, and makes
it available to later app-generator and device-profile-generator sessions.
Authored roots must contain existing canonical `apps`, `recipes`, and applicable
generator destination directories. Saving rechecks APK file identity,
revalidates both typed drafts, reruns both-directory collision analysis, writes
and syncs temporary siblings, and publishes both files with create-new
semantics. A failed second publication removes the first publication when
safe. Successful publication opens the generated recipe through the existing
recipe document session and invalidates the generator session. The Config
Editor permits only one app or device generator wizard at a time.

Generated recipes are minimal: artifact resolution when needed, APK installation, a verified package-installed skip condition, and an optional explicit launch step only when a launcher component was verified and the author enables it. Generation does not infer configuration-copy, root, permission, force-stop, app-data, or device-plan behavior from repository prose.

Device profile generation is available from the Config Editor File menu as an
ephemeral guided wizard. Tauri invokes the existing device-listing and probing
operations with literal `adb` resolved from `PATH`. Generator-session handles
scope every selected device and the generator-local root handle; exact ADB
serials remain in trusted process memory and never enter React state, generated
YAML, or product-facing errors. The generator can bind the existing app-wide
authored-root selection into a fresh opaque session handle, and selecting a new
root updates that shared editor setting. Generator UI receives only safe device
facts and a constant authored-root label.

Standard capture is read-only and includes manufacturer, brand, model, product,
device, board, hardware, ordered de-duplicated ABI values, Android major
version, and Android API level. Drafts propose exact manufacturer and brand
tokens, a regex-escaped anchored model pattern, and Android major as the minimum
without a maximum. Schema version 1 and `device_profile` kind are fixed; all
other proposed authored values remain editable with their original evidence
classification and an edited-from-proposal marker.

Device-profile draft generation, canonical YAML emission, validation, and
collision analysis are side-effect free. Existing ID and destination conflicts
block saving; likely matching overlap warns. An incomplete collision scan still
allows draft review but produces a blocking diagnostic. Final save revalidates
and rescans, derives the destination under the selected root's existing
`device_profiles` directory, rejects overwrite and path escapes, and publishes a
synced temporary sibling with atomic create-new/no-clobber semantics. Extended
shared-storage, package-manager, activity-manager, root, APK, or other capability
checks are not implemented.

Typed schema-v1 `AppDefinitionV1` and `DeviceProfileV1` models are the shared authority for generator output, structural parsing, canonical emission, save validation, catalog loading, and future dedicated editors. Proposed values retain verified, derived, suggested, or missing evidence in draft DTOs; author edits do not replace that provenance, and final YAML contains only reviewed authored values. The complete approved plan is documented in `docs/product/config-editor-authored-generation.md`.

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
resolved inputs, diagnostics, and a backend-authored user-facing review
projection. The projection uses authored setup, feature, action, and input
labels; groups populated preparation, download, install, copy, permission,
launch, and device-change sections in deterministic order; and includes only
authoritative action counts, known waits, device destinations, warnings, and
blockers. Sensitive inputs appear only as `Provided`; approved portable host
inputs appear as filenames or counts rather than host paths. It performs no ADB commands, downloads,
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

The Troubleshooting panel exports a deterministic schema-v1 ZIP through a
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

Completed ADB/device commands with bounded, anchored no-space evidence use the
private `device_storage_exhausted` failure kind and stable issue code. The
classifier accepts recognized ENOSPC output from either bounded stdout or
stderr and either completed exit code, but never overrides timeout, process,
transport, identity, or root failures and never matches arbitrary filenames or
echoed prose. Storage exhaustion fails the active real step, preserves earlier
evidence and conservative possible-partial-change reporting, stops later
device/host work, leaves never-started work pending for terminal **Not
attempted** projection, releases the active slot, and requires fresh device
qualification, plan generation, review, and execution after the user frees
space. EmuChef never deletes user data, retries, resumes, or continues the old
execution automatically. Tauri exposes only authored storage guidance and no
raw ADB text, paths, serials, or command arguments.

Physical storage qualification has a separate dependency-free operator utility
at `tools/device-storage-preflight.mjs`. Its immutable
`phase-6d6-low-storage` profile requires exactly one selected authorized ADB
device and `com.emuchef.fixture`, verifies that `/sdcard/Download` and the
qualification destination share the same reported filesystem and mount, and
uses device-local `dd` to create non-sparse chunks only under the exact marked
`/sdcard/Download/EmuChefStoragePreflight/phase-6d6-low-storage` directory.
Status and dry-run are non-mutating; preparation requires explicit confirmation,
remeasures after each chunk, stops inside the 4,194,304–5,308,416 KiB window,
and resumes only an exactly marked directory containing recognized chunk
names. Cleanup rejects symlinks, missing or mismatched markers, and unknown
entries; it removes only the owned profile directory, synchronizes the device,
verifies absence, and reports restored capacity. This preflight allocation is
operator setup rather than physical evidence or production execution. It
remains across both low-storage repetitions while the harness independently
owns and cleans its run-scoped reserve, filler, payload, and sentinel state.

Host suspension does not create a second execution owner. The same locally
owned child/future tree remains authoritative if the process generation
survives sleep; after wake the existing operation may complete, report typed
transport loss, or reach `operation_timed_out` according to the fixed Rust
deadline semantics. If the sidecar generation is lost, the terminal outcome is
`runtime_session_lost` and Tauri projects `execution_unavailable` with an
indeterminate real-device outcome. Application restart never resumes an
execution. Host-sleep qualification measures whether the existing timer source
observes suspended time on the qualified host. Passing evidence requires
samples from the production deadline clock at start, before sleep, after wake,
and terminal; remaining budget before and after suspension; wall duration;
tolerance and rationale; and the scenario phase. Classification is derived
from clock advancement and budget consumption, never from the terminal result.
Excluded suspension may still time out after later active time. Transport loss,
missing samples, or inconsistent measurements block qualification. The owned
process timer now shares one exact monotonic start/deadline basis with the
qualification observations: the harness samples the exact deadline clock
immediately before the `sleep-entered` handoff and again after the `wake`
marker, and the owner records the terminal clock sample from the same basis.
The retained basis keeps a truthful post-wake sample available even when the
owner selected terminal immediately after resume. `sleep-entered` is the final
operator handoff immediately before physical suspension (not an OS sleep-entry
event) and is the activeProcess action boundary; `wake` is the first post-resume
operator acknowledgement. Both host-sleep scenarios reuse the private
`/dev/zero -> /dev/null` `DeviceCopy` stimulus with a scoped, one-shot,
`#[cfg(test)]` 120-second qualification deadline; production `DeviceCopy`
remains 300 seconds. The implementation blocker is removed, but the two
physical host-sleep repetitions are still missing until an operator runs them.
It does not add a sleep inhibitor, OS plugin, checkpoint, resume token, or
replay path. The host-sleep deadline phase is anchored to the exact
owned-process deadline-clock start (`DeadlineClockStarted.at`), which is also
serialized as `hostSleep.operationStartedAt`; the earlier
`operation-started` sentinel marker remains an independent chronology
observation and is not the 120-second deadline threshold authority. That wall
timestamp is the wall observation retained alongside construction of the exact
monotonic timer basis, not a later observer-install timestamp. The final
host-sleep lifecycle snapshot is taken only after the bounded watcher has
published its retained-basis post-wake sample, so the owner may reach terminal
immediately after resume and terminal may precede that post-wake sample.
Host-sleep evidence records persist the sanitized owned-process lifecycle
observations in `trace.lifecycle` before deriving `activeProcess` and
`hostSleep`; blocked attempts retain the partial lifecycle trace and the exact
watcher gate error, so a null projection never discards the source
observations.
`transport_loss` requires zero owner-emitted `DeadlineReached` events; the
owner event is the only authority for whether the timeout branch won, and a
monotonic clock sample at or beyond the nominal deadline never converts a
transport failure into a timeout.

Physical interruption qualification is isolated to the ignored Phase 6D.6
harness. It requires one exact selected serial, the committed fixture package
and roots, explicit destructive/root/authorization/identity/host-sleep opt-ins,
an operator-controlled ten-minute sentinel checkpoint, and two clean
repetitions per mandatory scenario. Active interruption requires exact target
child and mutation liveness immediately before the action; a runner callback,
delayed poll, post-operation probe, or harness boolean cannot qualify it. The
current physical adapter emits no exact child evidence and blocks active cases.
Each record has unique run, scope, sentinel, nonce, slot, path, trace, and
canonical content identities, and UI-smoke subcases bind digested UI-state
artifacts to distinct physical backend runs and traces. Host simulations and
deterministic tests are regression evidence only, not physical-device evidence.
Until the complete sanitized matrix and automated verification pass, Phase 6D
remains In progress.

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

The Apple Silicon end-user app implements user-triggered signed update
discovery and manual DMG browser handoff. Rust owns the source-pinned manifest
and DMG URL policies, dedicated metadata public key, no-proxy identity-only HTTP
client, response status/header/length checks, 64 KiB streamed body bound,
canonical fixed-JSON parsing, Ed25519 verification, stable target and version
policy, expiry, retained candidate, activity leases, and operating-system
opener. React receives only display metadata and availability; no URL, key,
signature, raw response, path, or generic opener argument crosses IPC.

Production update trust contains only schema version 1 and `configured: false`.
The local unconfigured status performs no DNS, HTTP, proxy discovery, browser
action, or network-dependent migration. Checks are user-triggered only and
never block startup or ordinary local operation.

The browser downloads the DMG. EmuChef does not inspect or verify the local
file, mount it, install it, replace the app, or restart. Signed size and SHA-256
are release identity metadata rather than proof of the local download. Apple
Developer ID, hardened runtime, notarization, stapling, and Gatekeeper are the
executable trust controls. Saved configurations, recovery, diagnostics, cache,
and managed Platform-Tools remain in app data and outside update authority.

Release tooling reuses Phase 3E credentialed verification, proves that a
read-only release-tooling mount contains the exact verified app tree, emits
deterministic unsigned fixed JSON, and finalizes only after verifying an
external Ed25519 signature. Production trust, hosting, credentialed release
metadata, and clean-Mac manual replacement evidence remain pending. The former
in-place updater remains rejected because its pinned API could not meet the
single-response pre-deserialization trust model.

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
saved setups, execution results, and Troubleshooting. It provides a
skip link, stable landmarks and headings, semantic collections, associated
input diagnostics, focusable error summaries, bounded polite/assertive live
regions, and native determinate execution progress. Status and availability
always have text and structural differences in addition to color. The visual
system uses semantic color, spacing, radius, focus, and status tokens with
system fonts and a dark-only application appearance. Controls, panels, dialogs,
empty states, disabled explanations, long-text wrapping, and result groups use
shared styles. Responsive source and DOM regressions target approximately
380-pixel reflow at 200% zoom, reduced motion, forced colors, focus visibility,
and screen-reader structure without changing the workflow architecture. Those
automated targets do not constitute manual macOS visual or assistive-technology
qualification.

Normal workflow terminology describes the app service, Platform-Tools, setup
catalog, saved setups, installation progress, recovery drafts, and app-owned
cache without exposing implementation language, schema versions, digests,
catalog identifiers, or raw enum values. The header readiness indicator says
only `Ready`; technical identity remains available only through bounded
troubleshooting and diagnostic projections.

Custom prompts and confirmations use exactly-once controllers. Each pending
request has one safe cancellation result, teardown never implies a destructive
or execution-confirming choice, and an overlapping request cannot replace the
live resolver. Runtime restart, configuration replacement, app reset, unmount,
and the top-level frontend error boundary safely cancel pending prompts.
Troubleshooting owns and cancels its nested cleanup confirmation before the
parent closes.

All React modal surfaces use `AccessibleDialog` for focus containment,
deterministic restoration, Escape handling, and the shared dialog-heading and
footer-action placement convention. The Updates dialog disables and explains
its Close action while an update check or browser handoff is active, announces
blocked dismissal attempts, and becomes dismissible again when the unsafe
operation ends.

Modal and native-dialog focus restoration first validates the recorded invoker,
then uses a transition-specific workflow or error destination, the current
workflow fallback, main content, and finally the header Troubleshooting
action. Disconnected, hidden, disabled, or inert invokers are skipped. Focus
never silently falls to the document body, and generation checks prevent stale
restoration from stealing focus from a newer modal or workflow transition.

The app creates, previews, opens, edits, saves, renames, duplicates, imports,
exports, and reuses named portable setup files. Schema V2 contains a generated
configuration identity, presentation name, one authored device-plan reference,
selected recipe IDs, explicitly nonsensitive user bindings, safe additive
extensions, and authored-contract compatibility fingerprints. Fingerprints use
canonical device-plan, recipe dependency/capability/artifact/step/constraint,
input-contract, authored override, and profile-capability semantics. They do
not use presentation labels or prose, resolved values or artifacts, generated
plans, catalog load order, host paths, device facts, runtime state, review, or
execution state. Configuration identity changes do not change fingerprints.

Schema V1 remains readable through an in-memory migration. Because V1 has no
historical contract fingerprints, inspection can establish only whether the
intent validates against the current catalog; it never claims historical
compatibility. The V1 source is not rewritten during inspection or preview.
Its first explicit V2 save establishes the durable compatibility baseline.
Unsupported future schemas and malformed or authority-bearing fields fail
closed without changing the source. Additive V2 `x-*` extensions are preserved.
Other structurally safe unknown fields remain visible as pending sanitation
until an explicit save removes them and reports that consequence.

Tauri owns native setup dialogs, absolute paths, sidecar document identifiers,
opaque document and preview handles, compatibility and comparison decisions,
repair revisions, file writes, and the private ten-entry Recent index. Preview
confirmation rechecks source bytes, source digest, catalog digest, runtime
revision, and preview revision. React receives filename-only context, sanitized
summaries, portable intent, dirty and compatibility states, and friendly repair
actions. It never receives saved-file paths, source document IDs, fingerprint
internals, raw catalog IDs in diagnostics, credentials, or runtime authority.

The native File menu provides New, Open, Open Recent, Save, Save As, Import,
and Export, plus access to the focused Saved setups manager. The manager owns
Rename, Duplicate, missing-file relink, and Remove from Recent equivalents.
Recents use canonical path identity, deterministic last-opened ordering, and a
private path tie-breaker. Distinct paths with the same configuration ID remain
separate and are marked as an identity conflict. Missing entries remain visible
with filename-only context; relinking changes only the private Recent path and
never edits or deletes either file.

Save writes the active identity. Save As writes a new identity and makes that
copy active. Rename rewrites the internal display name and sibling filename
while retaining the configuration ID. Duplicate writes a new identity and adds
it to Recents without changing the active workflow. Import validates a preview,
writes a new-identity copy to an explicit destination, and opens it. Export
writes a sanitized new-identity copy without changing the active workflow or
Recents. Every destination operation is no-clobber. Ordinary writes use a
flushed and synced temporary file followed by an atomic replacement. Rename
with an internal-name change safely creates and syncs the destination before
removing the source; if source removal fails, both valid files remain and the
app reports that state rather than claiming pairwise atomicity.

Opening or importing replaces portable intent and invalidates review and
execution authority. Repair and portable-input relink change intent and also
require fresh validation, planning, and review. Ordinary Save, pure Save As,
and Rename preserve the current workflow stage and review because persistence
identity and presentation name are not plan inputs. Duplicate and Export leave
the active workflow and authority untouched. Repair is explicit and bounded to
exact authored aliases, removal of unavailable optional recipes or retired
bindings, selection of a current valid option, one portable-input relink, and
re-entry of omitted sensitive input. Repair-required setups cannot replace the
active workflow until authoritative validation succeeds. The current authored
recipe schema has no rename-alias contract, so removed recipe IDs are never
matched heuristically; a future exact alias can be applied only after that
authority is defined in the catalog schema.

Opening a setup classifies bindings through the authoritative
`describeConfiguration` operation. React receives only active bindings marked
explicitly nonsensitive. Sensitive, inactive, and unclassified bindings stay
backend-owned and appear only as a pending-sanitation count or label-based
re-entry requirement. Opening, previewing, cancelling, or closing never
rewrites the source. Explicit Save and Save As remove omitted bindings before
writing, so credentials and resolved sensitive values are never persisted or
exported.

Phase 2B guarded real-device execution is implemented behind the
default-disabled Cargo feature `real-execution`. Rust projects the immutable
`ExecutionCapabilities` contract containing `realExecutionCompiled`,
`platformToolsStatus`, and `executorReadiness`. Ordinary development and
production commands report real execution as not compiled, Platform-Tools as
not applicable, and the executor as not compiled; they remain simulation-only
and do not perform readiness validation. The development-only
`tauri:dev:real` npm command intentionally passes `--features real-execution`
through the Tauri CLI and reports real execution as compiled; it does not alter
Cargo defaults or any build, bundle, release, or packaging command.

## Developer commands

The repository root `Makefile` provides the standard developer workflow.
`make build` builds the Rust backend and both frontend codebases. `make test`
runs the full automated Rust, application, security, typecheck, and lint suite.
`make emuchef-app` launches the end-user app, `make config-editor` launches the
Config Editor, and `make dev` launches both apps concurrently. These Makefile
targets use ordinary simulation-only development commands; they intentionally
do not use the separate real-device `tauri:dev:real` command.

The checked-in `EmuChef execution feature matrix` GitHub Actions workflow is the
continuous compile-policy authority for Phase 6A. It runs `cargo check` and
`cargo test` for the Tauri crate with `--no-default-features` and with
`--no-default-features --features real-execution`, using the committed lockfile.
A separate workflow job runs the source-level security policy suite, which
proves that Cargo defaults and ordinary development, packaging, and release
scripts remain feature-disabled and that only `tauri:dev:real` enables the
feature. The workflow does not build or publish a production artifact with real
execution enabled.

Feature-enabled capability refresh clones the resolved managed or development
ADB state and its revision while holding the trusted mutex, releases the mutex,
then performs bounded local signature, retained-file, and `adb version`
validation without device enumeration, server startup, or device commands. A
result is published only if both the ADB revision and runtime generation still
match. Platform-Tools reports ready, not found, invalid, or check failed, while
executor readiness reports ready, blocked, or unknown. Infrastructure failures
surface only the sanitized command error
`execution_capabilities_unavailable`; tooling outcomes remain typed capability
states.

Capability refresh runs at startup, after app-service restart, and after
managed Platform-Tools install, replacement, or removal. While a lifecycle
refresh is active, React retains the prior valid capability value and marks the
diagnostic rows as refreshing. A failed refresh retains that value and reports
status unavailable rather than manufacturing a new diagnostic. Capability
readiness is informational: it does not change real-execution visibility,
eligibility, confirmation, mode selection, or start authority. Guarded real
execution independently revalidates Platform-Tools immediately before listing
devices or starting execution. Trusted Tauri code owns confirmation, mode
selection, exact target identity, reviewed plans and digests, Platform-Tools
revalidation, unambiguous retained BYO file/directory checks, and sidecar
identifiers. Platform-specific packaged-device evidence, privacy and security
approval, an operator runbook, and a separate release decision are required
before a release build opts into the feature.

Tauri launches the sidecar and negotiates the `phase0_end_user_runtime`
extension plus `describeCatalog`, `listAdbDevices`, `probeDevice`,
`matchDevice`, `describeConfiguration`, `planConfiguration`, `startExecution`,
`getExecution`, `getExecutionEvents`, `cancelExecution`, and the schema-v1
user-configuration document operations before ADB is needed. Runtime and
catalog startup are independent from Platform-Tools setup.
A missing ADB installation blocks only device discovery and displays the
Platform-Tools setup flow.

The end-user app's maintained macOS packaging target is a thin Apple Silicon
`aarch64-apple-darwin` release bundle with a macOS 11.0 minimum. It bundles the
release Rust sidecar, authored catalog snapshot, and checked-in qualification
policy through Tauri resources. Intel and universal end-user bundles are not
qualified. Platform-Tools and user content remain external.

The checked-in `app-icon.png` is the byte-preserved approved branding master.
Tauri packages a generated 512-by-512 `icon.png` and a multi-resolution
`icon.icns` derived from that master without cropping, stretching, padding,
background insertion, or redesign. The native About menu uses the package name
and version from the running Tauri application's package metadata and includes
the product description and GNU GPL v3.0 credit; frontend and menu source do
not duplicate the configured version.

Local macOS qualification is ad-hoc, requires no private Apple credentials,
removes the fixed Apple credential allowlist from its child environment, and
explicitly sets the ad-hoc signing identity. It statically verifies product
metadata, thin arm64 executables, nested and deep signatures, resources,
catalog content, production CSP, minimal capabilities, path independence, and
default-disabled real execution. A copied-app probe uses a canonical temporary
root and temporary home/data/cache locations to negotiate the packaged sidecar,
load the bundled catalog, and perform a read-only catalog operation without
ADB, network access, a device, Platform-Tools, BIOS files, or ROMs.

Credentialed developer-id mode is an explicit separate operation. Only that
mode validates the fixed supported Apple variable allowlist; values stay in
the caller environment or keychain and are not printed or serialized. Local
qualification cannot establish Developer ID identity, notarization, stapling,
Gatekeeper acceptance, clean-Mac behavior, or public release readiness.

End-user macOS manifests record toolchain versions, target, release mode,
source commit, tracked dirty state, qualification-policy version, normalized
security-relevant content, and raw per-build artifact identities. Normalized
content covers signature-removed executable bytes, Info.plist semantics,
catalog resources, capability policy, Tauri security configuration, sidecar
identity, and default-disabled real execution. Code signatures, timestamps,
DMG container metadata, mtimes, temporary names, and caller-specific absolute
paths are excluded or normalized. Raw app and DMG hashes identify one build
and are not claimed to remain byte-identical across rebuilds.

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

The end-user app keeps at most one schema-v1 recovery record for dirty portable
intent under a Tauri-owned fixed application-data path. The record is bounded,
atomically replaced, owner-only on Unix, and generation checked. It can contain
a safe display name, authored device-plan reference, recipe IDs,
schema-permitted bindings, key-only omitted-binding markers, and a source
saved-configuration identity. It never contains device, generated-plan,
review, execution, confirmation, launch, dialog, cache, diagnostics, sidecar,
log, process-output, or raw-error authority.

Startup validates the recovery record before normal workflow use and offers
Restore, Discard, or Not now. Restore creates fresh document/frontend sessions
and requires current device selection, probing, validation, description,
planning, and review. Not now keeps the exact generation on disk across clean
shutdown and offers it on the next launch. Deferred disposition is separate
from current-session dirty state. Explicit discard, successful Save or Save As,
or a newer atomically staged dirty generation clears or supersedes it.

Only authored input `sensitive` metadata controls binding recovery. Explicitly
non-sensitive values follow the existing portable saved-configuration rules,
including existing file/directory bindings. Sensitive and unclassified values
are omitted without hashes, masks, or length leakage and produce a sanitized
required-re-entry diagnostic after restore. Key names and value shapes never
classify secrecy.

## End-User Troubleshooting and Local State

EmuChef proper exposes one Rust/Tauri-authored troubleshooting projection for
the app service, Platform-Tools, connected devices, the bundled catalog,
app-owned cache, updates, saved-setup and recovery state, and retained
execution status. The projection contains only the subsystem label, bounded
severity, plain-language summary and consequence, optional stable public
support code, and closed corrective-action variants needed by the support
surface. Healthy state is summarized once, affected subsystems receive primary
emphasis, and neutral optional or unconfigured capabilities do not receive a
failure code. The projection is not an independent application state machine.

Public support codes are fixed uppercase ASCII constants in a closed Rust
registry. Each code has one subsystem and bounded meaning; dynamic internal
error text, internal result names, paths, serials, command lines, logs, stack
traces, credentials, configuration contents, input values, plans, and runtime
authority never enter the code or normal troubleshooting DTO. Unknown internal
failures map to a deterministic bounded fallback without projecting their
text. React exhaustively routes known corrective-action variants and fails
closed for unknown variants.

Corrective actions use the narrow authority revision owned by the affected
subsystem. App-service restart uses the service generation, managed
Platform-Tools changes use the Platform-Tools revision, device refresh uses the
device generation, and cache deletion uses its inventory generation and opaque
entry handles. Destructive work is revalidated immediately before mutation.
Cache cleanup is restricted to fingerprint-matching direct children of the
canonical app-owned cache root that remain managed, removable, in an approved
category, and not in use. Missing approved entries are idempotent; changed,
external, unmanaged, protected, or in-use entries are not deleted. Replace and
Remove Platform-Tools are offered only for the app-managed installation and
never delete a user-selected, PATH-resolved, system, or external installation.

Support and diagnostics outcomes are scoped to their modal and operation
generations. Opening or closing Troubleshooting, refreshing inventory, or
starting a retry clears superseded presentation notices, and stale completions
cannot replace current state. Diagnostics export creates a local ZIP no larger
than 2 MiB and never uploads it. Schema version 2 contains only the fixed
members `manifest.json`, `runtime.json`, `catalog.json`,
`configuration-summary.json`, `execution-summaries.json`,
`cache-summary.json`, and `support-status.json`; the archive contains bounded
aggregate state and active public support codes, not support UI state, action
or reset handles, presentation generations, historical notices, raw errors,
logs, paths, serials, credentials, configuration bodies, input values, or
plans.

Reset Local App State presents only categories that have current app-owned
state and issues one-shot opaque reset handles bound to that category's
revision. Current categories clear the Recent setup index, approved app-owned
cache entries, or the recovery draft. Each has separate description, scope,
consequence, availability, and explicit confirmation. Clearing Recents does
not close an active document or delete saved setup files. Cache reset does not
delete configurations or external content. Recovery reset is distinct from
Restore and Discard workflow decisions and does not clear the live-process
marker, active workflow, saved files, portable intent, review authority, or
external content.

The recovery active-session marker represents process lifetime, not window
lifetime. Closing one window or the final macOS window, cancelling a close, or
restarting the local app service leaves the marker active while the process is
alive. Only an accepted application exit, including Cmd+Q, or an
application-controlled exit/relaunch finalizes it synchronously before process
termination. A crash or other unclean termination leaves the marker for the
next launch, preserving genuine interruption detection.

Packaged catalog data is materialized under the application resource directory.
The trusted backend requires the four product directories, ignores only regular
`.gitkeep` placeholders, rejects symlinks and every other unsupported entry,
computes the canonical catalog SHA-256, and passes a
resolved snapshot to the sidecar. React receives source identity, version, and
digest without a catalog filesystem path. The catalog remains a bundled MVP
source; Phase 4B release discovery does not update or replace catalog content.

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

Each accepted configuration description replaces the runtime session's current
native input-contract snapshot. A request sequence rejects out-of-order
descriptions without creating a separate authority generation. Tauri derives
picker kind, multiplicity, extension filters, and immediate filesystem checks
from that snapshot; React supplies only the input key and portable edit intent.
Device paths never receive host-picker authority.

The Inputs stage presents authored labels and descriptions, required or
optional state, single or multiple value state, accepted extensions, and safe
authoritative constraints. Untouched missing requirements remain neutral until
the user edits the field or explicitly requests validation or review. Host path
validation detects missing, inaccessible, wrong-kind, unsupported-extension,
and canonically duplicate entries before review. Canonical duplication inside
one input is blocking; reuse across active inputs is a non-blocking warning that
names both labels. Errors, logs, omission notices, diagnostics, and support
surfaces do not include host paths, while portable user-selected paths remain
visible in the ordinary input UI as selected intent.

Multi-file inputs expose per-entry validation and support add, replace, relink,
remove, and clear. Relinking replaces only the selected entry and retains the
device plan, recipe selection, and unrelated bindings. Missing or inaccessible
retained paths are repaired by explicit user selection; EmuChef never searches
the filesystem heuristically. Drag-and-drop is not part of the current input
workflow.

The trusted configuration-description response retains the exact target device
binding and verified catalog identity/digest. The React projection omits the
target binding, catalog root, raw sidecar payload, and all exact serials.
Sidecar failures map to stable sanitized configuration error codes and useful
recovery messages. Debug builds log the complete internal error to the Rust
terminal only after redacting exact serials and absolute paths; release builds
do not expose raw internal errors.

Tauri retains the complete immutable reviewed-plan result, exact target binding,
catalog identity and digest, and canonical plan digest behind an opaque review
handle. Rust owns the review meaning and classification. Tauri only retains the
exact plan, verifies trusted identity and digest state, attaches the opaque
handle to the backend-authored projection, and defensively redacts the exact
serial. React renders the projection without receiving the plan digest, recipe
or step IDs, input keys, capability tokens, raw parameters, hashes, host paths,
or diagnostic codes. A review marked unsafe by the backend cannot start either
simulation or real execution. At most 16 live reviews
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

`getExecution` snapshots are authoritative for feature-grouped progress,
current user-facing action, completion counts, and terminal state. Incremental
events are presentation data only and use their own accepted-event cursor;
snapshot sequence values do not skip unseen events. Snapshot and event polling
share generation and opaque-handle guards, and a terminal state cannot be
downgraded by a late active snapshot. Polling stops after the terminal snapshot
and its concurrently retrieved event batch are accepted. Visible timestamps
are localized by React while canonical timestamps remain in retained and
exported reports. Export-success presentation is scoped to the execution
generation and handle, not to changing sequence values.
Cancellation is cooperative: completed steps remain visible, no new work starts
after cancellation is observed, and the current atomic operation may finish.
Real execution provides no rollback or restoration. New real projections are
serial-free and allowlist messages, target facts, and report fields; Android
version is omitted unless an existing trusted string supplies it and is never
derived from API level. User-facing issues resolve opaque executor identity
inside trusted code to authored feature and action text, pair it with
backend-classified recovery guidance, and omit raw verifier/dependency codes.
Failed, cancelled, unavailable, stale, and repaired runs require a fresh plan
and review before execution; a prior review may remain visible only as
non-executable history. React projections omit the sidecar execution id, full
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
targets, reviewed production update trust and hosting, credentialed signed
release metadata, clean-Mac manual replacement evidence, and cross-platform
release automation.

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

## Phase 6B device qualification contract

Phase 6B device qualification is a Rust-owned projection keyed by an opaque
session device handle. React receives no resolved ADB serial and uses only the
opaque handle, runtime generation, and qualification revision for stale-result
guards. Passive qualification distinguishes feature-disabled, absent,
unauthorized, offline, ambiguous, unsupported, incompletely qualified, and
supported states using sanitized normalized facts; multiple devices never
produce an implicitly selected target.

Root qualification is an explicit user action. The native command resolves the
selected handle to its current trusted serial and asks the sidecar to run the
bounded probe `adb -s <serial> shell su -c id` with a 30-second timeout. Its
sanitized result is one of `granted`, `denied`, `unavailable`, or
`checkFailed` with `timedOut`, `transport`, or `unexpectedResponse`. Only
recognized ADB/device transport failures use the `transport` reason; completed
shell failures use the unavailable, denied, or unexpected-response rules.
Results are held in an in-memory native store bound to the handle, runtime
generation, and Platform-Tools revision. A root result change invalidates
reviews and capability-dependent configuration projections. Polling removes
cached evidence whose device, runtime, availability, or Platform-Tools context
no longer matches; the removed opaque handle is used to stale only reviews
planned with that evidence. A failed prerequisite is resolved before a root
check is reserved, so it cannot leave a check permanently in flight.

Real execution performs one lazy root preflight before any root-capable step,
app-private skip check, app-private read/write, or app-private verification.
The outcome is cached for the run; a failed preflight aborts later steps before
any privileged operation. Ordinary simulation and ordinary development/build
scripts remain unaffected. Automated tests cover the protocol, projection,
classification, single-flight/stale behavior, capability masking, and
executor guard. Physical-device, packaged-GUI, signing, notarization, and
release qualification are not implied by those tests.

Host input binding validation reports a missing-path diagnostic only when host
metadata cannot be read; an existing path of the wrong kind reports a kind
mismatch, and an existing path of the expected kind is accepted. This keeps
configuration planning usable for the root-capability review gate.

## Device qualification repository contract

The repository-owned device qualification Node tool remains the canonical
authority for target registration validation, qualification-candidate
promotion, evidence-bundle validation, canonical digests, compatibility
projection, current-evidence selection, and matrix rendering. Production
runtime behavior is unchanged by these repository contracts.

`docs/testing/device-qualification/device-targets.json` now uses schema
version 2. Every material target fact is stored as an exact `{ value, source }`
wrapper, with legal provenance restricted by field: ordinary identity facts use
`production_observation`, `rootState` uses `explicit_root_check`, and
`connectionType` may use `operator_attestation` or
`production_observation`. Registered target IDs are deterministic
`device-target-sha256:<64 hex>` digests over the wrapped facts' values only, so
policy arrays and provenance-source changes do not alter identity while
material fact-value changes do.

Device qualification evidence records and fingerprints also use schema version
2. Embedded `deviceTarget` facts preserve the same typed provenance wrappers as
the registered target, while compatibility fingerprints project only the fact
values needed by workflow invalidation. `fingerprint.emuchefBuild` is a strict
object containing the application version, exact Git commit, canonical
material-build digest, `realExecutionEnabled`, and
`qualificationContract`. Compatibility for the `emuchef_build` dimension
compares the application version, material-build digest, `realExecutionEnabled`,
and qualification contract only; the exact Git commit is preserved as audit
provenance but does not invalidate evidence by itself. The material-build
digest is repository-owned and covers the current working tree contents of the
tracked product/runtime/authored inputs under `authored/`,
`crates/emuchef-rust-backend/`, and the application source roots plus the exact
package/lock/config files declared by `tools/device-qualification.mjs`, while
excluding qualification-only UI/runtime files and evidence artifacts.

Physical-device evidence is now bundle-shaped. Each recorded run directory under
`docs/testing/device-qualification/evidence/` contains `evidence.json` and,
when report capture succeeded, a digest-bound `execution-report.json`. Every
valid run must contain exactly one `artifacts` entry for
`execution-report.json`, the matching required automated observation, and
report bytes whose SHA-256 matches the bound artifact. Invalid
`qualificationOutcome: "not_observed"` audit bundles may omit the artifact and
report file only when report capture itself was unavailable. Synthetic evidence
fixtures under `tests/fixtures/device-qualification/` follow the same bundle
layout.

The canonical workflow contract for `retroarch-plus-bios` is now version 2.
Its prerequisite `clean_or_deliberately_reset_device` remains declared in
`prerequisites` and is also a required human checkpoint with allowed outcomes
`pass`, `fail`, and `unable_to_verify`. This checkpoint is a pre-execution
qualification gate: only `pass` may participate in a valid record, while
`fail` or `unable_to_verify` force an invalid `not_observed` run rather than a
product qualification failure.

Qualification promotion is create-new and repository-bounded. Target and run
promotion accept only `qualification-candidate-<32 lowercase hex>` identifiers
through `node tools/device-qualification.mjs --register-target <candidate-id>`
and `node tools/device-qualification.mjs --record-run <candidate-id>`. Before
any canonical mutation the tool rechecks that the candidate build commit still
matches `HEAD`, the tracked worktree is clean, the candidate material-build
digest matches the current repository state, `qualificationContract` still
matches the current tool contract, and `realExecutionEnabled` remains true. Run
promotion also reloads the current workflow catalog, target registry, authored
recipe digests, and runtime contract, rebuilds the expected fingerprint, binds
the embedded evidence `deviceTarget` from the registered target rather than the
candidate payload, and rejects any drift before sealing the run.

`tools/device-qualification.mjs` is also the sole repository authority for
deriving build/runtime identity. `node tools/device-qualification.mjs
--build-identity` prints the canonical current build identity, optionally with
`--require-clean` to reject tracked worktree drift. `node
tools/device-qualification.mjs --describe` prints a machine-readable repository
description containing schema version 1, runtime contract
`real-execution-v1`, qualification contract version 1, the canonical build
identity, the validated workflow catalog, and the validated device-target
registry. Active repository projection and validation no longer read
`EMUCHEF_PHASE_6F_BUILD_IDENTITY` or `EMUCHEF_PHASE_6F_RUNTIME_CONTRACT`.

Recordable qualification application builds use the pinned
`apps/emuchef-app/scripts/run-device-qualification.mjs` launcher. It performs
the frontend build, prepares the development Rust sidecar, and then starts the
Tauri application with the `real-execution` Cargo feature and
`EMUCHEF_DEVICE_QUALIFICATION=1`; it does not use Vite hot reload or `tauri
dev`. When that opt-in is present, the Tauri build script obtains the build
identity only from `tools/device-qualification.mjs --build-identity
--require-clean` and embeds the resulting strict camelCase JSON as
`EMUCHEF_QUALIFICATION_BUILD_IDENTITY`. Rust qualification mode requires a
debug build, the compiled `real-execution` feature, the explicit runtime opt-in,
and a successfully parsed embedded identity whose real-execution capability is
enabled. Before invoking the canonical Node command, an opted-in Cargo build
watches that tool, the declared authored/backend/application material roots and
exact package/lock/config files, every tracked repository path, and the Git
reference/index state used by the identity command, so a changed input cannot
reuse stale embedded metadata. Ordinary application builds omit these watches
and the identity, retaining their normal production behavior.

Active schema-v2 run records use the domain-oriented immutable ID form
`qualification-run-sha256:<64 lowercase hex characters>`, derived from the
unsealed record payload and then sealed with canonical `fingerprintDigest` and
`recordDigest`. Current-state projection selects only valid, compatibility-clean
bundles as current evidence; invalid runs remain historical audit evidence and
cannot replace current qualification state. The current production target
registry remains empty, so repository validation and matrix generation still
make no claim that any physical device is qualified.

## Phase 6D.6 physical interruption qualification

The ignored Rust physical harness is fail-closed and fail-reporting: explicit
execution returns a test error for missing gates, blocked checkpoints,
unexpected terminal issue codes, incomplete step accounting, unsafe cleanup, or
an incomplete scenario contract. It runs one exact scenario and repetition
through a reviewed execution plan and the real ADB executor boundary.

The checked-in scenario manifest is the authority shared by the Rust harness
and the dependency-free Node evidence validator. Each contract specifies
expected execution type and issue codes, accepted step/Not-attempted counts,
partial-change and authority dispositions, exact target-process facts,
production-slot lifecycle, host deadline-clock measurement and phase,
transition chronology, UI artifacts, cleanup, and residual requirements.
Evidence includes canonical record and trace digests plus unique run, scope,
sentinel, nonce, path, and slot identities; copied or relabelled records are
rejected.

Active-cancellation qualification requires the exact target mutation child to
be spawned, started, and alive immediately before cancellation, with the action
strictly before the child's terminal event and bound to the same run and
operation. A runner callback, post-operation probe, delayed poll, or local
boolean is insufficient. The harness now binds a production-owned process
observation to the first reviewed host `Push` and creates `active-ready` only
after sampling that exact child alive. A safe-boundary cancellation uses a
distinct finished-before-request phase. The active-slot record observes the
RAII guard owning the production execution-session slot, carries the exact
run-scope and execution identity, and cannot pass on an auxiliary lease,
shadow flag, early release, or another run's lifecycle.

Same-serial replacement qualification polls successful ADB inventory samples
and stable fingerprints to prove original attachment, a serial-absent interval,
replacement attachment after disconnect, changed identity, and no simultaneous
target. Authorization qualification is a safe-boundary reconnect case: the
first operation completes, stored trust is revoked, the exact selected serial
is proven absent, and the same serial must reconnect as genuinely
`unauthorized` before `operator-action` releases the second operation. The
second operation must fail before mutation. Its terminal issue may be
`device_unauthorized`, or `device_identity_unverified` when the production
pre-operation identity guard cannot collect complete evidence from that
independently observed unauthorized device. The identity branch never qualifies
without the exact authorization chronology. Explicit reauthorization and a
final authorized cleanup state remain required; changed identity, generic
identity failure, offline, disconnect, and mismatched issue evidence are
insufficient. Earlier blocked attempts remain non-passing audit evidence under
their exact historical contracts.
Physical `device_offline` evidence is conditional diagnostic evidence rather
than a closure requirement. ADB offline is normally a transient transport
initialization or failure state with no general operator-controlled transition.
The harness and schema continue to accept truthful offline attempts, while the
twelve-scenario mandatory matrix uses the active and boundary USB-disconnect
cases as its physical transport proof.
Development-build UI smoke is mandatory closure evidence: two composite
records each contain cancellation, transport, root, storage, and
host-sleep/runtime-loss subcases. The transport subcase binds only to passing
active or boundary USB-disconnect evidence, not conditional offline evidence.
Every subcase binds an exact physical backend
run and trace to a distinct UI sub-run, development-build digest, exact authored
projection, **Not attempted** count, partial-change and authority recovery
state, forbidden-control absence, canonical UI-state artifact, and
artifact-bound operator observation. Nested unsafe text is rejected.

The mandatory `operation_timeout` physical scenario uses the reviewed first
copy step with the exact hard-coded private pseudo-device paths `/dev/zero`
(source) and `/dev/null` (destination). No FIFO or other special file is
created on the device; the real path is `copy_files` with
`source.location == "device"` through `RealAdbDevice::copy_on_device` and the
owned `DeviceCopy` process. A private, thread-local, one-shot `#[cfg(test)]`
deadline override selects exactly 15 seconds for this ignored qualification
entry point; production `ProcessOperation::DeviceCopy.deadline()` remains 300
seconds and has no public or environment-controlled timeout configuration. The
exact child must be sampled alive about 12 seconds after its matching
mutation, then the actual timer transition must be recorded as
`DeadlineReached` before existing kill/reap cleanup and `Terminal`. Passing
evidence uses one opaque operation identity, `actionKind: "deadline_reached"`,
confirmed process cleanup, `operation_timed_out`, a failed first step with the
second step Not attempted, and clean run-scope residual verification; the
unique run scope remains authoritative even though the timeout copy creates no
persistent payload. No operator marker or Terminal 2 procedure is used for
this scenario; active host-push observation remains limited to cancellation,
USB-disconnect, and conditional offline cases. The `/dev/zero` and `/dev/null`
paths are private to this qualification and never enter evidence, cleanup
inventory, authored plans, or ordinary execution.
The timeout lifecycle observer may contain unrelated Probe, Predicate, or other
operation events. Timeout evidence selects exactly one `DeviceCopy`
`DeadlineReached` event at 15 seconds, then requires that selected operation's
single Spawned, MutationStarted, live LivenessSampled, DeadlineReached, and
Terminal events in raw chronological order. Missing, duplicate, contradictory,
wrong-class, wrong-deadline, or mixed-identity target events are rejected while
unrelated operation IDs are ignored.

Host-sleep qualification uses the same private `/dev/zero -> /dev/null`
stimulus with a 120-second scoped qualification deadline and an exact
deadline-clock observation seam. The operator creates `sleep-requested` while
awake; the harness proves the exact child alive, samples the exact deadline
clock, and creates the internal `sleep-ready` marker; the operator then creates
`sleep-entered` within four seconds (the final awake handoff and activeProcess
action boundary), suspends the host, and creates `wake` immediately after
resume. The post-wake sample is requested only after `wake` is observed and may
legitimately follow the owner terminal sample when the deadline became ready
during suspension. The measurement tolerance is 8,000 ms and the timer
classification derives from measured clock advancement, wall duration, and
remaining budget; `indeterminate` and `contradictory` block. The owner-recorded
`DeadlineReached` event is authoritative for whether the deadline branch won:
a timeout requires it, and completion at the deadline boundary without it is
not relabelled as a timeout. `sleepEnteredAt` means the last operator/harness
handoff immediately before physical suspension, and `wakeAt` means the first
post-resume acknowledgement; neither claims an exact OS event timestamp.
Physical host-sleep repetitions remain unqualified.

Schema-v1 compatibility is additive for historical non-timeout records: their
`timeout` object and `activeProcess.actionKind` may be absent. Every
`operation_timeout` record must carry exactly the four timeout fields with the
300,000 ms production deadline, 15,000 ms scoped qualification deadline, and
`test_only_scoped_override` source. `processCleanup` is one of `confirmed`,
`uncertain`, or `not_observed`; only a passing timeout record requires
`confirmed`. A non-null timeout `activeProcess` always requires
`actionKind: "deadline_reached"`, while a passing timeout still requires that
process evidence to be present.

Low-storage qualification requires a disposable selected device with between
4 GiB and 5,308,416 KiB free before mutation, a verified fixture-owned 1 GiB
recovery reserve, a bounded run-scoped filler capped at 4 GiB with 64 MiB of
cleanup headroom, a generated 128 MiB host fixture payload for genuine
production-path ENOSPC, and verified payload/filler/sentinel/reserve cleanup
with free-space restoration.
The evidence validator retains failed, skipped, and blocked attempts with
their truthful cleanup/residual facts, but only passing records can satisfy a
scenario contract or count toward matrix completion. The host-only delay seam
is a one-shot, thread-local regression arm for exactly one DeviceCopy process;
it cannot delay status/output polling, turn an exited child into a timeout, or
qualify physical evidence. Identity probes and other operations are unaffected,
and the arm clears on normal, timeout, panic, or parallel return.

The development UI-smoke binding/capture plumbing is implemented; manual
UI-smoke qualification remains deferred until the required compatible
host-sleep physical binding exists and the operator chooses to perform the
deferred manual work; the final five-subcase composite additionally cannot be
completed until a compatible passing host-sleep physical binding exists;
`identity_replacement` and the remaining host-sleep physical repetitions are
still required for Phase 6D closure; both UI-smoke repetitions themselves
remain missing because manual UI-smoke qualification has not been run.
`tools/phase-6d6-evidence.mjs` derives and verifies the
checked-in `docs/testing/phase-6d6/ui-binding-index.json`, which lists only
UI-contract-compatible passing physical bindings plus source and raw
evidence/trace digests; default validation is read-only and
`--regenerate-ui-binding-index` writes the index only after the base evidence
contract passes. A debug-only Tauri bridge (`phase6d6_ui_smoke.rs`) activates
only with the `real-execution` Cargo feature,
`EMUCHEF_RUN_REAL_ADB_TESTS=1`, and `EMUCHEF_PHASE_6D6_UI_SMOKE=1`; it verifies
the index self digest, source digests, exact evidence/trace raw bytes, and the
parsed run/record/trace identities itself, then projects a fixed terminal
report through the production real-execution projection and renders it in the
normal React terminal UI. React receives and sends only opaque handles plus the
UI repetition. Capture writes a canonical create-new `ui_state_capture`
artifact under `docs/testing/phase-6d6/evidence/ui/` bound to the exact backend
run/trace and trusted development-build identity. The application never
creates the operator observation or the final `ui_smoke_composite` record.
Accepted passing physical evidence exists for `cancellation_active`,
`cancellation_boundary`, `usb_disconnect_active`, `usb_disconnect_boundary`,
`device_unauthorized`, `identity_stability`, `root_revocation`, `low_storage`,
and `operation_timeout`. Only the two accepted `usb_disconnect_active` records
with `device_transport_lost` satisfy the mandatory transport UI contract;
passing `usb_disconnect_boundary` records reporting `device_disconnected`
remain accepted physical evidence but are excluded from the UI binding.
`identity_replacement` repetitions 1–2, `host_sleep_before_deadline`
repetitions 1–2, `host_sleep_after_deadline` repetitions 1–2, and
`ui_smoke_composite` repetitions 1–2 remain missing; no UI-smoke repetition has
been run or counted. `device_offline` remains conditional diagnostic evidence.

Phase 6D remains In Progress until all twelve mandatory scenarios have two clean,
sanitized, contract-valid passing repetitions, both UI-smoke repetitions pass,
and the complete automated matrix is green. `device_offline` remains supported
conditional evidence and does not contribute to the missing-repetition count.
The exact backend `clippy -D warnings` gate passes; the Tauri strict Clippy
gate now passes under both the default and `real-execution` feature sets,
resolving the lint findings that previously reproduced identically in an
isolated clean checkout at `HEAD` (`b8bf14a`), so repository-wide strict Clippy
is green. The validator is the authority for accepted and missing repetitions,
and blocked mandatory scenarios do not count as closure.

## Recipe qualification current state

Standalone RetroArch automated qualification covers the real
`app.retroarch.provision` workflow. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/retroarch/qualification-contract.json`,
and the active qualification module is
`crates/emuchef-rust-backend/src/recipe_qualification_retroarch_tests.rs`.
The qualification uses the real authored catalog through
`runtime_configuration::plan_configuration` with the
`ayaneo.konkr_pocket_fit.base` device plan, exercises the production review
projection, and executes the unchanged generated plan through
`ExecutorAdapters::with_sandbox_roots`. It explicitly qualifies the authored
first-launch/bootstrap lifecycle as bootstrap launch -> 1500 ms wait -> force-stop
followed by permission launch -> 5000 ms wait -> force-stop, including generated
plan dependency/order checks, successful deterministic execution records, and a
test-private lifecycle failure regression that preserves prior results and blocks
dependent work. It also preserves optional configuration behavior, repeated-install
skip behavior, PPSSPP verification failure semantics, and production review
coverage without live public network access or ADB. Physical and full end-to-end
qualification remain deferred.

Standalone BIOS automated qualification covers the real authored
`feature.copy_bios` workflow. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/bios/qualification-contract.json`, bound
to the raw authored recipe SHA-256
`1a3b04aa3f26720701ccbe56336d1f451d3f402c9a092be10ef80682cd9a998b`, and its
active qualification module is
`crates/emuchef-rust-backend/src/recipe_qualification_bios_tests.rs`.
Qualification uses `ayaneo.generic.base` as production capability context but
explicitly selects only `feature.copy_bios`. It proves production planning and
review, required-input rejection, recursive nested copy through normal sandbox
adapters, and authored destination-verification failure through a private test
device wrapper. No authored YAML, device-plan/profile semantics, public API,
or production executor source is changed.

Standalone ROM/content-copy automated qualification covers the real authored
`feature.copy_roms` workflow. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/roms/qualification-contract.json`, bound
to the raw authored recipe SHA-256
`956838151ed9048421e4c88d0895abe5b7f1a1998731c7dd2fbbee9cc13c2041`, and its
qualification document is `docs/product/recipe-qualification-roms.md`.
Qualification uses `ayaneo.generic.base` as production capability context but
explicitly selects only `feature.copy_roms`. It proves production planning and
review, required/default/alternate/invalid bindings, deterministic nested
`merge`, `replace`, and directory-style `sync` execution, and truthful copy
failure reporting without ADB, live network, physical hardware, or packaged
GUI work. The authored `verify: []` remains without verification-predicate
coverage. Directory-style `sync` is the authored “Mirror source” contract and
removes destination-only files; the correction is limited to the shared
fake-device directory-copy branch, while single-file, path-list, and unrelated
execution branches retain their existing behavior. Physical qualification is
deferred with cleanup authority
`not_authorized_for_recipe_qualification`.

Standalone Obtainium automated qualification covers the real authored
`app.obtainium.install` workflow. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/obtainium/qualification-contract.json`,
bound to the raw authored recipe SHA-256
`d3f96f4d6f0fa812af75b0ddc18edad9da69b7b2ceae62468c0bd3c8b645caa7`, and its
active qualification module is
`crates/emuchef-rust-backend/src/recipe_qualification_obtainium_tests.rs`.
Qualification uses `ayaneo.generic.base` only as the production planning and
capability context and explicitly selects only `app.obtainium.install`; the
device plan does not contain Obtainium and is not treated as product
provenance. The qualification covers production planning and review, authored
URL/default-cache preservation with a seeded exact cache filename,
deterministic install execution without network or ADB, package-state-driven
repeated-install skipping, and truthful install-failure semantics through a
private test-only device adapter. No authored YAML, device-plan/profile
semantics, public API, or production executor source is changed.

Physical qualification for all three standalone workflows is deferred by owner
with cleanup authority `not_authorized_for_recipe_qualification`.
Composition-level automated qualification is complete for the real
`ayaneo.konkr_pocket_fit.base` default,
which selects `app.retroarch.provision` followed by `feature.copy_bios` through
`selected_recipes: None`. Its strict source-bound contract is at
`tests/fixtures/recipe-qualification/retroarch-bios/qualification-contract.json`
and its qualification document is
`docs/product/recipe-qualification-retroarch-bios.md`. The combined result covers
production planning/review, required BIOS binding failure, deterministic sandbox
execution, repeated-run install skipping with BIOS re-execution, and failure
semantics in which BIOS copy operations precede the forced BIOS destination
verification failure, all results actually produced before the failure remain
truthful, and the BIOS step and overall run fail. The generated production plan
remains unchanged, and this makes no claim that BIOS follows the complete
RetroArch workflow. The fake filesystem does not treat `/sdcard` and
`/storage/emulated/0` as aliases. Phase 6E remains In progress for
ROM/content and other workflow qualification and physical/full end-to-end work;
Phase 6D remains In progress with all existing missing physical and UI-smoke
evidence unchanged. The automated work follows the owner's sequencing decision
and does not waive any Phase 6D requirement.
