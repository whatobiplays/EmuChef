# EmuChef Context

This is the working context document for the repo.

When behavior, schema, workflow, or device assumptions change, update this file
in the same change.

## Project State

EmuChef is a CLI-first Android handheld provisioner.

The current flow is:

1. Load authored YAML from `authored/`
2. Build a draft plan from a selected device plan and optional operations/bindings
3. Emit a normalized execution plan
4. Apply that plan through a single-threaded executor

Generated execution-plan YAML is an output artifact, not a maintained source
fixture. The repository root does not contain tracked `*.plan.yaml` examples;
current plans are generated on demand from authored YAML under `authored/`.

A manual checklist for opt-in RetroArch real-device validation lives under
`docs/manual/`. Device, network, and ADB validation remains explicit, mutating
apply is not part of normal CI, and the checklist itself is not completed
validation evidence.

The code is intentionally split into:

- `src/emuchef/io`: authored loading, validation, YAML I/O
- `src/emuchef/planner`: normalization, draft/session logic, execution-plan emission
- `src/emuchef/executor`: runtime ref resolution, ADB, shared step runtime, and shared execution helpers
- `src/emuchef/steps`: first-party built-in step plugins, step specs, planner hooks,
  step-local executor handlers, and editor-safe step metadata
- `src/emuchef/domain`: typed models and enums
- `src/emuchef_editor/core`: UI-agnostic recipe document, canonical YAML, ref indexing, validation adapters, workspace discovery, and PySide-free editor metadata
- `apps/config-editor`: primary Tauri development/editor UI for authored recipe files

The base Python package is retained temporarily for legacy/reference/developer
workflows, including the Python CLI/planner/executor reference implementation
and golden generation. A clean default install and the normal active
runtime/test path do not require, import, or launch PySide6. The legacy PySide6
editor source, Python editor API source, corresponding normal tests, optional
dependency extra, and Python GUI console script are not present.

