# Rust Planner Cutover Readiness

This document classifies current evidence and remaining blockers before Python
planner fallback/reference behavior can be retired or the Python planner can be
removed. Python remains the explicit fallback and deletion-readiness reference.
P8AO makes the default no-backend `emuchef plan` route Rust-owned through the
existing production-equivalent Rust subprocess path. During this transition,
the default Rust route requires `--rust-planner-bin <path>` and emits
Python-compatible output. Explicit `--planner-backend python` keeps the previous
Python planning behavior. The other Python CLI routes to Rust planning are the
explicit developer-only
`--planner-backend rust-shadow --rust-planner-bin <path>` path, the explicit
non-default migration route
`--planner-backend rust-experimental --rust-planner-bin <path>`, and the
explicit production-equivalent route
`--planner-backend rust-production-equivalent --rust-planner-bin <path>`.
`rust-production-equivalent` reuses the supplied Rust shadow binary, always uses
Python-compatible output, and routes detected-facts fixture or live-probe
wrapper inputs through Rust-owned shadow-binary plumbing when selected
explicitly. The no-backend default route reuses production-equivalent subprocess
and output behavior but keeps detected-facts fixture and live-probe wrapper
flags explicit-only in P8AO.
`rust-experimental` is a cutover rehearsal route. Its name and behavior may
change before Rust becomes the default planner backend. It is not the default
planner, not a stable final public contract, and not Python planner deletion.
The future default Rust planner CLI output compatibility decision is accepted in
`docs/adr/0002-rust-planner-cli-output-compatibility.md`: default Rust planner
routing must preserve the current Python-owned `emuchef plan` output and
exit-code contract unless a separate accepted breaking-change decision says
otherwise.
The future default Rust planner real-device context ownership decision is
accepted in `docs/adr/0003-rust-real-device-context-ownership.md`: Rust should
own real-device probing and detected-device profile mismatch warning parity for
future default Rust planner cutover. P8M records that ownership decision.
`docs/adr/0004-default-route-live-probe-cutover-design.md` records the
default-route live-probe cutover design: future default-route probing should be
Rust-owned, and P8X-P8AA migration evidence does not clear
`real_device_probing_not_cut_over` or
`detected_device_profile_mismatch_warning_not_cut_over`.
`docs/rust-default-route-probe-request-response.md` records the intended future
default-route probe request/response shape without implementing probing or
clearing readiness blockers.
`docs/rust-production-equivalent-live-probe-smoke.md` records the evidence bar
for production-equivalent live probe smoke evidence and records the P8AJ manual
smoke tool. P8AL allows the readiness gate to consume a manually saved P8AJ
report only when it is explicitly supplied through
`--p8aj-live-probe-report <path>`. Tool existence alone is insufficient.
Accepted P8AJ evidence moves the `real_device_probing_not_cut_over` blocker
entry to `evidence_accepted`; it does not clear executor/apply or Python planner
deletion readiness, and the top-level readiness status remains `blocked`.
`docs/rust-default-route-mismatch-warning-parity.md` records the evidence bar
for future default-route mismatch-warning parity.
P8AK adds optional/manual fixture-backed mismatch-warning parity smoke tooling
for the explicit production-equivalent route. The tool can produce evidence for
`detected_device_profile_mismatch_warning_not_cut_over` when run manually, but
tool existence alone is insufficient. P8AL allows the readiness gate to consume
a manually saved P8AK report only when it is explicitly supplied through
`--p8ak-mismatch-warning-report <path>`. Accepted P8AK evidence moves the
`detected_device_profile_mismatch_warning_not_cut_over` blocker entry to
`evidence_accepted`; it does not clear executor/apply or Python planner
deletion readiness, and the top-level readiness status remains `blocked`.
`docs/rust-production-equivalent-route-implementation-plan.md` records the P8AG
implementation plan for an explicit production-equivalent route. P8AH
recognizes and reserves the backend name in the CLI parser, and P8AI wires it
as an explicit non-default Rust subprocess route backed by the supplied shadow
binary. It does not add required manual evidence, readiness-gate execution, or
blocker reclassification.
`docs/rust-default-planner-cutover-contract.md` records the default-backend
cutover contract. P8AM performs no cutover, P8AN is the evidence-preflight
phase, and P8AO is the default-route implementation phase after required
evidence is accepted.
`docs/rust-default-planner-binary-resolution.md` records the P8AQ future
binary-resolution design. P8AQ is design-only: `--rust-planner-bin <path>`
remains required, no packaged binary lookup, Cargo fallback, arbitrary `PATH`
search, host-path guessing, or silent Python fallback is implemented, and
packaged release readiness remains future work.
P8AR adds only an internal explicit-path resolver foundation for the existing
`--rust-planner-bin <path>` validation. It does not add packaged lookup, Cargo
fallback, arbitrary `PATH` search, environment-variable lookup, repo-local
guessing, or silent Python fallback, and it does not change readiness status.
`docs/rust-packaged-planner-binary-location.md` records the P8AS packaged
planner binary location contract. P8AS is documentation-only: no packaged lookup
is implemented, `--rust-planner-bin <path>` remains required, explicit
`--planner-backend python` remains available, no Cargo fallback, arbitrary
`PATH` search, env-var lookup, repo-local guessing, or silent Python fallback is
implemented, existing Tauri sidecar packaging is not automatically planner
binary packaging, and packaged release readiness remains future work.
P8AT adds an inert packaged resolver placeholder in the Python CLI. The
placeholder currently returns `None`; no packaged lookup is implemented,
`--rust-planner-bin <path>` remains required, explicit `--planner-backend
python` remains available, no Cargo fallback, `PATH` search, env-var lookup,
repo-local guessing, or silent Python fallback is implemented, and packaged
release readiness remains future work.
P8N adds a crate-local Rust probe abstraction, fake probe, and tests for layering
detected facts over synthetic/profile-derived context. P8O adds fake/test-backed
planner-input construction that applies detected facts over
synthetic/profile-derived context. P8P adds pure/test-backed Rust
detected-device profile mismatch warning foundation for supplied detected facts
and authored profile criteria; it evaluates the current Python criteria surface
of manufacturer, brand, model regex, and Android minimum version. Authored
Android maximum values are parsed but not evaluated because the current Python
warning path does not evaluate them. P8P does not implement live ADB probing,
route wiring, readiness gate behavior, normal runtime warning emission, or
current Python default planning behavior. Intended future context precedence is
synthetic/profile context -> detected facts -> explicit CLI overrides. Executor,
apply, real-device, ADB, artifact materialization, network, Tauri protocol,
normal runtime checks, and default user-facing CLI behavior are unchanged.
P8R adds an explicit local detected-facts fixture mode to the dev-only Rust
shadow binary with `--detected-facts-json <path>`. That mode reads strict local
JSON matching `DetectedDeviceFacts`, runs the fake/test-backed detected-facts
planning-result composition path, and emits the same shadow `PlanningResult`
JSON format as normal shadow planning. P8T lets only the explicit
`rust-experimental` Python route forward a local fixture path with
`--rust-detected-facts-json <path>`, which becomes
`--detected-facts-json <path>` for the supplied Rust shadow binary. Python does
not load or validate that fixture. P8R/P8T do not add live ADB probing,
`rust-shadow` fixture forwarding, route-level detection, executor/apply
behavior, Tauri/protocol behavior, normal runtime checks, readiness gate
reclassification, or default planner cutover.
P8S adds `tools/smoke_rust_detected_facts_fixture.py` as optional/manual smoke
evidence for that Rust shadow-binary fixture harness. The smoke invokes a
supplied `emuchef-plan-shadow` binary directly, creates temporary local fixture
files, and emits a deterministic report separate from the P8T Python
`rust-experimental` forwarding path. The smoke is not wired into normal checks
or the static readiness gate.
P8U adds `tools/smoke_rust_experimental_detected_facts_fixture.py` as
optional/manual smoke evidence for the P8T Python `rust-experimental`
fixture-forwarding route. The smoke creates temporary local fixture files,
invokes `python3 -m emuchef plan --planner-backend rust-experimental` with
`--rust-detected-facts-json <path>`, and requires Python-compatible summary
stdout rather than raw Rust JSON. It is separate from the direct P8S Rust
fixture smoke and is not wired into normal checks or the static readiness gate.
P8V adds a crate-local, pure/test-backed Rust ADB probe foundation in
`crates/emuchef-rust-backend/src/device_probe.rs`. It models the future
`adb [-s SERIAL] shell getprop` command as argv only and parses supplied
bracketed `getprop` stdout into `DetectedDeviceFacts` for the current Python
device-fact fields. P8V does not execute ADB, start subprocesses, read
environment variables, access the filesystem, access the network, wire live
probing into `rust-shadow`, `rust-experimental`, Python CLI routes,
Tauri/protocol, executor/apply, smoke runners, normal runtime checks, or the
static readiness gate. Fixture evidence from P8R/P8S/P8T/P8U remains separate.
P8W adds a crate-local live ADB probe adapter foundation on top of the P8V
command model and parser. It executes modeled argv only through an injectable
runner, keeps production subprocess use isolated to `ProcessCommandRunner`, and
uses fake runners for normal tests.
P8X wires that adapter only into the direct dev-only Rust shadow binary through
explicit `--probe-adb-getprop`, optional `--adb-path <path>`, and optional
`--serial <serial>` flags. The live mode is mutually exclusive with
`--detected-facts-json <path>` fixture mode, uses the same detected-facts
planning-result composition path, and remains separate from Python CLI routes,
`rust-shadow`, `rust-experimental`, Tauri/protocol, executor/apply, smoke
runners, normal runtime checks, readiness-gate execution, and default planner
cutover.
P8Y adds `tools/smoke_rust_shadow_live_adb_probe.py` as optional/manual smoke
evidence for the direct P8X Rust shadow live mode. The smoke requires explicit
`--rust-planner-bin`, `--adb-path`, and `--serial`, invokes the supplied Rust
shadow binary directly with `--probe-adb-getprop`, and emits deterministic JSON
with scrubbed command metadata only. It does not discover devices, run
`adb devices`, invoke Cargo, call Python CLI routes, mutate devices beyond
`adb shell getprop`, or wire into normal checks or readiness-gate execution.
When an intentionally selected device does not match the authored plan,
`device_profile_mismatch` is acceptable route evidence for this smoke only.
Production route-level probing and mismatch-warning parity remain cutover
blockers, and Python planner deletion remains future work.
P8Z lets only the explicit `rust-experimental` Python route forward live-probe
intent with `--rust-probe-adb-getprop`, `--rust-adb-path <path>`, and
`--rust-serial <serial>`. Python forwards those exact argparse strings to the
supplied Rust shadow binary as `--probe-adb-getprop`, `--adb-path <path>`, and
`--serial <serial>`. Python does not invoke ADB, discover devices, run
`adb devices`, parse `getprop`, or validate, normalize, expand, or stat the ADB
path or serial. P8AO no-backend default Rust routing and `rust-shadow` reject
the wrapper flags before ADB resolution, planner/session construction, device
probing, or subprocess execution. `--rust-detected-facts-json <path>` fixture forwarding and
live-probe forwarding are mutually exclusive detected-facts sources. P8Z is not
default planner cutover, not production route-level probing parity, not
readiness-gate executed evidence, and not Python planner deletion.
P8AI lets the explicit `rust-production-equivalent` Python route use the same
Rust-owned detected-facts fixture and live-probe wrapper inputs as
`rust-experimental`. It requires `--rust-planner-bin`, invokes the supplied Rust
shadow binary, always uses Python-compatible output, and keeps Python out of
ADB execution, device discovery, `getprop` parsing, planner session
construction, and apply work. P8AI is not smoke evidence, not readiness-gate
executed evidence, not default planner cutover, and not Python planner deletion.
P8AJ adds `tools/smoke_rust_production_equivalent_live_adb_probe.py` as
optional/manual smoke tooling for the explicit `rust-production-equivalent`
live-probe route. The smoke invokes the Python CLI route with the Rust
live-probe wrapper flags, requires Python-compatible output instead of raw Rust
JSON, emits deterministic JSON with scrubbed inputs including
`live_probe_requested: true`, and is not part of normal checks or
readiness-gate execution. The tool can produce production-equivalent live probe
evidence when run manually with real device inputs, but tool existence alone
does not clear `real_device_probing_not_cut_over`.
P8AK adds `tools/smoke_rust_production_equivalent_mismatch_warning.py` as
optional/manual fixture-backed smoke tooling for the separate mismatch-warning
evidence bar. The smoke invokes the same Python CLI production-equivalent route
with temporary `--rust-detected-facts-json` fixtures, requires Python-compatible
output instead of raw Rust JSON, and checks matched, manufacturer-mismatched,
model-mismatched, Android-minimum-mismatched, and Android-minimum-matching
cases. It does not run live probing, participate in normal checks, add
readiness-gate execution, or reclassify
`detected_device_profile_mismatch_warning_not_cut_over`.
P8AA adds `tools/smoke_rust_experimental_live_adb_probe.py` as optional/manual
smoke evidence for the P8Z Python `rust-experimental` live-probe forwarding
route. The smoke invokes `python3 -m emuchef plan --planner-backend
rust-experimental` with the Python wrapper probe flags and requires
Python-compatible route output instead of raw Rust JSON. It does not call the
Rust shadow binary directly, discover devices, run `adb devices`, invoke Cargo,
or inspect, normalize, expand, or stat the supplied ADB path or serial.
`device_profile_mismatch` is acceptable route evidence when the selected live
device intentionally does not match the authored plan. P8AA is not default
planner cutover, not production route-level probing parity, not readiness-gate
executed evidence, and not Python planner deletion. The
`real_device_probing_not_cut_over` and
`detected_device_profile_mismatch_warning_not_cut_over` blockers remain blocked.
For the consolidated P8X-P8AA live-probe evidence, default-route,
production-route, and readiness-gate gap summary, see
`docs/rust-live-probe-evidence-and-cutover-gap.md`.

