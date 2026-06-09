# Rust Planner Cutover Readiness

This document classifies current evidence and remaining blockers before Rust
planner ownership can move into a user-facing planner route or before the Python
planner can be removed. Python remains the current CLI/reference planner owner.
Rust planner output remains shadow/dev-only. The default `emuchef plan` route
remains Python-owned; the only Python CLI route to Rust planning is the explicit
developer-only `--planner-backend rust-shadow --rust-planner-bin <path>` path
and the explicit non-default migration route
`--planner-backend rust-experimental --rust-planner-bin <path>`.
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
future default Rust planner cutover. P8M records that decision only; it does not
implement probing or change current Python default planning behavior.
Executor, apply, real-device, ADB, artifact materialization, network, Tauri
protocol, and default user-facing CLI behavior are unchanged.

P8I adds `tools/check_rust_planner_cutover_readiness.py` as a static,
developer-only readiness gate for any future PR that proposes making Rust the
default `emuchef plan` backend. The gate verifies current static prerequisites,
lists required manual/live evidence commands, and reports remaining default
cutover blockers. It does not run live comparison or smoke tooling, Cargo, npm,
ADB, executor/apply, Tauri/protocol checks, device probing, network access,
artifact materialization, fixture/golden regeneration, or Python planner
deletion work. Its top-level report status is expected to remain `blocked` until
future phases intentionally clear default-cutover blockers.

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
  construction. P8G is cutover rehearsal routing evidence only. It does not change
  default Python planner ownership, executor/apply behavior, real-device
  behavior, Tauri/protocol behavior, Cargo fallback behavior, fixture/golden
  ownership, or normal runtime checks. Its name and behavior may change before
  Rust becomes the default planner backend.
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

- Default Python `emuchef plan` CLI cutover.
- Python `emuchef draft` CLI cutover.
- Executor/apply parity.
- Real-device probing parity.
- ADB behavior.
- Artifact download, extraction, copy, install, or materialization behavior.
- Network behavior.
- Tauri protocol, command, or UI integration for planning.
- Full schema parity or future authored scenario parity.
- Python planner deletion.

## Rust Planner User-Facing Cutover Blockers

Before a user-facing route can use the Rust planner, these blockers must be
resolved or explicitly accepted for a narrower experimental route:

| Blocker | Current classification |
| --- | --- |
| CLI routing strategy | Python `src/emuchef/cli.py` remains the current default `draft` and `plan` route. P8A adds an explicit dev-only `emuchef plan --planner-backend rust-shadow --rust-planner-bin <path>` bridge. It requires a supplied shadow binary, never invokes Cargo, passes through Rust JSON stdout/stderr by default, and is not a replacement command path or fallback policy. P8B adds a dev-only matrix smoke for that bridge's raw passthrough invocation path, and P8F adds dev-only matrix smoke for the same bridge with explicit `--rust-shadow-output python-compatible`; neither makes Rust authoritative. P8G adds `emuchef plan --planner-backend rust-experimental --rust-planner-bin <path>` as an explicit non-default migration route that reuses the Rust shadow planner invocation and Python-compatible formatter by default. P8H adds dev-only matrix smoke evidence for that explicit route across the current scenario matrix. `rust-experimental` is a cutover rehearsal route, not the default planner, not a stable final public contract, and not Python planner deletion. Its name and behavior may change before Rust becomes the default planner backend. |
| Device probing and context resolution | Python CLI resolves ADB/device facts before planning in `_resolve_device_context(...)`. P8J lets the explicit Rust routes accept supplied `--manufacturer`, `--model`, `--android-version`, and repeated `--device-tag` values, and P8K/P8L provide matrix schema/tooling plus static coverage evidence for meaningful supplied `device_context` scenarios. P8M accepts the future-cutover ownership decision that Rust should own real-device probing and detected-device mismatch-warning parity, but it does not implement probing. Rust shadow planning still does not probe real devices or create detected-device facts. The former broad `real_device_context_probing_not_cut_over` blocker is narrowed, not resolved: `real_device_probing_not_cut_over` remains blocked. |
| Argument and binding parity | `emuchef-plan-shadow` accepts explicit `--authored-root`, `--device-plan`, explicit device context flags, and string `--bind` values. It mirrors repeated-bind grouping and ordered repeated device tags, but is not full future Rust CLI binding type parity, ops replay parity, detected-device parity, or common-flag parity. |
| Output format compatibility | Rust emits private JSON `PlanningResult` through the shadow command. P8A passes that JSON through directly from the explicit dev-only Python CLI bridge, and P8C guards that omitted `--rust-shadow-output` and explicit `--rust-shadow-output passthrough` remain Rust stdout/stderr/exit-code passthrough. P8E adds explicit `--rust-shadow-output python-compatible` formatting for usable Rust `PlanningResult` JSON: concise summary labels mirror the visible Python CLI labels, structured YAML is produced from the Rust JSON mapping through `dump_yaml(...)`, and `--output` writes that YAML while stdout stays concise unless `--verbose` is selected. Python CLI default planning still owns the default concise summary, verbose YAML, `--output`, and exit-code behavior. P8D accepts the future default-route target: Rust must preserve the Python-owned output and exit-code contract before default planner cutover unless a separate accepted breaking-change decision says otherwise. Rust-native JSON requires a future explicit structured-output mode such as `--format json`. P8E is output-compatibility path evidence only; it is not default Rust planner routing. |
| Error and warning compatibility | Rust covers selected planner result, warning/error shape, and focused diagnostics. Full CLI stderr/stdout, detected-device profile mismatch warnings, operation replay failures, exit codes, and broader planner diagnostics are not proven. P8J intentionally does not add profile mismatch warnings, and P8L keeps `detected_device_profile_mismatch_warning_not_cut_over` blocked as a separate default-cutover blocker. |
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
  mismatch warning parity for future default Rust planner cutover. This is a
  decision-only slice. Python remains the current default/reference planner
  implementation, Rust routes remain explicit and non-default, explicit context
  support remains separate from real-device probing, and the readiness gate
  remains blocked until Rust probing and mismatch-warning evidence exists.
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
5. A future user-facing experimental Rust planner flag or route is added only for
   explicitly supported scenarios.
6. The default planner route switches to Rust only for supported scenarios with
   approved fallback/error behavior.
7. Python planner remains available as fallback/reference while unsupported
   scenarios, diagnostics, CLI output, and deletion blockers are retired.
8. Python planner tests, docs, and fixture/golden references are ported,
   retired, or converted to historical evidence.
9. Python planner code is deleted only after no user-facing, test, fixture,
   docs, or developer-tool dependency still requires it.