The current Rust/Python ownership decision is recorded in
[ADR 0001](docs/adr/0001-rust-tauri-editor-runtime-ownership.md).
CLI/planner/executor/apply parity strategy is documented in
[docs/rust-cli-executor-parity.md](docs/rust-cli-executor-parity.md); this is a
documentation clarification and does not change current runtime ownership.
The Rust planner CLI output compatibility target is recorded in
[ADR 0002](docs/adr/0002-rust-planner-cli-output-compatibility.md). P8AO makes
no-backend `emuchef plan` route through Rust-owned planning using the existing
production-equivalent Rust subprocess path. During this transition, the default
Rust route requires `--rust-planner-bin <path>` and emits Python-compatible
summary/YAML output. P8BJ removes the explicit public
`--planner-backend python` route, P8BK removes fallback routing from
`_run_plan(...)` to `_run_python_plan(...)`, and P8BL deletes the private
`_run_python_plan(...)` function without deleting `src/emuchef/planner.py` or
broader CLI imports from `emuchef.planner`. P8BM removes direct
`src/emuchef/cli.py` imports from `emuchef.planner` by adding
`src/emuchef/planner_cli.py` as a transitional CLI-facing compatibility module
for still-needed Python planner symbols. P8BM does not delete
`src/emuchef/planner.py` or remove Python draft/session/profile helper
behavior. Rust-native JSON requires a separate accepted structured-output
option.
P8BN removes stale Rust readiness and launcher-smoke assumptions that explicit
`--planner-backend python` remains available. It removes
`python_planner_deletion_not_ready` as a Rust planner cutover readiness blocker
and stops requiring standalone `python` as a planner-backend token in the
readiness gate; the gate may still require `python-compatible` output wording.
P8BN does not delete `src/emuchef/planner.py`, remove Python
draft/session/profile helper behavior, or change plan routing, draft behavior,
executor/apply, Rust planner behavior, packaging, Tauri/config-editor, Rust
backend code, or `.local` evidence. The Python planner deletion preflight
remains blocked until remaining docs/context detector surfaces are retired.
P8AR centralizes the current explicit `--rust-planner-bin <path>` validation in
an internal Python CLI resolver helper. That helper remains explicit-path-only:
no packaged lookup, Cargo fallback, arbitrary `PATH` search,
environment-variable lookup, repo-local guessing, shell execution, or silent
Python fallback is implemented.
P8AS records the packaged planner binary location contract in
[docs/rust-packaged-planner-binary-location.md](docs/rust-packaged-planner-binary-location.md).
P8AS is documentation-only: no packaged lookup is implemented,
`--rust-planner-bin <path>` remains required, no Cargo fallback, `PATH` search,
env-var lookup,
repo-local guessing, or silent Python fallback is implemented, existing Tauri
sidecar packaging is not automatically planner binary packaging, and packaged
release readiness remains future work. P8BJ later removes the explicit public
`--planner-backend python` route.
P8AT adds an inert packaged resolver placeholder in the Python CLI. The
placeholder currently returns `None`; no packaged lookup is implemented,
`--rust-planner-bin <path>` remains required, no Cargo fallback, `PATH` search,
env-var lookup,
repo-local guessing, or silent Python fallback is implemented, and packaged
release readiness remains future work.
P8AU adds tests for this packaged candidate seam only. The actual helper still
returns `None`, no packaged lookup is implemented, `--rust-planner-bin <path>`
remains required for real Rust routes, no Cargo fallback, `PATH` search,
env-var lookup, repo-local guessing, or silent Python fallback is implemented,
and packaged release readiness remains future work.
P8AV tightens resolver error-contract tests only. It does not change runtime
behavior or implement packaged lookup; `--rust-planner-bin <path>` remains
required for real Rust routes, and packaged release readiness remains future
work.
P8AW records the packaged resolver implementation design in
[docs/rust-packaged-planner-resolver-implementation-design.md](docs/rust-packaged-planner-resolver-implementation-design.md).
P8AW is documentation-only. The proposed future mechanism is a
package/runtime-provided absolute path, once a later implementation defines the
integration point that supplies it. No packaged lookup is implemented,
`_packaged_rust_planner_bin_candidate(args)` still returns `None`,
`--rust-planner-bin <path>` remains required for real Rust routes, no Cargo
fallback, `PATH` search, env-var lookup, repo-local guessing, or silent Python
fallback exists, existing
Tauri sidecar/resource paths are not planner binary paths unless later
designated by a planner-specific decision, and packaged release readiness
remains future work.
P8AX is documentation-only. [ADR 0005](docs/adr/0005-packaged-rust-planner-path-integration.md)
accepts a launcher-supplied absolute path through the existing
`--rust-planner-bin <path>` option as the first packaged planner integration
path. No packaged lookup is implemented, no new CLI flag is added, no env-var
lookup is added, `_packaged_rust_planner_bin_candidate(args)` still returns
`None`, `--rust-planner-bin` remains required unless a launcher supplies it,
and packaged release readiness remains future work.
P8AY is documentation-only. [docs/rust-launcher-injected-planner-smoke-contract.md](docs/rust-launcher-injected-planner-smoke-contract.md)
defines smoke evidence for launcher-injected
`--rust-planner-bin <path>`. P8AY did not implement a smoke tool, packaged
lookup, or readiness-blocker clearing.
`_packaged_rust_planner_bin_candidate(args)` still returns `None`,
`--rust-planner-bin` remains required unless a launcher supplies it, and
packaged release readiness remains future work.
P8AZ is documentation-only. [docs/rust-launcher-injected-planner-smoke-report-schema.md](docs/rust-launcher-injected-planner-smoke-report-schema.md)
defines the `rust_launcher_injected_planner_smoke` report schema with
`schema_version: 1`. P8AZ did not implement a smoke tool, readiness intake,
readiness-blocker clearing, resolver changes, or packaging behavior changes.
Reports of this kind must not record full local paths, serials, commands,
stdout/stderr, or environment variables. Packaged release readiness remains
future work.
P8BA implements [tools/smoke_launcher_injected_planner.py](tools/smoke_launcher_injected_planner.py)
and focused tests for the launcher-injected planner smoke report only. The
report kind remains `rust_launcher_injected_planner_smoke` and
`schema_version` remains `1`. P8BA does not add readiness intake, clear a
readiness blocker, change CLI resolver behavior, implement packaged lookup, or
change packaged release readiness. Packaged release readiness remains future
work.
P8BB is documentation-only. [docs/rust-launcher-injected-planner-readiness-intake-design.md](docs/rust-launcher-injected-planner-readiness-intake-design.md)
defines readiness intake rules for P8BA reports. P8BC implements supplied-report
readiness intake for `rust_launcher_injected_planner_smoke` reports with
`schema_version: 1`. Accepted P8BC reports may satisfy only the packaged
launcher-injection evidence item. P8BC does not change the smoke tool, CLI
resolver behavior, packaged lookup, runtime behavior, Tauri/config-editor code,
packaging scripts, Rust backend code, executor/apply behavior, Python planner
deletion preflight, or broader packaged release readiness. Top-level readiness
remains blocked while executor/apply, packaged release, and unsatisfied
evidence-dependent blockers remain blocked.
P8BD is documentation-only. [docs/rust-packaged-readiness-blocker-taxonomy.md](docs/rust-packaged-readiness-blocker-taxonomy.md)
separates packaged launcher-injection evidence from packaged release readiness,
executor/apply readiness, the Python planner deletion preflight, and top-level
readiness. Accepted P8BC evidence can satisfy only the packaged
launcher-injection evidence item; packaged release readiness remains future
work.
P8BE only improves readiness report explanatory output. It does not change
evidence acceptance rules, blocker computation, smoke tooling, CLI/runtime
behavior, packaging, Rust backend code, executor/apply behavior, or `.local`
evidence. Top-level readiness remains blocked until separate blockers are
resolved.
P8BF adds a checked-in static readiness report fixture for the current
no-evidence report shape. It does not change readiness semantics, evidence
acceptance rules, smoke tooling, CLI/runtime behavior, packaging, Rust backend
code, executor/apply behavior, or `.local` evidence. Fixture maintenance is
documented in
[docs/rust-readiness-fixture-maintenance.md](docs/rust-readiness-fixture-maintenance.md).
P8BH is documentation-only. [docs/rust-planner-readiness-docs-index.md](docs/rust-planner-readiness-docs-index.md)
is the navigation index for Rust planner readiness and cutover evidence docs. It
does not change readiness semantics, evidence acceptance rules, smoke tooling,
CLI/runtime behavior, packaging, Rust backend behavior, executor/apply
behavior, or `.local` evidence.
The future Rust planner real-device context ownership model is recorded in
[ADR 0003](docs/adr/0003-rust-real-device-context-ownership.md). For future
default Rust planner cutover, Rust should own real-device probing and
detected-device profile mismatch warning parity. Python remains the explicit
fallback/reference planner owner for deletion readiness.
P8M is the accepted ADR slice for this ownership decision. P8N adds a
crate-local Rust probe abstraction, fake probe, and tests for layering detected
facts over profile-derived planner context. P8O adds fake/test-backed
planner-input construction that applies detected facts over
synthetic/profile-derived context. P8P adds a pure/test-backed Rust foundation
for detected-device profile mismatch warnings by comparing supplied detected
facts with authored profile match criteria. P8P mirrors the current Python
mismatch criteria for manufacturer, brand, model regex, and Android minimum
version; authored Android maximum values are parsed but not evaluated because
the current Python warning path does not evaluate them. P8Q composes the
fake/test-backed detected-facts path into crate-private `PlanningResult`
construction and appends detected-device profile mismatch warnings for that
test path only. P8Q does not implement live ADB/device probing, route wiring,
normal runtime warning emission, or default planner cutover behavior. P8R adds
an explicit local detected-facts fixture harness to the dev-only Rust shadow
binary with `--detected-facts-json <path>`. The fixture path reads strict local
JSON matching `DetectedDeviceFacts`, runs the P8Q planning-result composition
path, and emits the usual shadow `PlanningResult` JSON. P8R does not implement
live ADB/device probing, wire route-level detection into `rust-shadow` or
`rust-experimental`, or change executor/apply, Tauri/protocol, normal runtime
checks, readiness gate blockers, or default planner behavior. P8T exposes only
an explicit Python wrapper flag, `--rust-detected-facts-json <path>`, for
`emuchef plan --planner-backend rust-experimental`. Python forwards the exact
argparse string as `--detected-facts-json <path>` to the supplied Rust shadow
binary and does not open, stat, expand, normalize, parse, or schema-validate the
fixture. P8AO keeps this wrapper flag explicit-only: no-backend default Rust
routing and `rust-shadow` reject it before ADB resolution,
planner/session construction, device probing, or subprocess execution; the raw
Rust `--detected-facts-json` flag remains unrecognized by Python CLI routes.
P8S adds optional/manual smoke evidence
for that Rust shadow-binary fixture harness through
`tools/smoke_rust_detected_facts_fixture.py`. The smoke invokes a supplied
`emuchef-plan-shadow` binary directly with temporary fixture files, emits
deterministic JSON, and is not wired into Python CLI routes, normal runtime
checks, the static readiness gate, live probing, executor/apply, Tauri/protocol,
or Python planner deletion behavior.
P8U adds optional/manual smoke evidence for the Python `rust-experimental`
fixture-forwarding route through
`tools/smoke_rust_experimental_detected_facts_fixture.py`. The smoke writes
temporary detected-facts fixtures, invokes `python3 -m emuchef plan
--planner-backend rust-experimental` with `--rust-detected-facts-json <path>`,
requires Python-compatible summary stdout, and treats raw Rust JSON stdout as a
route-smoke failure. The mismatching output-file case is checked with stdlib
text validation for `device_profile_mismatch`; deterministic reports omit temp
paths, output paths, fixture paths, full stdout/stderr, and environment data.
P8U does not implement live ADB/device probing, expose fixture forwarding
through no-backend default planning or `rust-shadow`, alter direct Rust fixture
smoke, change readiness gate blocker IDs, wire the smoke into normal runtime
checks, modify executor/apply or Tauri/protocol behavior, or make Python planner
deletion ready.
P8V adds a crate-local Rust ADB probe foundation for future planner probing
ownership. `crates/emuchef-rust-backend/src/device_probe.rs` can model the
future `adb [-s SERIAL] shell getprop` command as an argv vector and can parse
supplied bracketed `getprop` text for `ro.product.manufacturer`,
`ro.product.brand`, `ro.product.model`, `ro.build.version.release`, and
`ro.build.version.sdk` into `DetectedDeviceFacts`. This foundation is pure and
test-backed only: it does not execute ADB, start subprocesses, read environment
variables, access the filesystem, access the network, probe real devices, or wire
detected facts into `rust-shadow`, `rust-experimental`, the Python CLI,
Tauri/protocol, executor/apply, smoke runners, normal runtime checks, or the
static readiness gate. P8AO later makes no-backend `emuchef plan` Rust-owned
through the production-equivalent Rust route; P8BJ later removes the explicit
Python fallback route.
P8W adds a crate-local live ADB probe adapter foundation on top of that command
model and parser. `AdbDeviceProbe` executes the modeled argv only through an
injectable command runner, `ProcessCommandRunner` uses direct argv process
execution without a platform shell, and normal tests use fake runners rather
than ADB. Stable probe errors do not include raw stderr, OS errors, paths,
durations, process ids, serial values, or other host-specific details. P8X wires
that adapter only into the direct dev-only Rust shadow binary through explicit
`--probe-adb-getprop`, optional `--adb-path <path>`, and optional
`--serial <serial>` flags. That live mode is separate from
`--detected-facts-json <path>` fixture mode; both are detected-facts sources and
are mutually exclusive. P8X does not expose or forward live probing through
`rust-shadow`, `rust-experimental`, the Python CLI, Tauri/protocol,
executor/apply, smoke runners, normal runtime checks, or the static readiness
gate. P8AO later makes no-backend `emuchef plan` Rust-owned through the
production-equivalent Rust route; P8BJ later removes the explicit Python
fallback route.
Fixture-based P8R/P8S/P8T/P8U evidence remains separate, and
`real_device_probing_not_cut_over` plus
`detected_device_profile_mismatch_warning_not_cut_over` remain default-cutover
blockers until production route-level probing and mismatch-warning evidence
exist.
P8Y adds optional/manual smoke evidence for the direct P8X Rust shadow live mode
through `tools/smoke_rust_shadow_live_adb_probe.py`. The smoke requires explicit
`--rust-planner-bin`, `--adb-path`, and `--serial`, invokes only the supplied
Rust shadow binary with `--probe-adb-getprop`, and emits deterministic JSON with
scrubbed command metadata. It does not discover devices, run `adb devices`,
invoke Cargo, call Python CLI routes, reuse fixture or matrix smoke tooling,
write artifacts, mutate devices beyond `adb shell getprop`, participate in
normal checks, or count as readiness-gate executed evidence. P8Z adds explicit
Python CLI forwarding for that live-probe intent only through `emuchef plan
--planner-backend rust-experimental`. The Python wrapper flags are
`--rust-probe-adb-getprop`, `--rust-adb-path <path>`, and
`--rust-serial <serial>`; Python forwards the exact argparse strings to the
supplied Rust shadow binary as `--probe-adb-getprop`, `--adb-path <path>`, and
`--serial <serial>`. Python does not invoke ADB, discover devices, run
`adb devices`, parse `getprop`, or validate, normalize, expand, or stat the ADB
path or serial. P8AO keeps those wrapper flags explicit-only: no-backend default
Rust routing and `rust-shadow` reject them before ADB resolution,
planner/session construction, device probing, or subprocess execution.
`--rust-detected-facts-json <path>` fixture forwarding and
live-probe forwarding are mutually exclusive detected-facts sources. A
`device_profile_mismatch` warning is acceptable route evidence for the P8Y smoke
when the selected device intentionally does not match the authored plan. P8Z is
not default planner cutover, not production route-level probing parity, not
readiness-gate executed evidence, and not Python planner deletion. The same
`real_device_probing_not_cut_over` and
`detected_device_profile_mismatch_warning_not_cut_over` blockers remain blocked.
P8AA adds optional/manual smoke evidence for the P8Z Python
`rust-experimental` live-probe forwarding route through
`tools/smoke_rust_experimental_live_adb_probe.py`. The smoke invokes
`python3 -m emuchef plan --planner-backend rust-experimental` with
`--rust-probe-adb-getprop`, `--rust-adb-path <path>`, and
`--rust-serial <serial>`, expects Python-compatible route output instead of raw
Rust JSON, and emits deterministic JSON with scrubbed command metadata. It does
not call the Rust shadow binary directly, discover devices, run `adb devices`,
invoke Cargo, inspect, normalize, expand, or stat the supplied ADB path or
serial, participate in normal checks, or count as readiness-gate executed
evidence. `device_profile_mismatch` is acceptable route evidence for this smoke
when the selected live device intentionally does not match the authored plan.
P8AA is not default planner cutover, not production route-level probing parity,
and not Python planner deletion. The `real_device_probing_not_cut_over` and
`detected_device_profile_mismatch_warning_not_cut_over` blockers remain blocked.
`docs/rust-live-probe-evidence-and-cutover-gap.md` is the consolidated P8X-P8AA
live-probe evidence, default-route, production-route, and readiness-gate gap
summary.
ADR 0004 records that future default-route live probing should be Rust-owned
and that P8X-P8AA migration evidence does not clear the default-route probing or
mismatch-warning blockers.
P8AD records the future default-route probe request/response shape in
`docs/rust-default-route-probe-request-response.md`; it does not implement
probing or clear readiness blockers.
P8AE records the evidence bar for a future production-equivalent live probe
smoke in `docs/rust-production-equivalent-live-probe-smoke.md`; it does not
implement smoke tooling or clear readiness blockers.
P8AF records the evidence bar for future default-route mismatch-warning parity
in `docs/rust-default-route-mismatch-warning-parity.md`; it does not implement
warning parity or clear readiness blockers.
P8AG records the implementation plan for a future explicit
production-equivalent planner route in
`docs/rust-production-equivalent-route-implementation-plan.md`. P8AI wires the
backend name `rust-production-equivalent` as an explicit non-default Rust
subprocess route backed by a supplied Rust shadow binary. It always uses
Python-compatible output and can forward Rust-owned detected-facts fixture or
live-probe wrapper inputs. It does not add readiness-gate manual evidence and
does not clear readiness blockers.
P8AJ adds optional/manual smoke tooling for the explicit
`rust-production-equivalent` live-probe route through
`tools/smoke_rust_production_equivalent_live_adb_probe.py`. The smoke invokes
`python -m emuchef plan --planner-backend rust-production-equivalent` with
`--rust-probe-adb-getprop`, `--rust-adb-path <path>`, and
`--rust-serial <serial>`, requires Python-compatible output instead of raw Rust
JSON, and emits deterministic JSON with scrubbed inputs including
`live_probe_requested: true`. It does not call the supplied Rust binary
directly, discover devices, run `adb devices`, inspect, normalize, expand, or
stat the supplied ADB path or serial, participate in normal checks, or count as
readiness-gate executed evidence. The tool can produce production-equivalent
live probe evidence when run manually with real device inputs, but tool
existence alone does not clear `real_device_probing_not_cut_over`.
`detected_device_profile_mismatch_warning_not_cut_over` remains separate for
P8AK evidence, and both blockers remain blocked after P8AJ.
P8AK adds optional/manual smoke tooling for the explicit
`rust-production-equivalent` fixture-backed mismatch-warning route through
`tools/smoke_rust_production_equivalent_mismatch_warning.py`. The smoke creates
temporary detected-facts JSON fixtures for `ayaneo.pocket_s_mini.base`, invokes
`python -m emuchef plan --planner-backend rust-production-equivalent` with
`--rust-detected-facts-json <path>`, requires Python-compatible output instead
of raw Rust JSON, and checks matched, manufacturer-mismatched, model-mismatched,
Android-minimum-mismatched, and Android-minimum-matching cases. It does not run
live probing, invoke the supplied Rust binary directly, discover devices, run
`adb devices`, participate in normal checks, or count as readiness-gate executed
evidence. The tool can produce evidence for
`detected_device_profile_mismatch_warning_not_cut_over` when run manually, but
tool existence alone does not clear the blocker. P8AJ and P8AK cover separate
evidence bars, and both blockers remain blocked by tool existence alone.
P8AL adds supplied-report evidence intake to
`tools/check_rust_planner_cutover_readiness.py` for manually saved P8AJ and P8AK
JSON reports. The gate reads only report paths explicitly supplied through
`--p8aj-live-probe-report <path>` and
`--p8ak-mismatch-warning-report <path>`. It does not run smoke tools, ADB, Cargo,
npm, Tauri/protocol checks, executor/apply, network checks, or planner runtime
paths. Accepted reports move only the relevant blocker entry to
`evidence_accepted`; they do not clear executor/apply, packaged release, or
unsatisfied evidence-dependent readiness, and the top-level readiness status
remains `blocked`.
P8AM records the default-backend cutover contract in
`docs/rust-default-planner-cutover-contract.md`. P8AM performs no cutover. P8AN
is evidence preflight only and verifies accepted P8AJ/P8AK reports without
editing repo files. P8AO makes no-backend `emuchef plan` route through the
existing production-equivalent Rust subprocess path. `rust-production-equivalent`
also remains available as an explicit backend. P8AP updates the static readiness
gate for this current state by preserving `default_cli_backend_still_python`
with status `resolved`. Top-level readiness remains `blocked` because
executor/apply, packaged release, and unsatisfied evidence-dependent blockers
remain unresolved. The Python planner deletion preflight remains separate future
work while docs/context surfaces are present.
P8BI adds `tools/check_python_planner_deletion_preflight.py` and
`docs/python-planner-deletion-preflight.md` as a static preflight for the Python
planner deletion sequence. The preflight reads checked-in source and docs text
only, emits deterministic JSON, and reports `blocked` while CLI runtime paths,
CLI imports, readiness blockers, fallback assertions, or docs/context
references still keep Python planner deletion incomplete. P8BI does not delete
Python planner code, change runtime behavior, run smoke tools, read `.local`,
require Rust binaries, or clear readiness blockers.
P8BJ removes the explicit public `--planner-backend python` route from
`emuchef plan`. It does not delete Python planner code, remove
`_run_python_plan`, remove CLI imports from `emuchef.planner`, or change draft
behavior, executor/apply, Rust planner behavior, smoke tooling, readiness
validators, packaging, Tauri/config-editor behavior, or `.local` evidence. The
P8BI preflight no longer reports `cli_explicit_python_backend` or
`test_cli_explicit_python_backend_behavior` for the real repo, but it still
reports `blocked` while other Python planner deletion surfaces remain.
P8BK removes `_run_plan(...)` fallback routing to `_run_python_plan(...)`.
It does not delete `_run_python_plan(...)`, delete Python planner code, remove
CLI imports from `emuchef.planner`, or change draft behavior, executor/apply,
Rust planner behavior, smoke tooling, readiness validators, packaging,
Tauri/config-editor, Rust backend code, or `.local` evidence. The P8BI
preflight no longer reports `cli_run_plan_routes_to_python_plan` for the real
repo, but it still reports `blocked` while other Python planner deletion
surfaces remain.
P8BL deletes the private `_run_python_plan(...)` function and removes the
transitional direct tests of that private function. It does not delete
`src/emuchef/planner.py`, remove broader CLI imports from `emuchef.planner`, or
change draft behavior, executor/apply, Rust planner behavior, smoke tooling,
readiness validators, packaging, Tauri/config-editor, Rust backend code, or
`.local` evidence. The P8BI preflight no longer reports
`cli_run_python_plan_function` for the real repo, but it still reports
`blocked` while other Python planner deletion surfaces remain.
P8BM removes direct `src/emuchef/cli.py` imports from `emuchef.planner` by
adding `src/emuchef/planner_cli.py` as a transitional CLI-facing compatibility
module for still-needed Python planner symbols. P8BM does not delete
`src/emuchef/planner.py`, remove Python draft/session/profile helper behavior,
or change plan routing, draft behavior, executor/apply, Rust planner behavior,
smoke tooling, readiness validators, packaging, Tauri/config-editor, Rust
backend code, or `.local` evidence. The P8BI preflight no longer reports
`cli_imports_emuchef_planner` for the real repo, but it still reports `blocked`
while non-runtime readiness/test/docs/context surfaces remain.
P8AQ records the future default-route Rust planner binary resolution design in
`docs/rust-default-planner-binary-resolution.md`. P8AQ is documentation-only:
`--rust-planner-bin` remains required, no packaged binary lookup, Cargo
fallback, arbitrary `PATH` search, host-path guessing, or silent Python fallback
is implemented, and executor/apply, Python planner deletion, and packaged
release readiness remain unresolved. P8BJ later removes the explicit public
`--planner-backend python` route.
P8AR adds only a behavior-preserving internal explicit-path resolver foundation
for `--rust-planner-bin <path>` validation. It does not add packaged lookup,
Cargo fallback, arbitrary `PATH` search, env-var lookup, repo-local guessing,
shell execution, or silent Python fallback. P8AS records the future packaged
planner binary location contract in
`docs/rust-packaged-planner-binary-location.md`. P8AS is documentation-only:
no packaged lookup is implemented, `--rust-planner-bin <path>` remains required,
existing Tauri sidecar packaging is not automatically planner binary packaging,
and packaged release readiness remains future work.
P8AW records the packaged resolver implementation design in
`docs/rust-packaged-planner-resolver-implementation-design.md`. The proposed
future mechanism is a package/runtime-provided absolute path, once a later
implementation defines the integration point that supplies it. P8AW is
documentation-only: no packaged lookup is implemented, the inert helper still
returns `None`, `--rust-planner-bin <path>` remains required for real Rust
routes, no Cargo fallback, `PATH` search, env-var lookup, repo-local guessing,
or silent Python fallback exists, existing Tauri sidecar/resource paths are not
planner binary paths unless later designated by a planner-specific decision, and
packaged release readiness remains future work.
P8AY records the smoke evidence contract for launcher-injected
`--rust-planner-bin <path>` in
`docs/rust-launcher-injected-planner-smoke-contract.md`. P8AY is
documentation-only: it did not implement a smoke tool, packaged lookup, or
readiness-blocker clearing. The inert helper still returns
`None`, `--rust-planner-bin <path>` remains required for real Rust routes
unless a launcher supplies it, and packaged release readiness remains future
work.
P8AZ records the `rust_launcher_injected_planner_smoke` report schema in
`docs/rust-launcher-injected-planner-smoke-report-schema.md` with
`schema_version: 1`. P8AZ is documentation-only: it did not implement a smoke
tool, readiness intake, readiness-blocker clearing, runtime behavior changes,
or packaging behavior changes. Reports of this kind must not record full local
paths, serials, commands, stdout/stderr, or environment variables. Packaged
release readiness remains future work.
P8BA implements the launcher-injected planner smoke tool at
`tools/smoke_launcher_injected_planner.py` and its focused tests only. The tool
emits `kind: rust_launcher_injected_planner_smoke` reports with
`schema_version: 1`, uses an explicit launcher-supplied `--rust-planner-bin`
path, treats an argv0-observing wrapper as the launcher-supplied subprocess
entrypoint, and originally retained a static CLI-help check for explicit Python
fallback availability. P8BN removes that stale CLI-help check. P8BA does not add
readiness intake, clear a readiness blocker, change CLI resolver behavior,
implement packaged lookup, change runtime or packaging behavior, or write
`.local` evidence. Packaged release readiness remains future work.
P8BC implements readiness intake for explicitly supplied
`rust_launcher_injected_planner_smoke` reports with `schema_version: 1`. The
readiness gate validates the report JSON only and does not execute the P8BA
smoke tool, import smoke tool code, execute planner code, probe devices, or
read `.local` evidence implicitly. Accepted P8BC evidence may mark only the
packaged launcher-injection evidence item as accepted. Executor/apply readiness,
broader packaged release readiness, unsatisfied evidence-dependent blockers, and
top-level readiness remain blocked until later work intentionally clears them.
P8BD documents that `evidence_accepted` means a supplied evidence item passed
validation and does not mean the broader release blocker is resolved unless the
blocker is specifically scoped to that evidence item.