P8I adds `tools/check_rust_planner_cutover_readiness.py` as a static,
developer-only readiness gate for any future PR that proposes making Rust the
default `emuchef plan` backend. The gate verifies current static prerequisites,
lists required manual/live evidence commands, can classify explicitly supplied
P8AJ/P8AK JSON reports, and reports remaining default cutover blockers. It does
not run live comparison or smoke tooling, Cargo, npm, ADB, executor/apply,
Tauri/protocol checks, device probing, network access, artifact materialization,
fixture/golden regeneration, or Python planner deletion work. P8AP updates the
gate after P8AO so the historical `default_cli_backend_still_python` entry is
preserved with status `resolved`. The top-level report status remains `blocked`
until future phases intentionally clear executor/apply and Python planner
deletion blockers.

## Current Evidence

The Rust planner evidence is planner-only and migration-focused:

- `crates/emuchef-rust-backend/src/planner.rs` and
  `crates/emuchef-rust-backend/src/planner_tests.rs` cover the P7B-P7O behavior
  span for the private Rust planner surface: Python-shaped
  `PlanningResult`/`ExecutionPlan` values, selected recipe expansion, selected
  emitted-step dependencies, focused `steps.*` ref handling, focused emitted
  param contract checks, internal permission-intent construction from
  step-local `grant_permissions` params, DTO shape checks, authored-corpus
  parsing/planning, checked-in device-plan/profile ingestion,
  defaults/overrides classification, and repo-plan E2E composition.
