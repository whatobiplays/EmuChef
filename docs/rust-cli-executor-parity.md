# Rust CLI and Executor Parity Strategy

This document records the current Python/Rust ownership and parity strategy for
CLI, planner, executor, and real-device apply behavior. It is documentation
only. It does not change command behavior, planner semantics, executor
semantics, authored schema, Tauri UI behavior, tests, fixtures, scripts, or CI.

## Non-Goals

This document does not:

1. Promote Rust CLI or executor behavior to production.
2. Retire Python CLI, planner, or executor surfaces.
3. Approve Rust real-device apply.
4. Change Tauri editor runtime ownership.
5. Add or require CI coverage.

## Current Ownership

Rust owns the active Tauri config-editor backend runtime. The Tauri editor
launches the Rust JSONL sidecar directly through `apps/config-editor/src-tauri`
and has no Python fallback, backend selector, backend toggle, environment
backend choice, or protocol negotiation path.

Python remains the production/reference owner for the CLI, planner, executor,
and real-device apply behavior. The Python `emuchef` CLI in `src/emuchef/cli.py`
continues to expose `draft`, `plan`, `detect`, `detect-profiles`, `validate`,
and `apply`.
Python also remains the compatibility reference for CLI argument behavior,
output shape, diagnostics, exit codes, and execution plan file behavior until an
explicit ownership change is documented.
For future Rust planner default routing, the accepted CLI output and exit-code
compatibility target is recorded in
`docs/adr/0002-rust-planner-cli-output-compatibility.md`.
For future Rust planner default routing, the accepted real-device context
ownership model is recorded in
`docs/adr/0003-rust-real-device-context-ownership.md`: Rust should own
real-device probing and detected-device profile mismatch warning parity before
default Rust planner cutover. P8M records that ownership decision. P8N adds
only the crate-local Rust fake/test probe foundation. P8O adds fake/test-backed
detected-facts planner-input construction. Intended future context precedence:
synthetic/profile context -> detected facts -> explicit CLI overrides. P8P adds
pure/test-backed mismatch-warning helper logic for supplied detected facts and
authored profile criteria. P8Q composes that helper into fake/test-backed
planning-result construction. P8R exposes the P8Q composition path only through
a local `emuchef-plan-shadow --detected-facts-json <path>` fixture harness.
P8S adds optional/manual smoke evidence for that direct Rust shadow-binary
fixture harness through `tools/smoke_rust_detected_facts_fixture.py`. P8T lets
only the explicit `rust-experimental` Python route forward a local fixture path
with `--rust-detected-facts-json <path>`; Python forwards the exact string to
the Rust shadow binary as `--detected-facts-json <path>` and does not load,
normalize, or validate the fixture. These slices do not change Python default
planning behavior, live probing, default Python backend fixture forwarding,
`rust-shadow` fixture forwarding, route-level detection, executor/apply,
Tauri/protocol, readiness gate behavior, normal runtime checks, or normal
planner warning emission.
P8U adds optional/manual smoke evidence for that Python
`rust-experimental` fixture-forwarding route through
`tools/smoke_rust_experimental_detected_facts_fixture.py`. The smoke expects
Python-compatible summary output, treats raw Rust JSON as a route-smoke failure,
and validates the mismatching output-file case with stdlib text checks only. It
does not change default `emuchef plan`, `rust-shadow`, direct Rust fixture smoke,
executor/apply, Tauri/protocol, live probing, normal runtime checks, readiness
gate blocker IDs, or Python planner deletion behavior.
P8V adds only a Rust-side, crate-local ADB probe foundation for future planner
probing work: `device_probe.rs` models `adb [-s SERIAL] shell getprop` as argv
and parses supplied bracketed `getprop` text into `DetectedDeviceFacts`. It does
not execute ADB, start subprocesses, read environment variables, access the
filesystem, access the network, change Python CLI behavior, expose Rust
`detect`/`detect-profiles`, wire probing into planner routes, alter
executor/apply or Tauri/protocol behavior, add normal runtime checks, or
reclassify readiness blockers.
P8W adds the crate-local live ADB probe adapter foundation on top of that model
and parser. It uses an injectable command runner, keeps production process
execution isolated to `ProcessCommandRunner`, and uses fake runners in normal
tests. P8X wires that adapter only into the direct dev-only Rust shadow binary
through explicit `--probe-adb-getprop` mode. It does not expose Rust
`detect`/`detect-profiles`, change Python CLI behavior, wire live probing into
`rust-shadow` or `rust-experimental`, alter executor/apply or Tauri/protocol
behavior, add smoke-runner probing, add readiness-gate executed evidence, or
reclassify readiness blockers.
P8Y adds optional/manual smoke evidence for that direct Rust shadow live mode
through `tools/smoke_rust_shadow_live_adb_probe.py`. The smoke requires explicit
`--rust-planner-bin`, `--adb-path`, and `--serial`, invokes only the supplied
Rust shadow binary with `--probe-adb-getprop`, and emits deterministic JSON with
scrubbed command metadata. It does not discover devices, run `adb devices`,
invoke Cargo, call Python CLI routes, wire live probing into `rust-shadow` or
`rust-experimental`, alter executor/apply or Tauri/protocol behavior, add normal
runtime checks, add readiness-gate executed evidence, or reclassify blockers.
`device_profile_mismatch` is acceptable smoke evidence when the selected device
intentionally does not match the authored plan. Production route-level probing,
production mismatch-warning parity, and Python planner deletion remain future
work.
P8Z adds explicit Python CLI forwarding only for
`emuchef plan --planner-backend rust-experimental`: `--rust-probe-adb-getprop`,
`--rust-adb-path <path>`, and `--rust-serial <serial>` are forwarded to the
supplied Rust shadow binary as `--probe-adb-getprop`, `--adb-path <path>`, and
`--serial <serial>`. Python does not invoke ADB, discover devices, run
`adb devices`, parse `getprop`, or validate, normalize, expand, or stat the ADB
path or serial. The default Python backend and `rust-shadow` reject these wrapper
flags before session construction, ADB resolution, Python planner work, or Rust
subprocess execution. Fixture forwarding with
`--rust-detected-facts-json <path>` and live-probe forwarding are mutually
exclusive detected-facts sources. P8Z does not change default `emuchef plan`,
Python planner behavior, executor/apply, Tauri/protocol, smoke runners, normal
checks, readiness-gate blockers, Cargo fallback behavior, or Python golden
ownership.
P8AA adds optional/manual smoke evidence for that Python `rust-experimental`
live-probe forwarding route through
`tools/smoke_rust_experimental_live_adb_probe.py`. The smoke invokes
`python3 -m emuchef plan --planner-backend rust-experimental` with the Python
wrapper live-probe flags, expects Python-compatible output instead of raw Rust
JSON, and treats `device_profile_mismatch` as acceptable route evidence for an
intentionally mismatched selected device. It does not call the Rust shadow
binary directly, discover devices, run `adb devices`, invoke Cargo, inspect,
normalize, expand, or stat the supplied ADB path or serial, alter
executor/apply or Tauri/protocol behavior, participate in normal checks, add
readiness-gate executed evidence, reclassify blockers, or make Python planner
deletion ready. The `real_device_probing_not_cut_over` and
`detected_device_profile_mismatch_warning_not_cut_over` blockers remain blocked.