## Current Authored Model

Recipes now use:

- `inputs:` as a map keyed by input id
- `artifacts:` as a map keyed by artifact id
- `artifact_groups:` as a map keyed by group id
- ordered `steps:`

Author refs stay recipe-local in YAML:

- `inputs.<id>`
- `artifacts.<id>.<field>`
- `steps.<id>`
- `steps.<id>.outputs.<field>`

Planner-internal execution-plan refs are normalized and namespaced, but that
does not leak into authored YAML.

Literal params are authored directly. Only refs use `{ ref: ... }`.

Permission intent is authored on `grant_permissions` steps, not on the recipe
root. A recipe with top-level `permissions:` is invalid, even if it also
contains `grant_permissions.params`.

`grant_permissions.params` supports:

- `runtime`: runtime permission grants
- `appops`: app-op grants
- `policy`: local failure policy for that step's applicable actions

Multiple `grant_permissions` steps may appear in one recipe. Each step grants
only the actions declared in its own params, policy applies only to that local
action set, and there is no implicit merge across grant steps. Empty grant params
are a valid clean no-op.

The Rust planner's internal permission-intent helper reads selected
`grant_permissions` step params as the declaration source. It constructs
structured runtime-permission and app-op intent for crate-local tests without
adding a serialized `permission_plan` field to execution plans.

The desktop editor uses the same authored recipe model and in-process validation path as the CLI-facing authored loader.
The editor remains in authored-ref space:

- it shows recipe-local refs
- it emits recipe-local refs
- it does not expose planner-normalized refs or execution-style ids

The current editor scope is recipe-authoring only. It edits:

- Overview
- Inputs
- Artifacts
- Artifact Groups
- Steps

## Current Editor Protocol

The Rust backend is the active editor JSON protocol owner. It provides one-shot
JSON requests for stateless protocol operations and a persistent JSON Lines
sidecar for document sessions. The Python `emuchef_editor.api` package is not
present in active source, normal tests, default Python distributions, or Tauri
runtime paths.

Every API response uses one of these envelopes:

- `{"ok": true, "result": {...}}`
- `{"ok": false, "error": {"code": "...", "message": "...", "details": {...}}}`

Failure responses include diagnostic debug details only when a request sets
`debug: true`. Frontends treat debug details as diagnostics, not behavior
contracts.

`RecipeDocumentDto` contains document state: document id, path, authored root,
dirty state, undo/redo availability, recipe DTO, current canonical YAML,
diagnostics, and ref index. It does not contain step specs.

The Rust sidecar stores `authoredRoot` as open-document session context. The
`setDocumentAuthoredRoot` sidecar request updates that context for an existing
document, refreshes derived diagnostics and DTO metadata, and returns a full
document DTO without reloading recipe YAML from disk. `authoredRoot: null`
clears the stored context for that document. Non-null authored roots follow the
same normalization rules used when opening a recipe.

`RecipeDto` exposes current authored recipe sections for overview, dependencies,
provided features, inputs, artifacts, artifact groups, and steps. Top-level
permissions are invalid authored data and are absent from `RecipeDto`.
Permission authoring appears as normal step data on `grant_permissions` steps.

Step specs are returned only by the `listStepSpecs` API request. Step spec data
comes from the built-in step registry and includes editor-safe labels, supported
status, outputs, param ordering, defaults, and typed ref filter hints where the
registry exposes them.

The Rust backend owns the static StepSpec DTO metadata returned by
`listStepSpecs` in `crates/emuchef-rust-backend/src/step_specs.rs`. Normal Rust,
Tauri, and default package verification do not consume a Python-generated
StepSpec fixture or invoke Python to serve StepSpec metadata.

Fixture and golden ownership is classified in
`docs/python-fixture-golden-ownership.md`. Normal Rust/Tauri active checks may
consume checked-in fixtures and goldens, but they do not invoke Python fixture
or golden regeneration. Remaining Python regeneration commands are
dev-only/reference-only and are not setup, runtime, packaging, or Rust/Tauri
verification prerequisites.