- `crates/emuchef-rust-backend/src/plan_shadow.rs`,
  `crates/emuchef-rust-backend/src/bin/emuchef-plan-shadow.rs`, and
  `crates/emuchef-rust-backend/tests/phase7m_plan_shadow.rs` provide the P7M
  dev-only shadow planner command. The command builds
  `PlannerInput::from_authored_device_plan(...)`, calls `plan_execution(...)`,
  and emits pretty JSON planner results for explicit authored-root/device-plan
  inputs.
- `tools/compare_rust_python_plan.py` and
  `tests/test_compare_rust_python_plan.py` provide the P7N dev-only comparison
  harness for Python planner API output versus Rust `emuchef-plan-shadow`
  output under a shared synthetic/profile-derived planner context.
- `src/emuchef/cli.py` exposes a P8A explicit developer-only bridge:
  `emuchef plan --planner-backend rust-shadow --rust-planner-bin <path>`.
  The route forwards authored-root, device-plan, and repeated raw `--bind`
  values to the supplied `emuchef-plan-shadow` binary and passes through Rust
  JSON stdout/stderr without translating it into Python planner YAML or summary
  output. This is not a cutover and does not make Rust the default planner.
- `tools/smoke_rust_shadow_cli_matrix.py` and
  `tests/test_smoke_rust_shadow_cli_matrix.py` provide P8B dev-only smoke
  evidence for the explicit Python CLI bridge across the current scenario
  matrix. The smoke requires a supplied `--rust-planner-bin`, never invokes
  Cargo, and proves route invocation only; P7P remains the Python-vs-Rust
  planner-output comparison evidence.
- `tests/test_cli.py` guards the P8C output compatibility contract for the
  explicit Python CLI `rust-shadow` bridge. The bridge is Rust stdout/stderr/exit
  code passthrough; it does not translate Rust JSON `PlanningResult` output into
  Python YAML, Python concise planning summary text, or Python planner
  structures. Default and explicit `--planner-backend python` planning remain
  Python-owned for output and exit-code behavior.
