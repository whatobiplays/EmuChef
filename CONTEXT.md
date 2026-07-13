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

`emuchef plan` accepts an authored root, device-plan id, optional input
bindings, optional explicit device context, optional ADB path/serial, an output
path, and verbose output. Supplying `--adb` or `--serial` enables live device
probing. Generated execution plans use `plan.<device-plan>.001` identifiers.

Step parameter specifications declare their accepted value sources and runtime
value types. Authored recipe refs remain recipe-local (`inputs.<id>`,
`artifacts.<id>.<field>`, and `steps.<id>[.outputs.<name>]`). Validation rejects
literal or ref namespaces that a parameter does not accept and rejects refs
whose declared runtime type is incompatible. `copy_files.dest` and
`copy_files.copy_policy` accept either literals or compatible input refs, while
`copy_files.source` remains limited to compatible input, artifact, and step
output refs. Canonical recipe YAML continues emitting direct literal values and
single-field `{ ref: ... }` mappings.

## Execution

Execution is single-threaded and dependency-aware. A failed step blocks
dependent steps while unrelated steps may continue. Skip conditions produce
skipped steps according to the execution-plan contract. Verification runs after
the step action and can fail the step.

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
and network setup. New bytes are streamed to unique same-directory partial
files, flushed, synced, and published without clobbering. `cache: none` always
transfers and uses a unique runtime path on collision. Failures are typed and
redacted, remove partial files when cleanup succeeds, and block dependent steps
without preventing unrelated work.

## Editor

`apps/config-editor` is a React/Tauri application. Tauri builds and packages the
Rust `emuchef` binary as an external sidecar and launches it with `--sidecar`.
The JSONL sidecar owns persistent document sessions.

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
does not expose planning or apply through its Tauri or JSONL command surfaces.
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