The Rust planner parity boundary is documented in
`docs/rust-planner-parity-boundary.md`. Rust planner cutover readiness and
Python planner deletion blockers are classified in
`docs/rust-planner-cutover-readiness.md`. Python remains the CLI/reference
owner for planner behavior. Rust planner coverage remains crate-internal and
fixture-scoped. The Rust planner tests include an intentional Phase 6M/6N
fixture inventory/parsing guard that consumes checked-in authored fixtures and
planner goldens only, plus focused coverage for selected emitted step-output
ref validation, shorthand step-ref rewriting, selected/emitted step dependency
validation, internal permission-intent construction from selected
`grant_permissions` step params, execution-plan DTO shape/normalization for the
supported fixture-scoped surface, and emitted-step param contract validation for
`copy_files`, `extract_artifacts`, `extract_archive`, `install_apk`, `wait`, and
`grant_permissions`. Rust planner tests also cover the checked-in
`authored/recipes` corpus through private planner inputs: recipe discovery is
explicit, every recipe parses through the Rust domain model, supported
manually-selected synthetic contexts emit execution plans, required unbound
inputs produce classified `binding_missing` errors, and RetroArch's optional
config step is pruned when unbound and included when bound. This authored-corpus
coverage also pins private Rust selected-recipe expansion semantics for direct
`recipe_dependencies`: dependencies appear before dependents, sibling
dependencies preserve authored order, explicit selected recipe closures expand
in selected-ref order, and duplicate dependencies are suppressed without moving
their first occurrence. Unknown selected recipe refs and unknown dependency refs
produce classified error results with source-specific diagnostic context, and
dependency cycles produce classified error results with cycle context. The
current checked-in recipe corpus has no non-empty `recipe_dependencies`
metadata, so current checked-in corpus and device-plan selected refs match
expanded refs as current-state evidence only. Future non-empty checked-in recipe
dependencies require intentional test and documentation updates. This
authored-corpus coverage also has private checked-in device-plan/profile
ingestion for Rust planner inputs: device plan/profile inventory is path/id
explicit, selected recipe refs are loaded from required
`selected_by_default: true` entries in authored order, profile
`capability_defaults` and `device_tags` are mapped into planner input, and
planner-only synthetic `DeviceContext` values are derived
from profile fields. The synthetic context uses the first declared
`match.manufacturer_contains` value or `profile:<profile_id>`, the profile name
or `profile:<profile_id>`, `match.android_version.min` or `0`, and no API level
unless a future authored field explicitly supplies one. Private Rust
device-plan ingestion parses checked-in `defaults.show_advanced_steps` and
`overrides.config_variants` as inactive metadata; current checked-in device
plans do not contain ref-shaped override binding keys. In temporary
authored-root planner tests, private Rust ingestion accepts only strict
`<recipe_ref>/<input_id>` top-level `device_plan.overrides` keys as planner
input bindings, inserts those bindings in YAML order, and applies explicit test
bindings afterward so explicit bindings take precedence. Private Rust repo-plan
tests run checked-in device-plan/profile contexts through
`PlannerInput::from_authored_device_plan(...)` and the internal Rust planner.
`ayaneo.konkr_pocket_fit.base` and `ayaneo.pocket_s_mini.base` currently
produce private Rust planner success results without required external
bindings; optional `app.retroarch.provision/retroarch_cfg` remains optional,
and a planner-only temporary `.cfg` binding includes `seed_retroarch_cfg`.
`ayaneo.generic.base`, `ayaneo.pocket_air_mini.base`, and
`ayaneo.pocket_s2.base` also produce private Rust planner success results when
their required planner-only BIOS directory or XaniteOG `.apk` bindings are
supplied. For profiles where `app_data_write: false` prevents RetroArch
app-data copy steps from being available, Rust selection-time pruning removes
dependent steps such as `launch_retroarch` instead of reporting
`unknown_step_dependency` for authored step ids that exist but are unavailable.
Device-plan defaults as bindings, config variant selection, broader Python
override key forms such as `inputs.<id>`, profile matching against detected
facts, real device facts, CLI operation replay, executor/apply behavior, Python
invocation, ADB, remote URL resolution, downloads, artifact materialization,
and real APK/BIOS payload validation remain outside this Rust planner slice.
DTO shape coverage asserts
successful and error
`PlanningResult`/`ExecutionPlan` key sets, selected normalized params, semantic
list ordering, and the absence of a serialized `permission_plan` field without
turning arbitrary JSON object key order into a public contract. Focused Rust
planner param contract diagnostics include recipe, step, step type, param,
expected value/mode, and actual value context. Unknown param rejection in this
planner parity slice is limited to those focused step types, and deterministic
multi-error ordering is covered only for an existing focused unknown-param path.
The crate also provides a dev-only `emuchef-plan-shadow` Cargo binary for
manual Rust planner inspection. The shadow binary builds a private
`PlanningResult` from an explicit authored root and checked-in device plan,
emits deterministic pretty JSON to stdout for planner success and planner error
results, and exits non-zero for planner error results. Argument errors and
authored-root/device-plan load errors write stable process text to stderr with
no stdout JSON. Shadow `--manufacturer`, `--model`, and `--android-version`
values override the synthetic/profile-derived planner `DeviceContext` for that
invocation. Repeated `--device-tag` values replace profile-derived tags exactly
in the supplied order; when no explicit tags are supplied, profile-derived tags
remain unchanged. Shadow `--bind` values are parsed as strings; repeated binds
for the same `<recipe_ref>/<input_id>` are grouped into a string array to match
the current Python CLI parser. The shadow binary does not execute/apply plans,
access the network, materialize artifacts, expose Tauri or sidecar protocol
commands, or replace the Python planner CLI. Live device probing is available
only when the direct shadow binary is invoked with explicit
`--probe-adb-getprop`; no probing occurs when that flag is omitted. The shadow
binary also accepts a dev/test-only `--detected-facts-json <path>` fixture path.
That mode reads a local strict JSON `DetectedDeviceFacts` fixture, applies detected
facts over the profile-derived planner context, evaluates mismatch warnings
from those fixture facts, then applies any explicit `--manufacturer`, `--model`,
`--android-version`, or repeated `--device-tag` overrides to the emitted
`execution_plan.device_context`. The direct Rust fixture smoke remains
`tools/smoke_rust_detected_facts_fixture.py`; it creates temporary matching,
mismatching, and explicit-context override fixture cases and reports only stable
classifications, not temp paths or full process output.

The direct Rust shadow live-probe mode builds `adb [-s SERIAL] shell getprop`
from `AdbProbeConfig`, executes it through `AdbDeviceProbe<ProcessCommandRunner>`,
parses stdout into `DetectedDeviceFacts`, and routes those facts through the same
detected-facts planning-result composition path used by fixture mode. Probe
launch failures and non-zero ADB exits are process errors with stable
`adb_probe_unavailable` or `adb_probe_failed` stderr classifications and no
stdout JSON. `--adb-path` and `--serial` are usage errors unless
`--probe-adb-getprop` is present.

`tools/smoke_rust_shadow_live_adb_probe.py` is the optional/manual smoke for
that direct Rust shadow live-probe mode. It requires an already-built
`emuchef-plan-shadow` binary, an explicit ADB path, and an explicit selected
serial; it never discovers devices or runs `adb devices`. The generated command
uses only the supplied Rust binary plus `--authored-root`, `--device-plan`,
`--probe-adb-getprop`, `--adb-path`, and `--serial`. Its deterministic report
records stable status, stdout/stderr classes, planning status, device-context
field presence, mismatch-warning presence, and scrubbed command metadata. It
does not record the raw serial, full command, full stdout/stderr, absolute local
paths, raw ADB output, timestamps, durations, environment variables, or process
ids.

The Python CLI exposes a developer-only bridge to an already-built Rust shadow
planner binary:
`emuchef plan --planner-backend rust-shadow --rust-planner-bin <path>`. The
default `emuchef plan` behavior remains Python-owned for planning output and
exit-code behavior. `rust-shadow` requires `--rust-planner-bin` and the Python
CLI route never invokes Cargo. The route forwards `--authored-root`,
`--device-plan`, explicitly supplied device context flags, and repeated raw
`--bind` arguments to the supplied `emuchef-plan-shadow` binary in original
order. Omitted `--rust-shadow-output` and explicit `--rust-shadow-output
passthrough` preserve JSON/text passthrough: Rust stdout is passed through to
Python stdout, Rust stderr is passed through to Python stderr, and the Rust
process exit code is returned. In passthrough mode, `--ops`, `--output`,
`--verbose`, `--debug`, `--adb`, and `--serial` are not supported with
`rust-shadow`; explicit `--manufacturer`, `--model`, `--android-version`, and
repeated `--device-tag` values are supported.

The explicit `--rust-shadow-output python-compatible` mode is a formatter bridge
for usable Rust `PlanningResult` JSON. It formats concise stdout with the visible
Python planning summary labels, emits structured YAML to stdout for `--verbose`,
and writes structured YAML to `--output` paths while stdout remains concise. The
structured YAML is produced from the Rust JSON mapping through the existing
`dump_yaml(...)` path; the mode does not reconstruct full Python planner domain
objects or make Rust the default planner. If Rust emits usable planning JSON with
a non-zero exit code, the CLI formats the result and preserves that non-zero exit
code. If Rust exits zero but the parsed result has no execution plan, the CLI
formats the result and returns non-zero. Empty, invalid, or non-planning-result
stdout is a compatibility-mode error.

`emuchef plan --planner-backend rust-experimental --rust-planner-bin <path>` is
an explicit non-default migration route. It reuses the same supplied Rust shadow
planner binary invocation as `rust-shadow`, always uses Python-compatible output
formatting, forwards the same explicit device context flags as `rust-shadow`,
forwards explicit `--rust-detected-facts-json <path>` as
`--detected-facts-json <path>` without Python-side fixture inspection, forwards
explicit live-probe wrapper flags `--rust-probe-adb-getprop`,
`--rust-adb-path <path>`, and `--rust-serial <serial>` as
`--probe-adb-getprop`, `--adb-path <path>`, and `--serial <serial>`, and allows
`--verbose` and `--output` through that compatibility formatting path. Fixture
forwarding and live-probe forwarding are mutually exclusive detected-facts
sources. Python does not invoke ADB, discover devices, run `adb devices`, parse
`getprop`, or validate, normalize, expand, or stat the forwarded ADB path or
serial.
`--rust-shadow-output` is only valid with
`--planner-backend rust-shadow`; Python, `rust-experimental`, and
`rust-production-equivalent` reject it before ADB resolution,
planner/session construction, or subprocess execution. `rust-experimental` is a
cutover rehearsal route, not the default planner, not a stable final public
contract, and not Python planner deletion. Its name and behavior may change
before Rust becomes the default planner backend.

`emuchef plan --planner-backend rust-production-equivalent --rust-planner-bin
<path>` is an explicit non-default production-equivalent route-plumbing path.
It reuses the supplied Rust shadow binary, always uses Python-compatible output,
forwards explicit device context flags and bindings like the other Rust
subprocess routes, accepts `--rust-detected-facts-json <path>`, and accepts the
complete live-probe wrapper flag set `--rust-probe-adb-getprop`,
`--rust-adb-path <path>`, and `--rust-serial <serial>`. Fixture forwarding and
live-probe forwarding are mutually exclusive. Python does not invoke ADB,
discover devices, run `adb devices`, parse `getprop`, validate or normalize the
forwarded ADB path or serial, construct a planner session, or run apply logic
for this route. P8AI adds no production-equivalent-specific flags, smoke
evidence, readiness-gate execution, or blocker reclassification.
P8AK adds optional/manual fixture-backed mismatch-warning smoke tooling for this
explicit route only. It does not add normal checks, readiness-gate execution, or
blocker reclassification.

The Python Rust shadow bridge does not execute/apply plans, invoke ADB, access
the network, materialize artifacts, use Tauri commands, expose sidecar protocol
requests, invoke Cargo, regenerate fixtures/goldens, or make Rust planner output
authoritative. `rust-experimental` and `rust-production-equivalent` accept
Python-facing detected-facts fixture and live-probe wrapper flags; the default
Python backend and `rust-shadow` reject them. Direct Rust shadow fixture and
live-probe modes can emit detected-device profile mismatch warnings, and the
explicit Rust subprocess routes can format forwarded Rust `PlanningResult` JSON
through the Python-compatible path, but production user-facing planning routes
do not own that warning path. Real device context/probing remains a default-cutover
blocker. CLI output
compatibility remains a blocker before any default Rust planner routing. Future
default Rust planner routing must preserve Python concise summary output, Python
`--verbose` structured YAML, Python `--output` YAML file behavior, and Python
exit-code behavior unless a separate accepted breaking-change decision changes
that target. Rust-native JSON output requires a future explicit structured-output
mode such as `--format json`.