- `docs/adr/0002-rust-planner-cli-output-compatibility.md` records the P8D
  forward-looking default-route decision. Current `rust-shadow` remains JSON
  passthrough and dev-only, while any future default Rust planner route must
  preserve Python concise summary output, Python `--verbose` structured YAML,
  Python `--output` YAML file behavior, and Python exit-code behavior unless a
  separate accepted breaking-change decision replaces that target. Rust-native
  JSON requires a future explicit structured-output mode such as `--format json`.
- `src/emuchef/cli.py` also exposes the P8E explicit
  `--rust-shadow-output python-compatible` formatter mode for the existing
  dev-only `rust-shadow` bridge. Omitted `--rust-shadow-output` and
  `--rust-shadow-output passthrough` preserve P8C JSON passthrough and text
  passthrough.
  `python-compatible` parses usable Rust `PlanningResult` JSON, formats concise
  Python-compatible CLI summary labels, emits structured YAML through the
  existing `dump_yaml(...)` mapping path for `--verbose`, and writes that YAML to
  `--output` paths while printing the concise summary. This is a formatter bridge
  over Rust JSON, not Python planner object reconstruction and not default Rust
  planner cutover.
- `tools/smoke_rust_shadow_cli_matrix.py` also provides P8F dev-only matrix smoke
  evidence for the explicit `rust-shadow` bridge when
  `--rust-shadow-output python-compatible` is selected. In that mode, successful
  scenarios must exit `0` and classify stdout as the concise Python-compatible
  planning summary. Raw Rust JSON stdout remains `stdout_json` and fails the
  compatibility-mode smoke for successful scenarios. P8F proves CLI-route
  invocation plus compatibility-format smoke across the current matrix; it does
  not replace P7P planner DTO/result comparison evidence or make Rust the
  default planner.
- `src/emuchef/cli.py` exposes the P8G explicit non-default
  `--planner-backend rust-experimental` migration route. It requires
  `--rust-planner-bin`, reuses the same Rust shadow planner command construction
  as `rust-shadow`, always uses the Python-compatible formatter path, and accepts
  `--verbose` and `--output` through that formatter. `--rust-shadow-output` is
  valid only with `--planner-backend rust-shadow`; Python and
  `rust-experimental` reject it before ADB resolution or Python planner/session
  construction. P8G is cutover rehearsal routing evidence only. It did not change
  the then-current planner owner, executor/apply behavior, real-device behavior,
  Tauri/protocol behavior, Cargo fallback behavior, fixture/golden ownership, or
  normal runtime checks. Its name and behavior are not the default route
  contract.
- `tools/smoke_rust_shadow_cli_matrix.py` provides P8H dev-only matrix smoke
  evidence for `--planner-backend rust-experimental`. It reuses the existing
  route backend and effective output-mode machinery, generates commands without
  `--rust-shadow-output`, reports `route_backend: rust-experimental` and
  `route_output_mode: python-compatible`, and requires successful scenarios to
  exit `0` with stdout classified as `python_summary`. Raw Rust JSON remains
  `stdout_json` and fails the P8H smoke for successful scenarios.
- `tools/check_rust_planner_cutover_readiness.py` provides the P8I static
  readiness gate for future default Rust planner proposals. It is stdlib-only,
  imports no planner/runtime modules, derives checked-in device-plan coverage
  from `authored/device_plans/*.yaml` and `authored/device_plans/*.yml`
  filenames, verifies durable references in this document, verifies stable CLI
  backend tokens in `src/emuchef/cli.py`, and emits deterministic JSON. Its
  `required_manual_evidence` entries are advisory commands that must be run
  before a future default-cutover PR can be evaluated; the gate does not execute
  or claim those commands passed.
- P8J adds explicit device context flag support to the explicit Rust planner
  routes only. `emuchef-plan-shadow` accepts `--manufacturer`, `--model`,
  `--android-version`, and repeated `--device-tag` values; Python
  `rust-shadow` and `rust-experimental` routes forward only values supplied on
  the command line. These values override or replace the private
  synthetic/profile-derived Rust planner context for that invocation. P8J does
  not add ADB probing, detected-device facts, detected-device profile mismatch
  warnings, default Rust planner routing, executor/apply behavior, Tauri/protocol
  behavior, Cargo fallback behavior, fixture/golden regeneration, or Python
  planner deletion readiness.
- P8K extends the dev-only scenario matrix, comparison harness, smoke runner, and
  static readiness gate with optional explicit `device_context` evidence. Matrix
  context values are forwarded to the hidden Python planner worker and to Rust
  shadow commands using the existing P8J flags. Reports include stable context
  presence/key metadata only, not full context values. This is matrix evidence
  for supplied context values, not ADB/device probing, detected-device facts, or
  profile mismatch warning support. Because the P8J CLI flag surface cannot
  encode an explicit empty tag override separately from omitted tags, matrix
  validation rejects `device_tags: []`; omitted `device_tags` means no tag
  override and non-empty `device_tags` replace profile-derived tags in order.
- P8L classifies explicit device context support as statically covered only when
  the scenario matrix includes at least one valid `device_context` scenario with
  at least one meaningful explicit context field. The readiness gate keeps
  optional `device_context` schema validation separate from coverage evidence:
  an empty `device_context: {}` remains schema-valid but does not satisfy
  explicit-context readiness coverage. This reclassification narrows the former
  broad real-device context blocker into separate unresolved blockers for real
  device probing and detected-device profile mismatch warning parity.
- `docs/adr/0003-rust-real-device-context-ownership.md` records the P8M
  forward-looking ownership decision for future default Rust planner cutover:
  Rust should own real-device probing and detected-device profile mismatch
  warning parity. P8M does not implement ADB probing, detected-device facts,
  mismatch-warning behavior, default Rust planner routing, executor/apply
  behavior, Tauri/protocol behavior, Cargo fallback behavior, fixture/golden
  regeneration, normal runtime-check wiring, or Python planner deletion
  readiness.