Rust planner, executor, and CLI behavior is fixture-scoped, test-scoped,
internal, or editor-backend-scoped unless explicitly promoted by later work.
Current Rust coverage for these areas is limited to selected
tests/fixtures/internal paths. That coverage does not by itself establish
production CLI or real-device apply ownership.

Planner ownership stays declarative: it loads authored data, applies draft
operations and bindings, detects device facts where the Python CLI requires
them, and emits normalized execution-plan artifacts. Executor ownership stays
runtime-oriented: it resolves runtime refs, downloads or extracts artifacts,
evaluates conditions, executes step handlers, performs ADB/device operations,
and reports step results.

Real-device apply remains Python-owned until Rust has explicit production
CLI/API exposure, parity tests against the Python reference, and completed
manual real-device evidence.

## Command and Capability Status

| Area | Current production/reference owner | Current Rust status | Gap before Rust can claim parity | Promotion/retirement notes |
| --- | --- | --- | --- | --- |
| `validate` | Python CLI owns full catalog, directory, and path validation. | Rust supports selected explicit recipe-file validation paths and editor-backend validation fixtures. Rust CLI rejects default/catalog validation and deferred flags such as `--verbose`, `--debug`, and `--adb`. | Match Python CLI arguments, default catalog behavior, diagnostics, stdout/stderr, exit codes, authoredRoot coverage, and broader corpus behavior. | Do not treat editor validation or selected CLI fixtures as production CLI replacement. |
| `draft` / create-from-template workflows | Python planner/CLI still owns `draft` behavior. | Rust has no `draft` CLI command. The Rust sidecar implements `createRecipeFromTemplate` for backend protocol parity, but Tauri does not expose a create-from-template UI. | Implement or intentionally retire CLI draft behavior, operation replay, input binding, and device fact handling before Rust can claim CLI parity. | GUI create-from-template is retired from the normal editor path unless a future product requirement reintroduces it. |
| `plan` | Python planner/CLI owns default planning result emission, concise/verbose/`--output` behavior, detected-device context resolution, and exit-code behavior. Future default Rust planner routing must preserve that Python-owned output and exit-code contract unless a separate accepted breaking-change decision says otherwise. | Rust planner is an internal fixture-backed module that emits Python-shaped `PlanningResult`/`ExecutionPlan` values for selected tests. P8A adds an explicit developer bridge, `emuchef plan --planner-backend rust-shadow --rust-planner-bin <path>`, that forwards raw binds to the supplied shadow binary. Omitted `--rust-shadow-output` and explicit `--rust-shadow-output passthrough` preserve Rust JSON stdout/stderr/exit-code passthrough as guarded by P8C. P8E adds explicit `--rust-shadow-output python-compatible`, which formats usable Rust `PlanningResult` JSON into visible Python-compatible summary labels and structured YAML over the Rust JSON mapping for `--verbose` or `--output`. P8B adds a dev-only matrix smoke for raw passthrough route invocation. P8F adds dev-only matrix smoke for the same explicit route with `--rust-shadow-output python-compatible`; successful scenarios require concise Python-compatible summary stdout. P8G adds `emuchef plan --planner-backend rust-experimental --rust-planner-bin <path>` as an explicit non-default migration route that reuses the same Rust shadow planner invocation and defaults to Python-compatible output formatting. P8H adds dev-only matrix smoke for that explicit route; generated commands omit `--rust-shadow-output`, and successful scenarios require exit `0` plus `python_summary` stdout. P8I adds `tools/check_rust_planner_cutover_readiness.py`, a stdlib-only static readiness gate for future default Rust planner proposals. It checks durable static prerequisites, derives checked-in device-plan coverage from filenames, lists required manual evidence commands, and keeps the report status `blocked` until default-cutover blockers are intentionally cleared. P8J adds explicit device context support to the explicit Rust routes: supplied `--manufacturer`, `--model`, `--android-version`, and repeated `--device-tag` values are forwarded to `emuchef-plan-shadow` and applied to the private Rust planner context. P8K adds optional explicit `device_context` data to the dev-only scenario matrix and forwards it through comparison and smoke tooling while reporting only stable context presence/key metadata. P8L classifies explicit-context readiness coverage in the static gate separately from optional `device_context` schema validity. P8M records that future default Rust planner cutover should use Rust-owned real-device probing and detected-device mismatch-warning parity. P8N adds fake probe/context layering, P8O adds fake/test-backed detected-facts planner input construction, P8P adds pure mismatch-warning helper logic, and P8Q composes those pieces into crate-private fake/test-backed `PlanningResult` construction. P8Q does not add ADB probing, route warning emission, `rust-shadow` or `rust-experimental` behavior, Python CLI behavior, Tauri/protocol behavior, executor/apply behavior, readiness gate reclassification, or normal runtime checks. `rust-experimental` is a cutover rehearsal route; its name and behavior may change before Rust becomes the default planner backend. It is not a stable final public contract, not Python planner deletion, and not a default planner route. P8D records that Rust-native JSON requires a future explicit structured-output mode such as `--format json`. None of these paths is exposed as a protocol request, Tauri command, production/default CLI command, Cargo fallback, or replacement planner. | Full authored catalog loading, device profile/plan behavior, detected-device probing/profile mismatch warnings, draft operations, bindings, diagnostics, complete output/exit-code parity, default-route ownership, and Python planner deletion readiness. | Keep planner declarative; do not add runtime behavior to planning parity work. P8E is an explicit formatter bridge only, P8F is compatibility-mode route/output smoke only, P8G is a non-default migration route only, P8H is dev-only route/output smoke evidence only, P8I is a static/advisory gate only, P8J is explicit context input support only, P8K is explicit-context matrix evidence only, P8L is static readiness reclassification only, and P8Q is crate-private fake/test-backed result composition only. |
| `apply` | Python CLI and executor own dry-run and real execution. | Rust CLI supports only selected non-verbose `apply --plan-file ... --dry-run` fixtures. Non-dry-run apply is refused. | Full execution plan IO, dry-run semantics, real executor behavior, step handlers, artifact behavior, device selection, ADB, stdout/stderr, exit codes, and failure propagation. | Selected dry-run fixture behavior is not production apply parity. |
| `detect` | Python CLI owns ADB device fact detection. | No Rust production CLI command or Tauri/API replacement. | ADB executable resolution, selected serial handling, device fact parsing, error behavior, stdout/stderr, and exit code parity. | Do not expose real-device behavior without explicit device targeting rules. |
| `detect-profiles` | Python CLI owns ADB detection plus authored profile matching. | No Rust production CLI command or Tauri/API replacement. | Authored catalog loading, device detection, profile matching, summaries, verbose output, and diagnostics. | Promotion requires parity for both device probing and profile matching. |
| Artifact download/cache | Python executor owns production artifact resolution with Python stdlib networking and cache/runtime paths. | Rust executor has selected sandboxed/file-url and pre-cached fixture behavior. Network downloads are disabled in the Rust executor fixture path. | Network download behavior, TLS/error semantics, cache naming, cache hit behavior, runtime path behavior, and failure diagnostics. | Keep Python as owner until network/cache parity is proven outside temp-root fixtures. |
| Archive extraction | Python executor owns host and device extraction behavior. | Rust supports selected host ZIP extraction fixtures under explicit sandbox roots. Device extraction is rejected as out of scope. | Host/device extraction parity, cleanup behavior, path safety decisions, output shape, and failure diagnostics. | Any safety-hardening differences must be documented before production promotion. |
| ADB install/copy/launch | Python executor and ADB abstraction own production install, push/copy, launch, force-stop, path checks, and app-private command handling. | Rust has fake dry-run behavior, crate-private real-ADB adapter foundations, and ignored/manual tests. No production real apply surface uses them. | Real ADB command parity, serial selection, app-private path behavior, copy policies, install/launch/force-stop failure handling, and manual evidence. | Fake/manual/internal foundations do not equal production parity. |
| Permission/appops | Python `grant_permissions` step handler owns step-local runtime permission and appops behavior. | Rust has selected fake/manual/internal coverage for step-local permission/appops behavior. | Command parity, policy parity, `when` filters, API/root behavior, optional failure behavior, summaries, dependency blocking, and real-device evidence. | Permission intent stays on `grant_permissions` steps; do not revive a planner-owned runtime permission phase. |
| Real-device apply | Python CLI/executor owns mutating apply against selected devices. | Rust real-ADB tests are ignored/manual and require explicit environment opt-ins. Rust CLI refuses non-dry-run apply. | Explicit production CLI/API surface, mandatory selected-device targeting, all mutating step behavior, safety gates, and completed manual matrix records. | Rust real-device apply requires a deliberate product decision before promotion. |
| Fixture/golden generation | Python remains the dev-only/reference-only generator owner for selected planner and executor goldens. StepSpec DTO metadata is Rust-owned for the normal Tauri editor path. | Rust normal tests consume checked-in fixtures and goldens without Python regeneration. Concrete groups, paths, consumers, commands, and classifications are tracked in `docs/python-fixture-golden-ownership.md`. | Replace generation with Rust-native tooling, freeze fixtures intentionally, or keep a documented generator-only Python owner. | Deleting Python requires following the ownership document's retirement criteria. |
| Stdout/stderr/exit-code behavior | Python CLI owns broad user-facing CLI output behavior. | Rust checks selected `validate` and `apply --dry-run` output shapes and exit codes in crate-local tests. | Full command matrix parity, verbose/debug modes, error text semantics, usage errors, progress output, summaries, and stderr failure markers. | Output compatibility must be intentionally accepted or intentionally changed before ownership changes. |
| Execution plan IO compatibility | Python `src/emuchef/io/execution_plan_io.py` owns production plan-file loading. | Rust accepts selected `kind: execution_plan` and `kind: planning_result` fixtures for dry-run CLI tests. Broader inputs/artifacts and unsupported shapes are deferred. | YAML loading breadth, schema rejection behavior, planner-only field rejection, runtime value/ref compatibility, diagnostics, and fixture coverage. | Plan IO parity is required before Rust can replace Python `apply`. |