The Rust planner readiness gate treats optional `device_context` schema validity
and explicit-context readiness coverage as separate static facts. Valid matrix
schema fields are `manufacturer`, `model`, `android_version`, and non-empty
ordered `device_tags`. A scenario with `device_context: {}` is schema-valid but
does not count as explicit-context coverage; explicit-context coverage requires
at least one valid scenario with a meaningful supplied context field. Explicit
device context support is covered by matrix schema/tooling evidence, but default
Rust planner cutover remains blocked. The former broad real-device context
blocker is narrowed into two still-blocked areas: real-device probing and
detected-device profile mismatch warning parity. Default `emuchef plan` remains
Python-owned, and `rust-shadow` and `rust-experimental` remain explicit
non-default Rust routes.

P8M's accepted ADR 0003 decides the future ownership model for those remaining
context blockers: Rust should own real-device probing and detected-device
profile mismatch warning parity before default Rust planner cutover. P8N adds
the crate-local Rust abstraction and fake/non-live tests for the first step of
that path. P8O adds fake/test-backed planner-input construction that applies
detected facts over synthetic/profile-derived context. The intended future
precedence is synthetic/profile context -> detected facts -> explicit CLI
overrides. P8P adds pure/test-backed mismatch-warning helper logic and tests
only; explicit device context remains separate from detected facts and
real-device probing. P8Q adds crate-private fake/test-backed planner-result
composition that applies detected facts to planner context and appends
detected-device profile mismatch warnings only in that helper/test path. P8Q
does not wire probing or mismatch warnings into `rust-shadow`,
`rust-experimental`, the Python CLI, Tauri/protocol, executor/apply, the
readiness gate, or normal runtime checks. Real-device probing and route-level
detected-device profile mismatch warning parity remain blocked until live
implementation and route evidence exist. P8R adds only a local detected-facts
fixture harness to `emuchef-plan-shadow`; explicit context remains separate and
overrides the emitted fixture-derived device context when both inputs are
supplied. Executor/apply, Tauri/protocol, Cargo fallback behavior,
fixture/golden data, normal runtime checks, and Python planner deletion remain
separate future work.

P8K extends the dev-only planner matrix tooling with optional explicit
`device_context` data per scenario. Valid matrix context fields are
`manufacturer`, `model`, `android_version`, and non-empty ordered `device_tags`.
Detected-device, `adb`, `serial`, and probing-related fields are rejected.
Omitted `device_context` preserves the existing synthetic/profile-derived
planner context. Supplied scalar context fields override synthetic/profile-derived
fields, and supplied tags replace profile tags in order. Empty `device_tags`
lists are rejected because the existing P8J CLI flags cannot distinguish an
explicit empty tag override from omitted tag flags.

`tools/smoke_rust_shadow_cli_matrix.py` is the dev-only matrix smoke for the
explicit Python CLI Rust planner migration routes. It is standalone stdlib-only
tooling and does not import the planner comparison harness or Python planner
modules at module load. The smoke loads `tools/plan_parity_scenarios.json`,
requires `--authored-root` and an explicit `--rust-planner-bin`, optionally
accepts `--python-executable`, and supports `--planner-backend rust-shadow` or
`--planner-backend rust-experimental`. The default smoke-runner backend remains
`rust-shadow`; its default output mode remains `passthrough`, and generated
commands omit `--rust-shadow-output passthrough` so the raw P8B route remains
stable. For `rust-experimental`, generated commands omit `--rust-shadow-output`
and the effective output mode is Python-compatible. Successful
`rust-experimental` smoke scenarios require actual route exit code `0` and
stdout classification `python_summary`. Raw Rust JSON stdout remains classified
as `stdout_json` and fails the `rust-experimental` smoke for successful
scenarios. The smoke creates only planner-visible temporary placeholder resources
for matrix bindings and invokes `python -m emuchef plan` for each scenario. Its
deterministic JSON report uses stable basenames for local executables, records
the selected route backend and effective route output mode, includes scenario
ids, device-plan ids, binding refs/kinds, expected and actual route exit codes,
pass/fail status, stable command classifications, stdout/stderr classifications,
expected stdout class where enforced, stable context presence/key metadata when a
scenario supplies `device_context`, and bounded normalized failure summaries.
The report does not include timestamps, durations, generated temp paths, random
ids, full context values, detected-device data, or full volatile process output.

P8F uses the same smoke runner for explicit
`--rust-shadow-output python-compatible` matrix smoke. In that mode, a successful
scenario requires exit `0` and stdout classification `python_summary`, meaning
the Python CLI bridge emitted the concise Python-compatible planning summary.
Raw Rust JSON stdout remains classified as `stdout_json` and fails the
compatibility-mode smoke for successful scenarios. YAML-like stdout can classify
as `python_yaml`, but it is not the default P8F success expectation unless a
future scenario explicitly defines that expectation.

`tools/smoke_rust_shadow_live_adb_probe.py` is separate from the matrix smoke,
fixture smokes, and readiness gate. It is direct-shadow-only optional/manual
evidence for P8X live ADB getprop mode and does not call Python route commands,
Cargo, comparison tooling, matrix smoke tooling, fixture smoke tooling,
executor/apply, Tauri/protocol, normal runtime checks, or readiness-gate
execution.

P7P is Python-vs-Rust planner DTO/result comparison evidence, P8B is raw
passthrough Python CLI `rust-shadow` route invocation evidence, P8C is the
passthrough CLI output contract for the explicit shadow route, P8E is the
explicit formatter bridge, P8F is Python-compatible `rust-shadow` route/output
matrix smoke, P8G is the explicit non-default `rust-experimental` migration
route, and P8H is dev-only matrix smoke evidence for that explicit route across
the current scenario matrix. None of these is default Rust planner cutover
readiness or Python planner deletion work. The smoke does not execute/apply
plans, probe devices, invoke ADB, access the network, materialize artifacts,
regenerate goldens, participate in normal Rust/Tauri runtime checks, or make
Rust planner output authoritative.

`tools/compare_rust_python_plan.py` is a dev-only deterministic comparison
harness for Python planner API output versus Rust shadow planner output. It uses
the current Python planner API path (`load_authored_catalog`,
`Planner.start_session`, `session.bind_input`, and `session.emit_execution_plan`)
under the same synthetic/profile-derived planner context used by Rust shadow
planning unless a matrix scenario supplies explicit P8K `device_context` fields.
For those scenarios, the harness forwards supplied values to both the hidden
Python planner worker and the Rust shadow command and reports only stable context
presence/key metadata. This comparison does not call `emuchef plan`, does not
prove Python CLI/device-probing parity, and does not execute plans, probe
devices, invoke ADB, access the network, download or materialize artifacts,
expose Tauri or sidecar protocol commands, update checked-in fixtures/goldens,
or participate in normal Rust/Tauri runtime checks. Reports classify differences as `match`,
`rust_missing`, `python_missing`, `value_mismatch`, `known_gap`,
`intentional_shape_difference`, or `unsupported`, and compare top-level status,
selected and expanded refs, execution-plan presence, step count, step ids/order,
step types, dependencies, normalized params, warning/error shape, and serialized
`permission_plan` presence. The default Rust command mode uses offline Cargo;
developers can pass `--cargo-online`, set
`EMUCHEF_PLAN_COMPARE_CARGO_OFFLINE=0`, or pass `--rust-bin <path>` for local
development.

`tools/plan_parity_scenarios.json` is a dev-only current comparison scenario
manifest for checked-in device plans. Matrix mode runs the Python planner API
and Rust shadow planner for each listed scenario, creates only planner-visible
temporary placeholder resources for required bindings, forwards optional
explicit `device_context` values where present, and emits one deterministic JSON
report to stdout. The report includes scenario ids, device plan ids, binding
refs/kinds, stable context presence/key metadata when applicable, expected and
actual classifications, expectation pass/fail status, summary counts, known
gaps, diagnostics, and mismatch classification buckets. It does not include
generated temporary path values or full context values.
In this matrix, `match` means only that the dev-only comparison harness found
no unclassified differences for the compared fields under the supplied
planner-only bindings and shared planner context. It does not mean Python CLI
parity, real-device parity, executor/apply parity, artifact/network/
materialization parity, full schema parity, future scenario parity, or Rust
planner cutover readiness by itself. Matrix mode exits `0` only when every
scenario actual classification matches its expected classification. This exit
status is an expectation check for the current dev-only scenario matrix, not a
claim of full planner correctness. Current checked-in base matrix scenarios for
`ayaneo.konkr_pocket_fit.base`, `ayaneo.pocket_s_mini.base`,
`ayaneo.generic.base`, `ayaneo.pocket_air_mini.base`, and
`ayaneo.pocket_s2.base` are all expected `match`. P8K adds
`ayaneo_pocket_s_mini_base_explicit_context`, another expected-`match` scenario
for `ayaneo.pocket_s_mini.base` with supplied manufacturer, model, Android
version, and ordered tags. Future scenarios may intentionally expect
`known_gap`. Matching matrix status is necessary evidence for the compared
planner-only fields, not sufficient user-facing planner cutover or Python
planner deletion readiness. The matrix remains a dev-only manual artifact in
the current state; it is not wired into normal Rust/Tauri checks.

`tools/check_rust_planner_cutover_readiness.py` is the P8I static readiness gate
for future PRs that propose making Rust the default `emuchef plan` backend. It is
stdlib-only, imports no planner/runtime modules, parses only JSON and source/docs
text, derives checked-in device-plan ids from
`authored/device_plans/*.yaml` and `authored/device_plans/*.yml` filenames, and
emits deterministic JSON. The report verifies static prerequisites and stable
references, validates optional P8K `device_context` shape, lists required manual
evidence commands, and keeps its top-level status `blocked` even when all static
checks pass. The gate does not run live
comparison or smoke tooling, Cargo, npm, ADB, executor/apply, Tauri/protocol,
device probing, network access, artifact materialization, fixture/golden
regeneration, or Python planner deletion work. Python remains the default
CLI/reference planner owner, and `rust-experimental` remains explicit,
non-default cutover rehearsal routing.

The Rust backend supports one-shot stateless requests through:

- `hello`
- `ping`
- `listStepSpecs`
- `validateRecipePath`
- `emitRecipeYamlFromPath`

The Rust backend supports a persistent JSON Lines sidecar mode through:

```bash
cargo run --manifest-path crates/emuchef-rust-backend/Cargo.toml -- --sidecar
```

The sidecar reads one UTF-8 JSON request per stdin line and writes one UTF-8
JSON response per stdout line. Stdout is machine-readable JSONL only; diagnostics
and human-readable logs belong on stderr. Every valid sidecar request includes
an opaque string id, every response echoes that id exactly, and malformed JSON
lines return `id: null`. Sidecar request-level failures use the same `ok:false`
API envelope as one-shot requests and do not terminate the process. The sidecar
exits cleanly when stdin reaches EOF.

The sidecar uses the Rust `DocumentSessionManager` for live document sessions.
It supports session-backed requests for listing step specs, opening documents,
creating documents from templates, getting and closing documents, applying
recipe commands, undo, redo, saving, Save As, validation, canonical YAML
emission, authored-root context updates, and ref-index retrieval. The sidecar
protocol includes a backend-agnostic `hello` request. `hello`
returns integer `protocolVersion: 1` and a string `capabilities` list. The
protocol does not expose `implementation` or `implementationVersion` fields,
and there is no protocol negotiation. Capability names describe editor protocol
operations rather than backend implementation details. The legacy/reference
Python API reports the capabilities it supports, including optional protocol
operations such as create-from-template, document close, and Save As.

## Current Tauri Config Editor

`apps/config-editor` is the primary Tauri development/editor UI for authored
recipe files. It uses Tauri v2, React, TypeScript, Vite, Tailwind, npm, and the
official Tauri dialog plugin. The app now has host-target Tauri packaging for
the Rust sidecar and Phase 6X app-local release-hardening checks for Rust runtime
scripts, no-Python-runtime assurance, sidecar bundle-input inspection, and
simulated-packaged sidecar smoke coverage. Signing, notarization, updater
support, real packaged GUI E2E, cross-platform release automation, and public
release readiness remain later work.