- `docs/adr/0004-default-route-live-probe-cutover-design.md` records the P8AC
  default-route live-probe cutover design. Future default-route live probing
  should be Rust-owned, explicit migration-route evidence remains migration
  evidence, and the live-probing plus mismatch-warning readiness blockers stay
  blocked until production/default-route evidence exists.
- `docs/rust-production-equivalent-live-probe-smoke.md` records the P8AE
  evidence bar and P8AJ optional/manual smoke tooling for production-equivalent
  live probe evidence. It does not add readiness-gate execution or reclassify
  blockers.
- `docs/rust-default-route-mismatch-warning-parity.md` records the P8AF
  evidence bar for future default-route mismatch-warning parity. It does not add
  readiness-gate execution or reclassify blockers. P8AK adds optional/manual
  fixture-backed production-equivalent smoke tooling for this separate evidence
  bar, but that tooling is not required by the readiness gate.
- `docs/rust-production-equivalent-route-implementation-plan.md` records the
  P8AG implementation plan for an explicit production-equivalent route. P8AI
  wires that route as executable, explicit, non-default,
  Rust-shadow-binary-backed, Python-compatible route plumbing only. P8AJ and
  P8AK add optional/manual smoke tooling only. These phases do not add
  readiness-gate execution or reclassify blockers.
- `tools/plan_parity_scenarios.json` is the P7P scenario matrix for the current
  checked-in device-plan scenarios plus P8K explicit-context evidence. The
  current checked-in scenario matrix expects all six scenarios to classify as
  `match`.
- The five current scenario ids are `ayaneo_konkr_pocket_fit_base`,
  `ayaneo_pocket_s_mini_base`, `ayaneo_generic_base`,
  `ayaneo_pocket_air_mini_base`, and `ayaneo_pocket_s2_base`. The additional
  explicit-context scenario id is
  `ayaneo_pocket_s_mini_base_explicit_context`.
- A matching matrix means the dev-only compared fields align for current
  scenarios; it does not prove default CLI cutover, real-device CLI context
  resolution, executor/apply compatibility, artifact materialization, or Python
  planner deletability.
- `docs/python-fixture-golden-ownership.md`, `CONTEXT.md`, and
  `crates/emuchef-rust-backend/README.md` define the no-Python-runtime,
  no-PySide-runtime, and no-Python-fixture/golden-regeneration guard
  boundaries. Normal Rust/Tauri active checks may consume checked-in fixtures
  and goldens, but they do not run Python regeneration.

## Current Non-Goals

Current evidence does not prove:

- Python `emuchef draft` CLI cutover.
- Executor/apply parity.
- Packaged default Rust planner readiness without an explicit
  `--rust-planner-bin`.
- Real-device probing parity outside accepted production-equivalent route
  evidence.
- ADB behavior.
- Artifact download, extraction, copy, install, or materialization behavior.
- Network behavior.
- Tauri protocol, command, or UI integration for planning.
- Full schema parity or future authored scenario parity.
- Python planner deletion.

## Rust Planner User-Facing Cutover Blockers

Before broader Rust planner ownership, packaging, or Python planner deletion can
be claimed, these blockers must be resolved or explicitly accepted for a
narrower route:

| Blocker | Current classification |
| --- | --- |
| CLI routing strategy | P8AO makes no-backend `emuchef plan` use Rust-owned planning through the existing production-equivalent subprocess path with Python-compatible output. The transition route still requires a supplied `--rust-planner-bin` and does not invoke Cargo or search host paths. Explicit `--planner-backend python` remains the fallback for the previous Python planner route. P8A/P8E keep the explicit `rust-shadow` developer bridge and formatter behavior, P8G keeps `rust-experimental` as an explicit migration route, and P8AI keeps `rust-production-equivalent` as an explicit evidence route. P8AP preserves the historical `default_cli_backend_still_python` readiness entry with status `resolved`; it does not clear executor/apply, packaged-release, or Python planner deletion readiness. |
| Device probing and context resolution | The P8AN preflight supplies accepted P8AJ live-probe evidence, so `real_device_probing_not_cut_over` can be reported as `evidence_accepted` when those report paths are supplied to the readiness gate. P8AO no-backend default routing still keeps detected-facts fixture and live-probe wrapper flags explicit-only; they remain accepted on the explicit production-equivalent route. Explicit `--manufacturer`, `--model`, `--android-version`, and repeated `--device-tag` values are forwarded to Rust planning. Python does not invoke ADB, discover devices, run `adb devices`, parse `getprop`, or validate/normalize forwarded ADB paths or serials for Rust routes. |
| Argument and binding parity | `emuchef-plan-shadow` accepts explicit `--authored-root`, `--device-plan`, explicit device context flags, and string `--bind` values. It mirrors repeated-bind grouping and ordered repeated device tags, but is not full future Rust CLI binding type parity, ops replay parity, detected-device parity, or common-flag parity. |
| Output format compatibility | P8AO default Rust routing uses the Python-compatible formatter over Rust `PlanningResult` JSON: concise summary labels mirror the visible Python CLI labels, structured YAML is produced from the Rust JSON mapping through `dump_yaml(...)`, and `--output` writes that YAML while stdout stays concise unless `--verbose` is selected. `rust-shadow` omitted `--rust-shadow-output` and explicit `passthrough` continue to preserve Rust stdout/stderr/exit-code passthrough. Rust-native JSON still requires a future explicit structured-output mode such as `--format json`. |
| Error and warning compatibility | Rust covers selected planner result, warning/error shape, and focused diagnostics. P8AN supplies accepted P8AK fixture-backed production-equivalent route smoke evidence for matched and mismatched warning behavior, so `detected_device_profile_mismatch_warning_not_cut_over` can be reported as `evidence_accepted` when those report paths are supplied to the readiness gate. Broader planner diagnostics, operation replay parity, and Python planner deletion readiness remain separate work. |
| Required normal-check gating | The P7P comparison matrix and P8B CLI-route smoke are not part of normal Rust/Tauri checks. A cutover route needs an approved gate policy before the route becomes user-facing. |
| Unsupported scenarios outside the matrix | The checked-in matrix covers all five current device plans with six current scenarios, including one P8K explicit-context scenario for `ayaneo.pocket_s_mini.base`. Future authored plans, non-empty recipe dependencies, broader override forms, profile matching, and scenario drift require intentional coverage updates. |
| Authored/device-plan drift | `tools/plan_parity_scenarios.json` and this readiness doc must be updated when checked-in scenarios change. The static doc guard only checks scenario id/tool references. |
| Matrix as cutover gate | Matching matrix status is necessary evidence for the current compared fields, not sufficient cutover readiness. A routing PR should decide whether matrix execution becomes required for that PR. |