## Near-Term Recommendation

Keep Rust focused on the active Tauri editor backend runtime. That path is
already Rust-sidecar-only and should remain free of Python fallback behavior.

Keep Python as the production/reference CLI, planner, executor, and real-device
apply implementation. The P8A `rust-shadow` planner bridge is explicit,
developer-only, requires a supplied shadow binary, and is not a Cargo fallback or
default planner route. The P8B matrix smoke is developer-only route-invocation
evidence for that bridge and is not Python-vs-Rust planner-output comparison
evidence, not a normal Rust/Tauri runtime check, and not a default planner route.
The P8C contract keeps omitted `--rust-shadow-output` and explicit
`--rust-shadow-output passthrough` as Rust stdout/stderr/exit-code passthrough.
P8E adds explicit `--rust-shadow-output python-compatible` for formatter bridge
work: usable Rust `PlanningResult` JSON is formatted into visible
Python-compatible summary labels and structured YAML over the Rust JSON mapping.
This mode is not default routing and does not reconstruct full Python planner
domain objects. P8F adds a dev-only matrix smoke for the explicit
`python-compatible` mode across the same current scenario matrix; successful
scenarios must emit concise Python-compatible summary stdout, while raw Rust JSON
remains a compatibility-smoke failure. P8D records the future default-route
compatibility decision in
`docs/adr/0002-rust-planner-cli-output-compatibility.md`: if Rust later becomes
the default planner backend for `emuchef plan`, default output and exit-code
behavior must remain compatible with the current Python-owned contract unless a
separate accepted breaking-change decision says otherwise. Rust-native JSON
requires a future explicit structured-output mode such as `--format json`.
P8G adds `rust-experimental` as an explicit non-default migration route that
reuses the Rust shadow planner invocation and defaults to Python-compatible
output. Its name and behavior may change before Rust becomes the default planner
backend. It is not the default planner, not a stable final public contract, and
not Python planner deletion. P8H adds dev-only matrix smoke evidence for that
route across the current scenario matrix; it requires successful scenarios to
exit `0` and emit concise Python-compatible summary stdout, and raw Rust JSON is
a smoke failure for successful scenarios. P8T adds
`--rust-detected-facts-json <path>` only to this explicit route and forwards it
unchanged as `--detected-facts-json <path>` to the Rust shadow binary. The raw
Rust flag remains unrecognized by the Python CLI, and the Python backend and
`rust-shadow` reject the Python wrapper flag before planner or subprocess work.
P8U adds optional/manual smoke evidence for that Python fixture-forwarding
route. It invokes `python3 -m emuchef plan --planner-backend rust-experimental`
with temporary `--rust-detected-facts-json` fixtures, requires concise
Python-compatible summary stdout, and does not add live probing, normal checks,
readiness-gate execution, default-route behavior, or `rust-shadow` fixture
forwarding.
P8Z adds `--rust-probe-adb-getprop`, `--rust-adb-path <path>`, and
`--rust-serial <serial>` only to the same explicit `rust-experimental` route; it
forwards them as raw Rust shadow live-probe flags and keeps fixture and live
detected-facts sources mutually exclusive. Python still does not invoke or parse
ADB, and the default Python backend plus `rust-shadow` reject the wrapper flags.
P8I adds a static readiness report for
future default-cutover PRs; it lists required manual evidence but does not run
comparison/smoke tooling, Cargo, npm, ADB, executor/apply, Tauri/protocol,
network, artifact, or golden-regeneration checks, and it remains `blocked` while
default-route, real-device probing, detected-device profile mismatch warning
parity, executor/apply, and Python planner deletion blockers remain unresolved.
P8J adds explicit device context flags to the
explicit Rust routes only. Supplied manufacturer, model, Android version, and
device tags are forwarded to the shadow command; no synthetic/profile-derived
values are forwarded by Python, no ADB/device probing is added, and
detected-device profile mismatch warnings remain unsupported.
P8W adds only the crate-local adapter foundation that can execute the modeled
getprop argv through an injected runner. P8X wires it only into the direct
dev-only Rust shadow binary with `--probe-adb-getprop`; this does not make Rust
the production owner of `detect`, `detect-profiles`, Python CLI route context
resolution, or default planner routing.
P8Y adds only optional/manual smoke evidence for that direct shadow live path.
The smoke requires an explicit selected serial and does not discover devices,
run `adb devices`, invoke Cargo, call Python CLI routes, participate in normal
checks, or count as readiness-gate executed evidence.
P8Z lets only the explicit `rust-experimental` Python route forward live-probe
wrapper flags to the supplied Rust shadow binary and keeps Python out of ADB
execution, device discovery, `getprop` parsing, and path/serial normalization.
The default Python backend and `rust-shadow` reject those wrapper flags, and
readiness blockers remain blocked.
P8K extends only the dev-only matrix evidence: optional scenario
`device_context` values are validated, forwarded to both comparison sides and to
smoke-runner CLI commands, and reported only as stable presence/key metadata.
Empty `device_tags` lists are rejected because omitted tags already mean no tag
override in the existing P8J flag surface.
P8L reclassifies that evidence in the static readiness gate: optional
`device_context` schema validity is separate from explicit-context readiness
coverage, and at least one valid scenario with a meaningful supplied context
field is required for the coverage checks to pass. This does not add
real-device probing, detected-device profile mismatch warning behavior,
executor/apply behavior, Tauri/protocol behavior, or default Rust planner
ownership.