The editor opens authored recipe YAML files through a native file picker, calls
the Rust sidecar through Rust Tauri commands, and displays recipe data,
diagnostics, canonical YAML, sidecar status, and available step specs. It keeps a
reusable `documentId` for the currently open sidecar document. After a document
is opened through the sidecar, document-specific actions use document-id-based
sidecar requests so unsaved in-memory edits remain visible to validation, YAML
emission, undo, redo, and save.

The Tauri editor runs in development mode with a local Rust sidecar binary. The
Tauri `beforeDevCommand` runs `npm run sidecar:dev` before Vite, so normal
`npm run tauri dev` starts by building the debug Rust sidecar with Cargo
incremental rebuilds and preparing the Tauri v2 `externalBin` input. The Rust
bridge still resolves development sidecars from the crate-local or repo-root
Cargo `target/debug` directories and starts the binary with `--sidecar`.
Packaged release builds run `npm run sidecar:build` before the frontend build,
copy the release sidecar to `src-tauri/binaries/emuchef-rust-backend-$TARGET_TRIPLE`,
and let Tauri bundle the target-triple-stripped `emuchef-rust-backend` beside
the packaged app executable.

Routine local runtime verification can use `npm run check:rust-runtime` from
`apps/config-editor`. That aggregate runs app-local naming/unit checks,
no-Python-runtime scanning, no-Python-editor-API scanning,
no-Python-fixture-regeneration scanning, no-PySide-runtime scanning, TypeScript
typecheck, and frontend logic tests; it
does not run Python golden regeneration, ADB/device tests, release builds, or a
real Tauri package build.
Packaging-specific checks remain explicit through
`npm run check:sidecar:bundle-input:debug`, `npm run check:sidecar:bundle-input`,
and `npm run smoke:sidecar:simulated-packaged`.

Real packaged GUI E2E has a manual checklist at
[docs/manual/packaged-gui-e2e.md](docs/manual/packaged-gui-e2e.md); the
checklist is a manual validation procedure, not completed validation evidence.

The Tauri editor supports sidecar-backed recipe editing. The Overview screen
edits recipe name and description. Recipe id, schema version, and kind are
read-only in the Overview screen. Inputs, artifacts, and artifact groups support
CRUD-style editing where the Rust sidecar exposes the corresponding document
command. Add, rename, and duplicate actions collect required text with
app-owned dialogs, destructive actions require confirmation, and artifact
creation collects both artifact id and URL before submitting a command. Artifact
group duplication creates a new group with the same ordered artifact members as
the source group and does not rewrite step refs or selections.

The Tauri Steps screen supports basic step lifecycle editing, step dependency
editing, existing step param editing, and JSON-backed advanced step internals
editing through the Rust sidecar. Step add, delete, duplicate, reorder,
display-name edits, `user_toggleable` edits, dependency updates, param updates,
and advanced internals updates are available when the matching backend command
codec mappings exist. Existing step ids and step types are read-only. Step id
and step type are chosen only when adding a step. Add Step collects step id,
step type, and an optional display name, and the frontend does not synthesize
params or other required runtime fields.

Dependency updates use `UpdateStepDependencies` with the complete next
dependency id list. The frontend blocks obvious self-dependency and duplicate
dependency selections, treats missing or null dependency lists as empty for UI
rendering only, and leaves graph validity, cycles, unknown step ids, and final
execution ordering to backend validation and planning. Missing or unknown
authored dependency ids remain visible as raw ids and can be removed. Delete Step
uses backend safe-delete behavior, so supported downstream step dependencies,
`conflicts_with` entries, and step refs are removed by the sidecar rather than by
TypeScript cleanup logic.

Step param updates use `UpdateStepParams` with the complete next params object
for the selected step. The frontend edits authored step params, keeps literal
values as JSON primitives, objects, or lists, and sends refs in the authored
`{ ref: "..." }` shape. A literal JSON `null` param value is distinct from
clearing a param; clearing removes the param key from the submitted params
object. The Rust sidecar backend remains authoritative for command application,
validation diagnostics, canonical YAML, dirty state, and undo/redo state.
`StepSpecDto` improves UI rendering for param order, enum controls, ref
filters, and known param shapes, but it is UI metadata only and is not mutation
authority.

The Tauri Steps screen uses rich structured editors for known-shape step params
when those shapes are backed by Python `ParamSpec` schema metadata projected
through `StepSpecDto`. Schema-backed ordered artifact id and artifact-group id
list params use list controls with add, remove, up, and down actions. Runtime
and app-op permission params use row editors for the schema fields. Policy
params use schema-backed select and checkbox controls. Structured object and
object-list editors preserve unknown or extra keys where the existing authored
value can be copied and updated without data loss. Raw JSON remains the editor
for free-form, unknown, unsupported, or incompatible params, including metadata
unless schema metadata is added for it later.

The ref picker uses the current document `refIndex`, prefers `candidates`,
falls back to `allRefs`, and keeps current missing or incompatible refs visible
for repair. Selecting a ref does not automatically add or rewrite step
dependencies.

Advanced step internals use a collapsed Advanced section. Constraints use a
structured editor when the sidecar DTO shape is a lossless object with optional
`capabilities` and `conflictsWith` string arrays. Constraint edits use DTO/API
naming internally and submit through the existing `UpdateStepConstraints`
command with `conflictsWith`. The raw JSON fallback displays authored-facing
`conflicts_with`, converts that key back to `conflictsWith` before command
submission, and rejects drafts that contain both conflict-field spellings.
Unsupported, malformed, or future-shaped constraint objects remain editable as
raw JSON instead of being normalized by the frontend. `skip_if` remains a plain
JSON textarea editor.

`verify` uses a structured list editor for condition entries whose top-level
shape is exactly `{ type, params }` and whose supported known field is a string:
`path_exists.params.path`, `file_exists.params.path`, and
`package_installed.params.package_name`. Structured verify edits preserve
unknown keys inside `params`. Unsupported, malformed, or future-shaped verify
entries remain editable as per-entry JSON and are submitted without frontend
normalization when their shape is accepted by the existing `UpdateStepVerify`
command codec. Advanced internals editors keep local drafts until the user
applies or commits an edit, then send `UpdateStepConstraints`,
`UpdateStepSkipIf`, or `UpdateStepVerify` through
`sidecar_apply_recipe_command`. Revert restores drafts from the current returned
document value. JSON `null` is a literal JSON value and is not a clear action;
if the command codec requires a different top-level shape, the frontend reports
a local shape error and does not submit. Advanced JSON values that contain
objects shaped like `{ "ref": "..." }` are ordinary JSON values in this editor.
The Tauri editor has no specialized `skip_if` condition builder or ref picker
inside advanced internals. The Rust sidecar remains authoritative for advanced
internals command application, canonical YAML, semantic validation diagnostics,
dirty state, undo state, and redo state. Advanced internals command success with
`changed: false` is treated as a no-op rather than an applied edit.

Inputs, artifacts, artifact groups, and steps use master-detail panes inside the
Tauri editor. The editor frame does not scroll when item lists scroll. Each
screen keeps the list column and detail column in independent scroll regions,
and the list column can be resized with a vertical separator handle. Resized
list widths are local UI preferences and do not affect recipe data, dirty state,
undo, redo, validation, YAML emission, or sidecar commands.

The Tauri editor sends explicit editor commands through
`sidecar_apply_recipe_command`. The frontend treats the returned
`RecipeDocumentDto` as the replacement document state and uses returned dirty,
undo, redo, diagnostics, and YAML values. The frontend does not reconstruct YAML
from DTOs and does not use path-based one-shot validation or YAML emission for
open sidecar documents.

Editable Tauri text controls disable browser writing aids such as spellcheck,
autocorrect, and autocapitalization. When a user types or pastes text into an
editable Tauri input or textarea, smart double quotes and smart single quotes
are normalized to ASCII quotes before the frontend stores the local draft or
sends a sidecar command. Read-only surfaces such as the YAML preview,
diagnostics, and DTO-rendered values are not normalized unless the user edits
that value through a Tauri text control.

The frontend keeps one shared command-in-flight guard for sidecar-backed app and
document commands. While a command is in flight, conflicting document actions
are disabled and duplicate submissions are ignored. Explicit operations such as
opening recipes, saving, undo, redo, validation, YAML refresh, and mutation
commands show transient operation text inline in the toolbar. Save success
feedback is UI-only and does not mutate dirty state, undo/redo state, diagnostics,
YAML, or DTO data beyond the returned document state from the Rust sidecar.

The Tauri editor exposes primary app actions through native menus. File contains
Open Recipe, Set Authored Root, Clear Authored Root, Save, and Save As. Edit
contains Undo and Redo. Utilities contains Validate and Refresh YAML. Menu items
are context-aware and disabled when an action is not valid, the backend is known
to be incompatible, the document session is invalid, or a conflicting command is
in flight. The app follows Tauri v2 desktop menu behavior: macOS uses the native
app menu convention, while Windows and Linux use native window menu bars.
Debug-only controls are not exposed in the normal menu/UI path.

The Tauri editor keeps a frontend-owned selected authored-root preference for
future recipe opens. `null` means no explicit authored root is selected, so the
backend default or inference behavior applies. Selecting a directory with Set
Authored Root does not validate the path in TypeScript; the directory picker only
collects the user's selected directory. Opening a recipe passes the selected
authored root to the Rust sidecar. The editor displays this selected root
separately from the current document's returned `authoredRoot` because inferred
or normalized document context does not become the frontend preference for future
opens.

Authored root is validation and catalog context for the editor session, not
recipe YAML content. When a document is open, Set Authored Root and Clear
Authored Root call the Rust sidecar `setDocumentAuthoredRoot` request and replace
local document state with the returned `RecipeDocumentDto`. This updates derived
diagnostics, canonical YAML, ref index, dirty state, and undo/redo metadata from
the backend DTO without reloading recipe YAML from disk. A failed backend
context update leaves the selected frontend authored-root preference unchanged.

Native confirmation prompts guard opening another recipe with unsaved changes
and closing the Tauri window with unsaved changes or an operation in flight when
Tauri close-request interception is available. Dirty open confirmation happens
before the native file picker, file-picker cancellation keeps the current
document/session unchanged, and failed opens preserve the current document when
the old session remains valid. Dirty close handling uses frontend
`RecipeDocumentDto.dirty` state and does not save implicitly. Close handling
does not cancel in-flight operations.

The Tauri editor does not expose dependency graph visualization, dependency
reorder controls, drag-and-drop, executor/apply-device UI,
create-from-template UI, Python bundling, installer packaging, or broad Rust
ports of Python planner/executor behavior. YAML preview is read-only.

Future editor UX opportunities:

- when a user selects a ref that points to another step output, the editor may
  offer to add the producing step as a dependency or warn if the dependency is
  missing. This should be an explicit user-confirmed action, not automatic
  mutation.

The frontend talks to Rust through Tauri `invoke(...)` commands named:

- `list_step_specs`
- `open_recipe`
- `validate_recipe_path`
- `emit_recipe_yaml_from_path`
- `sidecar_status`
- `sidecar_ping`
- `sidecar_restart`
- `sidecar_list_step_specs`
- `sidecar_open_recipe`
- `sidecar_get_document`
- `sidecar_apply_recipe_command`
- `sidecar_undo`
- `sidecar_redo`
- `sidecar_save_recipe`
- `sidecar_save_recipe_as`
- `sidecar_validate`
- `sidecar_emit_yaml`
- `sidecar_get_ref_index`
- `sidecar_set_document_authored_root`