### Comparison Matrix Gating Policy

- Current state: `tools/plan_parity_scenarios.json`,
  `tools/compare_rust_python_plan.py`, and
  `tools/smoke_rust_shadow_cli_matrix.py` are dev-only/manual artifacts and are
  not part of normal Rust/Tauri checks.
- P8B smoke state: `tools/smoke_rust_shadow_cli_matrix.py` creates only
  planner-visible placeholder binding resources, runs the explicit Python CLI
  `rust-shadow` route for each current scenario, classifies stdout/stderr
  stably, and emits deterministic JSON route-invocation evidence. It does not
  compare Python and Rust planner outputs.
- P8C contract state: `tests/test_cli.py` guards the explicit route's default
  CLI output boundary. Omitted `--rust-shadow-output` and explicit
  `--rust-shadow-output passthrough` pass through Rust stdout, stderr, and exit
  code; Python YAML, summary text, `--output`, and `--verbose` are outside that
  passthrough contract.
- P8D decision state: `docs/adr/0002-rust-planner-cli-output-compatibility.md`
  records that future default Rust planner routing must preserve the current
  Python `emuchef plan` output and exit-code contract unless a separate accepted
  breaking-change decision says otherwise. This is a decision only; it does not
  implement the formatter/translation layer and does not make Rust default.
- P8E formatter state: `emuchef plan --planner-backend rust-shadow
  --rust-planner-bin <path> --rust-shadow-output python-compatible` is an
  explicit dev-only bridge from Rust `PlanningResult` JSON to Python-compatible
  CLI summary labels and structured YAML. The default `rust-shadow` behavior
  remains passthrough, Python remains the default planner, and executor/apply,
  real-device, ADB, artifact, network, Tauri, protocol, Cargo fallback, and
  Python planner deletion behavior remain unchanged.
- P8F compatibility-smoke state: `tools/smoke_rust_shadow_cli_matrix.py
  --rust-shadow-output python-compatible` runs the same explicit Python CLI
  `rust-shadow` route across the current scenario matrix and requires successful
  scenarios to emit concise Python-compatible summary stdout. It never calls
  Cargo, the P7P comparison harness, executor/apply, ADB, Tauri/protocol,
  network, artifact materialization, fixture/golden regeneration, or normal
  Rust/Tauri runtime checks.
- P8G experimental-route state: `emuchef plan --planner-backend
  rust-experimental --rust-planner-bin <path>` reuses the Rust shadow planner
  invocation and defaults to Python-compatible output formatting. It remains
  explicit opt-in, non-default, and non-stable.
- P8H experimental-route smoke state: `tools/smoke_rust_shadow_cli_matrix.py
  --planner-backend rust-experimental` runs the same current scenario matrix
  through the explicit non-default migration route. Generated commands omit
  `--rust-shadow-output`, report an effective Python-compatible output mode, and
  require successful scenarios to exit `0` and emit concise Python-compatible
  summary stdout. This extends the existing smoke runner without renaming it or
  wiring it into normal runtime checks.
- P8I static-readiness state: `tools/check_rust_planner_cutover_readiness.py
  --authored-root authored --scenario-matrix tools/plan_parity_scenarios.json`
  checks static prerequisites for any future default Rust planner PR and emits a
  deterministic JSON report. The report lists the P7P comparison matrix command,
  P8H `rust-experimental` smoke command, focused Python tests, and Rust/Tauri
  checks as required manual evidence, but it does not execute them. The report
  remains `blocked` even when static checks pass because the default CLI backend,
  executor/apply, real-device probing, detected-device profile mismatch warning
  parity, and Python planner deletion blockers remain unresolved.
- P8J explicit-context state: `emuchef plan --planner-backend rust-shadow
  --rust-planner-bin <path>` and `emuchef plan --planner-backend
  rust-experimental --rust-planner-bin <path>` accept explicitly supplied
  `--manufacturer`, `--model`, `--android-version`, and repeated `--device-tag`
  values and forward them to `emuchef-plan-shadow`. The shadow command applies
  those values to the private Rust planner `DeviceContext`; explicit tags replace
  profile-derived tags only when at least one `--device-tag` is supplied. This is
  explicit input support only. ADB/device probing and detected-device profile
  mismatch warnings remain unsupported; P8L narrows the old broad context
  blocker into `real_device_probing_not_cut_over` and
  `detected_device_profile_mismatch_warning_not_cut_over`, both still blocked.
- P8K explicit-context matrix state: `tools/plan_parity_scenarios.json` supports
  optional `device_context` objects for dev-only comparison and smoke evidence.
  The checked-in matrix includes
  `ayaneo_pocket_s_mini_base_explicit_context` for
  `ayaneo.pocket_s_mini.base` with supplied manufacturer, model, Android version,
  and ordered tags. The comparison harness forwards those values to both the
  hidden Python planner worker and the Rust shadow command; the smoke runner
  forwards them to generated `emuchef plan --planner-backend rust-shadow` or
  `rust-experimental` commands. Reports record only stable context presence and
  key names. This does not add ADB/device probing, detected-device facts,
  executor/apply behavior, Tauri/protocol behavior, Cargo fallback behavior,
  fixture/golden regeneration, normal runtime-check wiring, or Python planner
  deletion readiness.