P8M records an accepted ADR for future real-device context ownership:
`docs/adr/0003-rust-real-device-context-ownership.md` says Rust should own
real-device probing and detected-device profile mismatch warning parity for
future default Rust planner cutover. Python remains the current default
CLI/reference planner owner, Rust routes remain explicit and non-default, and
real-device probing plus mismatch-warning parity remain blocked until
implemented and evidenced. P8N adds only the Rust crate-local probe abstraction,
fake probe, and context layering helper. P8P adds only pure/test-backed
mismatch-warning helper logic. P8Q adds fake/test-backed result composition, and
P8R makes it executable only through a local Rust shadow-binary fixture file.
P8X wires live probing only into the direct Rust shadow binary. These slices do
not wire live probing or production mismatch warnings into default Python
planning, `rust-shadow`, Tauri/protocol, executor/apply, normal runtime checks,
or the readiness gate. P8Z adds only `rust-experimental` wrapper forwarding for
that direct Rust shadow live mode and does not make Python the owner of ADB
probing or mismatch-warning parity.
Existing Rust planner, executor, and CLI slices should remain scoped as parity
scaffolding until a later phase explicitly promotes or retires the corresponding
Python surface.

Treat Rust executor and CLI parity as separately scoped future work. Fixture
coverage must not silently become production ownership.

