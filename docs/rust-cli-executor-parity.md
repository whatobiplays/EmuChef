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
| `plan` | Python planner/CLI owns default planning result emission. | Rust planner is an internal fixture-backed module that emits Python-shaped `PlanningResult`/`ExecutionPlan` values for selected tests. P8A adds only an explicit developer bridge, `emuchef plan --planner-backend rust-shadow --rust-planner-bin <path>`, that forwards raw binds to the supplied shadow binary and passes through Rust JSON stdout/stderr. It is not exposed as a protocol request, Tauri command, production/default CLI command, Cargo fallback, or replacement planner. | Full authored catalog loading, device profile/plan behavior, draft operations, bindings, diagnostics, output YAML/text, CLI `--output`/verbose parity, and default-route ownership. | Keep planner declarative; do not add runtime behavior to planning parity work. |
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
default planner route. Existing Rust planner, executor, and CLI slices should
remain scoped as parity scaffolding until a later phase explicitly promotes or
retires the corresponding Python surface.

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
5. CLI/API compatibility is intentionally accepted or intentionally changed.

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