The Tauri editor runtime launches the experimental Rust backend sidecar directly
for editor protocol requests. The sidecar client starts
`emuchef-rust-backend --sidecar` on the first sidecar request, sends `hello`,
requires protocol version 1 plus the editor's required capabilities, and then
continues the original request without a frontend retry. A `Running` Rust
sidecar state means the process is started and handshake-compatible. The client
serializes requests as one send-line/read-line operation at a time and treats
non-handshake API `ok:false` envelopes as successful Rust transport results. For
local development and tests, the bridge resolves a previously built Rust binary
from the crate-local or repo-root Cargo `target/debug` directories. In packaged
release builds, it resolves the Tauri-bundled Rust sidecar from the app
executable directory and does not fall back to development paths.

`sidecar_status` reports local Rust process and compatibility state only. It
does not start the sidecar and does not perform a fresh `hello` call. Before the
first sidecar request, compatibility is unchecked. After a sidecar process has
started, status reports cached compatibility metadata such as compatibility,
protocol version, capabilities, and the last compatibility or transport error.

`sidecar_ping` sends a lightweight `ping` request to the current Rust sidecar
and receives `{ "healthy": true }` when the sidecar transport and protocol
dispatcher are responsive. `sidecar_restart` is an explicit process reset path:
it kills any owned Rust sidecar process directly, starts a fresh
`emuchef-rust-backend --sidecar` process, performs the `hello` compatibility
handshake, and reports the resulting status with
`documentSessionsPreserved: false`. Restart does not send `ping`, `shutdown`,
`hello`, or any other request to the old process.

If the sidecar exits unexpectedly, an unrecoverable transport failure occurs, or
the backend is incompatible, the frontend marks the document session invalid,
leaves the stale document visible for reference, disables or short-circuits
document-specific actions, and requires an explicit sidecar restart plus a
recipe reopen to create a fresh document session. Normal document and sidecar
request paths do not automatically restart an exited sidecar. Previous document
ids are invalid after process loss or explicit sidecar restart.

The Tauri editor exposes an explicit Restart Sidecar recovery action. A
successful restart replaces the Rust sidecar process and does not preserve
backend document sessions. If a document is visible when the sidecar restarts,
the editor keeps the current `RecipeDocumentDto`, diagnostics, canonical YAML
preview, path, and dirty flag visible as a stale read-only reference. A
compatible running sidecar allows path-backed stale documents to be explicitly
reopened from disk to create a new document session. Dirty stale documents
require explicit confirmation before reopening from disk, and stale documents
without a disk path do not offer Reopen from Disk. The editor does not silently
save, replay commands, reload, reopen, or discard stale in-memory document
state after sidecar loss or restart.

There is no backend selector, runtime backend toggle, environment variable,
config option, UI switch, protocol negotiation path, or Python fallback in the
Tauri editor runtime. Python and the Python CLI remain in the repo only for
legacy/reference/developer/golden workflows until later confirmed replacement or
retirement phases. PySide6 editor source, Python editor API source, and their
normal tests are not present.

The active PySide removal invariant is that a clean default install and normal
active runtime/test path do not require, import, or launch PySide6. `npm run
check:no-pyside-runtime` enforces this by rejecting PySide6 in base or optional
Python dependencies, a published Python `emuchef-editor` console script, active
source imports of PySide6 or `emuchef_editor.app`, normal test imports of
PySide6 or `emuchef_editor.app`, and any Python files under the removed legacy
PySide source/test paths.

The active Python editor API removal invariant is that normal source, test, and
distribution entrypoint paths do not expose `emuchef_editor.api`. `npm run
check:no-python-editor-api` enforces this by rejecting Python editor API source
files, normal Python source/test imports of `emuchef_editor.api`, and console
scripts that publish `emuchef_editor.api` entrypoints.

The active Python fixture/golden regeneration invariant is that normal
Rust/Tauri active checks consume checked-in fixture data only. `npm run
check:no-python-fixture-regeneration` enforces this by scanning the active
`check:rust-runtime` npm script closure, app-local runtime/packaging guard
scripts, Tauri Rust sources/tests, and Rust backend test files for Python
fixture/golden generator invocations.

`crates/emuchef-rust-backend` is an experimental standalone Rust backend
skeleton for migration work. It currently implements `hello`, response
envelopes, one-shot requests, JSON Lines sidecar requests, `ping`,
`listStepSpecs`,
path-based recipe YAML emit/validation, sidecar document open/get/save/close,
sidecar document Save As, sidecar document `emitYaml`/`validate`, sidecar
`getRefIndex`, sidecar `createRecipeFromTemplate`, snapshot undo/redo,
`SetOverviewField` for recipe `name` and `description`, and
fixture-covered `applyRecipeCommand` mutations for inputs, artifacts,
artifact groups, step lifecycle, step dependencies, step params, and advanced
internals. Supported non-step
command families include add, rename,
update-field, delete, and duplicate for inputs and artifacts, plus add, rename,
delete, duplicate, reorder, add member, remove member, and reorder member for
artifact groups. It also has fixture-covered authoredRoot/catalog-context
validation, a private crate-internal planner skeleton that emits Python-shaped
`PlanningResult`/`ExecutionPlan` values for focused fixtures, and a private
crate-internal executor skeleton that emits Python-shaped `ExecutionRunResult`
values for selected safe dry-run fixtures. The Rust planner skeleton reads only
loaded/top-level authored recipe fixture data, namespaced refs, StepSpec
defaults, explicit recipe and step dependencies, and supplied fixture device
context/capabilities. Its Phase 6M/6N planner parity evidence is inventoried and
parsed by Rust tests so new checked-in planner goldens must be classified
intentionally. It also has authored-corpus tests for the checked-in
`authored/recipes` files that use manually supplied selected recipe refs and
synthetic fixture context; those tests classify required binding gaps, cover
RetroArch optional-input pruning, and guard current selected-recipe expansion
evidence. Current checked-in recipes either declare an empty
`recipe_dependencies` list or omit the field so it parses as empty, and current
Rust corpus/device-plan selected refs match expanded refs for that reason only.
The Rust planner parses `provides.features` as recipe metadata but does not
resolve recipe dependencies from `provides` or an active `requires` field. It
also has private checked-in
device-plan/profile ingestion coverage that parses repo `device_plans` and
`device_profiles`, freezes path/id/profile/selected-ref inventory, maps
selected recipe refs in authored order, maps profile capabilities/tags, accepts
supplied test bindings, and emits at least one deterministic plan from a
checked-in profile/plan context. The dev-only comparison harness reports Python
planner API versus Rust shadow planner classifications for those contexts
without making Rust planner authoritative. It validates malformed `steps.*` refs,
unknown selected step targets, unknown step outputs, and shorthand refs to
selected steps without primary outputs for emitted planner steps; non-step refs
remain outside that validation slice. It asserts focused execution-plan DTO
shape for successful and error planner results, selected normalized params,
semantic list ordering, and stable internal error-message shape for the
supported fixture-scoped surface. It
builds internal structured permission intent from selected step-local
`grant_permissions` runtime/app-op/policy params without serializing a plan-level
`permission_plan` field. It also validates selected/emitted step dependencies
for unknown or non-emitted targets, self-dependencies, static dependency cycles,
and focused `grant_permissions` param shapes while preserving duplicate authored
dependencies in emitted execution steps. Runtime dependency outcomes such as
blocked, skipped, or failed step propagation remain executor behavior. The Rust
executor skeleton is internal test scaffolding only: it
models selected `wait`, `grant_permissions`, dependency, skip, verify,
temp-dir-confined filesystem/artifact behavior, fake dry-run device semantics,
and Phase 6R real-ADB adapter foundations without public API exposure. It does
not add production planner or CLI replacement, backend selection, production
device discovery, real network downloads, signing/notarization, updater
support, cross-platform release automation, or Python bundling.
It now has crate-internal fake-device/DryRunAdb parity fixtures plus an
explicitly constructed real-ADB adapter and ignored/manual real-device tests; it
still has no public real-device executor surface. Its `hello` response reports
only capabilities implemented in the crate, but capability parity is not full
product parity. Rust `listStepSpecs` uses Rust-owned static DTO metadata; Python
remains the reference implementation for broader non-editor behavior until later
replacement work is explicitly approved. The private planner skeleton and
private safe executor skeleton are
scaffolding only and should be replaced or broadened with Rust-native schema
builders, full executor parity, and broader parity tests before any backend
cutover is attempted.

The Tauri editor presents authored-root selection in the normal UI and native
menu path. There is no backend selector, runtime backend toggle, execution/apply
UI, or Python fallback in this workflow.

## Current PySide6 Status

PySide6 is removed from the Python dependency metadata, active source tree, and
normal test tree. The legacy `src/emuchef_editor/app` source package,
`tests/legacy` PySide tests, `pyside-editor` optional dependency extra, and
`emuchef-editor` console script are not present. The Python editor API source
package is also not present. The Tauri editor is the only GUI editor path.

`createRecipeFromTemplate` is implemented by the Rust sidecar backend for
protocol parity. GUI create-from-template is not part of the normal Tauri editor
path unless a future product requirement reintroduces a GUI template-creation
workflow.

## Current Step Plugin Architecture

Supported step behavior is registered through first-party, in-repo step plugins.
The built-in step registry is the canonical source for:

- supported step specs and params
- planner validation hooks
- planner normalization hooks
- direct executor handler callables
- primary output metadata
- editor-safe labels, param ordering, and typed ref-filter hints

Step type ids are plain strings owned by the built-in registry. Authored recipes
and execution plans use the same visible YAML values, such as `copy_files` and
`grant_permissions`, and `schema_version: 1` remains current. `STEP_SPECS` and
primary-output maps are compatibility projections derived from the built-in
registry, not independent sources of truth. `StepSpec.executor_handler` is
transitional metadata only; runtime dispatch uses `StepPlugin.handler` callables
from step-local handler modules. Core plugins do not import PySide or construct
editor widgets; Qt-specific param panels remain in the editor package and are
keyed by step metadata.

External plugin discovery is deferred design work. Adding a currently supported
in-repo step should start by adding a built-in step plugin rather than changing
central planner or executor dispatch branches.

The editor supports in-file refactor tooling for authored recipe ids, input ids,
artifact ids, artifact-group ids, and step ids. Rename, usage analysis, and
delete cleanup are scoped to the currently open recipe file only.

Editor interaction rules:

- edits apply immediately to the in-memory recipe document
- save is explicit and writes canonical YAML to disk
- Save As writes canonical YAML to a new path, updates the open document path and saved baseline, and keeps undo/redo history intact
- template-backed document creation is available through the PySide-free Python core helper and Rust sidecar protocol, but the normal Tauri editor UI does not expose a create-from-template flow
- the workspace lists authored recipes separately from recipe templates
- the workspace list auto-refreshes when authored recipe files or recipe template files are added, removed, or renamed on disk while the workspace is open
- if an open recipe file disappears from the workspace because of an external remove or rename, the in-memory document stays open and the workspace selection clears until that exact path reappears
- unsaved-changes prompts gate opening another recipe and closing the window
- diagnostics and YAML preview refresh after each committed edit
- undo and redo operate at command granularity and persist across saves for the open document
- dirty state is a semantic comparison against the last saved canonical YAML baseline
- form-based editor pages keep labels right-aligned while data-entry fields stay left-aligned, expand to the available pane width, and anchor the entry group at the top-left of the editor pane
- current field surfaces across Overview, Inputs, Artifacts, Artifact Groups, and Steps expose hover tooltips that explain authored field purpose, accepted values, read-only semantics, and creation-time id or dialog semantics
- the Steps page uses a master-detail layout with:
  - ordered step list actions for add, delete, duplicate, reorder, and `user_toggleable`
  - step list action buttons wrap to additional rows when the list pane is narrowed
  - grouped step detail sections for basics, dependencies, params, constraints / `skip_if`, and `verify`
  - a dependency editor card with add/remove actions over existing step ids only
  - structured ref pickers over typed authored refs only
  - schema-backed rich step param editors for artifact lists, artifact-group lists, step-local `grant_permissions` runtime/app-op rows, and step-local `grant_permissions` policy fields
  - auto-sizing params content that shrinks and grows with the active step form, visible preserved-content blocks, and `extract_archive.extract_on`
  - auto-sizing ordered lists for dependencies, capabilities, `conflicts_with`, `skip_if`, and `verify`, with one visible empty row when a list has no items
  - live diagnostics and YAML refresh after committed step edits