## Port, Retire, and Ownership Criteria

Rust can be considered for production replacement only after explicit parity
exists for:

1. CLI command surface and argument names.
2. Default behavior.
3. Exit codes.
4. Stdout/stderr behavior.
5. Diagnostics and error message semantics.
6. Planner emission.
7. Execution plan IO.
8. Step handlers.
9. Artifact download, network, and cache behavior.
10. Archive extraction behavior.
11. ADB install, copy, and launch behavior.
12. Permission and appops behavior.
13. Dry-run semantics.
14. Device mutation safety.
15. Fixture/golden generation strategy.
16. Python reference comparison tests.
17. Real-device/manual evidence for mutating apply behavior.

Do not retire any of these surfaces without an explicit product decision and
replacement evidence:

1. Create-from-template flows.
2. Python CLI commands.
3. Python planner.
4. Python executor.
5. Fixture/golden generation.
6. Real-device apply surface.

Ownership may change only after:

1. Tests prove parity against the Python reference.
2. Documentation records the ownership change.
3. Mutating real-device behavior has manual evidence.
4. Failure behavior and diagnostics are documented.
5. CLI/API compatibility is intentionally accepted or intentionally changed. For
   default Rust planner routing, the accepted default is compatibility with the
   current Python `emuchef plan` output and exit-code contract. For real-device
   context, the accepted future default-cutover model is Rust-owned probing and
   detected-device profile mismatch warning parity.

## Rust Real-Device Apply Promotion Checklist

Rust real-device apply must not be considered production-ready until there is
evidence for:

1. Explicit CLI/API surface intended for real apply.
2. Mandatory selected-device targeting.
3. No implicit/default ADB device mutation.
4. Full artifact download/cache behavior.
5. Archive extraction behavior.
6. Install/copy/launch behavior.
7. Permission/appops behavior.
8. App-private path behavior.
9. Dry-run versus mutating behavior.
10. Failure recovery and blocked downstream behavior.
11. Stdout/stderr/exit-code behavior.
12. Completed manual real-device matrix result records.

The existing manual real-device checklist at
`docs/manual/real-device-retroarch-matrix.md` is an operator checklist until
filled result records, logs, and artifacts are attached to a specific validation
run.