- P8L explicit-context readiness state: the P8I static gate now emits separate
  checks for optional `device_context` schema support and meaningful
  explicit-context coverage. `explicit_context_supported_by_matrix_schema`
  describes the supported matrix fields, while
  `explicit_context_scenario_present` and `explicit_context_scenario_valid`
  distinguish meaningful supplied-field coverage from schema-valid supplied-field
  coverage.
  The gate still lists manual comparison/smoke commands as advisory only and does
  not run them. Its top-level report remains `blocked` even when all static
  explicit-context checks pass because default backend ownership, real-device
  probing, detected-device profile mismatch warning parity, executor/apply, and
  Python planner deletion readiness remain unresolved.
- P8M real-device context ownership decision: `docs/adr/0003-rust-real-device-context-ownership.md`
  records that Rust should own real-device probing and detected-device profile
  mismatch warning parity for future default Rust planner cutover. Python
  remains the current default/reference planner implementation, Rust routes
  remain explicit and non-default, explicit context support remains separate from
  real-device probing, and the readiness gate remains blocked until Rust probing
  and mismatch-warning evidence exists.
- P8N fake probe foundation:
  `crates/emuchef-rust-backend/src/device_probe.rs` defines a crate-local probe
  trait, stable detected-facts/error types, fake probe, and helper for applying
  detected facts over planner `DeviceContext`. It is fake/test-only foundation.
- P8O detected-facts planner-input construction:
  `crates/emuchef-rust-backend/src/planner_device_plan.rs` adds crate-private
  fake/test-backed construction of `PlannerInput` with detected facts layered
  over synthetic/profile-derived `DeviceContext`. Intended future context
  precedence is synthetic/profile context -> detected facts -> explicit CLI
  overrides. It does not implement live ADB probing, detected-device profile
  mismatch warning parity, `rust-shadow` or `rust-experimental` route wiring,
  Python CLI behavior, Tauri/protocol behavior, executor/apply behavior,
  readiness gate reclassification, or Python planner deletion readiness.
- P8P detected-device profile mismatch warning foundation:
  `crates/emuchef-rust-backend/src/device_profile_match.rs` adds pure
  crate-private warning construction for supplied detected facts and authored
  profile criteria. `crates/emuchef-rust-backend/src/planner_device_plan.rs`
  exposes private criteria loading for tests and future wiring. This is not live
  probing, not normal planner warning emission, not `rust-shadow` or
  `rust-experimental` route wiring, not readiness gate reclassification, and not
  Python planner deletion readiness. Real-device probing and mismatch-warning
  parity remain blocked until live implementation and route evidence exist.
- P8Q detected-facts planning-result composition:
  `crates/emuchef-rust-backend/src/planner_device_plan.rs` adds crate-private
  fake/test-backed construction of a `PlanningResult` from authored
  device-plan/profile data and supplied detected facts. It uses the P8O
  detected-context input path, runs the private Rust planner, and appends the
  P8P `device_profile_mismatch` warning only in this helper/test path. It does
  not implement live ADB probing, normal planner warning emission,
  `rust-shadow` or `rust-experimental` route wiring, Python CLI behavior,
  Tauri/protocol behavior, executor/apply behavior, readiness gate
  reclassification, normal runtime checks, or Python planner deletion readiness.
  Real-device probing and route-level mismatch-warning parity remain blocked
  until live implementation and route evidence exist.
- P8R/P8T detected-facts fixture forwarding:
  `emuchef-plan-shadow --detected-facts-json <path>` reads a local strict JSON
  `DetectedDeviceFacts` fixture and routes the invocation through the P8Q
  planning-result composition helper. It preserves existing shadow
  stdout/stderr/exit-code conventions, preserves `--bind` handling, keeps
  explicit context separate from detected facts, and applies explicit context
  values to the emitted `execution_plan.device_context` after fixture facts.
  The mismatch warning still evaluates the detected fixture facts. P8T exposes
  a Python wrapper flag only for `--planner-backend rust-experimental`:
  `--rust-detected-facts-json <path>` forwards the exact argparse string as
  `--detected-facts-json <path>`. P8AI exposes the same wrapper input for the
  explicit `rust-production-equivalent` route. P8AO keeps this wrapper input
  explicit-only: no-backend default Rust routing rejects it before ADB
  resolution, planner/session construction, or subprocess execution. The
  `rust-shadow` route also rejects the Python wrapper flag, and the raw Rust
  `--detected-facts-json` flag remains unrecognized by Python CLI routes. This
  forwarding is not
  included in the Python CLI matrix smoke runner, not wired into route-level
  detection, and not live ADB probing. Real-device probing and production
  route-level mismatch-warning parity remain blocked.
- P8S detected-facts fixture smoke evidence:
  `tools/smoke_rust_detected_facts_fixture.py` directly invokes a supplied
  `emuchef-plan-shadow` binary with temporary `--detected-facts-json` fixture
  files for matching, mismatching, and explicit-context override cases. It is
  stdlib-only, deterministic, optional/manual evidence for the P8R fixture
  harness. It does not call the Python planner, the Python CLI Rust routes, the
  comparison harness, the matrix smoke runner, ADB, Cargo, executor/apply,
  Tauri/protocol, network, fixture/golden regeneration, normal runtime checks,
  or the static readiness gate.
- P8U rust-experimental fixture-route smoke evidence:
  `tools/smoke_rust_experimental_detected_facts_fixture.py` invokes the Python
  CLI as `python3 -m emuchef plan --planner-backend rust-experimental` with
  temporary local fixtures passed through `--rust-detected-facts-json <path>`.
  It is stdlib-only, deterministic, optional/manual evidence for the Python
  forwarding route added in P8T. It expects Python-compatible summary stdout,
  treats raw Rust JSON as a route-smoke failure, validates the mismatching
  output-file case with text checks only, and records no temp paths or volatile
  stdout/stderr. It does not add live ADB probing, expose fixture forwarding
  through no-backend default planning or `rust-shadow`, call the direct P8S smoke,
  change readiness-gate blocker IDs, or run as a normal check.