- the editor persists explicit authored refs only:
  - `inputs.<id>`
  - `artifacts.<id>.<field>`
  - `steps.<id>.outputs.<field>`
- shorthand step refs may be offered as picker convenience labels, but saved YAML remains explicit `{ ref: ... }`
- unresolved step refs remain preserved in the open document and are surfaced in the step editor as unresolved picker values
- Find Usages shows a grouped read-only list of supported in-file usages for the selected recipe, input, artifact, artifact group, or step id
- rename actions update supported structured in-file references while preserving unsupported step content unchanged
- delete actions show a grouped usage summary before destructive deletion
- confirmed deletes remove the selected item and matching supported structured references, such as param refs, step dependencies, `conflicts_with`, artifact-group membership, and supported step artifact or artifact-group selection entries
- cleanup removes only matching structured references or list entries; surrounding steps, groups, params objects, and constraints objects remain unless they are the explicit delete target
- supported step-authoring surface currently includes:
  - `resolve_artifacts`
  - `extract_artifacts`
  - `extract_archive`
  - `copy_files`
  - `install_apk`
  - `grant_permissions`
  - `launch_app`
  - `wait`
  - `force_stop_app`
- step ids and step types are chosen at creation time; step type remains fixed, and step id changes use the explicit Rename action
- dependency additions append to the end of the authored dependency list and are not re-sorted by the UI or YAML writer
- unsupported authored step params, condition entries, and constraint entries that the current UI does not edit are preserved semantically and round-trip unchanged when supported sections of the same step are edited
- rename and delete tooling warns when preserved unsupported step content exists, because additional references may be present there and are not rewritten
- when unsupported constraint or condition entries are present, the affected destructive list operations stay locked and the preserved authored entries remain visible read-only
- read-only preserved step-content surfaces expose hover guidance explaining that unsupported authored content remains preserved on save unless explicitly replaced through a supported editor surface

Field-scope rules currently enforced by the editor:

- `kind` is read-only
- `schema_version` is read-only and reflects latest-schema-only support
- artifact kind support is currently limited to `remote_file`
- recipe, input, artifact, artifact-group, and step id fields are read-only in detail forms and are changed through explicit Rename actions
- permission editing lives in the selected `grant_permissions` step's params surface: `runtime`, `appops`, and `policy`
- `grant_permissions.params.policy.on_failure` is edited through a non-freeform dropdown seeded from the shared known policy values; if authored YAML contains another value, the editor shows it as a visible invalid option until the user replaces it
- top-level recipe `permissions:` fails load/validation explicitly and is not auto-coerced by the editor or loader
- deleting an input removes matching supported `inputs.<id>` param refs
- deleting an artifact removes matching supported artifact refs, artifact-group memberships, and supported step artifact-selection entries
- deleting an artifact group removes matching supported step artifact-group selection entries
- duplicating an artifact group copies the ordered artifact membership and leaves supported step artifact-group selections unchanged
- deleting a step removes matching supported step refs, step-output refs, dependencies, and `conflicts_with` entries

## Supported CLI

Current commands:

- `draft`
- `plan`
- `apply`
- `detect`
- `detect-profiles`
- `validate`

Common notes:

- No-backend `emuchef plan` is Rust-owned through the production-equivalent
  subprocess path. P8BJ removes the explicit public `--planner-backend python`
  route; the Python planner implementation remains present only for later
  deletion slices and private transitional coverage. Explicit Rust planner
  migration routes are `--planner-backend rust-shadow` and
  `--planner-backend rust-experimental`, and both require
  `--rust-planner-bin <path>`. `rust-shadow` remains diagnostic/dev-only and
  passthrough by default: omitted `--rust-shadow-output` and
  `--rust-shadow-output passthrough` pass through Rust JSON/text stdout, stderr,
  and exit code. Explicit `--rust-shadow-output python-compatible` is valid only
  with `rust-shadow`; it parses usable Rust `PlanningResult` JSON and formats it
  with Python-compatible visible summary labels, `--verbose` YAML, and
  `--output` YAML over the Rust JSON mapping. `rust-experimental` is an explicit
  non-default migration route. Its name and behavior may change before Rust
  becomes the default planner backend. It uses Python-compatible output by
  default and is not the default planner, not a stable final public contract, and
  not Python planner deletion. The dev-only matrix smoke for `rust-experimental`
  requires successful scenarios to exit `0` and emit stdout classified as
  `python_summary`; raw Rust JSON stdout fails that smoke for successful
  scenarios. `--planner-backend rust-production-equivalent` is an explicit
  non-default Rust subprocess route that requires `--rust-planner-bin`, reuses
  the supplied Rust shadow binary, always uses Python-compatible output, and can
  forward Rust-owned detected-facts fixture or live-probe wrapper inputs. It is
  route plumbing only and does not add smoke evidence or default cutover. Future
  default Rust planner routing must preserve
  Python-owned output and exit-code semantics unless a separate accepted
  breaking-change decision says otherwise; Rust-native JSON belongs behind a
  future explicit structured-output mode such as `--format json`.
- `--device-plan` expects a device plan id, not a device profile id
- `--adb` is supported on `draft`, `plan`, `apply`, and `detect`
- ADB resolution order is:
  1. `--adb`
  2. `EMUCHEF_ADB`
  3. config hook placeholder
  4. `adb` on `PATH`
- Current `--bind` input ids use the normalized draft input id form, for example:
  - `app.retroarch.provision/retroarch_cfg=/path/to/retroarch.cfg`
- The older `$input` bind form is not the current CLI contract

## Current Executor Semantics

Executor remains single-threaded and owns runtime command execution:

- evaluate dependency state
- evaluate capability/conflict gating
- evaluate `skip_if`
- execute the step
- run `verify`

Current step types:

- `resolve_artifacts`
- `extract_artifacts`
- `extract_archive`
- `copy_files`
- `install_apk`
- `grant_permissions`
- `launch_app`
- `wait`
- `force_stop_app`

Important execution details:

- `grant_permissions` constructs and executes ADB permission commands from the
  selected step's own `params.runtime` and `params.appops`
- planner normalization stays declarative and does not generate permission shell
  commands
- a failing `grant_permissions` step fails that step and blocks dependents
  through normal dependency flow; unrelated grant steps are unaffected unless
  dependencies connect them
- skipped and failed steps do not expose resolvable runtime-ref outputs, but a
  failed `grant_permissions` step may still keep collected permission action
  results in its step run record for reporting
- `wait` uses `duration_ms`
- `launch_app` now tries package-manager-based launcher resolution before falling
  back to `monkey`
- artifact downloads keep TLS verification strict
- TLS failures are reported as `tls_verification_failed`
- other fetch failures are reported as `artifact_download_failed`

Step completion states currently distinguish:

- `executed`
- `skipped`
- `blocked`
- `failed`

`blocked` means a dependency failed or was already blocked.

## Copy Semantics

`copy_files` is the unified copy step.

Current rules:

- if the source is `directory_path` or `path_list`, `dest` is treated as a
  destination directory
- if the source is `file_path` and `dest` exists as a directory, the file is
  copied into `dest/<basename>`
- otherwise `dest` is treated as the exact target path

Implication for authored recipes:

- if a single file should land at a specific filename, author the full
  destination file path
- do not rely on a bare directory path unless the runtime existence of that
  directory is intentionally part of the step behavior

Shared-storage behavior stays unprivileged.

App-private destinations under:

- `/data/user/`
- `/data/data/`

are treated as privileged app-data writes.

For those destinations:

- host sources are staged under `/data/local/tmp/emuchef/...`
- then copied into place through root-backed device operations
- `sync` currently behaves like `merge` in that privileged path
- verify/path checks against app-private destinations are root-aware

If runtime capabilities do not actually provide both `app_data_write` and
`root_shell`, app-private writes fail with `app_data_write_unavailable`.

## Validation

`emuchef validate` supports:

- full catalog validation via `--authored-root`
- single-file validation
- single-file validation with catalog context

Validation output now includes file-level context in `details`:

- `file`
- `object_kind`
- `object_id`
- `field` when known

Default CLI output groups issues by file.

The editor reuses the same validation path in-process and maps shared warnings/errors into diagnostics for the open recipe document.
When validating an open unsaved recipe against an authored root, the in-memory document replaces the current file's on-disk authored contribution for catalog-context checks.
Changing the authored root of an open document changes validation and catalog
context only; it does not change authored recipe YAML, saved YAML state, the
document path, dirty state, or undo/redo history.
Unknown step dependencies are validation errors, not typed-model construction errors, so the editor can still preserve authored broken dependency state and surface it through diagnostics.

## Device/Profile Behavior

Device matching is intentionally simple and deterministic:

- manufacturer contains
- brand contains
- model pattern match
- minimum Android version

AYANEO-specific notes:

- some AYANEO devices report `manufacturer=ARBOR`
- profiles were updated to allow that mismatch while still matching AYANEO brand
- current real-device work has focused on AYANEO handhelds, including:
  - Pocket S2
  - Pocket S Mini
  - Pocket Air Mini
  - KONKR Pocket FIT

## RetroArch Flow

The RetroArch provisioning recipe currently does all of the following:

1. resolve remote APK/core artifacts
2. install RetroArch
3. bootstrap launch
4. wait
5. force-stop
6. grant permissions
7. launch again after permissions
8. wait
9. force-stop
10. extract selected core archives
11. copy cores into app-private storage
12. copy `retroarch.cfg`
13. final launch

Current RetroArch-specific notes:

- core zips are grouped through `artifact_groups.retroarch_cores`
- core copy uses privileged app-data write behavior
- config copy now targets the explicit file:
  - `/sdcard/Android/data/com.retroarch.aarch64/files/retroarch.cfg`
- optional inputs may remain unbound during planning
- steps with direct `inputs.<id>` refs to unbound optional inputs are pruned
  before final binding checks

## Templates

Example authored templates live under `templates/authored/`.

They are intentionally outside `authored/` so the loader never treats them as
real authored inputs.

Template-backed document creation treats recipe templates as creation sources,
not as normal editable authored recipe documents. The normal Tauri editor UI
does not expose template creation.
Recipe template choices currently include:

- `recipe.template.yaml`
- `recipe.blank.template.yaml`

Creating a new recipe from a template writes the destination file immediately
and only opens the new document after the write succeeds.

The templates now document accepted values and current authoring conventions for:

- kinds
- roles
- input types
- step types
- `copy_policy`
- condition types
- capability names

The editor writes canonical authored recipe YAML instead of preserving arbitrary comments or formatting.
Current canonical top-level ordering for recipes is:

1. `schema_version`
2. `kind`
3. `id`
4. `name`
5. `description`
6. `recipe_dependencies`
7. `provides`
8. `inputs`
9. `artifacts`
10. `artifact_groups`
11. `steps`

Current ordering rules inside canonical recipe YAML:

- `inputs` preserve authored insertion order
- `artifacts` preserve authored insertion order
- `artifact_groups` preserve editor-managed order
- artifact-group membership lists preserve authored/editor-managed order
- grant-step permission item lists preserve authored/editor-managed order
- UI list sorting for inputs or artifacts is view-only and does not redefine authored order

## Current Known Gaps / Follow-up

Known intentional gaps:

- executor remains single-threaded
- artifact download uses Python stdlib networking only
- archive extraction is still ZIP-oriented in practice
- external step plugin discovery is not implemented
- app-op mode and permission-name catalogs are not implemented
- app-private write ownership/uid remapping is not implemented yet
- current CLI bind ids are still normalized internal-style ids rather than a
  cleaner authored ref syntax

If future work changes any of the above, update this document in the same change.