- P8V ADB probe parser foundation:
  `crates/emuchef-rust-backend/src/device_probe.rs` models a future
  `adb [-s SERIAL] shell getprop` command as argv only and parses supplied
  bracketed `getprop` text into `DetectedDeviceFacts`. It covers manufacturer,
  brand, model, Android release, and SDK fields without executing ADB, starting
  subprocesses, reading environment variables, using filesystem or network APIs,
  or wiring live probing into any CLI route, smoke runner, Tauri/protocol,
  executor/apply path, normal runtime check, or readiness-gate execution.
- P8W ADB probe adapter foundation:
  `crates/emuchef-rust-backend/src/device_probe.rs` adds `AdbDeviceProbe` with
  an injectable command runner and a `ProcessCommandRunner` that executes the
  modeled argv directly. Normal tests use fake runners and do not require ADB.
  P8X wires this adapter only into the direct dev-only Rust shadow binary. That
  route-level probing is explicit/manual shadow-binary foundation; it is not
  smoke-runner probing, readiness-gate execution evidence, default planner
  routing, executor/apply behavior, Python CLI behavior, or production
  mismatch-warning parity.
- P8Y direct live-probe smoke evidence:
  `tools/smoke_rust_shadow_live_adb_probe.py` invokes only a supplied
  `emuchef-plan-shadow` binary with `--probe-adb-getprop`, `--adb-path`, and
  `--serial`. It does not run Python CLI routes, Cargo, `adb devices`,
  comparison tooling, matrix smoke tooling, fixture smoke tooling,
  executor/apply, Tauri/protocol, normal runtime checks, or readiness-gate
  execution. Its report contains stable classifications and scrubbed metadata;
  it never records the raw serial, full command, full stdout/stderr, absolute
  local paths, raw ADB output, timestamps, durations, environment variables, or
  process ids.
- P8Z rust-experimental live-probe forwarding:
  `emuchef plan --planner-backend rust-experimental` accepts
  `--rust-probe-adb-getprop`, `--rust-adb-path <path>`, and
  `--rust-serial <serial>` only when all three are supplied and no
  `--rust-detected-facts-json <path>` fixture source is present. Python forwards
  them as `--probe-adb-getprop`, `--adb-path <path>`, and `--serial <serial>` to
  the supplied Rust shadow binary, then keeps using Python-compatible output
  formatting. Python does not invoke ADB, discover devices, run `adb devices`,
  parse `getprop`, or validate, normalize, expand, or stat the forwarded ADB path
  or serial. P8AO keeps these wrapper flags explicit-only: no-backend default
  Rust routing and `rust-shadow` reject them before ADB resolution,
  planner/session construction, device probing, or subprocess execution. This
  is explicit migration/evidence-route forwarding, not Python planner deletion.
- Pre-cutover candidate: planner routing work may use the matrix as an
  optional/manual gate to gather evidence before exposing any user-facing Rust
  planner path.
- Cutover candidate: PRs that route user-facing planning to Rust should make the
  matrix, or an approved successor, a required gate for supported scenarios.
- Deletion candidate: before Python planner deletion, the Python-dependent
  harness must be retired, converted to historical evidence, replaced with
  frozen reports, or replaced with another non-Python validation strategy.

## Python Planner Deletion Blockers

Before Python planner source can be removed, these blockers must be resolved:

| Blocker | Current classification |
| --- | --- |
| Python planner tests | `tests/test_planner_core.py`, planner-facing cases in `tests/test_cli.py`, `tests/test_validation.py`, and `tests/test_step_plugins.py` remain active reference coverage. They must be ported, retired, or reclassified as historical evidence. |
| Fixture/golden generation references | Phase 6M/6N planner goldens remain Python planner reference evidence with dev-only/reference-only generation commands documented in `crates/emuchef-rust-backend/README.md` and classified in `docs/python-fixture-golden-ownership.md`. |
| Docs and README references | Current docs still name Python as CLI/reference owner. These references must be updated only after replacement or retirement is actually complete. |
| Python CLI dependencies | `pyproject.toml` still exposes the `emuchef` console script through `src/emuchef/cli.py`. `draft` and `plan` still use Python planner modules. |
| Executor/apply output-shape dependencies | Python executor/apply consumes execution-plan and planning-result shapes. Rust planner deletion readiness must preserve or deliberately replace those contracts. |
| P7N/P7P comparison harness dependency | `tools/compare_rust_python_plan.py` depends on the Python planner API. Before Python planner deletion, it must be retired, converted to historical evidence, replaced with frozen reports, or replaced with another non-Python validation strategy. |
| Remaining Python planner APIs outside CLI | Imports and usage of `src/emuchef/planner/*` and domain planning models must be audited. Any remaining user-facing, test, docs, fixture, or developer-tool dependency must be ported or intentionally retired. |

## Proposed Cutover Ladder

1. Shadow parity remains maintained for the current checked-in scenario matrix.
2. The dev-only Rust planner command remains validated against the matrix for
   current scenarios.
3. Planner routing work may add an optional/manual matrix gate before exposing a
   user-facing route.
4. A developer-only explicit Rust shadow planner route is available for manual
   inspection with a supplied `emuchef-plan-shadow` binary, and a dev-only smoke
   runner can exercise that route across the current scenario matrix.
5. Explicit Rust planner routes remain available only for their supported
   migration/evidence scenarios.
6. P8AO makes no-backend `emuchef plan` use Rust-owned planning through the
   existing production-equivalent subprocess path while requiring
   `--rust-planner-bin` during the transition.
7. Python planner remains available as explicit fallback/reference while unsupported
   scenarios, diagnostics, CLI output, and deletion blockers are retired.
8. Python planner tests, docs, and fixture/golden references are ported,
   retired, or converted to historical evidence.
9. Python planner code is deleted only after no user-facing, test, fixture,
   docs, or developer-tool dependency still requires it.
